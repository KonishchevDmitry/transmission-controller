use std;
use std::io;
use std::path::{Path, PathBuf};

use common::GenericResult;
use fs;
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};

pub struct Controller {
    state: State,
    client: TransmissionClient,

    download_dir: String,
    free_space_threshold: Option<u8>,
    copy_to: Option<String>,
    move_to: Option<String>,
}

#[derive(PartialEq)]
enum State {
    Active,
    Paused,
}

impl Controller{
    pub fn new(client: TransmissionClient, download_dir: &str, free_space_threshold: Option<u8>,
               copy_to: Option<String>, move_to: Option<String>) -> Controller {
        Controller {
            state: State::Active,
            client: client,

            download_dir: s!(download_dir),
            free_space_threshold: free_space_threshold,
            copy_to: copy_to,
            move_to: move_to,
        }
    }

    pub fn control(&mut self) -> GenericResult<()> {
        if true {
            // FIXME:
            //self.client.set_processed("743bc6fad39e3a35460d31af5322c131dd196ac2");

            let torrents = try!(self.client.get_torrents());

            for torrent in &torrents {
                // FIXME
                debug!("{:?}", torrent);
                info!("Checking '{}' torrent...", torrent.name);

                //if torrent.status == TorrentStatus::Paused && self.state == State::Active {
                //    info!("Resuming '{}' torrent...", torrent.name);
                //    // FIXME: client
                //} else if torrent.status != TorrentStatus::Paused && self.state == State::Paused {
                //    info!("Pausing '{}' torrent...", torrent.name);
                //    // FIXME: client
                //}

                //// FIXME
                //if self.copy_to.is_some() {
                //    let destination = self.copy_to.as_ref().unwrap().clone();
                //    match self.copy_torrent(&torrent, &destination) {
                //        Ok(_) => {},
                //        Err(err) => error!("Failed to copy '{}' torrent: {}.", torrent.name, err)
                //    }
                //}
            }

            if self.copy_to.is_some() && self.move_to.is_some() {
                try!(move_copied_torrents(&self.copy_to.as_ref().unwrap(), &self.move_to.as_ref().unwrap()));
            }

            // FIXME: unwrap, only removable
            self.cleanup_fs(&torrents).unwrap();
        }

        Ok(())
    }

    fn cleanup_fs(&self, torrents: &Vec<Torrent>) -> GenericResult<()> {
        if torrents.len() == 0 || try!(self.check_free_space()) {
            return Ok(())
        }

        let download_dir_path = Path::new(&self.download_dir);
        let mut torrents: Vec<_> = torrents.iter()
            .filter(|&torrent| Path::new(&torrent.downloadDir) == download_dir_path)
            .collect();

        torrents.sort_by(|a, b| a.doneDate.cmp(&b.doneDate));

        for (id, torrent) in torrents.iter().enumerate() {
            // FIXME
            info!("Removing '{}' to get a free space on the disk...", torrent.name);

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

    fn copy_torrent(&mut self, torrent: &Torrent, destination: &str) -> GenericResult<()> {
        let download_dir_path = Path::new(&torrent.downloadDir);
        if !download_dir_path.is_absolute() {
            return Err(From::from(format!("Torrent's download directory is not an absolute path")))
        }

        let files = try!(self.client.get_torrent_files(&torrent.hashString));

        info!("Copying '{}' to '{}'...", torrent.name, destination);

        for file in files.iter().filter(|file| file.selected) {
            let file_path = file.name.trim_matches('/');
            if file_path.is_empty() {
                return Err(From::from(format!("The torrent has a file with empty name")))
            }

            let src_path = download_dir_path.join(file_path);
            debug!("Copying '{}'...", src_path.display());

            let dst_path = Path::new(destination).join(file_path);
            let dst_dir_path = dst_path.parent().unwrap();

            try!(std::fs::create_dir_all(dst_dir_path).map_err(|e| format!(
                "Failed to create '{}' directory: {}", dst_dir_path.display(), e)));

            try!(fs::copy_file(&src_path, &dst_path));
        }

        Ok(())
    }
}

fn move_copied_torrents(src: &str, dst: &str) -> GenericResult<()> {
    let map_src_dir_error = |e| format!(
        "Error while reading '{}' directory: {}", src, e);

    let src_dir = try!(std::fs::read_dir(src).map_err(&map_src_dir_error));

    for entry in src_dir {
        let entry = try!(entry.map_err(&map_src_dir_error));
        try!(move_file(&entry, dst));
    }

    Ok(())
}

fn move_file(entry: &std::fs::DirEntry, dst: &str) -> GenericResult<()> {
    let src_path = entry.path();

    for id in 0..10 {
        let mut file_name = entry.file_name().into_string().unwrap(); // FIXME
        if id != 0 {
            file_name = format!("DUP_{}.{}", id, file_name);
        }

        let dst_path = Path::new(dst).join(file_name);

        match std::fs::metadata(&dst_path) {
            Ok(_) => continue,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => {},
                _ => return Err(From::from(format!(
                    "Failed to stat() '{}': {}", dst_path.display(), err)))
            }
        }

        info!("Moving '{}' to '{}'...", src_path.display(), dst_path.display());
        try!(std::fs::rename(&src_path, &dst_path).map_err(|e| format!(
            "Failed to rename '{}' to '{}': {}", src_path.display(), dst_path.display(), e)));

        return Ok(())
    }

    Err(From::from(format!("Failed to move '{}' to '{}': the file is already exists",
        src_path.display(), dst)))
}
