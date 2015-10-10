#[macro_use] extern crate log;
#[macro_use] extern crate hyper;
extern crate rustc_serialize;
extern crate mime;

#[macro_use] mod common;
mod config;
mod json;
mod logging;
mod transmissionrpc;

use std::process;
use std::io::Write;

use log::LogLevel;

use common::GenericResult;
use config::{Config, ConfigReadingError};

fn get_rpc_url(config: &Config) -> String {
    let mut url = format!("http://{host}:{port}{path}",
        host=config.rpc_bind_address, port=config.rpc_port, path=config.rpc_url);

    if !url.ends_with("/") {
        url.push_str("/");
    }

    url.push_str("rpc");

    url
}

fn load_config() -> GenericResult<Config> {
    let path = "settings.json";

    let config = try!(config::read_config(path).map_err(
        |e| match e {
            ConfigReadingError::ValidationError(_) => {
                format!("Validation of '{}' configuration file failed: {}.", path, e)
            },
            _ => format!("Error while reading '{}' configuration file: {}.", path, e),
        }));

    debug!("Loaded config: {:?}", config);
    Ok(config)
}

fn daemon() -> GenericResult<i32> {
    //let log_level = LogLevel::Debug;
    let log_level = LogLevel::Trace;

    let mut log_target = Some(module_path!());
    if log_level >= LogLevel::Trace {
        log_target = None;
    }

    try!(logging::init(log_level, log_target));
    info!("Starting the daemon...");

    let config = try!(load_config());

    let rpc_url = get_rpc_url(&config);
    debug!("Use RPC URL: {}.", rpc_url);

    let mut client = transmissionrpc::TransmissionClient::new(&rpc_url);
    if config.rpc_authentication_required {
        client.set_authentication(&config.rpc_username, &config.rpc_plain_password.as_ref().unwrap());
    }

    info!("{:?}", client.get_torrents().unwrap());

    Ok(0)
}

fn main() {
    let exit_code = match daemon() {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(&mut std::io::stderr(), "Error: {}", err);
            1
        }
    };

    process::exit(exit_code);
}
