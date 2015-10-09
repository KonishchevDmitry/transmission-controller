use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;
use std::io::Read;

use hyper::Client;
use hyper::error::Error as HyperError;
use hyper::status::StatusCode;
use hyper::header::{Headers, Authorization, ContentType, Basic};

use mime;
use mime::Mime;

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
        match self._call(method, arguments).map_err(|e| format!("Test: {}", e)) {
            Ok(_) => {},
            Err(err) => warn!("{}", err),
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

        // FIXME: HERE
        if self.session_id.is_some() {
            request_headers.set(XTransmissionSessionId(self.session_id.as_ref().unwrap().clone()));
        }

        let request_json = json::encode(&Request {
            method: s!(method),
            arguments: &arguments,
        }).unwrap();

        let mut request = self.client.post(&self.url)
            .headers(request_headers.clone())
            .body(&request_json);

        let mut response = try!(request.send());

        if response.status == StatusCode::Conflict {
            {
            self.session_id = Some((**response.headers.get::<XTransmissionSessionId>().unwrap()).clone());
            request_headers.set(XTransmissionSessionId(self.session_id.as_ref().unwrap().clone()));
            }

            request = self.client.post(&self.url)
                .headers(request_headers)
                .body(&request_json);

            response = try!(request.send());
        }

        let content_type = (**response.headers.get::<ContentType>().unwrap()).clone();

        match content_type {
                Mime(mime::TopLevel::Application, mime::SubLevel::Json, _) => println!("matched json!"),
                    _ => ()
        }

        let mut body = String::new();
        response.read_to_string(&mut body).unwrap();

        // FIXME: unwraps
        println!("Response: {}", body);
        Ok(())
    }
}


#[derive(Debug)]
pub enum TransmissionClientError {
    ConnectionError(io::Error),
    ProtocolError(HyperError),
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
            ProtocolError(ref err) => write!(f, "{}", err),
        }
    }
}

impl From<HyperError> for TransmissionClientError {
    fn from(err: HyperError) -> TransmissionClientError {
        match err {
            HyperError::Io(err) => ConnectionError(err),
            _ => ProtocolError(err),
        }
    }
}


header! { (XTransmissionSessionId, "X-Transmission-Session-Id") => [String] }
