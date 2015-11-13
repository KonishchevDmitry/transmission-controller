use std::fmt;
use std::io::Write;
use std::sync::Mutex;

use itertools::Itertools;
use log;
use log::{LogRecord, LogLevel, LogMetadata, SetLoggerError};
use time;

use common::GenericResult;
use email::Mailer;
use util::time::{Duration, Timestamp};

struct Logger {
    target: Option<&'static str>,
    level: LogLevel,
    email_error_logger: Option<Mutex<EmailErrorLogger>>,
}

impl Logger {
    fn new(level: LogLevel, target: Option<&'static str>, mailer: Option<Mailer>) -> Logger {
        Logger {
            target: target,
            level: level,
            email_error_logger: match mailer {
                Some(mailer) => Some(Mutex::new(EmailErrorLogger::new(mailer))),
                None => None
            },
        }
    }

    fn log_record(&self, target: &str, file: &str, line: u32, level: LogLevel, args: &fmt::Arguments) {
        let mut prefix = String::new();

        if self.level >= LogLevel::Debug {
            let mut path = file;
            if path.starts_with("/") {
                path = target;
            }

            prefix = format!("{prefix}[{path:16.16}:{line:04}] ",
                prefix=prefix, path=path, line=line)
        }

        prefix = prefix + match level {
            LogLevel::Error => "E",
            LogLevel::Warn  => "W",
            LogLevel::Info  => "I",
            LogLevel::Debug => "D",
            LogLevel::Trace => "T",
        } + ": ";

        let _ = writeln!(&mut ::std::io::stderr(), "{}{}", prefix, args);
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        metadata.level() <= self.level && (
            self.target.is_none() ||
            metadata.target() == self.target.unwrap() ||
            metadata.target().starts_with(&(s!(self.target.unwrap()) + "::"))
        )
    }

    fn log(&self, record: &LogRecord) {
        let metadata = record.metadata();
        if !self.enabled(metadata) {
            return;
        }

        let location = record.location();
        self.log_record(metadata.target(), location.file(), location.line(), metadata.level(), record.args());

        if metadata.level() > LogLevel::Error {
            return;
        }

        if let Some(ref logger) = self.email_error_logger {
            let mut logger = logger.lock().unwrap();
            if let Err(error) = logger.log(record) {
                self.log_record(module_path!(), file!(), line!(), LogLevel::Error,
                                &format_args!("Failed to send an error via email: {}.", error));
            }
        }
    }
}


const FIRST_EMAIL_DELAY_TIME: Duration = 60;
const MIN_EMAIL_SENDING_PERIOD: Duration = 60 * 60;

// FIXME: flush errors on shutdown
struct EmailErrorLogger {
    mailer: Mailer,
    errors: Vec<String>,
    last_email_time: Timestamp,
}

impl EmailErrorLogger {
    fn new(mailer: Mailer) -> EmailErrorLogger {
        assert!(FIRST_EMAIL_DELAY_TIME <= MIN_EMAIL_SENDING_PERIOD);

        EmailErrorLogger {
            mailer: mailer,
            errors: Vec::new(),
            last_email_time: time::get_time().sec - MIN_EMAIL_SENDING_PERIOD + FIRST_EMAIL_DELAY_TIME,
        }
    }

    fn log(&mut self, record: &LogRecord) -> GenericResult<()> {
        self.errors.push(record.args().to_string());

        if time::get_time().sec - self.last_email_time < MIN_EMAIL_SENDING_PERIOD {
            return Ok(())
        }

        let message = s!("The following errors has occurred:\n") +
            &self.errors.iter().map(|error| s!("* ") + &error).join("\n");
        self.errors.clear();

        self.last_email_time = time::get_time().sec;
        Ok(try!(self.mailer.send("Transmission controller errors", &message)))
    }
}

pub fn init(level: LogLevel, target: Option<&'static str>, mailer: Option<Mailer>) -> Result<(), SetLoggerError> {
    log::set_logger(|max_log_level| {
        max_log_level.set(level.to_log_level_filter());
        Box::new(Logger::new(level, target, mailer))
    })
}
