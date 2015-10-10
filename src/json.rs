use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;

use rustc_serialize::json;
use rustc_serialize::json::{Encoder, Decoder};

pub use rustc_serialize::{Encodable, Decodable};
pub use rustc_serialize::json::Json;


#[derive(Debug)]
pub enum JsonEncodingError {
    EncodingError(String),
}
use self::JsonEncodingError::*;

#[derive(Debug)]
pub enum JsonDecodingError {
    IoError(io::Error),
    ParseError(String),
}
use self::JsonDecodingError::*;


pub fn encode<T: Encodable>(object: &T) -> Result<String, JsonEncodingError> {
    let mut string = String::new();
    {
        let mut encoder = Encoder::new(&mut string);
        try!(object.encode(&mut encoder));
    }
    Ok(string)
}

pub fn from_reader(reader: &mut io::Read) -> Result<Json, JsonDecodingError> {
    Ok(try!(Json::from_reader(reader)))
}

pub fn decode<T: Decodable>(json: Json) -> Result<T, JsonDecodingError> {
    let mut decoder = Decoder::new(json);
    Ok(try!(Decodable::decode(&mut decoder)))
}

pub fn decode_str<T: Decodable>(string: &str) -> Result<T, JsonDecodingError> {
    decode(try!(Json::from_str(string)))
}


impl Error for JsonEncodingError {
    fn description(&self) -> &str {
        "JSON encoding error"
    }
}

impl fmt::Display for JsonEncodingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            EncodingError(ref err) => write!(f, "{}", err),
        }
    }
}

impl From<json::EncoderError> for JsonEncodingError {
    fn from(err: json::EncoderError) -> JsonEncodingError {
        use rustc_serialize::json::EncoderError;

        match err {
            EncoderError::FmtError(err) => EncodingError(err.to_string()),
            EncoderError::BadHashmapKey => EncodingError(s!("Invalid hash map key")),
        }
    }
}


impl Error for JsonDecodingError {
    fn description(&self) -> &str {
        "JSON decoding error"
    }
}

impl fmt::Display for JsonDecodingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            IoError(ref err) => write!(f, "{}", err),
            ParseError(ref err) => write!(f, "{}", err),
        }
    }
}

impl From<json::ParserError> for JsonDecodingError {
    fn from(err: json::ParserError) -> JsonDecodingError {
        use rustc_serialize::json::ParserError;

        match err {
            ParserError::IoError(err) => IoError(err),
            ParserError::SyntaxError(code, _, _) => ParseError(s!(json::error_str(code))),
        }
    }
}

impl From<json::DecoderError> for JsonDecodingError {
    fn from(err: json::DecoderError) -> JsonDecodingError {
        use rustc_serialize::json::DecoderError;

        match err {
            DecoderError::ParseError(err) => From::from(err),
            DecoderError::ApplicationError(err) => ParseError(err),
            DecoderError::MissingFieldError(field) => ParseError(format!("Required '{}' field is missing", field)),
            _ => ParseError(s!("JSON validation error")),
        }
    }
}
