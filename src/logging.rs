use std::fmt;
use std::io::Write;
use std::sync::{Arc, Mutex, Weak};

use itertools::Itertools;
use log;
use log::{LogRecord, LogLevel, LogMetadata, SetLoggerError};
use time;

use common::GenericResult;
use email::Mailer;
use util::time::{Duration, Timestamp};


pub fn init(level: LogLevel, target: Option<&'static str>, mailer: Option<Mailer>) -> Result<LoggerGuard, SetLoggerError> {
    let logger = Box::new(Logger::new(level, target));

    let email_error_logger = match mailer {
        Some(mailer) => Some(Arc::new(EmailErrorLogger::new(mailer, "Transmission controller errors"))),
        None => None
    };

    try!(log::set_logger(|max_log_level| {
        max_log_level.set(level.to_log_level_filter());
        logger
    }));

    Ok(LoggerGuard{logger: email_error_logger})
}


pub struct LoggerGuard {
    logger: Option<Arc<EmailErrorLogger>>,
}

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        if let Some(ref logger) = self.logger {
            logger.flush();
            // FIXME
            println!("GGGG");
        }
    }
}


struct Logger {
    target: Option<&'static str>,
    level: LogLevel,
    email_error_logger: Option<Arc<EmailErrorLogger>>,
}

impl Logger {
    fn new(level: LogLevel, target: Option<&'static str>) -> Logger {
        Logger {
            target: target,
            level: level,
            // FIXME
            email_error_logger: None,
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
    subject: String,
    messages: Mutex<EmailMessages>,
}

impl EmailErrorLogger {
    fn new(mailer: Mailer, subject: &str) -> EmailErrorLogger {
        assert!(FIRST_EMAIL_DELAY_TIME <= MIN_EMAIL_SENDING_PERIOD);

        EmailErrorLogger {
            mailer: mailer,
            subject: s!(subject),
            messages: Mutex::new(EmailMessages::new()),
        }
    }

    fn log(&self, record: &LogRecord) -> GenericResult<()> {
        let message = {
            let mut messages = self.messages.lock().unwrap();
            messages.on_error(record.args().to_string())
        };

        if let Some(message) = message {
            try!(self.send(&message))
        }

        Ok(())
    }

    fn flush(&self) -> GenericResult<()> {
        if let Some(message) = self.messages.lock().unwrap().flush() {
            try!(self.send(&message))
        }

        Ok(())
    }

    fn send(&self, message: &str) -> GenericResult<()> {
        Ok(try!(self.mailer.send(&self.subject, message)))
    }
}

struct EmailMessages {
    errors: Vec<String>,
    last_flush_time: Timestamp,
}

impl EmailMessages {
    fn new() -> EmailMessages {
        assert!(FIRST_EMAIL_DELAY_TIME <= MIN_EMAIL_SENDING_PERIOD);

        EmailMessages {
            errors: Vec::new(),
            last_flush_time: time::get_time().sec - MIN_EMAIL_SENDING_PERIOD + FIRST_EMAIL_DELAY_TIME,
        }
    }

    fn on_error(&mut self, error: String) -> Option<String> {
        self.errors.push(error);

        // FIXME: set timer to flush errors on time
        if time::get_time().sec - self.last_flush_time < MIN_EMAIL_SENDING_PERIOD {
            return None;
        }

        self.flush()
    }

    fn flush(&mut self) -> Option<String> {
        if self.errors.is_empty() {
            return None;
        }

        let message = s!("The following errors has occurred:\n") +
            &self.errors.iter().map(|error| s!("* ") + &error).join("\n");

        self.errors.clear();
        self.last_flush_time = time::get_time().sec;

        Some(message)
    }
}
