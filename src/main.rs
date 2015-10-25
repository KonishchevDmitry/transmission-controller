extern crate argparse;
#[macro_use] extern crate enum_primitive;
extern crate itertools;
#[macro_use] extern crate log;
#[macro_use] extern crate hyper;
extern crate lettre;
extern crate mime;
extern crate num;
extern crate regex;
extern crate rustc_serialize;
extern crate time;

#[macro_use] mod common;
mod config;
mod controller;
mod fs;
mod json;
mod logging;
mod periods;
mod transmissionrpc;
mod util;

use std::collections::HashMap;
use std::io::Write;
use std::iter::FromIterator;
use std::path::PathBuf;
use std::process;

use itertools::Itertools;
use log::LogLevel;

use common::GenericResult;
use config::{Config, ConfigReadingError};
use controller::Action;
use periods::WeekPeriods;

struct Arguments {
    debug_level: usize,

    action: Option<Action>,
    action_periods: WeekPeriods,

    copy_to: Option<PathBuf>,
    move_to: Option<PathBuf>,

    free_space_threshold: Option<u8>,
}

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

fn parse_arguments() -> GenericResult<Arguments> {
    let mut args = Arguments {
        debug_level: 0,

        action: None,
        action_periods: Vec::new(),

        copy_to: None,
        move_to: None,

        free_space_threshold: None,
    };

    let mut action_string: Option<String> = None;
    let mut period_strings: Vec<String> = Vec::new();
    let mut copy_to_string: Option<String> = None;
    let mut move_to_string: Option<String> = None;

    let action_map = HashMap::<String, Action>::from_iter(
        [Action::StartOrPause, Action::PauseOrStart]
        .iter().map(|&action| (action.to_string(), action)));

    {
        use argparse::{ArgumentParser, StoreOption, IncrBy, Collect};

        let mut parser = ArgumentParser::new();
        parser.set_description("Transmission controller daemon.");

        parser.refer(&mut action_string).metavar(&action_map.keys().join("|")).add_option(
            &["-a", "--action"], StoreOption, "action that will be taken according to the specified time periods");
        parser.refer(&mut period_strings).metavar("PERIOD").add_option(
            &["-p", "--period"], Collect, "time period in D[-D]/HH:MM-HH:MM format to start/stop the torrents at");
        parser.refer(&mut copy_to_string).metavar("PATH").add_option(
            &["-c", "--copy-to"], StoreOption, "directory to copy the torrents to");
        parser.refer(&mut move_to_string).metavar("PATH").add_option(
            &["-m", "--move-to"], StoreOption, "directory to move the copied torrents to");
        parser.refer(&mut args.free_space_threshold).metavar("THRESHOLD").add_option(
            &["-t", "--free-space-threshold"], StoreOption,
            "free space threshold (%) after which downloaded torrents will be deleted until it won't be satisfied");
        parser.refer(&mut args.debug_level).add_option(
            &["-d", "--debug"], IncrBy(1usize), "debug mode");

        parser.parse_args_or_exit();
    }

    match action_string {
        Some(string) => {
            match action_map.get(&string) {
                Some(action) => {
                    if period_strings.is_empty() {
                        return Err!("Action must be specified with time periods")
                    }
                    args.action = Some(*action);
                },
                None => {
                    return Err!("Invalid action: {}", string)
                }
            }
        }
        None => {
            if !period_strings.is_empty() {
                return Err!("Time periods must be specified with action")
            }
        }
    }

    args.action_periods = try!(periods::parse_periods(&period_strings));

    {
        let paths: Vec<(&mut Option<String>, &mut Option<PathBuf>)> = vec![
            (&mut copy_to_string, &mut args.copy_to),
            (&mut move_to_string, &mut args.move_to),
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
    }

    if args.free_space_threshold.is_some() {
        let value = args.free_space_threshold.unwrap();
        if value > 100 {
            return Err(From::from(format!("Invalid free space threshold value: {}", value)))
        }
    }

    Ok(args)
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
    let args = try!(parse_arguments().map_err(|e| format!(
        "Command line arguments parsing error: {}", e)));

    try!(setup_logging(args.debug_level));
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
        &config.download_dir, args.copy_to, args.move_to, args.free_space_threshold);

    try!(controller.control());

    Ok(0)
}

fn main() {
    if false {
        use lettre::transport::smtp::{SmtpTransport, SmtpTransportBuilder};
        use lettre::email::EmailBuilder;
        use lettre::transport::EmailTransport;
        use lettre::mailer::Mailer;

        // Create an email
        let email = EmailBuilder::new()
            // Addresses can be specified by the couple (email, alias)
            .to(("konishchev@gmail.com", "Тестовое имя"))
            .from("server@konishchev.ru")
            .subject("Hi, Hello world")
            .body("Hello world.")
            .build().unwrap();

        // Open a local connection on port 25
        let mut mailer =
        Mailer::new(SmtpTransportBuilder::localhost().unwrap().build());
        // Send the email
        let result = mailer.send(email);

        assert!(result.is_ok());
        return
    }
    let exit_code = match daemon() {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(&mut std::io::stderr(), "Error: {}.", err);
            1
        }
    };

    process::exit(exit_code);
}
