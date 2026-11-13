use std::collections::BTreeMap;

use syswatch_core::provenance::ProcessProvenance;
use syswatch_core::{
    AggregateSnapshot, Collector, DetectorConfig, ProcessHistory, ProcessSnapshot,
    SuggestionsSnapshot, detector::detect_idle_memory_consumers, eligibility::monitor_identity,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessScan {
    pub processes: Vec<ProcessSnapshot>,
    pub observed_at_ms: u64,
    pub suggestions: Option<SuggestionsSnapshot>,
    /// Provenance from the same scan. Daemon-local: never serialized.
    pub provenance: BTreeMap<(u32, u64), ProcessProvenance>,
    /// Identity of this daemon's own process, when it appears in the scan.
    pub monitor: Option<(u32, u64)>,
}

pub trait Sampler {
    fn refresh_aggregate(&mut self) -> AggregateSnapshot;
    fn refresh_processes(&mut self) -> ProcessScan;
}

pub struct LiveSampler {
    collector: Collector,
    history: ProcessHistory,
    detector_config: DetectorConfig,
    started_at_ms: Option<u64>,
    scans: u64,
}

impl LiveSampler {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            collector: Collector::new().map_err(|error| error.to_string())?,
            history: ProcessHistory::new(),
            detector_config: DetectorConfig::default(),
            started_at_ms: None,
            scans: 0,
        })
    }

    pub fn history(&self) -> &ProcessHistory {
        &self.history
    }
}

impl Sampler for LiveSampler {
    fn refresh_aggregate(&mut self) -> AggregateSnapshot {
        self.collector.refresh_aggregate()
    }

    fn refresh_processes(&mut self) -> ProcessScan {
        let processes = self.collector.refresh_processes().to_vec();
        let observed_at_ms = self
            .collector
            .processes_updated_at_ms()
            .expect("process refresh sets timestamp");
        self.history.record_scan(observed_at_ms, &processes);
        let started_at_ms = *self.started_at_ms.get_or_insert(observed_at_ms);
        self.scans = self.scans.saturating_add(1);
        let provenance = self.collector.provenance().clone();
        let monitor = monitor_identity(&provenance);
        let suggestions = suggestions_ready(
            started_at_ms,
            observed_at_ms,
            self.scans,
            &self.detector_config,
        )
        .then(|| SuggestionsSnapshot {
            evaluated_at_ms: observed_at_ms,
            processes_updated_at_ms: observed_at_ms,
            findings: detect_idle_memory_consumers(
                &self.history,
                observed_at_ms,
                &provenance,
                &self.detector_config,
            ),
        });
        ProcessScan {
            processes,
            observed_at_ms,
            suggestions,
            provenance,
            monitor,
        }
    }
}

fn suggestions_ready(started_at_ms: u64, now_ms: u64, scans: u64, config: &DetectorConfig) -> bool {
    scans > config.min_valid_observations as u64
        && now_ms.saturating_sub(started_at_ms) >= config.min_span_ms
}

#[cfg(test)]
mod tests {
    use super::{LiveSampler, Sampler, suggestions_ready};
    use syswatch_core::DetectorConfig;

    #[test]
    fn suggestions_are_unavailable_before_the_observation_window() {
        assert!(
            LiveSampler::new()
                .unwrap()
                .refresh_processes()
                .suggestions
                .is_none()
        );
    }

    #[test]
    fn suggestions_ready_requires_scan_count_and_span() {
        let config = DetectorConfig {
            min_valid_observations: 3,
            min_span_ms: 1000,
            ..DetectorConfig::default()
        };
        assert!(!suggestions_ready(0, 1000, 3, &config));
        assert!(suggestions_ready(0, 1000, 4, &config));
        assert!(!suggestions_ready(0, 999, 4, &config));
        assert!(suggestions_ready(0, 1001, 4, &config));
    }

    #[test]
    fn live_sampler_records_every_process_scan_in_history() {
        let mut sampler = LiveSampler::new().unwrap();
        let scan = sampler.refresh_processes();
        assert_eq!(
            sampler.history().len(),
            scan.processes
                .len()
                .min(sampler.history().limits().max_processes)
        );
        let len = sampler.history().len();
        sampler.refresh_aggregate();
        assert_eq!(sampler.history().len(), len);
        sampler.refresh_processes();
    }

    #[test]
    fn live_scan_carries_provenance_and_monitor_for_this_process() {
        let scan = LiveSampler::new().unwrap().refresh_processes();
        let pid = std::process::id();
        assert!(
            scan.provenance
                .keys()
                .any(|(candidate, _)| *candidate == pid)
        );
        assert_eq!(scan.monitor.map(|(candidate, _)| candidate), Some(pid));
    }

    #[test]
    fn ready_window_publishes_suggestions_stamped_with_the_scan_time() {
        let mut sampler = LiveSampler::new().unwrap();
        sampler.detector_config = DetectorConfig {
            min_valid_observations: 1,
            min_span_ms: 0,
            ..DetectorConfig::default()
        };
        sampler.refresh_processes();
        let scan = sampler.refresh_processes();
        let suggestions = scan.suggestions.expect("second scan is ready");
        assert_eq!(suggestions.evaluated_at_ms, scan.observed_at_ms);
        assert_eq!(suggestions.processes_updated_at_ms, scan.observed_at_ms);
    }
}
