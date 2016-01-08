use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use time;

use common::{EmptyResult, GenericResult};
use consumer::Consumer;
use email::{Mailer, EmailTemplate};
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};
use util;
use util::time::{Duration, WeekPeriods};

pub struct Controller {
    state: State,

    action: Option<Action>,
    action_periods: WeekPeriods,

    download_dir: PathBuf,
    copy_to: Option<PathBuf>,
    move_to: Option<PathBuf>,

    seed_time_limit: Option<Duration>,
    free_space_threshold: Option<u8>,

    notifications_mailer: Option<Mailer>,
    torrent_downloaded_email_template: EmailTemplate,

    client: Arc<TransmissionClient>,
    consumer: Arc<Consumer>,
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
               seed_time_limit: Option<Duration>, free_space_threshold: Option<u8>,
               notifications_mailer: Option<Mailer>, torrent_downloaded_email_template: EmailTemplate) -> Controller {
        let client = Arc::new(client);

        Controller {
            state: State::Manual,

            action: action,
            action_periods: action_periods,

            download_dir: download_dir,
            copy_to: copy_to.clone(), // FIXME
            move_to: move_to.clone(), // FIXME

            seed_time_limit: seed_time_limit,
            free_space_threshold: free_space_threshold,

            notifications_mailer: notifications_mailer,
            torrent_downloaded_email_template: torrent_downloaded_email_template,

            client: client.clone(),
            consumer: Consumer::new(client),
        }
    }

    pub fn control(&mut self) -> EmptyResult {
        self.state = try!(self.calculate_state());
        debug!("Transmission daemon should be in {:?} state.", self.state);

        let removable_torrents = try!(self.control_torrents());

        // FIXME: move right after copy - only check here
        if let (Some(copy_to), Some(move_to)) = (self.copy_to.as_ref(), self.move_to.as_ref()) {
            try!(move_copied_torrents(copy_to, move_to).map_err(|e| format!(
                "Failed to move copied torrents: {}", e)));
        }

        try!(self.cleanup_fs(&removable_torrents));

        Ok(())
    }

    fn calculate_state(&self) -> GenericResult<State> {
        if self.action.is_none() || try!(self.client.is_manual_mode()){
            return Ok(State::Manual);
        }

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

    fn control_torrents(&self) -> GenericResult<Vec<Torrent>> {
        let torrents = try!(self.client.get_torrents());
        let mut removable_torrents = Vec::new();

        let consuming_torrents = self.consumer.get_in_process();

        for torrent in torrents {
            debug!("Checking '{}' torrent...", torrent.name);

            if torrent.status == TorrentStatus::Paused && self.state == State::Active {
                info!("Resuming '{}' torrent...", torrent.name);
                try!(self.client.start(&torrent.hash));
            } else if torrent.status != TorrentStatus::Paused && self.state == State::Paused {
                info!("Pausing '{}' torrent...", torrent.name);
                try!(self.client.stop(&torrent.hash));
            }

            // FIXME: done_time is always zero
            if torrent.done_time == 0 || consuming_torrents.contains(&torrent.hash) {
                continue;
            }

            // FIXME
            if false && torrent.processed {
                if let Some(ref seed_time_limit) = self.seed_time_limit {
                    if time::get_time().sec - torrent.done_time >= *seed_time_limit {
                        info!("'{}' torrent has seeded enough time to delete it. Deleting it...", torrent.name);
                        try!(self.client.remove(&torrent.hash));
                        continue;
                    }
                }

                removable_torrents.push(torrent);
            } else {
                if true {
                    info!("'{}' torrent has been downloaded.", torrent.name);
                    self.consumer.consume(&torrent.hash);
                } else {
                    try!(self.torrent_downloaded(&torrent));
                }
                continue;
            }
        }

        Ok(removable_torrents)
    }

    fn torrent_downloaded(&self, torrent: &Torrent) -> EmptyResult {
        info!("'{}' torrent has been downloaded.", torrent.name);

        if let Some(ref copy_to) = self.copy_to {
            try!(copy_torrent(&self.client, &torrent, &copy_to).map_err(|e| format!(
                "Failed to copy '{}' torrent: {}", torrent.name, e)));
        }

        try!(self.client.set_processed(&torrent.hash));

        if let Some(ref mailer) = self.notifications_mailer {
            let mut params = HashMap::new();
            params.insert("name", torrent.name.clone());

            if let Err(e) = self.torrent_downloaded_email_template.send(mailer, &params) {
                error!("Failed to send 'torrent downloaded' notification for '{}' torrent: {}.",
                    torrent.name, e);
            }
        }

        Ok(())
    }

    fn cleanup_fs(&self, torrents: &Vec<Torrent>) -> EmptyResult {
        if torrents.len() == 0 || try!(self.check_free_space()) {
            return Ok(());
        }

        let mut torrents: Vec<_> = torrents.iter()
            .filter(|&torrent| Path::new(&torrent.download_dir) == self.download_dir.as_path())
            .collect();
        torrents.sort_by(|a, b| a.done_time.cmp(&b.done_time));

        for (id, torrent) in torrents.iter().enumerate() {
            info!("Removing '{}' torrent to get a free space on the disk...", torrent.name);
            try!(self.client.remove(&torrent.hash));

            if id == torrents.len() - 1 || try!(self.check_free_space()) {
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

        let (device, usage) = try!(util::fs::get_device_usage(&self.download_dir));

        let free_space = 100 - usage;
        let needs_cleanup = free_space <= free_space_threshold;

        if needs_cleanup {
            info!("We don't have enough free space on {}: {}% vs allowed > {}%.",
                device, free_space, free_space_threshold)
        }

        Ok(!needs_cleanup)
    }
}

fn copy_torrent<P: AsRef<Path>>(client: &TransmissionClient, torrent: &Torrent, destination: P) -> EmptyResult {
    let destination = destination.as_ref();

    let download_dir_path = Path::new(&torrent.download_dir);
    if !download_dir_path.is_absolute() {
        return Err!("Torrent's download directory is not an absolute path: {}",
            torrent.download_dir)
    }

    let files = try!(client.get_torrent(&torrent.hash)).files.unwrap();

    info!("Copying '{}' to '{}'...", torrent.name, destination.display());

    for file in files.iter().filter(|file| file.selected) {
        let file_path = try!(validate_torrent_file_name(&file.name));
        let src_path = download_dir_path.join(&file_path);
        let dst_path = destination.join(&file_path);

        debug!("Copying '{}'...", src_path.display());

        if let Some(file_dir_path) = file_path.parent() {
            try!(util::fs::create_all_dirs_from_base(&destination, &file_dir_path));
        }

        try!(util::fs::copy_file(&src_path, &dst_path));
    }

    Ok(())
}

fn move_copied_torrents<S: AsRef<Path>, D: AsRef<Path>>(src: S, dst: D) -> EmptyResult {
    let map_src_dir_error = |e| format!(
        "Error while reading '{}' directory: {}", src.as_ref().display(), e);

    let src_dir = try!(fs::read_dir(&src).map_err(&map_src_dir_error));

    for entry in src_dir {
        let entry = try!(entry.map_err(&map_src_dir_error));
        try!(move_copied_torrent(&entry.path(), &entry.file_name(), &dst));
    }

    Ok(())
}

fn move_copied_torrent<S, N, D>(src: S, src_name: N, dst_dir: D) -> EmptyResult
                                where S: AsRef<Path>, N: AsRef<OsStr>, D: AsRef<Path> {
    let (src, src_name, dst_dir) = (src.as_ref(), src_name.as_ref(), dst_dir.as_ref());

    for id in 0..10 {
        let mut dst_file_name = OsString::new();
        if id != 0 {
            dst_file_name.push(&format!("DUP_{}.", id));
        }
        dst_file_name.push(src_name);

        let dst = dst_dir.join(dst_file_name);

        match fs::metadata(&dst) {
            Ok(_) => continue,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => {},
                _ => return Err!("Failed to stat() '{}': {}", dst.display(), err)
            }
        }

        info!("Moving '{}' to '{}'...", src.display(), dst.display());
        try!(fs::rename(&src, &dst).map_err(|e| format!(
            "Failed to rename '{}' to '{}': {}", src.display(), dst.display(), e)));

        return Ok(());
    }

    Err!("Failed to move '{}' to '{}': the file is already exists",
        src.display(), dst_dir.display())
}

fn validate_torrent_file_name(file_name: &str) -> GenericResult<PathBuf> {
    use std::path::Component::*;

    let mut empty = true;
    let mut file_path = PathBuf::new();

    for component in Path::new(file_name).components() {
        match component {
            Normal(component) => {
                file_path.push(component);
                empty = false;
            },
            Prefix(_) | RootDir | CurDir | ParentDir => {
                return Err!("Invalid torrent file name: '{}'", file_name);
            }
        }
    }

    if empty {
        return Err!("Invalid torrent file name: '{}'", file_name);
    }

    Ok(file_path)
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
