use std;
use std::cmp;
use std::fmt;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use itertools::Itertools;
use log::{self, Log, Record, Level, Metadata, SetLoggerError};
use time::{Duration, SteadyTime};
use util::helpers::SelfArc;

use email::Mailer;


pub fn init(level: Level, target: Option<&'static str>, mailer: Option<Mailer>) -> Result<LoggerGuard, SetLoggerError> {
    let mut logger = Logger::new(level, target);

    let stderr_handler = StderrHandler::new(level >= Level::Debug);
    logger.add_handler(stderr_handler.clone());

    if let Some(mailer) = mailer {
        logger.add_handler(EmailHandler::new("Transmission controller errors", mailer, stderr_handler));
    }

    let logger = Arc::new(logger);

    log::set_boxed_logger(Box::new(LoggerWrapper { logger: logger.clone() }))?;
    log::set_max_level(level.to_level_filter());

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

impl Log for LoggerWrapper {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.logger.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        self.logger.log(record)
    }

    fn flush(&self) {
        self.logger.flush()
    }
}


struct Logger {
    target: Option<&'static str>,
    level: Level,
    handlers: Vec<Arc<dyn LoggingHandler>>,
}

impl Logger {
    fn new(level: Level, target: Option<&'static str>) -> Logger {
        Logger {
            target: target,
            level: level,
            handlers: Vec::new(),
        }
    }

    fn add_handler(&mut self, handler: Arc<dyn LoggingHandler>) {
        self.handlers.push(handler);
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level && (
            self.target.is_none() ||
            metadata.target() == self.target.unwrap() ||
            metadata.target().starts_with(&(s!(self.target.unwrap()) + "::"))
        )
    }

    fn log(&self, record: &Record) {
        let metadata = record.metadata();
        if !self.enabled(metadata) {
            return;
        }

        for handler in &self.handlers {
            handler.log(metadata.target(), record.file(), record.line(), metadata.level(), record.args());
        }
    }

    fn flush(&self) {
        for handler in &self.handlers {
            handler.flush();
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
    fn log(&self, target: &str, file: Option<&str>, line: Option<u32>, level: Level, args: &fmt::Arguments) {
        let mut prefix = String::new();

        if let (true, Some(file), Some(line)) = (self.debug, file, line) {
            let mut path = file;
            if path.starts_with("/") {
                path = target;
            }

            prefix = format!("{prefix}[{path:16.16}:{line:04}] ",
                             prefix=prefix, path=path, line=line)
        }

        prefix = prefix + match level {
            Level::Error => "E",
            Level::Warn  => "W",
            Level::Info  => "I",
            Level::Debug => "D",
            Level::Trace => "T",
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
    fallback_handler: Arc<dyn LoggingHandler>,
    log: Mutex<EmailLog>,
    arc: SelfArc<EmailHandler>,
}

impl EmailHandler {
    fn new(subject: &str, mailer: Mailer, fallback_handler: Arc<dyn LoggingHandler>) -> Arc<EmailHandler> {
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
            self.fallback_handler.log(module_path!(), Some(file!()), Some(line!()), Level::Error,
                &format_args!("Failed to send an error via email: {}.", error));
        }
    }
}

impl LoggingHandler for EmailHandler {
    fn log(&self, _target: &str, _file: Option<&str>, _line: Option<u32>, level: Level, args: &fmt::Arguments) {
        if level > Level::Error {
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
    fn log(&self, target: &str, file: Option<&str>, line: Option<u32>, level: Level, args: &fmt::Arguments);
    fn flush(&self);
}
