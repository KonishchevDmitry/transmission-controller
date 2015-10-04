extern crate hyper;

use std::io::Read;

use hyper::Client;
use hyper::status::StatusCode;
use hyper::header::{Headers, Authorization, ContentType, Basic};

use rustc_serialize::json;
use rustc_serialize::Encodable;
use rustc_serialize::json::{Json, Encoder};

pub struct TransmissionClient {
    url: String,
    user: Option<String>,
    password: Option<String>,
    client: Client,
}

impl TransmissionClient{
    pub fn new(url: &str) -> TransmissionClient {
        TransmissionClient {
            url: s!(url),
            user: None,
            password: None,
            // FIXME: timeout
            client: Client::new(),
        }
    }

    pub fn set_authentication(&mut self, user: &str, password: &str) {
        self.user = Some(s!(user));
        self.password = Some(s!(password));
    }

    pub fn get_torrents(&self) {
        #[derive(RustcEncodable)]
        struct Arguments {
            fields: Vec<String>,
        }

        let arguments = Arguments {
            fields: vec![],
        };

        self.call("torrent-get", &arguments)
    }

    fn call<T: Encodable>(&self, method: &str, arguments: &T) {
        let mut request_headers = Headers::new();
        request_headers.set(ContentType::json());
        request_headers.set_raw("X-Transmission-Session-Id", vec![b"6EXLiSE1u5AuRilhCHuv7dUe7eJ192EdbF5pJhUfygr8OYPc".to_vec()]);

        if self.user.is_some() {
            request_headers.set(Authorization(
               Basic {
                   username: self.user.as_ref().unwrap().clone(),
                   password: Some(self.password.as_ref().unwrap().clone())
               }
            ));
        }

        let request_json = format!("{{\"method\":\"torrent-get\",\"arguments\":{}}}", json::encode(&arguments).unwrap());

        let request = self.client.post(&self.url)
            .headers(request_headers)
            .body(&request_json);

        let mut response = request.send().unwrap();

        let mut body = String::new();
        response.read_to_string(&mut body).unwrap();

        if response.status== StatusCode::Conflict {
            println!("{} {}", response.status, response.headers);
        }

        println!("Response: {}", body);
    }
}
