use std;
use std::convert::From;
use std::error::Error;
use std::fmt;
use std::io;

use num::FromPrimitive;

use rustc_serialize;
use rustc_serialize::Decoder as DecoderTrait;
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

pub fn from_str(string: &str) -> Result<Json, JsonDecodingError> {
    Ok(try!(Json::from_str(string)))
}

pub fn decode<T: Decodable>(mut json: Json) -> Result<T, JsonDecodingError> {
    try!(unify_json(&mut json));
    let mut decoder = Decoder::new(json);
    Ok(try!(Decodable::decode(&mut decoder)))
}

pub fn decode_enum<D: DecoderTrait, E: FromPrimitive>(decoder: &mut D, name: &str) -> std::result::Result<E, D::Error> {
    let value = try!(decoder.read_u64());

    match FromPrimitive::from_u64(value) {
        Some(value) => Ok(value),
        None => Err(decoder.error(&format!("Invalid {} enum value: {}", name, value))),
    }
}

pub fn decode_str<T: Decodable>(string: &str) -> Result<T, JsonDecodingError> {
    decode(try!(from_str(string)))
}


// Converts json object to representation that can be mapped to a Decodable by rustc_serialize.
// rustc_serialize doesn't support custom field names, so we have to replace '-' with '_' in field
// names to be able to decode the objects.
fn unify_json(json: &mut Json) -> Result<(), JsonDecodingError> {
    use json::Json::*;

    match *json {
        I64(_) | U64(_) | F64(_) | Boolean(_) | String(_) | Null => Ok(()),
        Object(ref mut obj) => unify_object(obj),
        Array(ref mut array) => unify_array(array),
    }
}

fn unify_object(obj: &mut json::Object) -> Result<(), JsonDecodingError> {
    for key in obj.keys().cloned().collect::<Vec<_>>() {
        unify_json(obj.get_mut(&key).unwrap());

        if key.find("-").is_none() {
            continue;
        }

        let unified_key = key.replace("-", "_");
        if obj.contains_key(&unified_key) {
            return Err(JsonDecodingError::ParseError(format!(
                "Failed to unify an object: it contains both '{}' and '{}' keys", key, unified_key)));
        }

        let value = obj.remove(&key).unwrap();
        obj.insert(unified_key, value);
    }

    Ok(())
}

fn unify_array(array: &mut json::Array) -> Result<(), JsonDecodingError> {
    for json in array {
        try!(unify_json(json));
    }

    Ok(())
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
        trace!("JSON parsing error: {}.", err);

        match err {
            DecoderError::ParseError(err) => From::from(err),
            DecoderError::ApplicationError(err) => ParseError(err),
            DecoderError::MissingFieldError(field) => ParseError(format!("Required '{}' field is missing", field)),
            _ => ParseError(format!("JSON validation error: {}", err)),
        }
    }
}
