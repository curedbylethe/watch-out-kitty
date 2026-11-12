use syswatch_core::{AggregateSnapshot, SuggestionsSnapshot, SystemSnapshot};

use crate::sampler::ProcessScan;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cached {
    pub snapshot: Option<SystemSnapshot>,
    pub suggestions: Option<SuggestionsSnapshot>,
}

#[derive(Default)]
pub struct LatestSnapshot {
    aggregate: Option<AggregateSnapshot>,
    processes: Option<ProcessScan>,
}

impl LatestSnapshot {
    pub fn set_aggregate(&mut self, aggregate: AggregateSnapshot) {
        self.aggregate = Some(aggregate);
    }
    pub fn set_processes(&mut self, processes: ProcessScan) {
        self.processes = Some(processes);
    }

    pub fn snapshot(&self) -> Option<SystemSnapshot> {
        let aggregate = self.aggregate.as_ref()?;
        let processes = self.processes.as_ref();
        Some(SystemSnapshot {
            timestamp_ms: aggregate.timestamp_ms,
            cpu_usage_percent: aggregate.cpu_usage_percent,
            cpu_updated_at_ms: aggregate.cpu_updated_at_ms,
            memory: aggregate.memory.clone(),
            processes: processes.map_or_else(Vec::new, |scan| scan.processes.clone()),
            processes_updated_at_ms: processes.map(|scan| scan.observed_at_ms),
        })
    }

    pub fn cached(&self) -> Cached {
        Cached {
            snapshot: self.snapshot(),
            suggestions: self
                .processes
                .as_ref()
                .and_then(|scan| scan.suggestions.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cached, LatestSnapshot};
    use syswatch_core::{AggregateSnapshot, MemorySnapshot, SuggestionsSnapshot};

    fn aggregate(timestamp_ms: u64) -> AggregateSnapshot {
        AggregateSnapshot {
            timestamp_ms,
            cpu_usage_percent: 0.0,
            cpu_updated_at_ms: None,
            memory: MemorySnapshot {
                total_bytes: 10,
                used_bytes: 5,
            },
        }
    }

    #[test]
    fn latest_snapshot_is_empty_before_the_first_tick() {
        assert!(LatestSnapshot::default().snapshot().is_none());
    }

    #[test]
    fn cached_is_empty_before_the_first_tick() {
        assert_eq!(LatestSnapshot::default().cached(), Cached::default());
    }

    #[test]
    fn cached_exposes_the_latest_tick_suggestions() {
        let mut latest = LatestSnapshot::default();
        let suggestions = SuggestionsSnapshot {
            evaluated_at_ms: 20,
            processes_updated_at_ms: 20,
            findings: vec![],
        };
        latest.set_aggregate(aggregate(10));
        latest.set_processes(crate::sampler::ProcessScan {
            processes: vec![],
            observed_at_ms: 20,
            suggestions: Some(suggestions.clone()),
        });
        let cached = latest.cached();
        assert!(cached.snapshot.is_some());
        assert_eq!(cached.suggestions, Some(suggestions));
    }

    #[test]
    fn latest_snapshot_combines_aggregate_and_process_parts() {
        let mut latest = LatestSnapshot::default();
        latest.set_aggregate(aggregate(10));
        latest.set_processes(crate::sampler::ProcessScan {
            processes: vec![],
            observed_at_ms: 20,
            suggestions: None,
        });
        let snapshot = latest.snapshot().unwrap();
        assert_eq!(snapshot.timestamp_ms, 10);
        assert_eq!(snapshot.processes_updated_at_ms, Some(20));
    }
}
