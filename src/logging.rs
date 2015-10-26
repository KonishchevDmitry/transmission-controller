use std::io::Write;

use log;
use log::{LogRecord, LogLevel, LogMetadata, SetLoggerError};
use time;

use email::Mailer;

struct Logger {
    target: Option<&'static str>,
    level: LogLevel,

    mailer: Option<Mailer>,
    errors: Vec<String>,
    last_email_time: i64,
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
            return
        }

        let mut prefix = String::new();

        if self.level >= LogLevel::Debug {
            let location = record.location();

            let mut path = location.file();
            if path.starts_with("/") {
                path = metadata.target();
            }

            prefix = format!("{prefix}[{path:16.16}:{line:04}] ",
                prefix=prefix, path=path, line=location.line())
        }

        prefix = prefix + match record.level() {
            LogLevel::Error => "E",
            LogLevel::Warn  => "W",
            LogLevel::Info  => "I",
            LogLevel::Debug => "D",
            LogLevel::Trace => "T",
        } + ": ";

        let _ = writeln!(&mut ::std::io::stderr(), "{}{}", prefix, record.args());

        // FIXME
        if self.mailer.is_some() {
            /*
            self.errors.push(format!("{}", record.args()));

            if time::get_time().sec - self.last_email_time >= 10 * 60 * 60 {
                let body = s!("The following errors has occurred:\n") + &self.errors.join("\n");
                self.errors.clear();

                self.mailer.unwrap().send("Transmission controller errors", &body);
            }
            */
        }
    }
}

pub fn init(level: LogLevel, target: Option<&'static str>, mailer: Option<Mailer>) -> Result<(), SetLoggerError> {
    log::set_logger(|max_log_level| {
        max_log_level.set(level.to_log_level_filter());
        Box::new(Logger {
            target: target,
            level: level,

            mailer: mailer,
            errors: Vec::new(),
            last_email_time: time::get_time().sec,
        })
    })
}
