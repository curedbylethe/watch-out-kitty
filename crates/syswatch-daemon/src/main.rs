use std::process::ExitCode;
use syswatch_core::MonitoringConfig;
use syswatch_daemon::{
    DaemonError, clock::SystemClock, sampler::LiveSampler, scheduler::Scheduler,
};

fn run() -> Result<(), DaemonError> {
    let config = MonitoringConfig::default();
    let sampler = LiveSampler::new().map_err(DaemonError::Collector)?;
    let mut scheduler = Scheduler::new(SystemClock, sampler, config);
    eprintln!(
        "syswatch-daemon: aggregate {:?}, processes {:?}",
        config.aggregate_interval(),
        config.process_interval()
    );
    scheduler.run().map_err(DaemonError::Scheduler)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("syswatch-daemon: {error}");
            ExitCode::FAILURE
        }
    }
}
