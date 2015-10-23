extern crate argparse;
#[macro_use] extern crate enum_primitive;
#[macro_use] extern crate log;
#[macro_use] extern crate hyper;
extern crate mime;
extern crate num;
extern crate regex;
extern crate rustc_serialize;

#[macro_use] mod common;
mod config;
mod controller;
mod fs;
mod json;
mod logging;
mod periods;
mod transmissionrpc;
mod util;

use std::path::PathBuf;
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
    let user_home = try!(std::env::home_dir().ok_or(
        "Unable to determine user's home directory path."));
    let path = user_home.join(".config/transmission-daemon/settings.json");

    let config = try!(config::read_config(&path).map_err(
        |e| match e {
            ConfigReadingError::ValidationError(_) => {
                format!("Validation of '{}' configuration file failed: {}.", path.display(), e)
            },
            _ => format!("Error while reading '{}' configuration file: {}.", path.display(), e),
        }));

    debug!("Loaded config: {:?}", config);
    Ok(config)
}

fn parse_arguments(debug_level: &mut usize,
                   copy_to: &mut Option<PathBuf>, move_to: &mut Option<PathBuf>,
                   free_space_threshold: &mut Option<u8>) -> GenericResult<()> {
    let mut start_at_strings: Vec<String> = Vec::new();
    let mut copy_to_string: Option<String> = None;
    let mut move_to_string: Option<String> = None;

    {
        use argparse::{ArgumentParser, StoreOption, IncrBy, Collect};

        let mut parser = ArgumentParser::new();
        parser.set_description("Transmission controller daemon.");

        parser.refer(&mut start_at_strings).metavar("PERIOD").add_option(
            &["--start-at"], Collect, "time periods to start the torrents at in D-D/HH:MM-HH:MM format");
        parser.refer(&mut copy_to_string).metavar("PATH").add_option(
            &["--copy-to"], StoreOption, "directory to copy the torrents to");
        parser.refer(&mut move_to_string).metavar("PATH").add_option(
            &["--move-to"], StoreOption, "directory to move the copied torrents to");
        parser.refer(free_space_threshold).metavar("THRESHOLD").add_option(
            &["--free-space-threshold"], StoreOption,
            "free space threshold (%) after which downloaded torrents will be deleted until it won't be satisfied");
        parser.refer(debug_level).add_option(&["-d", "--debug"], IncrBy(1usize),
            "debug mode");

        parser.parse_args_or_exit();
    }

    try!(periods::parse_periods(&start_at_strings));

    let paths: Vec<(&mut Option<String>, &mut Option<PathBuf>)> = vec![
        (&mut copy_to_string, copy_to),
        (&mut move_to_string, move_to),
    ];

    for (path_string, path) in paths {
        if path_string.is_none() {
            continue
        }

        let user_path = PathBuf::from(&path_string.as_ref().unwrap());
        if user_path.is_relative() {
            return Err(From::from("You must specify only absolute paths in command line arguments"))
        }

        *path = Some(user_path);
    }

    if free_space_threshold.is_some() {
        let value = free_space_threshold.unwrap();
        if value > 100 {
            return Err(From::from(format!("Invalid free space threshold value: {}", value)))
        }
    }

    Ok(())
}

fn setup_logging(debug_level: usize) -> GenericResult<()> {
    let log_level = match debug_level {
        0 => LogLevel::Info,
        1 => LogLevel::Debug,
        _ => LogLevel::Trace,
    };

    let mut log_target = Some(module_path!());
    if log_level >= LogLevel::Trace {
        log_target = None;
    }

    Ok(try!(logging::init(log_level, log_target)))
}

fn daemon() -> GenericResult<i32> {
    let mut debug_level = 0;
    let mut copy_to: Option<PathBuf> = None;
    let mut move_to: Option<PathBuf> = None;
    let mut free_space_threshold: Option<u8> = None;

    try!(parse_arguments(&mut debug_level, &mut copy_to, &mut move_to, &mut free_space_threshold)
        .map_err(|e| format!("Command line arguments parsing error: {}", e)));

    try!(setup_logging(debug_level));
    info!("Starting the daemon...");

    let config = try!(load_config());
    let rpc_url = get_rpc_url(&config);
    debug!("Use RPC URL: {}.", rpc_url);

    let mut client = transmissionrpc::TransmissionClient::new(&rpc_url);
    if config.rpc_authentication_required {
        client.set_authentication(&config.rpc_username, &config.rpc_plain_password.as_ref().unwrap());
    }

    let mut controller = controller::Controller::new(
        client, &config.download_dir, free_space_threshold, copy_to, move_to);

    try!(controller.control());

    Ok(0)
}

fn main() {
    let exit_code = match daemon() {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(&mut std::io::stderr(), "Error: {}.", err);
            1
        }
    };

    process::exit(exit_code);
}
