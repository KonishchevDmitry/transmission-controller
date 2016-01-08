use std::collections::HashSet;
use std::sync::{Arc, Mutex, Weak};
use std::sync::mpsc::channel;
use std::thread;

use transmissionrpc::{TransmissionClient, Torrent, TorrentStatus};

pub struct Consumer {
    client: Arc<TransmissionClient>,
    thread: thread::JoinHandle<()>,
    data: Mutex<Data>,
}

struct Data {
    in_process: HashSet<String>,
}

impl Consumer {
    pub fn new(client: Arc<TransmissionClient>) -> Arc<Consumer> {
        let (tx, rx) = channel();

        let consumer = Consumer {
            client: client,

            thread: thread::spawn(move || {
                if let Ok(consumer) = rx.recv() {
                    consumer_thread(consumer);
                }
            }),

            data: Mutex::new(Data {
                in_process: HashSet::new(),
            })
        };

        let consumer = Arc::new(consumer);
        tx.send(Arc::downgrade(&consumer));
        consumer
    }

    pub fn get_in_process(&self) -> HashSet<String> {
        let data = self.data.lock().unwrap();
        data.in_process.clone()
    }

    pub fn consume(&self, hash: &str) {
        {
            let mut data = self.data.lock().unwrap();
            data.in_process.insert(s!(hash));
        }

        self.thread.thread().unpark();
    }

    pub fn process(&self) {
        let in_process = {
            let data = self.data.lock().unwrap();
            data.in_process.clone()
        };

        for hash in in_process.iter() {
            info!("Consuming {:?}", hash);
        }
    }
}

fn consumer_thread(consumer: Weak<Consumer>) {
    // FIXME: drop
    loop {
        thread::park();

        let consumer = match consumer.upgrade() {
            Some(consumer) => consumer,
            None => break,
        };

        consumer.process();
    }
}
