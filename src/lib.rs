//! A logger for the [`log`] facade that writes to the systemd journal through its native protocol
//! when the process runs as a systemd service, and to stderr otherwise.

use std::{fmt, io::Write};

use env_logger::fmt::style::Style;
use jiff::{Timestamp, civil::DateTime, tz::TimeZone};
use log::{Record, SetLoggerError};

/// Installs the global logger, filtered by `RUST_LOG`.
///
/// When `RUST_LOG` is unset, the filter is the optional argument in the same syntax, such as
/// `init!("info,hyper=warn")`, or `info` without it.
///
/// A message is written as its level, the module path of its target and its text. The module
/// path is omitted for the root module of the calling crate, relative to the calling crate for
/// its submodules, and complete for other crates.
///
/// When stderr is connected to the systemd journal, every message is sent as one journal entry
/// with `PRIORITY`, `TARGET`, `SYSLOG_IDENTIFIER` and its source location. Otherwise each message
/// is written to stderr with the local time and colors.
///
/// # Errors
///
/// Fails when a global logger is already installed.
#[macro_export]
macro_rules! init {
    () => {
        $crate::init!("info")
    };
    ($default_filter:expr) => {
        $crate::init(env!("CARGO_CRATE_NAME"), $default_filter)
    };
}

#[doc(hidden)]
pub fn init(crate_name: &'static str, default_filter: &str) -> Result<(), SetLoggerError> {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| default_filter.to_owned());

    #[cfg(target_os = "linux")]
    if systemd_journal_logger::connected_to_journal() {
        match journal::JournalLogger::new(crate_name, &filter) {
            Ok(logger) => return logger.install(),
            Err(e) => {
                install_stderr(crate_name, &filter, false)?;
                log::warn!("cannot open the journal socket, writing to stderr: {e}");
                return Ok(());
            }
        }
    }

    install_stderr(crate_name, &filter, true)
}

fn install_stderr(
    crate_name: &'static str,
    filter: &str,
    timestamp: bool,
) -> Result<(), SetLoggerError> {
    let tz = TimeZone::system();
    env_logger::Builder::new()
        .parse_filters(filter)
        .format(move |f, record| {
            let line = StderrLine {
                time: timestamp.then(|| tz.to_datetime(Timestamp::now())),
                message: Message {
                    crate_name,
                    record,
                    level_style: f.default_level_style(record.level()),
                    path_style: Style::new().bold(),
                },
            };
            writeln!(f, "{line}")
        })
        .try_init()
}

/// A line on stderr. journald stamps the lines of a journal stream itself, so they carry no time.
struct StderrLine<'a> {
    time: Option<DateTime>,
    message: Message<'a>,
}

impl fmt::Display for StderrLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(time) = self.time {
            write!(f, "{} ", time.strftime("%Y-%m-%d %H:%M:%S%.3f"))?;
        }
        write!(f, "{}", self.message)
    }
}

/// The level, module path and text of a message.
struct Message<'a> {
    crate_name: &'a str,
    record: &'a Record<'a>,
    level_style: Style,
    path_style: Style,
}

impl fmt::Display for Message<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            level_style,
            path_style,
            ..
        } = self;
        let level = self.record.level();
        write!(f, "{level_style}{level:<5}{level_style:#} ")?;
        if let Some(prefix) = module_prefix(self.crate_name, self.record.target()) {
            write!(f, "{path_style}{prefix}{path_style:#} > ")?;
        }
        write!(f, "{}", self.record.args())
    }
}

/// Returns the module path written before a message, or `None` for the root module of the
/// calling crate.
fn module_prefix<'a>(crate_name: &str, target: &'a str) -> Option<&'a str> {
    match target.strip_prefix(crate_name) {
        Some("") => None,
        Some(sub) => Some(sub.strip_prefix("::").unwrap_or(target)),
        None => Some(target),
    }
}

#[cfg(target_os = "linux")]
mod journal {
    use env_logger::fmt::style::Style;
    use log::{Log, Metadata, Record, SetLoggerError};
    use systemd_journal_logger::JournalLog;

    use crate::Message;

    pub struct JournalLogger {
        crate_name: &'static str,
        filter: env_filter::Filter,
        journal: JournalLog,
    }

    impl JournalLogger {
        pub fn new(crate_name: &'static str, filter: &str) -> std::io::Result<Self> {
            Ok(Self {
                crate_name,
                filter: env_filter::Builder::new().parse(filter).build(),
                journal: JournalLog::new()?,
            })
        }

        pub fn install(self) -> Result<(), SetLoggerError> {
            let max_level = self.filter.filter();
            log::set_boxed_logger(Box::new(self))?;
            log::set_max_level(max_level);
            Ok(())
        }
    }

    impl Log for JournalLogger {
        fn enabled(&self, metadata: &Metadata) -> bool {
            self.filter.enabled(metadata)
        }

        fn log(&self, record: &Record) {
            if self.filter.matches(record) {
                let message = Message {
                    crate_name: self.crate_name,
                    record,
                    level_style: Style::new(),
                    path_style: Style::new(),
                };
                self.journal
                    .log(&record.to_builder().args(format_args!("{message}")).build());
            }
        }

        fn flush(&self) {}
    }
}

#[cfg(test)]
mod tests {
    use env_logger::fmt::style::{AnsiColor, Style};
    use jiff::civil::{DateTime, date};
    use log::{Level, Record};

    use super::{Message, StderrLine, module_prefix};

    fn stderr_line(
        time: Option<DateTime>,
        [level_style, path_style]: [Style; 2],
        level: Level,
        target: &str,
        text: &str,
    ) -> String {
        StderrLine {
            time,
            message: Message {
                crate_name: "app",
                record: &Record::builder()
                    .level(level)
                    .target(target)
                    .args(format_args!("{text}"))
                    .build(),
                level_style,
                path_style,
            },
        }
        .to_string()
    }

    #[test]
    fn message_starts_with_the_padded_level() {
        let plain = |level, target, text| stderr_line(None, [Style::new(); 2], level, target, text);
        assert_eq!(plain(Level::Info, "app", "started"), "INFO  started");
        assert_eq!(
            plain(Level::Error, "reqwest::connect", "503\nbody"),
            "ERROR reqwest::connect > 503\nbody",
        );
        assert_eq!(plain(Level::Warn, "app::core", "slow"), "WARN  core > slow");
    }

    #[test]
    fn stderr_line_starts_with_the_time_in_milliseconds() {
        let time = date(2026, 11, 17).at(9, 5, 3, 42_000_000);
        assert_eq!(
            stderr_line(
                Some(time),
                [Style::new(); 2],
                Level::Info,
                "app::core",
                "started"
            ),
            "2026-11-17 09:05:03.042 INFO  core > started",
        );
    }

    #[test]
    fn styles_wrap_the_level_and_the_module_path() {
        let styles = [AnsiColor::Green.on_default(), Style::new().bold()];
        assert_eq!(
            stderr_line(None, styles, Level::Info, "app::core", "started"),
            "\x1b[32mINFO \x1b[0m \x1b[1mcore\x1b[0m > started",
        );
    }

    #[test]
    fn module_prefix_is_relative_to_the_calling_crate() {
        assert_eq!(module_prefix("app", "app"), None);
        assert_eq!(module_prefix("app", "app::core"), Some("core"));
        assert_eq!(
            module_prefix("app", "app::core::cache"),
            Some("core::cache")
        );
        assert_eq!(
            module_prefix("app", "reqwest::connect"),
            Some("reqwest::connect")
        );
        assert_eq!(module_prefix("app", "appx::core"), Some("appx::core"));
    }
}
