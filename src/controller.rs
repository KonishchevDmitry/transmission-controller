use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;
use std::io::Read;

use common::GenericResult;
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};

pub struct Controller {
    state: State,
    client: TransmissionClient,
}

#[derive(PartialEq)]
enum State {
    Active,
    Paused,
}

impl Controller{
    pub fn new(client: TransmissionClient) -> Controller {
        Controller {
            state: State::Active,
            client: client,
        }
    }

    pub fn control(&mut self) -> GenericResult<()> {
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

        Ok(())
    }
}
