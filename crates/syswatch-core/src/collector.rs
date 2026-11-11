use std::{
    ffi::OsStr,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use sysinfo::{MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use crate::snapshot::{MemorySnapshot, ProcessSnapshot, SystemSnapshot};

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
    /// occupied RAM does not equal memory pressure. The first per-process CPU
    /// sample may not be representative.
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

        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .without_tasks(),
        );

        let processes = self
            .system
            .processes()
            .values()
            .map(|process| {
                process_snapshot(
                    process.pid().as_u32(),
                    process.parent().map(Pid::as_u32),
                    process.name(),
                    process.cpu_usage(),
                    process.memory(),
                    process.start_time(),
                )
            })
            .collect();

        let memory = MemorySnapshot {
            total_bytes: self.system.total_memory(),
            used_bytes: self.system.used_memory(),
        };
        snapshot_at(
            timestamp_ms(),
            self.last_cpu_usage_percent,
            memory,
            processes,
        )
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

fn process_snapshot(
    pid: u32,
    parent_pid: Option<u32>,
    name: &OsStr,
    cpu_usage_percent: f32,
    memory_bytes: u64,
    start_time_secs: u64,
) -> ProcessSnapshot {
    ProcessSnapshot {
        pid,
        parent_pid,
        name: name.to_string_lossy().into_owned(),
        cpu_usage_percent,
        memory_bytes,
        start_time_secs,
    }
}

fn snapshot_at(
    timestamp_ms: u64,
    cpu_usage_percent: f32,
    memory: MemorySnapshot,
    processes: Vec<ProcessSnapshot>,
) -> SystemSnapshot {
    SystemSnapshot {
        timestamp_ms,
        cpu_usage_percent,
        memory,
        processes,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        process::Command,
        time::{Duration, Instant},
    };

    use super::{Collector, cpu_refresh_due, process_snapshot, snapshot_at};
    use crate::snapshot::{MemorySnapshot, ProcessSnapshot};

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
    fn snapshot_mapping_copies_values_and_processes() {
        let memory = MemorySnapshot {
            total_bytes: 16_000,
            used_bytes: 8_000,
        };
        let processes = vec![ProcessSnapshot {
            pid: 42,
            parent_pid: Some(1),
            name: "worker".into(),
            cpu_usage_percent: 125.5,
            memory_bytes: 4096,
            start_time_secs: 1_700_000_000,
        }];
        let snapshot = snapshot_at(123, 42.5, memory.clone(), processes.clone());

        assert_eq!(snapshot.timestamp_ms, 123);
        assert_eq!(snapshot.cpu_usage_percent, 42.5);
        assert_eq!(snapshot.memory, memory);
        assert_eq!(snapshot.processes, processes);
    }

    #[test]
    fn collector_refresh_returns_snapshot_with_processes() {
        let mut collector = Collector::new().unwrap();

        let first = collector.refresh();
        assert_eq!(first.cpu_usage_percent, 0.0);
        assert!(
            first
                .processes
                .iter()
                .any(|process| process.pid == std::process::id())
        );
        assert!(
            collector
                .refresh()
                .processes
                .iter()
                .any(|process| process.pid == std::process::id())
        );
    }

    #[test]
    fn process_snapshot_copies_fields() {
        let process = process_snapshot(
            42,
            Some(1),
            OsStr::new("worker"),
            125.5,
            4096,
            1_700_000_000,
        );

        assert_eq!(process.pid, 42);
        assert_eq!(process.parent_pid, Some(1));
        assert_eq!(process.name, "worker");
        assert_eq!(process.cpu_usage_percent, 125.5);
        assert_eq!(process.memory_bytes, 4096);
        assert_eq!(process.start_time_secs, 1_700_000_000);
    }

    #[test]
    fn process_snapshot_preserves_missing_parent() {
        let process = process_snapshot(42, None, OsStr::new("worker"), 0.0, 0, 1_700_000_000);

        assert_eq!(process.parent_pid, None);
    }

    #[cfg(unix)]
    #[test]
    fn process_snapshot_lossy_decodes_non_utf8_name() {
        use std::os::unix::ffi::OsStrExt;

        let process = process_snapshot(
            42,
            None,
            OsStr::from_bytes(b"\xffworker"),
            0.0,
            0,
            1_700_000_000,
        );

        assert!(process.name.contains('\u{fffd}'));
    }

    #[test]
    fn collector_refresh_tolerates_process_exit() {
        let mut collector = Collector::new().unwrap();
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();

        assert!(
            collector
                .refresh()
                .processes
                .iter()
                .any(|process| process.pid == pid)
        );
        child.kill().unwrap();
        child.wait().unwrap();

        assert!(
            !collector
                .refresh()
                .processes
                .iter()
                .any(|process| process.pid == pid)
        );
    }
}
