use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;
use std::io::Read;

use hyper::Client;
use hyper::error::Error as HyperError;
use hyper::status::StatusCode;
use hyper::header::{Header, Headers, Authorization, ContentType, Basic};

use mime;

use json;
use json::Encodable;

pub struct TransmissionClient {
    url: String,
    user: Option<String>,
    password: Option<String>,
    session_id: Option<String>,
    client: Client,
}

pub type Result<T> = ::std::result::Result<T, TransmissionClientError>;

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

    pub fn get_torrents(&mut self) {
        #[derive(RustcEncodable)]
        struct Arguments {
            fields: Vec<String>,
        }

        let arguments = Arguments {
            fields: vec![s!("id")],
        };

        self.call("torrent-get", &arguments)
    }

    fn call<'a, T: Encodable>(&mut self, method: &str, arguments: &'a T) {
        match self._call(method, arguments) {
            Ok(_) => {},
            Err(err) => trace!("RPC error: {}.", err),
        }
    }
    fn _call<'a, T: Encodable>(&mut self, method: &str, arguments: &'a T) -> Result<()> {
        #[derive(RustcEncodable)]
        struct Request<'a, T: 'a> {
            method: String,
            arguments: &'a T,
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

        trace!("RPC call: {}...", request_json);
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
        trace!("RPC result: {}.", body);

        // (1) A required "result" string whose value MUST be "success" on success,
        //     or an error string on failure.
        // (2) An optional "arguments" object of key/value pairs
        // (3) An optional "tag" number as described in 2.1.

        Ok(())
    }
}


#[derive(Debug)]
pub enum TransmissionClientError {
    ConnectionError(io::Error),
    InternalError(String),
    ProtocolError(String),
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


header! { (XTransmissionSessionId, "X-Transmission-Session-Id") => [String] }
