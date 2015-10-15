use common::GenericResult;
use fs;
use transmissionrpc::{TransmissionClient, TorrentStatus};

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
        if self.free_space_threshold.is_some() {
            let (device, usage) = try!(fs::get_device_usage(&self.download_dir));
            let free_space = 100 - usage;
            debug!("{} free space: {}%.", device, free_space);

            let free_space_threshold = self.free_space_threshold.unwrap();
            if free_space <= free_space_threshold {
                info!("We don't have enough free space on {}: {}% vs {}%.",
                    device, free_space, free_space_threshold)
            }
        }

        if false {
            for torrent in try!(self.client.get_torrents()) {
                info!("Checking '{}' torrent...", torrent.name);

                if torrent.status == TorrentStatus::Paused && self.state == State::Active {
                    info!("Resuming '{}' torrent...", torrent.name);
                    // FIXME: client
                } else if torrent.status != TorrentStatus::Paused && self.state == State::Paused {
                    info!("Pausing '{}' torrent...", torrent.name);
                    // FIXME: client
                }
            }
        }

        Ok(())
    }
}
