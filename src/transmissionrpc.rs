#![allow(unexpected_cfgs)] // enum_primitive_serde_shim doesn't support modern Rust, but works with it

use std::convert::From;
use std::error::Error;
use std::fmt;
use std::str::FromStr;
use std::sync::RwLock;
use std::time::Duration;

use enum_primitive_serde_shim::impl_serde_for_enum_primitive;
use itertools::Itertools;
use mime::{self, Mime};
use reqwest::{Method, StatusCode, header};
use reqwest::blocking::{Client, Response};
use serde::{Serialize, Deserialize, de::DeserializeOwned};

use crate::util::time::Timestamp;

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
    pub upload_ratio: Option<f64>,
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

        // Our virtual states
        LostData     = -1 // Torrent's data is lost
    }
}
impl_serde_for_enum_primitive!(TorrentStatus);

enum_from_primitive! {
    #[derive(Debug, PartialEq, Clone, Copy)]
    pub enum TorrentError {
        Ok             = 0, // No error
        TrackerWarning = 1, // Tracker returned a warning
        TrackerError   = 2, // Tracker returned an error
        LocalError     = 3, // Local non-tracker error, e.g. disk full or file permissions
    }
}
impl_serde_for_enum_primitive!(TorrentError);

#[derive(Debug)]
pub struct TorrentFile {
    pub name: String,
    pub selected: bool,
}

#[derive(Serialize)]
struct EmptyRequest{
}

#[derive(Deserialize)]
struct EmptyResponse{
}

pub type Result<T> = std::result::Result<T, TransmissionClientError>;
pub type EmptyResult = Result<()>;

// Use this value of downloadLimit as marker for processed torrents
const TORRENT_PROCESSED_MARKER: u64 = 42;

const SESSION_ID_HEADER_NAME: &str = "X-Transmission-Session-Id";

impl TransmissionClient{
    pub fn new(url: &str) -> TransmissionClient {
        TransmissionClient {
            client: Client::builder().timeout(Duration::from_secs(60)).build().unwrap(),
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
        #[derive(Deserialize)]
        struct Response {
            alt_speed_enabled: bool,
        }

        let response: Response = self.call("session_get", &EmptyRequest{})?;

        Ok(response.alt_speed_enabled)
    }

    pub fn set_manual_mode(&self, enabled: bool) -> EmptyResult {
        #[derive(Serialize)]
        struct Request {
            alt_speed_enabled: bool,
        }

        let _: EmptyResponse = self.call("session_set", &Request {
            alt_speed_enabled: enabled,
        })?;

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
        #[derive(Serialize)]
        struct Request {
            #[serde(skip_serializing_if = "Option::is_none")]
            ids: Option<Vec<String>>,
            fields: Vec<&'static str>,
        }

        #[derive(Deserialize)]
        struct Response {
            torrents: Vec<TransmissionTorrent>,
        }

        #[derive(Debug, Deserialize)]
        struct TransmissionTorrent {
            hash_string: String,
            name: String,

            download_dir: String,
            files: Option<Vec<File>>,
            file_stats: Option<Vec<FileStats>>,
            wanted: Vec<bool>,

            status: TorrentStatus,
            error: TorrentError,
            error_string: String,

            added_date: Timestamp,
            left_until_done: u64,
            done_date: Timestamp,
            download_limit: u64,
            upload_ratio: f64,
        }

        #[derive(Debug, Deserialize)]
        struct File {
            name: String,
        }

        #[derive(Debug, Deserialize)]
        struct FileStats {
            wanted: bool,
        }

        let mut fields = vec![
            "hash_string", "name", "download_dir", "wanted", "status", "error", "error_string",
            "added_date","left_until_done", "done_date", "download_limit", "upload_ratio",
        ];
        if with_files {
            fields.push("files");
            fields.push("file_stats");
        }

        let response: Response = self.call("torrent_get", &Request {
            ids: hashes,
            fields: fields,
        })?;

        let mut torrents = Vec::with_capacity(response.torrents.len());

        for torrent in response.torrents {
            let mut files = None;

            if with_files {
                let file_infos = torrent.files.ok_or_else(|| Protocol(s!(
                    "Got a torrent with missing `files`")))?;

                let file_stats = torrent.file_stats.ok_or_else(|| Protocol(s!(
                    "Got a torrent with missing `file_stats`")))?;

                if file_infos.len() != file_stats.len() {
                    return Err(Protocol(s!("Torrent's `files` and `file_stats` don't match")))
                }

                files = Some(file_infos.iter().zip(&file_stats).map(|item| {
                    TorrentFile {
                        name: item.0.name.to_owned(),
                        selected: item.1.wanted,
                    }
                }).collect());
            }

            let mut status = torrent.status;

            // If we delete torrent's data, Transmission silently ignores file open errors during seeding.
            //
            // On daemon restart or torrent starting the torrent will be checked and missing data will be detected. In
            // this case the daemon stops the torrent and switches it to the error state (paused status + error
            // message), but this happens only when all torrent's files are missing – deletion only some of them won't
            // trigger the error.
            //
            // To fix the torrent you must issue verify local data command + start the torrent. Sole start commands
            // (without verify) are silently ignored. Verify command doesn't change the torrent state until its start.
            //
            // `left_until_done` is reset when torrent gets errors state. `done_date` will be updated after redownloading.
            if status == TorrentStatus::Paused && torrent.error == TorrentError::LocalError && (
                // `No data found! Ensure your drives are connected or use "Set Location". To re-download, use "Verify Local Data" and start the torrent afterwards.`
                torrent.error_string.starts_with("No data found!") || // On daemon restart

                // `Paused torrent as no data was found! Ensure your drives are connected or use "Set Location", then use "Verify Local Data" again. To re-download, start the torrent.`
                torrent.error_string.starts_with("Paused torrent as no data was found!") // On start or when we issued verify after daemon restart
            ) {
                status = TorrentStatus::LostData;
            }

            // It's not actually easy to determine when torrent is downloaded:
            // * `done_date` is not reset when we add new files to download
            // * `percent_done` may be 1.0 even when only 99% has been downloaded
            // * `left_until_done` looks like a best marker (or we can use files + wanted, but it's more expensive)
            let done = torrent.left_until_done == 0 && (
                // Ensure that we check torrent status not in the moment when user temporary unmarked all files to start
                // select only individual ones.
                torrent.wanted.iter().contains(&true)
            );

            let done_time = if done {
                // `done_date` is set only when torrent is downloaded. If we add a torrent that
                // already downloaded on the disk `done_date` won't be updated.
                Some(if torrent.done_date != 0 { torrent.done_date } else { torrent.added_date })
            } else {
                None
            };

            torrents.push(Torrent {
                hash:         torrent.hash_string,
                name:         torrent.name,
                status,
                files,
                download_dir: torrent.download_dir,
                done,
                done_time,
                upload_ratio: if torrent.upload_ratio > 0.0 {
                    Some(torrent.upload_ratio)
                } else {
                    None
                },
                processed: torrent.download_limit == TORRENT_PROCESSED_MARKER,
            });
        }

        Ok(torrents)
    }

    pub fn start(&self, hash: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            ids: [&'a str; 1],
        }

        let _: EmptyResponse = self.call("torrent_start", &Request {
            ids: [hash]
        })?;

        Ok(())
    }

    pub fn stop(&self, hash: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            ids: [&'a str; 1],
        }

        let _: EmptyResponse = self.call("torrent_stop", &Request {
            ids: [hash]
        })?;

        Ok(())
    }

    pub fn verify(&self, hash: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            ids: [&'a str; 1],
        }

        let _: EmptyResponse = self.call("torrent_verify", &Request {
            ids: [hash]
        })?;

        Ok(())
    }

    pub fn set_processed(&self, hash: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            ids: [&'a str; 1],
            download_limit: u64,
        }

        let _: EmptyResponse = self.call("torrent_set", &Request {
            ids: [hash],
            download_limit: TORRENT_PROCESSED_MARKER,
        })?;

        Ok(())
    }

    pub fn remove(&self, hash: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            ids: [&'a str; 1],
            delete_local_data: bool,
        }

        let _: EmptyResponse = self.call("torrent_remove", &Request {
            ids: [hash],
            delete_local_data: true,
        })?;

        Ok(())
    }

    fn call<I: Serialize, O: DeserializeOwned>(&self, method: &str, params: &I) -> Result<O> {
        self._call(method, params).inspect_err(|e| {
            trace!("RPC error: {e}.");
        })
    }

    fn _call<I: Serialize, O: DeserializeOwned>(&self, method: &str, params: &I) -> Result<O> {
        #[derive(Serialize)]
        struct Request<'a, T> {
            jsonrpc: &'a str,
            id: Option<&'a str>,
            method: &'a str,
            params: &'a T,
        }

        #[derive(Deserialize)]
        struct Response<T> {
            result: Option<T>,
            error: Option<Error>,
        }

        #[derive(Deserialize)]
        struct Error {
            message: String,
            data: Option<ErrorDetails>,
        }

        #[derive(Deserialize)]
        struct ErrorDetails {
            error_string: Option<String>,
        }

        let request_json = serde_json::to_string(&Request {
            jsonrpc: "2.0", id: None,
            method, params,
        }).map_err(|e| Internal(format!("Failed to encode the request: {e}")))?;

        trace!("RPC call: {request_json}");
        let mut response = self.send_request(&request_json)?;

        if response.status() == StatusCode::CONFLICT {
            let session_id = response.headers().get(SESSION_ID_HEADER_NAME)
                .ok_or_else(|| Protocol(format!(
                    "Got {} HTTP status code without {SESSION_ID_HEADER_NAME} header",
                    response.status())))
                .and_then(|value| {
                    Ok(value.to_str().map_err(|_| Protocol(format!(
                        "Got an invalid {SESSION_ID_HEADER_NAME} header value: {value:?}")))?.to_owned())
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
                    "Got an invalid Content-Type header value: {value:?}")))
            })
            .and_then(|content_type| {
                Mime::from_str(content_type).ok().and_then(|content_type| {
                    if content_type.type_() == mime::APPLICATION && content_type.subtype() == mime::JSON {
                        Some(content_type)
                    } else {
                        None
                    }
                }).ok_or_else(|| Protocol(format!(
                    "Server returned {} response with an invalid content type: {content_type}",
                    response.status(),
                )))
            })?;

        let mut body = Vec::new();
        response.copy_to(&mut body)?;

        let body = String::from_utf8(body).map_err(|_| Protocol(s!(
            "Server returned an invalid UTF-8 response")))?;
        let body = body.trim();

        trace!("RPC result: {body}");
        let response: Response<O> = serde_json::from_str(&body).map_err(|e| Protocol(format!(
            "Got an invalid response from server: {e}")))?;

        Ok(match response {
            Response{result: Some(result), error: None} => result,
            Response{result: None, error: Some(error)} => {
                let mut message = error.message;
                if let Some(ErrorDetails{error_string: Some(details)}) = error.data {
                    message = format!("{message}: {details}");
                }
                return Err(Rpc(GeneralError(message)))
            },
            _ => return Err(Protocol(format!("Server returned an invalid response: {body}"))),
        })
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
            Connection(ref err) => write!(f, "Failed to connect to Transmission daemon: {err}"),
            Internal(ref err) | Protocol(ref err) => write!(f, "Error in communication with Transmission daemon: {err}"),
            Rpc(ref err) => write!(f, "Transmission daemon returned an error: {err}"),
        }
    }
}

impl From<reqwest::Error> for TransmissionClientError {
    fn from(err: reqwest::Error) -> TransmissionClientError {
        // reqwest/hyper errors hide all details, so extract the underlying error
        let mut err: &dyn Error = &err;
        while let Some(source) = err.source() {
            err = source;
        }
        Connection(err.to_string())
    }
}


#[derive(Debug)]
pub enum TransmissionRpcError {
    GeneralError(String),
    #[allow(dead_code)]
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
