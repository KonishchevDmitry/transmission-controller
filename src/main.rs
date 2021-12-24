extern crate argparse;
#[macro_use] extern crate chan;
extern crate chan_signal; // Attention: this crate calls pthread_sigmask() in crate's init() which masks all signals
extern crate dirs;
extern crate email as libemail;
#[macro_use] extern crate enum_primitive;
extern crate itertools;
extern crate lettre;
extern crate lettre_email;
extern crate libc;
#[macro_use] extern crate log;
extern crate mime;
extern crate num;
extern crate regex;
extern crate reqwest;
extern crate rustc_serialize;
extern crate time;

#[macro_use] mod common;
mod cli_args;
mod config;
mod consumer;
mod controller;
mod email;
mod json;
mod logging;
mod transmissionrpc;
mod util;

use std::io::Write;
use std::path::PathBuf;
use std::process;

use chan_signal::Signal;
use time::SteadyTime;

use common::GenericResult;
use config::{Config, ConfigReadingError};
use email::Mailer;

fn get_rpc_url(config: &Config) -> String {
    let mut url = format!("http://{host}:{port}{path}",
        host=config.rpc_bind_address, port=config.rpc_port, path=config.rpc_url);

    if !url.ends_with('/') {
        url.push('/');
    }

    url.push_str("rpc");

    url
}

fn load_config() -> GenericResult<Config> {
    let user_home = dirs::home_dir().ok_or(
        "Unable to determine user's home directory path")?;
    let path = user_home.join(".config/transmission-daemon/settings.json");

    let config = config::read_config(&path).map_err(
        |e| match e {
            ConfigReadingError::Validation(_) => {
                format!("Validation of '{}' configuration file failed: {}", path.display(), e)
            },
            _ => format!("Error while reading '{}' configuration file: {}", path.display(), e),
        })?;

    debug!("Loaded config: {:?}", config);
    Ok(config)
}

fn setup_logging(debug_level: usize, error_mailer: Option<Mailer>) -> GenericResult<logging::LoggerGuard> {
    let mut log_target = Some(module_path!());

    let log_level = match debug_level {
        0 => log::Level::Info,
        1 => log::Level::Debug,
        2 => log::Level::Trace,
        _ => {
            log_target = None;
            log::Level::Trace
        }
    };

    Ok(logging::init(log_level, log_target, error_mailer)?)
}

fn daemon() -> GenericResult<i32> {
    let signal_channel = chan_signal::notify(
        &[Signal::INT, Signal::TERM, Signal::QUIT]);

    let args = cli_args::parse().map_err(|e| format!(
        "Command line arguments parsing error: {}", e))?;

    let _logging = setup_logging(args.debug_level, args.error_mailer)?;
    info!("Starting the daemon...");

    let config = load_config()?;
    let rpc_url = get_rpc_url(&config);
    debug!("Use RPC URL: {}.", rpc_url);

    let mut client = transmissionrpc::TransmissionClient::new(&rpc_url);
    if config.rpc_authentication_required {
        client.set_authentication(&config.rpc_username, config.rpc_plain_password.as_ref().unwrap());
    }

    let mut controller = controller::Controller::new(
        client, args.action, args.action_periods,
        PathBuf::from(&config.download_dir), args.copy_to, args.move_to,
        args.seed_time_limit, args.free_space_threshold,
        args.notifications_mailer, args.torrent_downloaded_email_template);

    let tick = chan::tick_ms(5000);
    let start_time = SteadyTime::now();

    loop {
        if let Err(e) = controller.control() {
            // Transmission RPC may not respond for some time after startup. Increase the severity
            // of error messages to not send emails after each reboot.
            if (SteadyTime::now() - start_time).num_minutes() < 1 {
                warn!("{}.", e)
            } else {
                error!("{}.", e)
            }
        }

        chan_select! {
            signal_channel.recv() => {
                info!("Got a termination UNIX signal. Exiting...");
                break;
            },
            tick.recv() => {}
        }
    }

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
