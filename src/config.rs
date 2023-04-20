#![allow(deprecated)] // We still use deprecated RustcDecodable here

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::util;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(rename = "download_dir")]
    pub download_dir: String,
    #[serde(rename = "rpc_enabled")]
    pub rpc_enabled: bool,
    #[serde(rename = "rpc_bind_address")]
    pub rpc_bind_address: String,
    #[serde(rename = "rpc_port")]
    pub rpc_port: u32,
    #[serde(rename = "rpc_authentication_required")]
    pub rpc_authentication_required: bool,
    #[serde(rename = "rpc_url")]
    pub rpc_url: String,
    #[serde(rename = "rpc_username")]
    pub rpc_username: String,
    #[serde(rename = "rpc_plain_password")]
    pub rpc_plain_password: Option<String>,
}

#[derive(Debug)]
pub enum ConfigReadingError {
    Io(io::Error),
    Parsing(String),
    Validation(String),
}
use self::ConfigReadingError::*;

pub type Result<T> = ::std::result::Result<T, ConfigReadingError>;

pub fn read_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let mut file = File::open(path)?;

    let config: Config = serde_json::from_reader(&mut file)?;
    validate_config(&config)?;

    Ok(config)
}

fn validate_config(config: &Config) -> Result<()> {
    let error = |e: &str| Err(Validation(s!(e)));

    if !config.download_dir.starts_with('/') {
        return error("Invalid 'download-dir' value: it must be an absolute path");
    }

    util::fs::check_directory(&config.download_dir).map_err(|e| Validation(format!(
        "Invalid 'download-dir': {}", e)))?;

    if !config.rpc_enabled {
        return error("RPC is disabled in config");
    }

    if config.rpc_bind_address.trim().is_empty() {
        return error("Invalid 'rpc-bind-address' value: it mustn't be empty");
    }

    if config.rpc_authentication_required && config.rpc_plain_password.is_none() {
        return error("'rpc-plain-password' is a required option when authentication is enabled");
    }

    Ok(())
}


impl Error for ConfigReadingError {
    fn description(&self) -> &str {
        "config reading error"
    }
}

impl fmt::Display for ConfigReadingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Io(ref err) => write!(f, "{}", err),
            Parsing(ref err) | Validation(ref err) => write!(f, "{}", err),
        }
    }
}

impl From<io::Error> for ConfigReadingError {
    fn from(err: io::Error) -> ConfigReadingError {
        Io(err)
    }
}

impl From<serde_json::Error> for ConfigReadingError {
    fn from(err: serde_json::Error) -> ConfigReadingError {
        Parsing(err.to_string())
    }
}
