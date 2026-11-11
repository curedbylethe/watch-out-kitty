pub mod clock;
pub mod latest;
pub mod sampler;
pub mod scheduler;
pub mod signals;

use std::{error::Error, fmt, io};

#[derive(Debug)]
pub enum DaemonError {
    Collector(String),
    Scheduler(scheduler::SchedulerError),
    Signal(io::Error),
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collector(error) => write!(f, "collector initialization failed: {error}"),
            Self::Scheduler(error) => write!(f, "scheduler failed: {error}"),
            Self::Signal(error) => write!(f, "signal-handler installation failed: {error}"),
        }
    }
}

impl Error for DaemonError {}
