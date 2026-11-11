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

    #[test]
    fn absent_process_is_marked_at_next_scan() {
        let mut history = ProcessHistory::new();
        history.record_scan(
            100,
            &[
                process(7, 10, "worker", 1.0, 20),
                process(8, 10, "other", 1.0, 30),
            ],
        );
        history.record_scan(200, &[process(8, 10, "other", 1.0, 30)]);

        assert_eq!(history.get((7, 10)).unwrap().disappeared_at_ms, Some(200));
        assert_eq!(history.get((8, 10)).unwrap().disappeared_at_ms, None);
    }

    #[test]
    fn no_scan_does_not_mark_disappearance() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(7, 10, "worker", 1.0, 20)]);

        assert_eq!(history.last_scan_ms(), Some(100));
        assert_eq!(history.get((7, 10)).unwrap().disappeared_at_ms, None);
    }

    #[test]
    fn returning_process_clears_disappearance() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(7, 10, "worker", 1.0, 20)]);
        history.record_scan(200, &[]);
        history.record_scan(300, &[process(7, 10, "worker", 2.0, 40)]);

        let record = history.get((7, 10)).unwrap();
        assert_eq!(record.disappeared_at_ms, None);
        assert_eq!(record.last_seen_ms, 300);
        assert_eq!(record.observations().len(), 2);
    }

    #[test]
    fn empty_scan_marks_prior_processes_disappeared() {
        let mut history = ProcessHistory::new();
        history.record_scan(
            100,
            &[
                process(7, 10, "worker", 1.0, 20),
                process(8, 10, "other", 1.0, 30),
            ],
        );
        history.record_scan(200, &[]);

        assert_eq!(history.get((7, 10)).unwrap().disappeared_at_ms, Some(200));
        assert_eq!(history.get((8, 10)).unwrap().disappeared_at_ms, Some(200));
    }

    #[test]
    fn first_empty_scan_marks_nothing() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[]);

        assert!(history.is_empty());
        assert_eq!(history.last_scan_ms(), Some(100));
    }

    #[test]
    fn disappearance_timestamp_never_precedes_last_seen() {
        let mut history = ProcessHistory::new();
        history.record_scan(1_000, &[process(7, 10, "worker", 1.0, 20)]);
        history.record_scan(500, &[]);

        let record = history.get((7, 10)).unwrap();
        assert_eq!(record.disappeared_at_ms, Some(1_000));
        assert!(record.disappeared_at_ms.unwrap() >= record.last_seen_ms);
        assert_eq!(history.last_scan_ms(), Some(1_000));
    }

    #[test]
    fn returning_process_identity_is_unchanged() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(7, 10, "old", 1.0, 20)]);
        history.record_scan(200, &[process(7, 11, "new", 1.0, 30)]);

        assert_eq!(history.len(), 2);
        assert_eq!(history.get((7, 10)).unwrap().disappeared_at_ms, Some(200));
        assert_eq!(history.get((7, 11)).unwrap().disappeared_at_ms, None);
    }

    fn limits(
        max_processes: usize,
        max_observations_per_process: usize,
        max_age_ms: u64,
    ) -> HistoryLimits {
        HistoryLimits {
            max_processes,
            max_observations_per_process,
            max_age_ms,
        }
    }

    #[test]
    fn observation_count_is_capped() {
        let mut history = ProcessHistory::with_limits(limits(10, 3, 10_000));
        for timestamp in 1..=5 {
            history.record_scan(timestamp, &[process(7, 10, "worker", 1.0, timestamp)]);
        }

        let observations = history.get((7, 10)).unwrap().observations();
        assert_eq!(
            observations
                .iter()
                .map(|o| o.memory_bytes)
                .collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert_eq!(
            observations
                .iter()
                .map(|o| o.timestamp_ms)
                .collect::<Vec<_>>(),
            [3, 4, 5]
        );
    }

    #[test]
    fn observation_age_is_capped() {
        let mut history = ProcessHistory::with_limits(limits(10, 10, 100));
        for timestamp in [0, 50, 200] {
            history.record_scan(timestamp, &[process(7, 10, "worker", 1.0, timestamp)]);
        }

        let observations = history.get((7, 10)).unwrap().observations();
        assert_eq!(
            observations
                .iter()
                .map(|o| o.timestamp_ms)
                .collect::<Vec<_>>(),
            [200]
        );
    }

    #[test]
    fn process_count_evicts_oldest_last_seen() {
        let mut history = ProcessHistory::with_limits(limits(2, 10, 10_000));
        history.record_scan(100, &[process(1, 1, "one", 1.0, 1)]);
        history.record_scan(200, &[process(2, 2, "two", 1.0, 2)]);
        history.record_scan(300, &[process(3, 3, "three", 1.0, 3)]);

        let identities = history
            .iter()
            .map(ProcessRecord::identity)
            .collect::<Vec<_>>();
        assert_eq!(identities, [(2, 2), (3, 3)]);
    }

    #[test]
    fn process_count_tie_breaks_on_identity() {
        let mut history = ProcessHistory::with_limits(limits(2, 10, 10_000));
        history.record_scan(
            100,
            &[
                process(3, 1, "three", 1.0, 3),
                process(1, 1, "one", 1.0, 1),
                process(2, 1, "two", 1.0, 2),
            ],
        );

        let identities = history
            .iter()
            .map(ProcessRecord::identity)
            .collect::<Vec<_>>();
        assert_eq!(identities, [(2, 1), (3, 1)]);
    }

    #[test]
    fn process_age_evicts_stale_entry() {
        let mut history = ProcessHistory::with_limits(limits(10, 10, 100));
        history.record_scan(0, &[process(1, 1, "old", 1.0, 1)]);
        history.record_scan(200, &[process(2, 2, "new", 1.0, 2)]);

        assert_eq!(
            history
                .iter()
                .map(ProcessRecord::identity)
                .collect::<Vec<_>>(),
            [(2, 2)]
        );
    }

    #[test]
    fn zero_max_processes_retains_nothing() {
        let mut history = ProcessHistory::with_limits(limits(0, 10, 10_000));
        history.record_scan(100, &[process(1, 1, "worker", 1.0, 1)]);

        assert!(history.is_empty());
    }

    #[test]
    fn zero_max_observations_keeps_metadata() {
        let mut history = ProcessHistory::with_limits(limits(10, 0, 10_000));
        history.record_scan(100, &[process(1, 1, "worker", 1.0, 1)]);

        let record = history.get((1, 1)).unwrap();
        assert_eq!(record.name, "worker");
        assert!(record.observations().is_empty());
    }

    #[test]
    fn eviction_does_not_mark_disappearance() {
        let mut history = ProcessHistory::with_limits(limits(1, 10, 10_000));
        history.record_scan(100, &[process(1, 1, "old", 1.0, 1)]);
        history.record_scan(200, &[process(2, 2, "new", 1.0, 2)]);

        assert!(history.get((1, 1)).is_none());
        assert_eq!(history.get((2, 2)).unwrap().disappeared_at_ms, None);
    }

    #[test]
    fn retention_is_deterministic_across_repeated_runs() {
        let run = || {
            let mut history = ProcessHistory::with_limits(limits(2, 2, 10_000));
            history.record_scan(
                100,
                &[
                    process(3, 1, "three", 1.0, 3),
                    process(1, 1, "one", 1.0, 1),
                    process(2, 1, "two", 1.0, 2),
                ],
            );
            history
                .iter()
                .map(ProcessRecord::identity)
                .collect::<Vec<_>>()
        };

        assert_eq!(run(), run());
    }
}
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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

/// Retention limits for process history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    /// Maximum number of process identities retained.
    pub max_processes: usize,
    /// Maximum observations retained for each process.
    pub max_observations_per_process: usize,
    /// Maximum age of retained records and observations in milliseconds.
    pub max_age_ms: u64,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        const MAX_PROCESSES: usize = 1_024;
        const MAX_OBSERVATIONS_PER_PROCESS: usize = 60;
        const MAX_AGE_MS: u64 = 600_000;

        Self {
            max_processes: MAX_PROCESSES,
            max_observations_per_process: MAX_OBSERVATIONS_PER_PROCESS,
            max_age_ms: MAX_AGE_MS,
        }
    }
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
    /// Latest scan timestamp that omitted this identity.
    pub disappeared_at_ms: Option<u64>,
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
    last_scan_ms: Option<u64>,
    limits: HistoryLimits,
}

impl ProcessHistory {
    /// Creates an empty process history.
    pub fn new() -> Self {
        Self::with_limits(HistoryLimits::default())
    }

    /// Creates an empty process history with explicit retention limits.
    pub fn with_limits(limits: HistoryLimits) -> Self {
        Self {
            records: BTreeMap::new(),
            last_scan_ms: None,
            limits,
        }
    }

    /// Records every process in one completed process scan.
    pub fn record_scan(&mut self, timestamp_ms: u64, processes: &[ProcessSnapshot]) {
        let identities: BTreeSet<_> = processes.iter().map(ProcessSnapshot::identity).collect();
        if self.last_scan_ms.is_some() {
            for (identity, record) in &mut self.records {
                if !identities.contains(identity) && record.disappeared_at_ms.is_none() {
                    record.disappeared_at_ms = Some(timestamp_ms.max(record.last_seen_ms));
                }
            }
        }

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
                        disappeared_at_ms: None,
                        observations,
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let record = entry.get_mut();
                    record.first_seen_ms = record.first_seen_ms.min(timestamp_ms);
                    record.last_seen_ms = record.last_seen_ms.max(timestamp_ms);
                    record.name.clone_from(&process.name);
                    record.parent_pid = process.parent_pid;
                    record.disappeared_at_ms = None;
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

        self.last_scan_ms = Some(
            self.last_scan_ms
                .map_or(timestamp_ms, |last| last.max(timestamp_ms)),
        );

        for record in self.records.values_mut() {
            record.observations.retain(|observation| {
                timestamp_ms.saturating_sub(observation.timestamp_ms) <= self.limits.max_age_ms
            });
            while record.observations.len() > self.limits.max_observations_per_process {
                record.observations.pop_front();
            }
        }

        self.records.retain(|_, record| {
            timestamp_ms.saturating_sub(record.last_seen_ms) <= self.limits.max_age_ms
        });
        while self.records.len() > self.limits.max_processes {
            let oldest = self
                .records
                .iter()
                .min_by_key(|(identity, record)| (record.last_seen_ms, **identity))
                .map(|(identity, _)| *identity)
                .unwrap();
            self.records.remove(&oldest);
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

    /// Returns the greatest timestamp of any ingested process scan.
    pub fn last_scan_ms(&self) -> Option<u64> {
        self.last_scan_ms
    }

    /// Returns the active retention limits.
    pub fn limits(&self) -> HistoryLimits {
        self.limits
    }
}

impl Default for ProcessHistory {
    fn default() -> Self {
        Self::new()
    }
}
