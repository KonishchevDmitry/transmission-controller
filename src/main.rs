extern crate argparse;
extern crate email as libemail;
#[macro_use] extern crate enum_primitive;
#[macro_use] extern crate hyper;
extern crate itertools;
extern crate lettre;
#[macro_use] extern crate log;
extern crate mime;
extern crate num;
extern crate regex;
extern crate rustc_serialize;
extern crate time;

#[macro_use] mod common;
mod cli_args;
mod config;
mod controller;
mod email;
mod fs;
mod json;
mod logging;
mod periods;
mod transmissionrpc;
mod util;

use std::io::Write;
use std::path::PathBuf;
use std::process;

use itertools::Itertools;
use log::LogLevel;

use common::{EmptyResult, GenericResult};
use config::{Config, ConfigReadingError};
use email::Mailer;

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

fn setup_logging(debug_level: usize, error_mailer: Option<Mailer>) -> EmptyResult {
    let mut log_target = Some(module_path!());

    let log_level = match debug_level {
        0 => LogLevel::Info,
        1 => LogLevel::Debug,
        2 => LogLevel::Trace,
        _ => {
            log_target = None;
            LogLevel::Trace
        }
    };

    Ok(try!(logging::init(log_level, log_target, error_mailer)))
}

fn daemon() -> GenericResult<i32> {
    let args = try!(cli_args::parse().map_err(|e| format!(
        "Command line arguments parsing error: {}", e)));

    try!(setup_logging(args.debug_level, args.error_mailer));
    info!("Starting the daemon...");

    let config = try!(load_config());
    let rpc_url = get_rpc_url(&config);
    debug!("Use RPC URL: {}.", rpc_url);

    let mut client = transmissionrpc::TransmissionClient::new(&rpc_url);
    if config.rpc_authentication_required {
        client.set_authentication(&config.rpc_username, &config.rpc_plain_password.as_ref().unwrap());
    }

    let mut controller = controller::Controller::new(
        client, args.action, args.action_periods,
        PathBuf::from(&config.download_dir), args.copy_to, args.move_to, args.free_space_threshold,
        args.notifications_mailer, args.torrent_downloaded_email_template);

    loop {
        // FIXME: Listen to UNIX signals
        if let Err(e) = controller.control() {
            error!("{}.", e)
        }
        std::thread::sleep_ms(60 * 1000);
    }
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
