use std::path::Path;

use common::GenericResult;
use fs;
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};

pub struct Controller {
    state: State,
    client: TransmissionClient,

    download_dir: String,
    free_space_threshold: Option<u8>,
}

#[derive(PartialEq)]
enum State {
    Active,
    Paused,
}

impl Controller{
    pub fn new(client: TransmissionClient, download_dir: &str, free_space_threshold: Option<u8>) -> Controller {
        Controller {
            state: State::Active,
            client: client,

            download_dir: s!(download_dir),
            free_space_threshold: free_space_threshold,
        }
    }

    pub fn control(&mut self) -> GenericResult<()> {
        if true {
            let torrents = try!(self.client.get_torrents());

            for torrent in &torrents {
                info!("Checking '{}' torrent...", torrent.name);

                if torrent.status == TorrentStatus::Paused && self.state == State::Active {
                    info!("Resuming '{}' torrent...", torrent.name);
                    // FIXME: client
                } else if torrent.status != TorrentStatus::Paused && self.state == State::Paused {
                    info!("Pausing '{}' torrent...", torrent.name);
                    // FIXME: client
                }
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
}
