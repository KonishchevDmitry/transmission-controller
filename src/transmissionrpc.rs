use std;
use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;
use std::io::Read;

use mime;
use rustc_serialize::Decoder;

use hyper::Client;
use hyper::error::Error as HyperError;
use hyper::status::StatusCode;
use hyper::header::{Header, Headers, Authorization, ContentType, Basic};

use json;
use json::{Encodable, Decodable};

pub struct TransmissionClient {
    url: String,
    user: Option<String>,
    password: Option<String>,
    session_id: Option<String>,
    client: Client,
}

enum_from_primitive! {
    #[derive(Debug, PartialEq)]
    pub enum TorrentStatus {
        Paused       = 0, // Paused
        CheckWait    = 1, // Queued for file checking
        Checking     = 2, // Checking files
        DownloadWait = 3, // Queued for downloading
        Downloading  = 4, // Downloading
        SeedWait     = 5, // Queued for seeding
        Seeding      = 6  // Seeding
    }
}

#[derive(Debug, RustcDecodable)]
pub struct Torrent {
    pub id: i32,
    pub status: TorrentStatus,
}

pub type Result<T> = std::result::Result<T, TransmissionClientError>;

impl TransmissionClient{
    pub fn new(url: &str) -> TransmissionClient {
        TransmissionClient {
            url: s!(url),
            user: None,
            password: None,
            session_id: None,
            // FIXME: timeout
            client: Client::new(),
        }
    }

    pub fn set_authentication(&mut self, user: &str, password: &str) {
        self.user = Some(s!(user));
        self.password = Some(s!(password));
    }

    pub fn get_torrents(&mut self) -> Result<Vec<Torrent>> {
        #[derive(RustcEncodable)]
        struct Request {
            fields: Vec<&'static str>,
        }

        #[derive(RustcDecodable)]
        struct Response {
            torrents: Vec<Torrent>,
        }

        let response: Response = try!(self.call("torrent-get", &Request {
            fields: vec!["id", "status"],
        }));

        Ok(response.torrents)
    }

    fn call<'a, I: Encodable, O: Decodable>(&mut self, method: &str, arguments: &'a I) -> Result<O> {
        self._call(method, arguments).or_else(|e| {
            trace!("RPC error: {}.", e);
            Err(e)
        })
    }

    fn _call<'a, I: Encodable, O: Decodable>(&mut self, method: &str, arguments: &'a I) -> Result<O> {
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

        if self.user.is_some() {
            request_headers.set(Authorization(Basic {
                username: self.user.as_ref().unwrap().clone(),
                password: Some(self.password.as_ref().unwrap().clone())
            }));
        }

        if self.session_id.is_some() {
            request_headers.set(XTransmissionSessionId(self.session_id.as_ref().unwrap().clone()));
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
            debug!("Session ID is expired.");

            let session_id = match response.headers.get::<XTransmissionSessionId>() {
                Some(session_id) => s!(**session_id),
                None => return Err(ProtocolError(format!(
                    "Got {} HTTP status code without {} header",
                    response.status, XTransmissionSessionId::header_name()))),
            };

            request_headers.set(XTransmissionSessionId(session_id.clone()));
            self.session_id = Some(session_id);

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
            return Err(ApiError(response.result))
        }

        match response.arguments {
            Some(arguments) => Ok(arguments),
            None => return Err(ProtocolError(s!("Got a successful reply without arguments."))),
        }
    }
}


#[derive(Debug)]
pub enum TransmissionClientError {
    ConnectionError(io::Error),
    InternalError(String),
    ProtocolError(String),
    ApiError(String),
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
            InternalError(ref err) | ProtocolError(ref err) | ApiError(ref err) => write!(f, "{}", err),
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


impl Decodable for TorrentStatus {
    fn decode<D: Decoder>(decoder: &mut D) -> std::result::Result<TorrentStatus, D::Error> {
        json::decode_enum(decoder, "torrent status")
    }
}


header! { (XTransmissionSessionId, "X-Transmission-Session-Id") => [String] }
