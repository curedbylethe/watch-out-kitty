#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::ProcessSnapshot;

    fn process(
        pid: u32,
        start_time_secs: u64,
        name: &str,
        cpu: f32,
        memory: u64,
    ) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: Some(1),
            name: name.to_owned(),
            cpu_usage_percent: cpu,
            memory_bytes: memory,
            start_time_secs,
        }
    }

    #[test]
    fn repeated_observations_keep_identity_and_samples() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(7, 10, "first", 4.0, 20)]);
        history.record_scan(200, &[process(7, 10, "second", 0.0, 40)]);

        let record = history.get((7, 10)).unwrap();
        assert_eq!((record.first_seen_ms, record.last_seen_ms), (100, 200));
        assert_eq!(record.name, "second");
        assert_eq!(record.observations().len(), 2);
        assert_eq!(record.observations()[0].memory_bytes, 20);
        assert_eq!(record.observations()[1].memory_bytes, 40);
        assert_eq!(record.observations()[0].cpu_usage_percent, None);
        assert_eq!(record.observations()[1].cpu_usage_percent, Some(0.0));
    }

    #[test]
    fn pid_reuse_creates_a_separate_record() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(7, 10, "old", 1.0, 20)]);
        history.record_scan(200, &[process(7, 11, "new", 2.0, 30)]);

        assert_eq!(history.len(), 2);
        assert_eq!(history.get((7, 10)).unwrap().name, "old");
        assert_eq!(history.get((7, 11)).unwrap().name, "new");
    }

    #[test]
    fn repeated_scans_update_process_metadata() {
        let mut history = ProcessHistory::new();
        let mut first = process(7, 10, "first", 1.0, 20);
        first.parent_pid = Some(2);
        history.record_scan(100, &[first]);
        let mut latest = process(7, 10, "latest", 2.0, 30);
        latest.parent_pid = Some(3);
        history.record_scan(200, &[latest]);

        let record = history.get((7, 10)).unwrap();
        assert_eq!(record.name, "latest");
        assert_eq!(record.parent_pid, Some(3));
    }

    #[test]
    fn non_monotonic_timestamps_keep_ordered_bounds() {
        let mut history = ProcessHistory::new();
        history.record_scan(1_000, &[process(7, 10, "worker", 1.0, 20)]);
        history.record_scan(500, &[process(7, 10, "worker", 2.0, 30)]);

        let record = history.get((7, 10)).unwrap();
        assert_eq!(record.first_seen_ms, 500);
        assert_eq!(record.last_seen_ms, 1_000);
        assert_eq!(record.observed_span_ms(), 500);
    }

    #[test]
    fn invalid_cpu_samples_stay_invalid() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(7, 10, "worker", 9.0, 20)]);
        history.record_scan(200, &[process(7, 10, "worker", f32::NAN, 30)]);

        let record = history.get((7, 10)).unwrap();
        assert_eq!(record.observations()[0].cpu_usage_percent, None);
        assert_eq!(record.observations()[1].cpu_usage_percent, None);
    }

    #[test]
    fn empty_scan_on_empty_history_is_safe() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[]);

        assert!(history.is_empty());
        assert_eq!(history.iter().count(), 0);
    }
}
use std::collections::{BTreeMap, VecDeque};

use crate::snapshot::ProcessSnapshot;

/// One process sample from a scan.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessObservation {
    /// Scan timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// CPU usage, or `None` for a cold or invalid sample.
    pub cpu_usage_percent: Option<f32>,
    /// Process memory in bytes.
    pub memory_bytes: u64,
}

/// Observations and metadata for one process identity.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRecord {
    /// Operating system process identifier.
    pub pid: u32,
    /// Process start time in seconds since the Unix epoch.
    pub start_time_secs: u64,
    /// Latest observed process name.
    pub name: String,
    /// Latest observed parent process identifier.
    pub parent_pid: Option<u32>,
    /// Earliest scan timestamp for this identity.
    pub first_seen_ms: u64,
    /// Latest scan timestamp for this identity.
    pub last_seen_ms: u64,
    observations: VecDeque<ProcessObservation>,
}

impl ProcessRecord {
    /// Returns this process's stable identity.
    pub fn identity(&self) -> (u32, u64) {
        (self.pid, self.start_time_secs)
    }

    /// Returns observations in scan ingestion order.
    pub fn observations(&self) -> &VecDeque<ProcessObservation> {
        &self.observations
    }

    /// Returns the non-negative span between first and last observation.
    pub fn observed_span_ms(&self) -> u64 {
        self.last_seen_ms.saturating_sub(self.first_seen_ms)
    }
}

/// In-memory process history keyed by process identity.
#[derive(Debug, Clone)]
pub struct ProcessHistory {
    records: BTreeMap<(u32, u64), ProcessRecord>,
}

impl ProcessHistory {
    /// Creates an empty process history.
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
        }
    }

    /// Records every process in one completed process scan.
    pub fn record_scan(&mut self, timestamp_ms: u64, processes: &[ProcessSnapshot]) {
        for process in processes {
            let identity = process.identity();
            match self.records.entry(identity) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let mut observations = VecDeque::new();
                    observations.push_back(ProcessObservation {
                        timestamp_ms,
                        cpu_usage_percent: None,
                        memory_bytes: process.memory_bytes,
                    });
                    entry.insert(ProcessRecord {
                        pid: process.pid,
                        start_time_secs: process.start_time_secs,
                        name: process.name.clone(),
                        parent_pid: process.parent_pid,
                        first_seen_ms: timestamp_ms,
                        last_seen_ms: timestamp_ms,
                        observations,
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let record = entry.get_mut();
                    record.first_seen_ms = record.first_seen_ms.min(timestamp_ms);
                    record.last_seen_ms = record.last_seen_ms.max(timestamp_ms);
                    record.name.clone_from(&process.name);
                    record.parent_pid = process.parent_pid;
                    record.observations.push_back(ProcessObservation {
                        timestamp_ms,
                        cpu_usage_percent: process
                            .cpu_usage_percent
                            .is_finite()
                            .then_some(process.cpu_usage_percent),
                        memory_bytes: process.memory_bytes,
                    });
                }
            }
        }
    }

    /// Returns a record by process identity.
    pub fn get(&self, identity: (u32, u64)) -> Option<&ProcessRecord> {
        self.records.get(&identity)
    }

    /// Iterates records in identity order.
    pub fn iter(&self) -> impl Iterator<Item = &ProcessRecord> + '_ {
        self.records.values()
    }

    /// Returns the number of process records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns whether the history has no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl Default for ProcessHistory {
    fn default() -> Self {
        Self::new()
    }
}
