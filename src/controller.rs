use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;
use std::io::Read;

use common::GenericResult;
use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};

pub struct Controller {
    client: TransmissionClient,
}

enum Action {
    Start,
    Stop,
    None,
}

impl Controller{
    pub fn new(client: TransmissionClient) -> Controller {
        Controller {
            client: client,
        }
    }

    pub fn control(&mut self) -> GenericResult<()> {
        for torrent in try!(self.client.get_torrents()) {
            if torrent.status == TorrentStatus::Paused {
                info!("{:?}", torrent);
            }
        }

        Ok(())
    }
}
