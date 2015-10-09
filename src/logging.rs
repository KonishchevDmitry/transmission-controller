extern crate log;

use std::io::Write;
use log::{LogRecord, LogLevel, LogMetadata, SetLoggerError};

struct StderrLogger {
    target: Option<&'static str>,
    level: LogLevel,
}

impl log::Log for StderrLogger {
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
    }
}

pub fn init(level: LogLevel, target: Option<&'static str>) -> Result<(), SetLoggerError> {
    log::set_logger(|max_log_level| {
        max_log_level.set(level.to_log_level_filter());
        Box::new(StderrLogger {
            target: target,
            level: level
        })
    })
}
