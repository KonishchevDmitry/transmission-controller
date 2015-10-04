use std::convert::From;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io;

use rustc_serialize::json;
use rustc_serialize::Decodable;
use rustc_serialize::json::{Json, Decoder};

#[derive(Debug, RustcDecodable)]
pub struct Config {
    download_dir: String,
}

#[derive(Debug)]
enum ConfigReadingError {
    IoError(io::Error),
    ParseError(String),
    ValidationError(String),
}
use self::ConfigReadingError::*;

pub type Result<T> = ::std::result::Result<T, ConfigReadingError>;

pub fn read_config(path: &str) -> Result<Config> {
    let mut file = try!(File::open(path));

    let mut json = try!(Json::from_reader(&mut file));
    try!(preprocess_config(&mut json));

    let mut decoder = Decoder::new(json);
    let config: Config = try!(Decodable::decode(&mut decoder));

    if !config.download_dir.starts_with("/") {
        return Err(ValidationError(s!("Invalid 'download-dir' value: it must be an absolute path")))
    }

    Ok(config)
}

fn preprocess_config(json: &mut Json) -> Result<()> {
    let mut obj = try!(json.as_object_mut().ok_or(
        ParseError(s!("JSON root element in not an object"))));

    for key in obj.keys().cloned().collect::<Vec<_>>() {
        if key.find("-").is_some() {
            let value = obj.remove(&key).unwrap();
            obj.insert(key.replace("-", "_"), value);
        }
    }

    Ok(())
}

impl fmt::Display for ConfigReadingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            IoError(ref err) => write!(f, "{}", err),
            ParseError(ref err) | ValidationError(ref err) => write!(f, "{}", err),
        }
    }
}

impl Error for ConfigReadingError {
    fn description(&self) -> &str {
        "config reading error"
    }
}

impl From<json::ParserError> for ConfigReadingError {
    fn from(err: json::ParserError) -> ConfigReadingError {
        use rustc_serialize::json::ParserError;

        match err {
            ParserError::IoError(err) => IoError(err),
            ParserError::SyntaxError(code, _, _) => ParseError(s!(json::error_str(code))),
        }
    }
}

impl From<json::DecoderError> for ConfigReadingError {
    fn from(err: json::DecoderError) -> ConfigReadingError {
        use rustc_serialize::json::DecoderError;

        match err {
            DecoderError::ParseError(err) => From::from(err),
            DecoderError::ApplicationError(err) => ParseError(err),
            DecoderError::MissingFieldError(field) => ParseError(format!("'{}' option is missing", field)),
            _ => ParseError(s!("JSON validation error")),
        }
    }
}

impl From<io::Error> for ConfigReadingError {
    fn from(err: io::Error) -> ConfigReadingError {
        IoError(err)
    }
}
