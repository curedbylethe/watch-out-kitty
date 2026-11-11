use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sysinfo::{MemoryRefreshKind, System};

use crate::snapshot::{MemorySnapshot, SystemSnapshot};

/// Stateful collector for aggregate CPU and physical-memory usage.
pub struct Collector {
    system: System,
    last_cpu_refresh: Option<Instant>,
    last_cpu_usage_percent: f32,
}

impl Collector {
    /// Creates a collector without taking a CPU sample.
    pub fn new() -> Result<Self, sysinfo::Error> {
        Ok(Self {
            system: System::new()?,
            last_cpu_refresh: None,
            last_cpu_usage_percent: 0.0,
        })
    }

    /// Refreshes memory on every call and CPU only after the minimum interval.
    ///
    /// The first call primes CPU counters and reports `0.0`; a later call at
    /// least `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` later reports CPU usage.
    /// Calls inside that interval reuse the prior CPU value. On macOS,
    /// occupied RAM does not equal memory pressure.
    pub fn refresh(&mut self) -> SystemSnapshot {
        let now = Instant::now();
        let cpu_refresh_due = cpu_refresh_due(self.last_cpu_refresh, now);

        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

        if cpu_refresh_due {
            self.system.refresh_cpu_usage();
            if self.last_cpu_refresh.is_some() {
                self.last_cpu_usage_percent = self.system.global_cpu_usage();
            }
            self.last_cpu_refresh = Some(Instant::now());
        }

        let memory = MemorySnapshot {
            total_bytes: self.system.total_memory(),
            used_bytes: self.system.used_memory(),
        };
        snapshot_at(timestamp_ms(), self.last_cpu_usage_percent, memory)
    }
}

fn cpu_refresh_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| {
        now.saturating_duration_since(last) >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL
    })
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn snapshot_at(
    timestamp_ms: u64,
    cpu_usage_percent: f32,
    memory: MemorySnapshot,
) -> SystemSnapshot {
    SystemSnapshot {
        timestamp_ms,
        cpu_usage_percent,
        memory,
        processes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Collector, cpu_refresh_due, snapshot_at};
    use crate::snapshot::MemorySnapshot;

    #[test]
    fn cpu_refresh_is_due_without_a_previous_sample() {
        assert!(cpu_refresh_due(None, Instant::now()));
    }

    #[test]
    fn cpu_refresh_waits_for_minimum_interval() {
        let last = Instant::now();
        let interval = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL;

        assert!(!cpu_refresh_due(Some(last), last));
        assert!(!cpu_refresh_due(
            Some(last),
            last + interval - Duration::from_nanos(1)
        ));
        assert!(cpu_refresh_due(Some(last), last + interval));
    }

    #[test]
    fn cpu_refresh_rejects_time_before_previous_sample() {
        let last = Instant::now();

        assert!(!cpu_refresh_due(
            Some(last),
            last - sysinfo::MINIMUM_CPU_UPDATE_INTERVAL
        ));
    }

    #[test]
    fn snapshot_mapping_copies_values_and_has_no_processes() {
        let memory = MemorySnapshot {
            total_bytes: 16_000,
            used_bytes: 8_000,
        };
        let snapshot = snapshot_at(123, 42.5, memory.clone());

        assert_eq!(snapshot.timestamp_ms, 123);
        assert_eq!(snapshot.cpu_usage_percent, 42.5);
        assert_eq!(snapshot.memory, memory);
        assert!(snapshot.processes.is_empty());
    }

    #[test]
    fn collector_refresh_returns_snapshot_without_processes() {
        let mut collector = Collector::new().unwrap();

        assert!(collector.refresh().processes.is_empty());
        assert!(collector.refresh().processes.is_empty());
    }
}
