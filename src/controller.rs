use std::path::{Path, PathBuf};
use std::sync::Arc;

use time;
use time::{SteadyTime, Duration};
use transmissionrpc;

use common::{EmptyResult, GenericResult};
use consumer::Consumer;
use email::{Mailer, EmailTemplate};
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};
use util;
use util::time::{WeekPeriods, Timestamp};

pub struct Controller {
    action: Option<Action>,
    action_periods: WeekPeriods,

    download_dir: PathBuf,
    free_space_threshold: Option<u8>,
    seed_time_limit: Option<util::time::Duration>,

    client: Arc<TransmissionClient>,
    consumer: Consumer,

    manual_time: Option<SteadyTime>,
}

#[derive(Debug, PartialEq)]
enum State {
    Active,
    Paused,
    Manual,
}

#[derive(Copy, Clone)]
pub enum Action {
    StartOrPause,
    PauseOrStart,
}

impl Controller {
    pub fn new(client: TransmissionClient,
               action: Option<Action>, action_periods: WeekPeriods,
               download_dir: PathBuf, copy_to: Option<PathBuf>, move_to: Option<PathBuf>,
               seed_time_limit: Option<util::time::Duration>, free_space_threshold: Option<u8>,
               notifications_mailer: Option<Mailer>, torrent_downloaded_email_template: EmailTemplate) -> Controller {
        let client = Arc::new(client);

        Controller {
            action: action,
            action_periods: action_periods,

            download_dir: download_dir,
            free_space_threshold: free_space_threshold,
            seed_time_limit: seed_time_limit,

            client: client.clone(),
            consumer: Consumer::new(client, copy_to, move_to, notifications_mailer, torrent_downloaded_email_template),

            manual_time: None,
        }
    }

    pub fn control(&mut self) -> transmissionrpc::EmptyResult {
        let state = self.calculate_state()?;
        debug!("Transmission daemon should be in {:?} state.", state);

        // Be careful here: we should get snapshot of current torrent status in exactly the
        // following order to not get into data race.
        let consuming_torrents = self.consumer.get_in_process();
        let torrents = self.client.get_torrents()?;

        let mut removable_torrents = Vec::new();

        for torrent in torrents {
            debug!("Checking '{}' torrent...", torrent.name);

            if torrent.status == TorrentStatus::Paused && state == State::Active {
                info!("Resuming '{}' torrent...", torrent.name);
                self.client.start(&torrent.hash)?;
            } else if torrent.status != TorrentStatus::Paused && state == State::Paused {
                info!("Pausing '{}' torrent...", torrent.name);
                self.client.stop(&torrent.hash)?;
            }

            if !torrent.done || consuming_torrents.contains(&torrent.hash) {
                continue;
            }

            if !torrent.processed {
                info!("'{}' torrent has been downloaded.", torrent.name);
                self.consumer.consume(&torrent.hash);
                continue;
            }

            if let Some(ref seed_time_limit) = self.seed_time_limit {
                if time::get_time().sec - torrent.done_time.unwrap() >= *seed_time_limit {
                    info!("'{}' torrent has seeded enough time to delete it. Deleting it...", torrent.name);
                    self.client.remove(&torrent.hash)?;
                    continue;
                }
            }

            removable_torrents.push(torrent);
        }

        if let Err(e) = self.cleanup_fs(&removable_torrents) {
            error!("Failed to cleanup the download directory: {}.", e)
        }

        Ok(())
    }

    fn calculate_state(&mut self) -> transmissionrpc::Result<State> {
        if self.action.is_none() {
            return Ok(State::Manual);
        }

        if self.client.is_manual_mode()? {
            if let Some(manual_time) = self.manual_time {
                if SteadyTime::now() - manual_time < Duration::days(1) {
                    return Ok(State::Manual);
                }

                error!("Reset outdated manual mode.");
                self.client.set_manual_mode(false)?;
            } else {
                self.manual_time = Some(SteadyTime::now());
                return Ok(State::Manual);
            }
        }

        self.manual_time = None;

        Ok(match self.action.unwrap() {
            Action::StartOrPause => {
                if util::time::is_now_in(&self.action_periods) {
                    State::Active
                } else {
                    State::Paused
                }
            }
            Action::PauseOrStart => {
                if util::time::is_now_in(&self.action_periods) {
                    State::Paused
                } else {
                    State::Active
                }
            }
        })
    }

    fn cleanup_fs(&self, torrents: &[Torrent]) -> EmptyResult {
        if torrents.is_empty() || self.check_free_space()? {
            return Ok(());
        }

        let mut torrents: Vec<_> = torrents.iter()
            .filter(|&torrent| Path::new(&torrent.download_dir) == self.download_dir.as_path())
            .collect();

        torrents.sort_by(|a, b| {
            let a = a.done_time.unwrap_or(Timestamp::max_value());
            let b = b.done_time.unwrap_or(Timestamp::max_value());
            a.cmp(&b)
        });

        for (id, torrent) in torrents.iter().enumerate() {
            info!("Removing '{}' torrent to get a free space on the disk...", torrent.name);
            self.client.remove(&torrent.hash)?;

            if id == torrents.len() - 1 || self.check_free_space()? {
                break;
            }
        }

        Ok(())
    }

    fn check_free_space(&self) -> GenericResult<bool> {
        let free_space_threshold = match self.free_space_threshold {
            Some(value) => value,
            None => return Ok(true),
        };

        let (device, usage) = util::fs::get_device_usage(&self.download_dir)?;

        let free_space = 100 - usage;
        let needs_cleanup = free_space <= free_space_threshold;

        if needs_cleanup {
            info!("We don't have enough free space on {}: {}% vs allowed > {}%.",
                device, free_space, free_space_threshold)
        }

        Ok(!needs_cleanup)
    }
}

impl ToString for Action {
    fn to_string(&self) -> String {
        use self::Action::*;

        s!(match *self {
            StartOrPause => "start-or-pause",
            PauseOrStart => "pause-or-start",
        })
    }
}
