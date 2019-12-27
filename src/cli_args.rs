use std::collections::HashMap;
use std::iter::FromIterator;
use std::path::PathBuf;

use itertools::Itertools;

use common::GenericResult;
use controller::Action;
use email::{Mailer, EmailTemplate};
use util;
use util::time::{Duration, WeekPeriods};

pub struct Arguments {
    pub debug_level: usize,

    pub action: Option<Action>,
    pub action_periods: WeekPeriods,

    pub copy_to: Option<PathBuf>,
    pub move_to: Option<PathBuf>,

    pub seed_time_limit: Option<Duration>,
    pub free_space_threshold: Option<u8>,

    pub error_mailer: Option<Mailer>,
    pub notifications_mailer: Option<Mailer>,
    pub torrent_downloaded_email_template: EmailTemplate,
}

pub fn parse() -> GenericResult<Arguments> {
    let mut args = Arguments {
        debug_level: 0,

        action: None,
        action_periods: WeekPeriods::new(),

        copy_to: None,
        move_to: None,

        seed_time_limit: None,
        free_space_threshold: None,

        error_mailer: None,
        notifications_mailer: None,
        torrent_downloaded_email_template: EmailTemplate::new(
            "Downloaded: {{name}}", "{{name}} torrent has been downloaded."),
    };

    let mut action_string: Option<String> = None;
    let mut period_strings: Vec<String> = Vec::new();
    let mut copy_to_string: Option<String> = None;
    let mut move_to_string: Option<String> = None;
    let mut seed_time_limit: Option<String> = None;

    let mut email_from: Option<String> = None;
    let mut email_errors_to: Option<String> = None;
    let mut email_notifications_to: Option<String> = None;
    let mut torrent_downloaded_email_template: Option<String> = None;

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
        parser.refer(&mut seed_time_limit).metavar("DURATION").add_option(
            &["-l", "--seed-time-limit"], StoreOption,
            "seeding time (in $number{m|h|d} format) after which downloaded torrents will be deleted");
        parser.refer(&mut args.free_space_threshold).metavar("THRESHOLD").add_option(
            &["-s", "--free-space-threshold"], StoreOption,
            "free space threshold (%) after which downloaded torrents will be deleted until it won't be satisfied");
        parser.refer(&mut email_from).metavar("ADDRESS").add_option(
            &["-f", "--email-from"], StoreOption, "address to send mail from");
        parser.refer(&mut email_errors_to).metavar("ADDRESS").add_option(
            &["-e", "--email-errors"], StoreOption, "address to send errors to");
        parser.refer(&mut email_notifications_to).metavar("ADDRESS").add_option(
            &["-n", "--email-notifications"], StoreOption, "address to send notifications to");
        parser.refer(&mut torrent_downloaded_email_template).metavar("PATH").add_option(
            &["-t", "--torrent-downloaded-email-template"], StoreOption, "template of 'torrent downloaded' notification");
        parser.refer(&mut args.debug_level).add_option(
            &["-d", "--debug"], IncrBy(1usize), "debug mode");

        parser.parse_args_or_exit();
    }

    if let Some(action_string) = action_string {
        match action_map.get(&action_string) {
            Some(action) => {
                if period_strings.is_empty() {
                    return Err!("Action must be specified with time periods");
                }
                args.action = Some(*action);
            },
            None => return Err!("Invalid action: {}", action_string)
        }
    } else {
        if !period_strings.is_empty() {
            return Err!("Time periods must be specified with action");
        }
    }

    args.action_periods = util::time::parse_periods(&period_strings)?;

    {
        let paths: Vec<(&mut Option<String>, &mut Option<PathBuf>)> = vec![
            (&mut copy_to_string, &mut args.copy_to),
            (&mut move_to_string, &mut args.move_to),
        ];

        for (path_string, path) in paths {
            if path_string.is_none() {
                continue;
            }

            let user_path = PathBuf::from(&path_string.as_ref().unwrap());
            if user_path.is_relative() {
                return Err!("You must specify only absolute paths in command line arguments");
            }

            util::fs::check_directory(&user_path)?;

            *path = Some(user_path);
        }
    }

    if let Some(ref duration) = seed_time_limit {
        args.seed_time_limit = Some(util::time::parse_duration(&duration)?);
    }

    if let Some(ref threshold) = args.free_space_threshold {
        if *threshold > 100 {
            return Err!("Invalid free space threshold value: {}", threshold);
        }
    }

    if let Some(ref to) = email_errors_to {
        if let Some(ref from) = email_from {
            args.error_mailer = Some(Mailer::new(&from, &to)?);
        } else {
            return Err!("--email-from must be specified when configuring email notifications");
        }
    }

    if let Some(to) = email_notifications_to {
        args.notifications_mailer = match email_from {
            Some(ref from) => Some(Mailer::new(&from, &to)?),
            None => return Err!("--email-from must be specified when configuring email notifications"),
        };
    }

    if let Some(path) = torrent_downloaded_email_template {
        args.torrent_downloaded_email_template = EmailTemplate::new_from_file(&path)
            .map_err(|e| format!("Error while reading email template: {}", e))?;
    }

    Ok(args)
}
