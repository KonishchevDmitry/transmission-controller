use std;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use common::GenericResult;
use email::{Mailer, EmailTemplate};
use fs;
use periods;
use periods::WeekPeriods;
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};

pub struct Controller {
    state: State,
    client: TransmissionClient,

    action: Option<Action>,
    action_periods: WeekPeriods,

    download_dir: String,
    copy_to: Option<PathBuf>,
    move_to: Option<PathBuf>,

    free_space_threshold: Option<u8>,

    notifications_mailer: Option<Mailer>,
    torrent_downloaded_email_template: EmailTemplate,
}

#[derive(Copy, Clone)]
pub enum Action {
    StartOrPause,
    PauseOrStart,
}

#[derive(Debug, PartialEq)]
enum State {
    Active,
    Paused,
    Manual,
}

impl Controller {
    pub fn new(client: TransmissionClient,
               action: Option<Action>, action_periods: WeekPeriods,
               download_dir: &str, copy_to: Option<PathBuf>, move_to: Option<PathBuf>, free_space_threshold: Option<u8>,
               notifications_mailer: Option<Mailer>, torrent_downloaded_email_template: EmailTemplate) -> Controller {
        Controller {
            state: State::Manual,
            client: client,

            action: action,
            action_periods: action_periods,

            download_dir: s!(download_dir),
            copy_to: copy_to,
            move_to: move_to,

            free_space_threshold: free_space_threshold,

            notifications_mailer: notifications_mailer,
            torrent_downloaded_email_template: torrent_downloaded_email_template,
        }
    }

    // FIXME: Manual mode by Transmission settings
    pub fn control(&mut self) -> GenericResult<()> {
        self.state = self.calculate_state();
        debug!("Transmission daemon should be in {:?} state.", self.state);

        let removable_torrents = try!(self.control_torrents());

        if self.copy_to.is_some() && self.move_to.is_some() {
            try!(move_copied_torrents(
                &self.copy_to.as_ref().unwrap(), &self.move_to.as_ref().unwrap()).map_err(|e| format!(
                    "Failed to move copied torrents: {}", e)));
        }

        try!(self.cleanup_fs(&removable_torrents));

        Ok(())
    }

    fn calculate_state(&self) -> State {
        if self.action.is_none() {
            return State::Manual
        }

        return match self.action.unwrap() {
            Action::StartOrPause => {
                if periods::is_now_in(&self.action_periods) {
                    State::Active
                } else {
                    State::Paused
                }
            }
            Action::PauseOrStart => {
                if periods::is_now_in(&self.action_periods) {
                    State::Paused
                } else {
                    State::Active
                }
            }
        }
    }

    fn control_torrents(&mut self) -> GenericResult<Vec<Torrent>> {
        let torrents = try!(self.client.get_torrents());
        let mut removable_torrents = Vec::new();

        for torrent in torrents {
            debug!("Checking '{}' torrent...", torrent.name);

            if torrent.status == TorrentStatus::Paused && self.state == State::Active {
                info!("Resuming '{}' torrent...", torrent.name);
                try!(self.client.start(&torrent.hashString));
            } else if torrent.status != TorrentStatus::Paused && self.state == State::Paused {
                info!("Pausing '{}' torrent...", torrent.name);
                try!(self.client.stop(&torrent.hashString));
            }

            if torrent.doneDate != 0 {
                try!(self.torrent_downloaded(&torrent));
                removable_torrents.push(torrent);

                // FIXME
                //if (
                //    SETTINGS.get("max-seed-time", -1) >= 0 and
                //    time.time() - torrent.doneDate >= SETTINGS["max-seed-time"]
                //):
                //    LOG.info("Torrent %s has seeded enough time to delete it. Deleting it...", torrent.name)
                //    remove_torrent(torrent)
                //else:
                //    removable_torrents.append(torrent)
            }
        }

        Ok(removable_torrents)
    }

    fn torrent_downloaded(&mut self, torrent: &Torrent) -> GenericResult<()> {
        if torrent.is_processed() {
            return Ok(())
        }

        info!("'{}' torrent has been downloaded.", torrent.name);

        if self.copy_to.is_some() {
            let destination = self.copy_to.as_ref().unwrap().clone();
            try!(self.copy_torrent(&torrent, &destination).map_err(|e| format!(
                "Failed to copy '{}' torrent: {}", torrent.name, e)))
        }

        try!(self.client.set_processed(&torrent.hashString));

        if let Some(ref mailer) = self.notifications_mailer {
            let mut params = HashMap::new();
            params.insert("name", torrent.name.to_owned());

            if let Err(e) = self.torrent_downloaded_email_template.send(&mailer, &params) {
                error!("Failed to send 'torrent downloaded' notification for '{}' torrent: {}.",
                    torrent.name, e);
            }
        }

        Ok(())
    }

    fn cleanup_fs(&mut self, torrents: &Vec<Torrent>) -> GenericResult<()> {
        if torrents.len() == 0 || try!(self.check_free_space()) {
            return Ok(())
        }

        let download_dir_path = Path::new(&self.download_dir);
        let mut torrents: Vec<_> = torrents.iter()
            .filter(|&torrent| Path::new(&torrent.downloadDir) == download_dir_path)
            .collect();

        torrents.sort_by(|a, b| a.doneDate.cmp(&b.doneDate));

        for (id, torrent) in torrents.iter().enumerate() {
            info!("Removing '{}' torrent to get a free space on the disk...", torrent.name);
            try!(self.client.remove(&torrent.hashString));

            if id >= torrents.len() - 1 || try!(self.check_free_space()) {
                break
            }
        }

        Ok(())
    }

    fn check_free_space(&self) -> GenericResult<bool> {
        let free_space_threshold = match self.free_space_threshold {
            Some(value) => value,
            None => return Ok(true),
        };

        let (device, usage) = try!(fs::get_device_usage(&self.download_dir));

        let free_space = 100 - usage;
        let needs_cleanup = free_space <= free_space_threshold;

        if needs_cleanup {
            info!("We don't have enough free space on {}: {}% vs allowed > {}%.",
                device, free_space, free_space_threshold)
        }

        Ok(!needs_cleanup)
    }

    fn copy_torrent<P: AsRef<Path>>(&mut self, torrent: &Torrent, destination: P) -> GenericResult<()> {
        let download_dir_path = Path::new(&torrent.downloadDir);
        if !download_dir_path.is_absolute() {
            return Err!("Torrent's download directory is not an absolute path")
        }

        let files = try!(self.client.get_torrent_files(&torrent.hashString));

        info!("Copying '{}' to '{}'...", torrent.name, destination.as_ref().display());

        for file in files.iter().filter(|file| file.selected) {
            let file_path = file.name.trim_matches('/');
            if file_path.is_empty() {
                return Err(From::from(format!("The torrent has a file with empty name")))
            }

            let src_path = download_dir_path.join(file_path);
            debug!("Copying '{}'...", src_path.display());

            let dst_path = destination.as_ref().join(file_path);
            let dst_dir_path = dst_path.parent().unwrap();

            try!(std::fs::create_dir_all(dst_dir_path).map_err(|e| format!(
                "Failed to create '{}' directory: {}", dst_dir_path.display(), e)));

            try!(fs::copy_file(&src_path, &dst_path));
        }

        Ok(())
    }
}

fn move_copied_torrents<P: AsRef<Path>>(src: P, dst: P) -> GenericResult<()> {
    let map_src_dir_error = |e| format!(
        "Error while reading '{}' directory: {}", src.as_ref().display(), e);

    let src_dir = try!(std::fs::read_dir(&src).map_err(&map_src_dir_error));

    for entry in src_dir {
        let entry = try!(entry.map_err(&map_src_dir_error));
        try!(move_file(&entry, &dst));
    }

    Ok(())
}

fn move_file<P: AsRef<Path>>(entry: &std::fs::DirEntry, dst_dir: P) -> GenericResult<()> {
    let src = entry.path();

    for id in 0..10 {
        let mut file_name = entry.file_name().into_string().unwrap(); // FIXME
        if id != 0 {
            file_name = format!("DUP_{}.{}", id, file_name);
        }

        let dst = dst_dir.as_ref().join(file_name);

        match std::fs::metadata(&dst) {
            Ok(_) => continue,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => {},
                _ => return Err!("Failed to stat() '{}': {}", dst.display(), err)
            }
        }

        info!("Moving '{}' to '{}'...", src.display(), dst.display());
        try!(std::fs::rename(&src, &dst).map_err(|e| format!(
            "Failed to rename '{}' to '{}': {}", src.display(), dst.display(), e)));

        return Ok(())
    }

    Err!("Failed to move '{}' to '{}': the file is already exists",
        src.display(), dst_dir.as_ref().display())
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
