//! Core types and logic for syswatch.

pub mod collector;
pub mod config;
pub mod detector;
pub mod eligibility;
pub mod findings;
pub mod history;
#[cfg(unix)]
pub mod ipc;
pub mod protocol;
pub mod provenance;
pub mod report;
pub mod snapshot;
pub use collector::Collector;
pub use config::{ConfigError, MonitoringConfig};
pub use detector::{DetectorConfig, IdleMemoryCandidate};
pub use eligibility::{
    Ancestry, AncestryOutcome, EligibilityAssessment, EligibilityReason, EligibilityState,
    MAX_ANCESTRY_DEPTH, SYSTEM_UID_THRESHOLD, TERMINAL_PROGRAMS, TerminalAssociation,
};
pub use findings::{
    AdvisoryFinding, Confidence, CpuActivity, FindingEvidence, FindingId, FindingKind,
    MemoryActivity,
};
pub use history::{HistoryLimits, ProcessHistory, ProcessObservation, ProcessRecord};
pub use snapshot::{AggregateSnapshot, MemorySnapshot, ProcessSnapshot, SystemSnapshot};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_matches_package() {
        assert_eq!(env!("CARGO_PKG_NAME"), "syswatch-core");
    }
}
