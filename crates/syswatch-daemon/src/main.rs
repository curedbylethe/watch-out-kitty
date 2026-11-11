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
    let socket = match syswatch_core::ipc::socket_path() {
        Ok(socket) => socket,
        Err(error) => {
            signal_handle.stop();
            return Err(DaemonError::SocketPath(error));
        }
    };
    let latest = scheduler.latest_handle();
    let server = match syswatch_daemon::ipc::start(socket.clone(), move || {
        latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
    }) {
        Ok(server) => server,
        Err(error) => {
            signal_handle.stop();
            return Err(DaemonError::Socket(error));
        }
    };
    eprintln!("syswatch-daemon: serving at {}", socket.display());
    let result = scheduler.run().map_err(DaemonError::Scheduler);
    if let Err(error) = server.shutdown() {
        eprintln!("syswatch-daemon: socket cleanup: {error}");
    }
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
