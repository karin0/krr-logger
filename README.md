# krr-logger

A logger for the [`log`](https://docs.rs/log) facade. A program that runs as a systemd service writes each message to the journal as one entry with its priority, and elsewhere the messages go to stderr with the local time and colors.

```toml
[dependencies]
krr-logger = { git = 'https://github.com/karin0/krr-logger.git' }
```

```rust
fn main() -> Result<(), log::SetLoggerError> {
    krr_logger::init!()?;
    log::info!("started");
    Ok(())
}
```

`RUST_LOG` filters the messages with the [`env_logger` syntax](https://docs.rs/env_logger/latest/env_logger/#enabling-logging). When it is unset, the filter is the argument of `init!`, such as `krr_logger::init!("info,hyper=warn")`, or `info` without one.

Each message starts with its level and the module path of its target. The path is omitted for the root module of the crate that calls `init!`, relative for its submodules, and complete for other crates:

```
INFO  started
INFO  core > cache written
WARN  reqwest::connect > retrying
```

When stderr is the journal stream that systemd sets up for a service, each journal entry also carries `PRIORITY`, `TARGET`, `SYSLOG_IDENTIFIER` (the executable name) and the source location in `CODE_FILE`, `CODE_LINE` and `CODE_MODULE`. `journalctl -u` may miss the last entries of a process that exits right after logging them, since journald looks up the unit of each entry by the sender's PID, and `journalctl -t <executable>` finds them.
