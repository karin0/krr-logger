//! A logger for the [`log`] facade that writes to the systemd journal through its native protocol
//! when the process runs as a systemd service, and to stderr otherwise.

use std::io::Write;

use env_logger::fmt::style::Style;
use jiff::{Timestamp, tz::TimeZone};
use log::SetLoggerError;

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
                install_stderr(crate_name, &filter)?;
                log::warn!("cannot open the journal socket, writing to stderr: {e}");
                return Ok(());
            }
        }
    }

    install_stderr(crate_name, &filter)
}

fn install_stderr(crate_name: &'static str, filter: &str) -> Result<(), SetLoggerError> {
    let tz = TimeZone::system();
    env_logger::Builder::new()
        .parse_filters(filter)
        .format(move |f, record| {
            let time = tz
                .to_datetime(Timestamp::now())
                .strftime("%Y-%m-%d %H:%M:%S%.3f");
            let level = record.level();
            let level_style = f.default_level_style(level);
            write!(f, "{time} {level_style}{level:<5}{level_style:#} ")?;
            if let Some(prefix) = module_prefix(crate_name, record.target()) {
                let bold = Style::new().bold();
                write!(f, "{bold}{prefix}{bold:#} > ")?;
            }
            writeln!(f, "{}", record.args())
        })
        .try_init()
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
    use std::fmt;

    use log::{Log, Metadata, Record, SetLoggerError};
    use systemd_journal_logger::JournalLog;

    use crate::module_prefix;

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
                let message = JournalMessage {
                    crate_name: self.crate_name,
                    record,
                };
                self.journal
                    .log(&record.to_builder().args(format_args!("{message}")).build());
            }
        }

        fn flush(&self) {}
    }

    /// The text of a message as written to the systemd journal.
    struct JournalMessage<'a> {
        crate_name: &'a str,
        record: &'a Record<'a>,
    }

    impl fmt::Display for JournalMessage<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let level = self.record.level();
            write!(f, "{level:<5} ")?;
            if let Some(prefix) = module_prefix(self.crate_name, self.record.target()) {
                write!(f, "{prefix} > ")?;
            }
            write!(f, "{}", self.record.args())
        }
    }

    #[cfg(test)]
    mod tests {
        use log::{Level, Record};

        use super::JournalMessage;

        fn journal_message(level: Level, target: &str, text: &str) -> String {
            JournalMessage {
                crate_name: "app",
                record: &Record::builder()
                    .level(level)
                    .target(target)
                    .args(format_args!("{text}"))
                    .build(),
            }
            .to_string()
        }

        #[test]
        fn journal_message_starts_with_the_padded_level() {
            assert_eq!(
                journal_message(Level::Info, "app", "started"),
                "INFO  started"
            );
            assert_eq!(
                journal_message(Level::Error, "reqwest::connect", "503\nbody"),
                "ERROR reqwest::connect > 503\nbody",
            );
            assert_eq!(
                journal_message(Level::Warn, "app::core", "slow"),
                "WARN  core > slow",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::module_prefix;

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
