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
    let action_latest = std::sync::Arc::clone(&latest);
    // Two process intervals: the scan behind an action may be one interval old.
    let max_observation_age_ms = config.process_interval().as_millis() as u64 * 2;
    let server = match syswatch_daemon::ipc::start(
        socket.clone(),
        move || {
            latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .cached()
        },
        move |command, finding| {
            let latest = action_latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            syswatch_daemon::action::answer(
                command,
                finding,
                latest.action_state().as_ref(),
                max_observation_age_ms,
                &mut syswatch_core::UnixSignalSender,
            )
        },
    ) {
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
