use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use itertools::Itertools;

use common::{EmptyResult, GenericResult};
use email::{Mailer, EmailTemplate};
use transmissionrpc::{TransmissionClient, Torrent, TransmissionClientError, TransmissionRpcError};
use util;

pub struct Consumer {
    data: Arc<Mutex<SharedData>>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

struct ConsumerThread {
    copy_to: Option<PathBuf>,
    move_to: Option<PathBuf>,

    notifications_mailer: Option<Mailer>,
    torrent_downloaded_email_template: EmailTemplate,

    client: Arc<TransmissionClient>,

    failed: HashSet<String>,
    data: Arc<Mutex<SharedData>>,
}

struct SharedData {
    stop: bool,
    in_process: HashSet<String>,
}

enum ProcessError {
    Cancelled(String),
    Temporary(String),
    Persistent(String),
}
type ProcessResult = Result<(), ProcessError>;

impl Consumer {
    pub fn new(client: Arc<TransmissionClient>, copy_to: Option<PathBuf>, move_to: Option<PathBuf>,
               notifications_mailer: Option<Mailer>, torrent_downloaded_email_template: EmailTemplate) -> Consumer {
        let data = Arc::new(Mutex::new(SharedData {
            stop: false,
            in_process: HashSet::new(),
        }));

        let mut consumer_thread = ConsumerThread {
            copy_to: copy_to,
            move_to: move_to,

            notifications_mailer: notifications_mailer,
            torrent_downloaded_email_template: torrent_downloaded_email_template,

            client: client,
            failed: HashSet::new(),
            data: data.clone(),
        };

        Consumer {
            thread_handle: Some(thread::spawn(move || { consumer_thread.run() })),
            data: data,
        }
    }

    pub fn get_in_process(&self) -> HashSet<String> {
        let data = self.data.lock().unwrap();
        data.in_process.clone()
    }

    pub fn consume(&self, hash: &str) {
        debug!("Scheduling {:?} torrent for consuming.", hash);

        {
            let mut data = self.data.lock().unwrap();
            data.in_process.insert(s!(hash));
        }

        if let Some(ref thread_handle) = self.thread_handle {
            thread_handle.thread().unpark();
        }
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        let mut thread_handle = None;
        mem::swap(&mut thread_handle, &mut self.thread_handle);

        if let Some(thread_handle) = thread_handle {
            debug!("Stopping torrent consuming thread...");

            self.data.lock().unwrap().stop = true;
            thread_handle.thread().unpark();

            if let Err(error) = thread_handle.join() {
                error!("Torrent consuming thread has panicked: {:?}.", error);
            } else {
                debug!("Torrent consuming thread has stopped.");
            }
        }
    }
}

impl ConsumerThread {
    fn run(&mut self) {
        if let (Some(copy_to), Some(_)) = (self.copy_to.as_ref(), self.move_to.as_ref()) {
            if let Err(error) = check_copy_to_directory(copy_to) {
                error!("Failed to check copy to directory: {}.", error);
            }
        }

        let mut retry_after = None;

        loop {
            if let Some(retry_after) = retry_after {
                thread::park_timeout(retry_after);
            } else {
                thread::park();
            }

            if self.data.lock().unwrap().stop {
                break;
            }

            retry_after = self.process();
        }
    }

    fn process(&mut self) -> Option<Duration> {
        let in_process: Vec<String> = {
            let data = self.data.lock().unwrap();
            data.in_process.difference(&self.failed).cloned().collect()
        };

        // A workaround for https://github.com/seanmonstar/reqwest/issues/1131
        if !in_process.is_empty() {
            thread::current().unpark();
        }

        for hash in &in_process {
            match self.process_torrent(hash)  {
                Ok(_) => {
                    assert!(self.data.lock().unwrap().in_process.remove(hash));
                },
                Err(error) => match error {
                    ProcessError::Cancelled(error) => {
                        warn!("{}.", error);
                        assert!(self.data.lock().unwrap().in_process.remove(hash));
                    },
                    ProcessError::Temporary(error) => {
                        error!("{}.", error);
                        return Some(Duration::from_secs(60));
                    },
                    ProcessError::Persistent(error) => {
                        error!("{}.", error);
                        assert!(self.failed.insert(hash.clone()));
                    },
                },
            }
        }

        None
    }

    fn process_torrent(&self, hash: &str) -> ProcessResult {
        let torrent = self.client.get_torrent(hash).map_err(|error| {
            if let TransmissionClientError::Rpc(TransmissionRpcError::TorrentNotFoundError(_)) = error {
                return ProcessError::Cancelled(format!(
                    "Failed to consume {} torrent: it has been removed", hash));
            }

            ProcessError::Temporary(format!("Failed to get '{}' torrent info: {}", hash, error))
        })?;

        if !torrent.done {
            return Err(ProcessError::Cancelled(format!(
                "Cancelling consuming of {} torrent: it has started to download", torrent.name)));
        }

        if let Err(error) = self.consume_torrent(&torrent) {
            return Err(ProcessError::Persistent(error.to_string()));
        }

        Ok(())
    }

    fn consume_torrent(&self, torrent: &Torrent) -> EmptyResult {
        info!("Consuming '{}' torrent...", torrent.name);

        if let Some(ref copy_to) = self.copy_to {
            let torrent_files = copy_torrent(torrent, &copy_to).map_err(|e| format!(
                "Failed to copy '{}' torrent: {}", torrent.name, e))?;

            if let Some(ref move_to) = self.move_to {
                for file_path in &torrent_files {
                    move_torrent_file(file_path, move_to).map_err(|e| format!(
                        "Failed to move '{}' torrent: {}", torrent.name, e))?;
                }
            }
        }

        self.client.set_processed(&torrent.hash)?;
        info!("'{}' torrent has been consumed.", torrent.name);

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
}

fn copy_torrent<P: AsRef<Path>>(torrent: &Torrent, destination: P) -> GenericResult<HashSet<PathBuf>> {
    let destination = destination.as_ref();

    let download_dir_path = Path::new(&torrent.download_dir);
    if !download_dir_path.is_absolute() {
        return Err!("Torrent's download directory is not an absolute path: {}",
            torrent.download_dir)
    }

    info!("Copying '{}' to '{}'...", torrent.name, destination.display());

    let mut torrent_files = HashSet::new();

    for file in torrent.files.as_ref().unwrap().iter().filter(|file| file.selected) {
        let (file_root_path, file_path, file_name) = validate_torrent_file_name(&file.name)?;

        if file_name.to_string_lossy().starts_with('.') {
            info!("'{}': Ignoring '{}'.", torrent.name, file_path.display());
            continue;
        }

        let src_path = download_dir_path.join(&file_path);
        let dst_path = destination.join(&file_path);

        debug!("Copying '{}'...", src_path.display());

        if let Some(file_dir_path) = file_path.parent() {
            util::fs::create_all_dirs_from_base(&destination, &file_dir_path)?;
        }

        util::fs::copy_file(&src_path, &dst_path)?;
        torrent_files.insert(destination.join(&file_root_path));
    }

    Ok(torrent_files)
}

fn validate_torrent_file_name(torrent_file_name: &str) -> GenericResult<(PathBuf, PathBuf, OsString)> {
    use std::path::Component::*;

    let mut file_root_path = None;
    let mut file_path = PathBuf::new();
    let mut file_name = None;

    for component in Path::new(torrent_file_name).components() {
        match component {
            Normal(component) => {
                if file_root_path == None {
                    file_root_path = Some(Path::new(component));
                }
                file_name = Some(component);
                file_path.push(component);
            },
            Prefix(_) | RootDir | CurDir | ParentDir => {
                return Err!("Invalid torrent file name: '{}'", torrent_file_name);
            }
        }
    }

    if let (Some(file_root_path), Some(file_name)) = (file_root_path, file_name) {
        return Ok((file_root_path.to_path_buf(), file_path, file_name.to_os_string()))
    }

    Err!("Invalid torrent file name: '{}'", torrent_file_name)
}

fn move_torrent_file<S, D>(src: S, dst_dir: D) -> EmptyResult where S: AsRef<Path>, D: AsRef<Path> {
    let (src, dst_dir) = (src.as_ref(), dst_dir.as_ref());
    let src_name = src.file_name().ok_or(format!("Invalid file name: {}", src.display()))?;

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
        fs::rename(&src, &dst).map_err(|e| format!(
            "Failed to rename '{}' to '{}': {}", src.display(), dst.display(), e))?;

        return Ok(());
    }

    Err!("Failed to move '{}' to '{}': the file is already exists",
        src.display(), dst_dir.display())
}

fn check_copy_to_directory<P: AsRef<Path>>(path: P) -> EmptyResult {
    let path = path.as_ref();
    let map_dir_reading_error = |e| format!(
        "Error while reading '{}' directory: {}", path.display(), e);

    let mut abandoned_files: Vec<String> = Vec::new();
    let directory = fs::read_dir(&path).map_err(&map_dir_reading_error)?;

    for entry in directory {
        let file_name = entry.map_err(&map_dir_reading_error)?.file_name();
        let file_name_lossy = file_name.to_string_lossy();

        if !file_name_lossy.starts_with('.') {
            abandoned_files.push(file_name_lossy.into_owned());
        }
    }

    if !abandoned_files.is_empty() {
        error!("'{}' has the following abandoned files: {}.", path.display(),
            abandoned_files.iter().map(|file_name| format!("'{}'", file_name)).join(", "));
    }

    Ok(())
}
