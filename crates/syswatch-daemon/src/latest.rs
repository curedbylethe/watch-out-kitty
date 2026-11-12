use syswatch_core::{AggregateSnapshot, SystemSnapshot};

use crate::sampler::ProcessScan;

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
}

#[cfg(test)]
mod tests {
    use super::LatestSnapshot;
    use syswatch_core::{AggregateSnapshot, MemorySnapshot};

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
