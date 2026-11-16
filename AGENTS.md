# krr-logger

`./check.sh` runs the formatter, clippy and the tests.

`cargo run --example demo` prints sample messages to the terminal. To see the journal entries, run the example as a user service and read them back:

```sh
systemd-run --user --wait --unit=krr-logger-demo target/debug/examples/demo
journalctl --user -u krr-logger-demo -o json
```

The journal output builds on `systemd-journal-logger`, which uses Unix sockets and compiles only on Linux, so every use of it stays under `cfg(target_os = "linux")` for dependents that build for Windows.
