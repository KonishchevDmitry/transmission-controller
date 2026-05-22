use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use time::{OffsetDateTime, Duration};

use crate::common::{EmptyResult, GenericResult};
use crate::consumer::Consumer;
use crate::email::{Mailer, EmailTemplate};
use crate::transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};
use crate::util;
use crate::util::time::{WeekPeriods, Timestamp};

pub struct Controller {
    action: Option<Action>,
    action_periods: WeekPeriods,

    download_dir: PathBuf,
    free_space_threshold: Option<u8>,

    upload_ratio_limit: Option<f64>,
    seed_time_limit: Option<util::time::Duration>,
    redownload_period: Option<util::time::Duration>,

    client: Arc<TransmissionClient>,
    consumer: Consumer,

    manual_time: Option<Instant>,
}

#[derive(Debug, PartialEq)]
enum State {
    Active,
    Paused,
    SoftManual,
    HardManual,
}

#[derive(Copy, Clone)]
pub enum Action {
    StartOrPause,
    PauseOrStart,
}

impl Controller {
    pub fn new(
        client: TransmissionClient, action: Option<Action>, action_periods: WeekPeriods,
        download_dir: PathBuf, copy_to: Option<PathBuf>, move_to: Option<PathBuf>,
        seed_time_limit: Option<util::time::Duration>, upload_ratio_limit: Option<f64>,
        free_space_threshold: Option<u8>, redownload_period: Option<util::time::Duration>,
        notifications_mailer: Option<Mailer>, torrent_downloaded_email_template: EmailTemplate,
    ) -> Controller {
        let client = Arc::new(client);

        Controller {
            action, action_periods,

            download_dir, free_space_threshold,
            upload_ratio_limit, seed_time_limit, redownload_period,

            client: client.clone(),
            consumer: Consumer::new(client, copy_to, move_to, notifications_mailer, torrent_downloaded_email_template),

            manual_time: None,
        }
    }

    pub fn control(&mut self) -> Result<()> {
        let state = self.calculate_state()?;
        debug!("Transmission daemon should be in {state:?} state.");

        // Be careful here: we should get snapshot of current torrent status in exactly the
        // following order to not get into data race.
        let consuming_torrents = self.consumer.get_in_process();
        let torrents = self.client.get_torrents()?;

        let mut removable_torrents = Vec::new();

        for torrent in torrents {
            debug!("Checking {:?} torrent...", torrent.name);

            match torrent.status {
                TorrentStatus::Paused if state == State::Active => {
                    info!("Resuming {:?} torrent...", torrent.name);
                    self.client.start(&torrent.hash)?;
                },

                TorrentStatus::LostData if self.redownload_period.is_some() && state != State::HardManual => {
                    info!("Retrying redownloading of {:?} torrent...", torrent.name);
                    self.redownload_torrent(&torrent.hash)?;
                },

                TorrentStatus::DownloadWait | TorrentStatus::Downloading |
                TorrentStatus::SeedWait | TorrentStatus::Seeding if state == State::Paused => {
                    info!("Pausing {:?} torrent...", torrent.name);
                    self.client.stop(&torrent.hash)?;
                },

                _ => {},
            }

            if !torrent.done || consuming_torrents.contains(&torrent.hash) {
                continue;
            }

            if !torrent.processed {
                info!("{:?} torrent has been downloaded.", torrent.name);
                self.consumer.consume(&torrent.hash);
                continue;
            }

            match (torrent.upload_ratio, self.upload_ratio_limit) {
                (Some(ratio), Some(limit)) if ratio >= limit => {
                    info!("{:?} torrent has seeded above upload ratio limit. Deleting it...", torrent.name);
                    self.client.remove(&torrent.hash)?;
                    continue;
                },
                _ => {},
            }

            if let Some(seed_time_limit) = self.seed_time_limit {
                if OffsetDateTime::now_utc().unix_timestamp() - torrent.done_time.unwrap() >= seed_time_limit {
                    info!("{:?} torrent has seeded enough time to delete it. Deleting it...", torrent.name);
                    self.client.remove(&torrent.hash)?;
                    continue;
                }
            }

            // XXX(konishchev): Randomize
            if let Some(redownload_period) = self.redownload_period && state != State::HardManual {
                if OffsetDateTime::now_utc().unix_timestamp() - torrent.done_time.unwrap() >= redownload_period {
                    info!("{:?} torrent has seeded enough time to redownload it. Redownloading it...", torrent.name);
                    self.redownload_torrent(&torrent.hash)?;
                    continue;
                }
            }

            removable_torrents.push(torrent);
        }

        if let Err(e) = self.cleanup_fs(&removable_torrents) {
            error!("Failed to cleanup the download directory: {e}.")
        }

        Ok(())
    }

    fn calculate_state(&mut self) -> Result<State> {
        if self.client.is_manual_mode()? {
            if let Some(manual_time) = self.manual_time {
                if manual_time.elapsed() < Duration::days(1) {
                    return Ok(State::HardManual);
                }

                error!("Reset outdated manual mode.");
                self.client.set_manual_mode(false)?;
            } else {
                self.manual_time = Some(Instant::now());
                return Ok(State::HardManual);
            }
        }

        self.manual_time = None;

        Ok(match self.action {
            None => State::SoftManual,
            Some(Action::StartOrPause) => {
                if util::time::is_now_in(&self.action_periods) {
                    State::Active
                } else {
                    State::Paused
                }
            }
            Some(Action::PauseOrStart) => {
                if util::time::is_now_in(&self.action_periods) {
                    State::Paused
                } else {
                    State::Active
                }
            }
        })
    }

    fn redownload_torrent(&self, hash: &str) -> Result<()> {
        let torrent = self.client.get_torrent(hash)?;

        let download_dir_path = Path::new(&torrent.download_dir);
        if !download_dir_path.is_absolute() {
            return Err!("Torrent's download directory is not an absolute path: {:?}", torrent.download_dir);
        }

        let mut deleted = false;

        for file in torrent.files.as_ref().unwrap() {
            let (_file_root_path, file_path, _file_name) = util::fs::validate_torrent_file_name(&file.name)?;

            let download_path = download_dir_path.join(&file_path);
            debug!("Deleting {download_path:?}...");

            match fs::remove_file(&download_path) {
                Ok(_) => deleted = true,
                Err(err) => if err.kind() == ErrorKind::NotFound {
                    debug!("{download_path:?} doesn't exist.");
                } else {
                    error!("Failed to delete {download_path:?}: {err}.");
                },
            }
        }

        if deleted {
            self.client.verify(hash)?;
            self.client.start(hash)?;
        }

        Ok(())
    }

    fn cleanup_fs(&self, torrents: &[Torrent]) -> EmptyResult {
        if torrents.is_empty() || self.check_free_space()? {
            return Ok(());
        }

        let mut torrents: Vec<_> = torrents.iter()
            .filter(|&torrent| Path::new(&torrent.download_dir) == self.download_dir.as_path())
            .collect();

        torrents.sort_by(|a, b| {
            let a = a.done_time.unwrap_or(Timestamp::MIN);
            let b = b.done_time.unwrap_or(Timestamp::MAX);
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

#[allow(clippy::to_string_trait_impl)]
impl ToString for Action {
    fn to_string(&self) -> String {
        use self::Action::*;

        s!(match *self {
            StartOrPause => "start-or-pause",
            PauseOrStart => "pause-or-start",
        })
    }
}
