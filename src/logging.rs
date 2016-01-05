use std::fmt;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex, Weak};

use itertools::Itertools;
use log;
use log::{LogRecord, LogLevel, LogMetadata, SetLoggerError};
use time;

use email::Mailer;
use util::time::{Duration, Timestamp};


pub fn init(level: LogLevel, target: Option<&'static str>, mailer: Option<Mailer>) -> Result<LoggerGuard, SetLoggerError> {
    let mut logger = Logger::new(level, target);

    let stderr_handler = Arc::new(StderrHandler::new(level >= LogLevel::Debug));
    logger.add_handler(stderr_handler.clone());

    if let Some(mailer) = mailer {
        logger.add_handler(Arc::new(
            EmailHandler::new("Transmission controller errors", mailer, stderr_handler)));
    }

    let logger = Arc::new(logger);

    try!(log::set_logger(|max_log_level| {
        max_log_level.set(level.to_log_level_filter());
        Box::new(LoggerWrapper { logger: logger.clone() })
    }));

    Ok(LoggerGuard { logger: Arc::downgrade(&logger) })
}


pub struct LoggerGuard {
    logger: Weak<Logger>
}

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        if let Some(logger) = self.logger.upgrade() {
            logger.flush();
        }
    }
}


struct LoggerWrapper {
    logger: Arc<Logger>
}

impl log::Log for LoggerWrapper {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        self.logger.enabled(metadata)
    }

    fn log(&self, record: &LogRecord) {
        self.logger.log(record)
    }
}


struct Logger {
    target: Option<&'static str>,
    level: LogLevel,
    handlers: Vec<Arc<LoggingHandler>>,
}

impl Logger {
    fn new(level: LogLevel, target: Option<&'static str>) -> Logger {
        Logger {
            target: target,
            level: level,
            handlers: Vec::new(),
        }
    }

    fn add_handler(&mut self, handler: Arc<LoggingHandler>) {
        self.handlers.push(handler);
    }

    fn flush(&self) {
        for handler in &self.handlers {
            handler.flush();
        }
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

        for handler in &self.handlers {
            handler.log(metadata.target(), location.file(), location.line(), metadata.level(), record.args());
        }
    }
}


struct StderrHandler {
    debug: bool,
    stderr: io::Stderr,
}

impl StderrHandler {
    fn new(debug: bool) -> StderrHandler {
        StderrHandler {
            debug: debug,
            stderr: io::stderr(),
        }
    }
}

impl LoggingHandler for StderrHandler {
    fn log(&self, target: &str, file: &str, line: u32, level: LogLevel, args: &fmt::Arguments) {
        let mut prefix = String::new();

        if self.debug {
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

        {
            let mut stderr = self.stderr.lock();
            if let Ok(_) = writeln!(stderr, "{}{}", prefix, args) {
                let _ = stderr.flush();
            }
        }
    }

    fn flush(&self) {
        let _ = self.stderr.lock().flush();
    }
}


struct EmailHandler {
    subject: String,
    mailer: Mailer,
    fallback_handler: Arc<LoggingHandler>,
    log: Mutex<EmailLog>,
}

impl EmailHandler {
    fn new(subject: &str, mailer: Mailer, fallback_handler: Arc<LoggingHandler>) -> EmailHandler {
        EmailHandler {
            mailer: mailer,
            subject: s!(subject),
            log: Mutex::new(EmailLog::new()),
            fallback_handler: fallback_handler,
        }
    }

    fn send(&self, message: &str) {
        if let Err(error) = self.mailer.send(&self.subject, message) {
            self.fallback_handler.log(module_path!(), file!(), line!(), LogLevel::Error,
                &format_args!("Failed to send an error via email: {}.", error));
        }
    }
}

impl LoggingHandler for EmailHandler {
    fn log(&self, _target: &str, _file: &str, _line: u32, level: LogLevel, args: &fmt::Arguments) {
        if level > LogLevel::Error {
            return;
        }

        let message = self.log.lock().unwrap().on_error(args.to_string());
        if let Some(message) = message {
            self.send(&message);
        }
    }

    fn flush(&self) {
        if let Some(message) = self.log.lock().unwrap().flush() {
            self.send(&message);
        }
    }
}


const FIRST_EMAIL_DELAY_TIME: Duration = 60;
const MIN_EMAIL_SENDING_PERIOD: Duration = 60 * 60;

struct EmailLog {
    errors: Vec<String>,
    last_flush_time: Timestamp,
}

impl EmailLog {
    fn new() -> EmailLog {
        assert!(FIRST_EMAIL_DELAY_TIME <= MIN_EMAIL_SENDING_PERIOD);

        EmailLog {
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


pub trait LoggingHandler: Send + Sync {
    fn log(&self, target: &str, file: &str, line: u32, level: LogLevel, args: &fmt::Arguments);
    fn flush(&self);
}
