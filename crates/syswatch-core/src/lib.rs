//! Core types and logic for syswatch.

pub mod collector;
pub mod config;
pub mod history;
#[cfg(unix)]
pub mod ipc;
pub mod protocol;
pub mod report;
pub mod snapshot;
pub use collector::Collector;
pub use config::{ConfigError, MonitoringConfig};
pub use history::{HistoryLimits, ProcessHistory, ProcessObservation, ProcessRecord};
pub use snapshot::{AggregateSnapshot, MemorySnapshot, ProcessSnapshot, SystemSnapshot};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_matches_package() {
        assert_eq!(env!("CARGO_PKG_NAME"), "syswatch-core");
    }
}
