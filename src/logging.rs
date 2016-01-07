use std;
use std::cmp;
use std::fmt;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use itertools::Itertools;
use log;
use log::{LogRecord, LogLevel, LogMetadata, SetLoggerError};
use time::{Duration, SteadyTime};
use util::helpers::SelfArc;

use email::Mailer;


pub fn init(level: LogLevel, target: Option<&'static str>, mailer: Option<Mailer>) -> Result<LoggerGuard, SetLoggerError> {
    let mut logger = Logger::new(level, target);

    let stderr_handler = StderrHandler::new(level >= LogLevel::Debug);
    logger.add_handler(stderr_handler.clone());

    if let Some(mailer) = mailer {
        logger.add_handler(EmailHandler::new("Transmission controller errors", mailer, stderr_handler));
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
    fn new(debug: bool) -> Arc<StderrHandler> {
        Arc::new(StderrHandler {
            debug: debug,
            stderr: io::stderr(),
        })
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
    arc: SelfArc<EmailHandler>,
}

impl EmailHandler {
    fn new(subject: &str, mailer: Mailer, fallback_handler: Arc<LoggingHandler>) -> Arc<EmailHandler> {
        let handler = Arc::new(EmailHandler {
            mailer: mailer,
            subject: s!(subject),
            fallback_handler: fallback_handler,
            log: Mutex::new(EmailLog::new()),
            arc: SelfArc::new(),
        });
        handler.arc.init(&handler);
        handler
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

        {
            let mut log = self.log.lock().unwrap();
            log.on_error(args.to_string());

            if log.flush_time.is_some() && log.flush_thread.is_none() {
                let weak_self = self.arc.get_weak();
                log.flush_thread = Some(thread::spawn(move || {
                    email_log_flush_thread(weak_self)
                }));
            }
        }
    }

    fn flush(&self) {
        if let Some(message) = self.log.lock().unwrap().flush() {
            self.send(&message);
        }
    }
}

fn email_log_flush_thread(weak: Weak<EmailHandler>) {
    loop {
        let strong = match weak.upgrade() {
            Some(strong) => strong,
            None => break,
        };

        let flush_time = {
            let mut log = strong.log.lock().unwrap();

            match log.flush_time {
                Some(flush_time) => flush_time,
                None => {
                    log.flush_thread = None;
                    break;
                }
            }
        };

        drop(strong);

        let sleep_time = (flush_time - SteadyTime::now()).num_milliseconds();
        if sleep_time > 0 {
            thread::park_timeout(std::time::Duration::from_millis(sleep_time as u64));
            continue;
        }

        if let Some(strong) = weak.upgrade() {
            strong.flush();
        } else {
            break;
        }
    }
}


struct EmailLog {
    errors: Vec<String>,
    flush_time: Option<SteadyTime>,
    last_flush_time: Option<SteadyTime>,
    flush_thread: Option<thread::JoinHandle<()>>,
}

impl EmailLog {
    fn new() -> EmailLog {
        EmailLog {
            errors: Vec::new(),
            flush_time: None,
            last_flush_time: None,
            flush_thread: None,
        }
    }

    fn on_error(&mut self, error: String) {
        if self.errors.is_empty() {
            let first_email_delay_time = Duration::minutes(1);
            let min_email_sending_period = Duration::hours(1);

            let mut flush_time = SteadyTime::now() + first_email_delay_time;
            if let Some(last_flush_time) = self.last_flush_time {
                flush_time = cmp::max(flush_time, last_flush_time + min_email_sending_period);
            }

            self.flush_time = Some(flush_time);
        }

        self.errors.push(error);
    }

    fn flush(&mut self) -> Option<String> {
        if self.errors.is_empty() {
            return None;
        }

        let message = s!("The following errors has occurred:\n") +
            &self.errors.iter().map(|error| s!("* ") + &error).join("\n");

        self.errors.clear();
        self.flush_time = None;
        self.last_flush_time = Some(SteadyTime::now());

        Some(message)
    }
}

impl Drop for EmailLog {
    fn drop(&mut self) {
        if let Some(ref flush_thread) = self.flush_thread {
            flush_thread.thread().unpark();
        }
    }
}


pub trait LoggingHandler: Send + Sync {
    fn log(&self, target: &str, file: &str, line: u32, level: LogLevel, args: &fmt::Arguments);
    fn flush(&self);
}
