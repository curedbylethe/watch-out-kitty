use std::process::ExitCode;
use syswatch_core::MonitoringConfig;
use syswatch_daemon::{
    DaemonError, clock::SystemClock, sampler::LiveSampler, scheduler::Scheduler, signals,
};

fn run() -> Result<(), DaemonError> {
    let config = MonitoringConfig::default();
    let sampler = LiveSampler::new().map_err(DaemonError::Collector)?;
    let (receiver, signal_handle) = signals::install().map_err(DaemonError::Signal)?;
    let mut scheduler = Scheduler::new(SystemClock::new(receiver), sampler, config);
    eprintln!(
        "syswatch-daemon: aggregate {:?}, processes {:?}",
        config.aggregate_interval(),
        config.process_interval()
    );
    let result = scheduler.run().map_err(DaemonError::Scheduler);
    signal_handle.stop();
    result?;
    eprintln!("syswatch-daemon: stopped");
    Ok(())
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
