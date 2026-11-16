mod sub {
    pub fn run() {
        log::info!("from a submodule");
    }
}

fn main() -> Result<(), log::SetLoggerError> {
    krr_logger::init!()?;
    log::info!("from the crate root");
    sub::run();
    log::warn!(target: "other::module", "first line\nsecond line");
    log::debug!("shown with RUST_LOG=debug");
    Ok(())
}
