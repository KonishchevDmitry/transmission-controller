use std;
use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;
use std::io::Read;
use std::sync::RwLock;

use mime;
use rustc_serialize::Decoder;

use hyper::Client;
use hyper::error::Error as HyperError;
use hyper::status::StatusCode;
use hyper::header::{Header, Headers, Authorization, ContentType, Basic};

use json;
use json::{Encodable, Decodable};
use util::time::Timestamp;

pub struct TransmissionClient {
    url: String,
    user: Option<String>,
    password: Option<String>,
    session_id: RwLock<Option<String>>,
    client: Client,
}

#[derive(Debug)]
pub struct Torrent {
    pub hash: String,
    pub name: String,
    pub download_dir: String,
    pub status: TorrentStatus,
    pub done_time: Timestamp,
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

// Use this value of downloadLimit as marker for processed torrents
const TORRENT_PROCESSED_MARKER: u64 = 42;

impl TransmissionClient{
    pub fn new(url: &str) -> TransmissionClient {
        TransmissionClient {
            url: s!(url),
            user: None,
            password: None,
            session_id: RwLock::new(None),
            // FIXME: socket timeouts
            client: Client::new(),
        }
    }

    pub fn set_authentication(&mut self, user: &str, password: &str) {
        self.user = Some(s!(user));
        self.password = Some(s!(password));
    }

    pub fn is_manual_mode(&self) -> Result<bool> {
        #[derive(RustcDecodable)] struct Response { alt_speed_enabled: bool }
        let response: Response = try!(self.call("session-get", &EmptyRequest));
        Ok(response.alt_speed_enabled)
    }

    pub fn get_torrents(&self) -> Result<Vec<Torrent>> {
        #[allow(non_snake_case)]
        #[derive(Debug, RustcDecodable)]
        struct TransmissionTorrent {
            hashString: String,
            name: String,
            downloadDir: String,
            status: TorrentStatus,
            doneDate: Timestamp,
            downloadLimit: u64,
        }
        #[derive(RustcEncodable)] struct Request { fields: Vec<&'static str> }
        #[derive(RustcDecodable)] struct Response { torrents: Vec<TransmissionTorrent> }

        let response: Response = try!(self.call("torrent-get", &Request {
            fields: vec!["hashString", "name", "downloadDir", "status", "doneDate", "downloadLimit"],
        }));

        Ok(response.torrents.iter().map(|torrent| Torrent {
            hash:         torrent.hashString.clone(),
            name:         torrent.name.clone(),
            download_dir: torrent.downloadDir.clone(),
            status:       torrent.status,
            done_time:    torrent.doneDate,
            processed:    torrent.downloadLimit == TORRENT_PROCESSED_MARKER,
        }).collect())
    }

    pub fn get_torrent_files(&self, hash: &str) -> Result<Vec<TorrentFile>> {
        #[derive(RustcEncodable)]
        struct Request {
            ids: Vec<String>,
            fields: Vec<&'static str>,
        }

        #[derive(RustcDecodable)] struct Response {
            torrents: Vec<TransmissionTorrent>,
        }

        #[allow(non_snake_case)]
        #[derive(RustcDecodable)]
        struct TransmissionTorrent {
            files: Vec<File>,
            fileStats: Vec<FileStats>,
        }

        #[derive(RustcDecodable)] struct File { name: String }
        #[derive(RustcDecodable)] struct FileStats { wanted: bool }

        let response: Response = try!(self.call("torrent-get", &Request {
            ids: vec![s!(hash)],
            fields: vec!["files", "fileStats"],
        }));

        let torrent = match response.torrents.len() {
            0 => return Err(RpcError(TorrentNotFoundError(s!(hash)))),
            1 => &response.torrents[0],
            _ => return Err(ProtocolError(s!("Got a few torrents when requested only one"))),
        };

        if torrent.files.len() != torrent.fileStats.len() {
            return Err(ProtocolError(s!("Torrent's `files` and `fileStats` don't match.")))
        }

        Ok(torrent.files.iter().zip(&torrent.fileStats).map(|item| {
            TorrentFile {
                name: item.0.name.to_owned(),
                selected: item.1.wanted,
            }
        }).collect())
    }

    pub fn start(&self, hash: &str) -> Result<()> {
        #[derive(RustcEncodable)] struct Request { ids: Vec<String> }

        let _: EmptyResponse = try!(self.call("torrent-start", &Request {
            ids: vec![s!(hash)]
        }));

        Ok(())
    }

    pub fn stop(&self, hash: &str) -> Result<()> {
        #[derive(RustcEncodable)] struct Request { ids: Vec<String> }

        let _: EmptyResponse = try!(self.call("torrent-stop", &Request {
            ids: vec![s!(hash)]
        }));

        Ok(())
    }

    pub fn set_processed(&self, hash: &str) -> Result<()> {
        #[allow(non_snake_case)]
        #[derive(RustcEncodable)] struct Request {
            ids: Vec<String>,
            downloadLimit: u64,
        }

        let _: EmptyResponse = try!(self.call("torrent-set", &Request {
            ids: vec![s!(hash)],
            downloadLimit: TORRENT_PROCESSED_MARKER,
        }));

        Ok(())
    }

    pub fn remove(&self, hash: &str) -> Result<()> {
        use rustc_serialize::json::{Object, ToJson};

        let mut request = Object::new();
        request.insert(s!("ids"), vec![s!(hash)].to_json());
        request.insert(s!("delete-local-data"), true.to_json());
        let request = request.to_json();

        let _: EmptyResponse = try!(self.call("torrent-remove", &request));

        Ok(())
    }

    fn call<I: Encodable, O: Decodable>(&self, method: &str, arguments: &I) -> Result<O> {
        self._call(method, arguments).or_else(|e| {
            trace!("RPC error: {}.", e);
            Err(e)
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

        let mut request_headers = Headers::new();
        request_headers.set(ContentType::json());

        if let (Some(user), Some(password)) = (self.user.as_ref(), self.password.as_ref()) {
            request_headers.set(Authorization(Basic {
                username: user.clone(),
                password: Some(password.clone()),
            }));
        }

        {
            let session_id = self.session_id.read().unwrap();
            if let Some(ref session_id) = *session_id {
                request_headers.set(XTransmissionSessionId(session_id.clone()));
            }
        }

        let request_json = try!(json::encode(&Request {
            method: s!(method),
            arguments: &arguments,
        }).map_err(|e| InternalError(format!("Failed to encode the request: {}", e))));

        trace!("RPC call: {}", request_json);
        let mut response = try!(self.client.post(&self.url)
            .headers(request_headers.clone())
            .body(&request_json)
            .send());

        if response.status == StatusCode::Conflict {
            let session_id = match response.headers.get::<XTransmissionSessionId>() {
                Some(session_id) => s!(**session_id),
                None => return Err(ProtocolError(format!(
                    "Got {} HTTP status code without {} header",
                    response.status, XTransmissionSessionId::header_name()))),
            };

            debug!("Session ID is expired. Got a new session ID.");

            request_headers.set(XTransmissionSessionId(session_id.clone()));
            *self.session_id.write().unwrap() = Some(session_id);

            response = try!(self.client.post(&self.url)
                .headers(request_headers)
                .body(&request_json)
                .send());
        }

        if response.status != StatusCode::Ok {
            return Err(InternalError(format!("Got {} HTTP status code", response.status)));
        }

        let content_type = match response.headers.get::<ContentType>() {
            Some(content_type) => s!(**content_type),
            None => return Err(ProtocolError(format!(
                "Got an HTTP response without {} header", ContentType::header_name()))),
        };

        match content_type {
            mime::Mime(mime::TopLevel::Application, mime::SubLevel::Json, _) => {},
            _ => return Err(ProtocolError(format!(
                "Got an HTTP response with invalid {}: '{}'",
                ContentType::header_name(), content_type)))
        }

        let mut body = String::new();
        try!(response.read_to_string(&mut body).map_err(|e| HyperError::Io(e)));
        trace!("RPC result: {}", body.trim());

        let response: Response<O> = try!(json::decode_str(&body).map_err(
            |e| ProtocolError(format!("Got an invalid response from server: {}", e))));

        if response.result != "success" {
            return Err(RpcError(GeneralError(response.result)))
        }

        match response.arguments {
            Some(arguments) => Ok(arguments),
            None => Err(ProtocolError(s!("Got a successful reply without arguments."))),
        }
    }
}


#[derive(Debug)]
pub enum TransmissionClientError {
    ConnectionError(io::Error),
    InternalError(String),
    ProtocolError(String),
    RpcError(TransmissionRpcError),
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
            ConnectionError(ref err) => write!(f, "{}", err),
            InternalError(ref err) | ProtocolError(ref err) => write!(f, "{}", err),
            RpcError(ref err) => write!(f, "{}", err),
        }
    }
}

impl From<HyperError> for TransmissionClientError {
    fn from(err: HyperError) -> TransmissionClientError {
        match err {
            HyperError::Io(err) => ConnectionError(err),
            _ => ProtocolError(err.to_string()),
        }
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


header! { (XTransmissionSessionId, "X-Transmission-Session-Id") => [String] }
