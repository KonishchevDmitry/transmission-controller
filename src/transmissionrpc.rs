#![allow(deprecated)] // We still use deprecated RustcDecodable here

use std;
use std::convert::From;
use std::error::Error;
use std::fmt;
use std::str::FromStr;
use std::sync::RwLock;
use std::time::Duration;

use rustc_serialize::Decoder;

use mime::{self, Mime};
use reqwest::{Method, StatusCode, header};
use reqwest::blocking::{Client, Response};

use json;
use json::{Encodable, Decodable};
use util::time::Timestamp;

pub struct TransmissionClient {
    client: Client,
    url: String,
    user: Option<String>,
    password: Option<String>,
    session_id: RwLock<Option<String>>,
}

#[derive(Debug)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    pub status: TorrentStatus,
    pub files: Option<Vec<TorrentFile>>,
    pub download_dir: String,
    pub done: bool,
    pub done_time: Option<Timestamp>,
    pub processed: bool,
}

enum_from_primitive! {
    #[derive(Debug, PartialEq, Clone, Copy)]
    pub enum TorrentStatus {
        Paused       = 0, // Paused
        CheckWait    = 1, // Queued for file checking
        Checking     = 2, // Checking files
        DownloadWait = 3, // Queued for downloading
        Downloading  = 4, // Downloading
        SeedWait     = 5, // Queued for seeding
        Seeding      = 6, // Seeding
    }
}

#[derive(Debug)]
pub struct TorrentFile {
    pub name: String,
    pub selected: bool,
}

#[derive(RustcEncodable)] struct EmptyRequest;
#[derive(RustcDecodable)] struct EmptyResponse;

pub type Result<T> = std::result::Result<T, TransmissionClientError>;
pub type EmptyResult = Result<()>;

// Use this value of downloadLimit as marker for processed torrents
const TORRENT_PROCESSED_MARKER: u64 = 42;

const SESSION_ID_HEADER_NAME: &str = "X-Transmission-Session-Id";

impl TransmissionClient{
    pub fn new(url: &str) -> TransmissionClient {
        TransmissionClient {
            client: Client::builder().timeout(Duration::from_secs(10)).build().unwrap(),
            url: s!(url),
            user: None,
            password: None,
            session_id: RwLock::new(None),
        }
    }

    pub fn set_authentication(&mut self, user: &str, password: &str) {
        self.user = Some(s!(user));
        self.password = Some(s!(password));
    }

    pub fn is_manual_mode(&self) -> Result<bool> {
        #[derive(RustcDecodable)] struct Response { alt_speed_enabled: bool }
        let response: Response = self.call("session-get", &EmptyRequest)?;
        Ok(response.alt_speed_enabled)
    }

    pub fn set_manual_mode(&self, enabled: bool) -> EmptyResult {
        #[derive(RustcEncodable)] struct Request { alt_speed_enabled: bool }
        let _: EmptyResponse = self.call("session-set", &Request { alt_speed_enabled: enabled })?;
        Ok(())
    }

    pub fn get_torrents(&self) -> Result<Vec<Torrent>> {
        self._get_torrents(None, false)
    }

    pub fn get_torrent(&self, hash: &str) -> Result<Torrent> {
        let mut torrents = self._get_torrents(Some(vec![s!(hash)]), true)?;
        match torrents.len() {
            0 => Err(Rpc(TorrentNotFoundError(s!(hash)))),
            1 => Ok(torrents.pop().unwrap()),
            _ => Err(Protocol(s!("Got a few torrents when requested only one"))),
        }
    }

    fn _get_torrents(&self, hashes: Option<Vec<String>>, with_files: bool) -> Result<Vec<Torrent>> {
        #[derive(RustcEncodable)]
        struct Request {
            ids: Option<Vec<String>>,
            fields: Vec<&'static str>,
        }

        #[derive(RustcDecodable)]
        struct Response {
            torrents: Vec<TransmissionTorrent>,
        }

        #[allow(non_snake_case)]
        #[derive(Debug, RustcDecodable)]
        struct TransmissionTorrent {
            hashString: String,
            name: String,
            downloadDir: String,
            status: TorrentStatus,
            addedDate: Timestamp,
            doneDate: Timestamp,
            downloadLimit: u64,
            files: Option<Vec<File>>,
            fileStats: Option<Vec<FileStats>>,
            percentDone: f64,
        }

        #[derive(Debug, RustcDecodable)] struct File { name: String }
        #[derive(Debug, RustcDecodable)] struct FileStats { wanted: bool }

        let mut fields = vec!["hashString", "name", "downloadDir", "status",
                              "addedDate", "doneDate", "downloadLimit", "percentDone"];
        if with_files {
            fields.push("files");
            fields.push("fileStats");
        }

        let response: Response = self.call("torrent-get", &Request {
            ids: hashes,
            fields: fields,
        })?;

        let mut torrents = Vec::with_capacity(response.torrents.len());

        for torrent in response.torrents {
            let mut files = None;

            if with_files {
                let file_infos = torrent.files.ok_or_else(|| Protocol(s!(
                    "Got a torrent with missing `files`")))?;

                let file_stats = torrent.fileStats.ok_or_else(|| Protocol(s!(
                    "Got a torrent with missing `fileStats`")))?;

                if file_infos.len() != file_stats.len() {
                    return Err(Protocol(s!("Torrent's `files` and `fileStats` don't match")))
                }

                files = Some(file_infos.iter().zip(&file_stats).map(|item| {
                    TorrentFile {
                        name: item.0.name.to_owned(),
                        selected: item.1.wanted,
                    }
                }).collect());
            }

            #[allow(clippy::float_cmp)]
            let done = torrent.percentDone == 1.0;
            let done_time = if done {
                // doneDate is set only when torrent is downloaded. If we add a torrent that
                // already downloaded on the disk doneDate won't be updated.
                Some(if torrent.doneDate != 0 { torrent.doneDate } else { torrent.addedDate })
            } else {
                None
            };

            torrents.push(Torrent {
                hash:         torrent.hashString,
                name:         torrent.name.clone(),
                status:       torrent.status,
                files:        files,
                download_dir: torrent.downloadDir.clone(),
                done:         done,
                done_time:    done_time,
                processed:    torrent.downloadLimit == TORRENT_PROCESSED_MARKER,
            });
        }

        Ok(torrents)
    }

    pub fn start(&self, hash: &str) -> EmptyResult {
        #[derive(RustcEncodable)] struct Request { ids: Vec<String> }

        let _: EmptyResponse = self.call("torrent-start", &Request {
            ids: vec![s!(hash)]
        })?;

        Ok(())
    }

    pub fn stop(&self, hash: &str) -> EmptyResult {
        #[derive(RustcEncodable)] struct Request { ids: Vec<String> }

        let _: EmptyResponse = self.call("torrent-stop", &Request {
            ids: vec![s!(hash)]
        })?;

        Ok(())
    }

    pub fn set_processed(&self, hash: &str) -> EmptyResult {
        #[allow(non_snake_case)]
        #[derive(RustcEncodable)] struct Request {
            ids: Vec<String>,
            downloadLimit: u64,
        }

        let _: EmptyResponse = self.call("torrent-set", &Request {
            ids: vec![s!(hash)],
            downloadLimit: TORRENT_PROCESSED_MARKER,
        })?;

        Ok(())
    }

    pub fn remove(&self, hash: &str) -> EmptyResult {
        #[derive(RustcEncodable)] struct Request {
            ids: Vec<String>,
            delete_local_data: bool,
        }

        let _: EmptyResponse = self.call("torrent-remove", &Request {
            ids: vec![s!(hash)],
            delete_local_data: true,
        })?;

        Ok(())
    }

    fn call<I: Encodable, O: Decodable>(&self, method: &str, arguments: &I) -> Result<O> {
        self._call(method, arguments).map_err(|e| {
            trace!("RPC error: {}.", e);
            e
        })
    }

    fn _call<'a, I: Encodable, O: Decodable>(&self, method: &str, arguments: &'a I) -> Result<O> {
        #[derive(RustcEncodable)]
        struct Request<'a, T: 'a> {
            method: String,
            arguments: &'a T,
        }

        #[derive(RustcDecodable)]
        struct Response<T: Decodable> {
            result: String,
            arguments: Option<T>,
        }

        let request_json = json::encode(&Request {
            method: s!(method),
            arguments: &arguments,
        }, false).map_err(|e| Internal(format!(
            "Failed to encode the request: {}", e
        )))?;

        trace!("RPC call: {}", request_json);
        let mut response = self.send_request(&request_json)?;

        if response.status() == StatusCode::CONFLICT {
            let session_id = response.headers().get(SESSION_ID_HEADER_NAME)
                .ok_or_else(|| Protocol(format!(
                    "Got {} HTTP status code without {} header",
                    response.status(), SESSION_ID_HEADER_NAME)))
                .and_then(|value| {
                    Ok(value.to_str().map_err(|_| Protocol(format!(
                        "Got an invalid {} header value: {:?}",
                        SESSION_ID_HEADER_NAME, value)))?.to_owned())
                })?;

            debug!("Session ID is expired. Got a new session ID.");
            *self.session_id.write().unwrap() = Some(session_id);
            response = self.send_request(&request_json)?;
        }

        if response.status() != StatusCode::OK {
            return Err(Internal(format!("Got {} HTTP status code", response.status())));
        }

        response.headers().get(header::CONTENT_TYPE)
            .ok_or_else(|| Protocol(format!(
                "Server returned {} response without Content-Type", response.status())))
            .and_then(|value| {
                value.to_str().map_err(|_| Protocol(format!(
                    "Got an invalid Content-Type header value: {:?}", value)))
            })
            .and_then(|content_type| {
                Mime::from_str(content_type).ok().and_then(|content_type| {
                    if content_type.type_() == mime::APPLICATION && content_type.subtype() == mime::JSON {
                        Some(content_type)
                    } else {
                        None
                    }
                }).ok_or_else(|| Protocol(format!(
                    "Server returned {} response with an invalid content type: {}",
                    response.status(), content_type
                )))
            })?;

        let mut body = Vec::new();
        response.copy_to(&mut body)?;

        let body = String::from_utf8(body).map_err(|_| Protocol(s!(
            "Server returned an invalid UTF-8 response")))?;
        trace!("RPC result: {}", body.trim());

        let response: Response<O> = json::decode_str(&body).map_err(|e| Protocol(format!(
            "Got an invalid response from server: {}", e)))?;

        if response.result != "success" {
            return Err(Rpc(GeneralError(response.result)))
        }

        match response.arguments {
            Some(arguments) => Ok(arguments),
            None => Err(Protocol(s!("Got a successful reply without arguments"))),
        }
    }

    fn send_request(&self, body: &str) -> Result<Response> {
        let mut request = self.client.request(Method::POST, &self.url)
            .header(header::CONTENT_TYPE, "application/json");

        if let (Some(user), Some(password)) = (self.user.as_ref(), self.password.as_ref()) {
            request = request.basic_auth(user, Some(password));
        }

        {
            let session_id = self.session_id.read().unwrap();
            if let Some(ref session_id) = *session_id {
                request = request.header(SESSION_ID_HEADER_NAME, session_id.as_str());
            }
        }

        Ok(request.body(body.to_owned()).send()?)
    }
}


#[derive(Debug)]
pub enum TransmissionClientError {
    Connection(String),
    Internal(String),
    Protocol(String),
    Rpc(TransmissionRpcError),
}
use self::TransmissionClientError::*;

impl Error for TransmissionClientError {
    fn description(&self) -> &str {
        "Transmission client error"
    }
}

impl fmt::Display for TransmissionClientError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Connection(ref err) => write!(f,
                                          "Failed to connect to Transmission daemon: {}", err),
            Internal(ref err) | Protocol(ref err) => write!(f,
                                                            "Error in communication with Transmission daemon: {}", err),
            Rpc(ref err) => write!(f,
                                   "Transmission daemon returned an error: {}", err),
        }
    }
}

impl From<reqwest::Error> for TransmissionClientError {
    fn from(err: reqwest::Error) -> TransmissionClientError {
        Connection(err.to_string())
    }
}


#[derive(Debug)]
pub enum TransmissionRpcError {
    GeneralError(String),
    TorrentNotFoundError(String),
}
use self::TransmissionRpcError::*;

impl Error for TransmissionRpcError {
    fn description(&self) -> &str {
        "Transmission RPC error"
    }
}

impl fmt::Display for TransmissionRpcError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GeneralError(ref err) => write!(f, "{}", err),
            TorrentNotFoundError(_) => write!(f, "The specified torrent doesn't exist"),
        }
    }
}


impl Decodable for TorrentStatus {
    fn decode<D: Decoder>(decoder: &mut D) -> std::result::Result<TorrentStatus, D::Error> {
        json::decode_enum(decoder, "torrent status")
    }
}