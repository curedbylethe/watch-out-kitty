/// System measurements captured at one point in time.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SystemSnapshot {
    /// Aggregate and memory observation time in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Aggregate CPU usage across all cores, from 0 to 100 percent.
    pub cpu_usage_percent: f32,
    /// Time when CPU usage was last sampled; `None` during warm-up.
    pub cpu_updated_at_ms: Option<u64>,
    /// Physical memory measurements reported by the operating system.
    pub memory: MemorySnapshot,
    /// Process observations from the last process refresh. Order is unspecified.
    pub processes: Vec<ProcessSnapshot>,
    /// Time when process observations were last collected.
    pub processes_updated_at_ms: Option<u64>,
}

/// Aggregate CPU and memory measurements without process observations.
#[derive(Debug, Clone, PartialEq)]
pub struct AggregateSnapshot {
    /// Memory observation time in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Aggregate CPU usage across all cores, from 0 to 100 percent.
    pub cpu_usage_percent: f32,
    /// Time when CPU usage was last sampled; can be older or `None` during warm-up.
    pub cpu_updated_at_ms: Option<u64>,
    /// Physical memory measurements reported by the operating system.
    pub memory: MemorySnapshot,
}

/// Physical memory measurements reported by the operating system.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemorySnapshot {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Occupied physical memory in bytes; this is not memory pressure on macOS.
    pub used_bytes: u64,
}

impl MemorySnapshot {
    /// Returns occupied memory as a percentage of total physical memory.
    ///
    /// On macOS, occupied RAM does not equal memory pressure.
    pub fn usage_percent(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }

        (self.used_bytes as f64 / self.total_bytes as f64 * 100.0).clamp(0.0, 100.0) as f32
    }
}

/// One observed process.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSnapshot {
    /// Operating system process identifier.
    pub pid: u32,
    /// Parent process identifier, or `None` when the operating system did not report one.
    /// A parent PID of 1 does not imply that this process was abandoned.
    pub parent_pid: Option<u32>,
    /// Lossy-decoded process name.
    pub name: String,
    /// Per-process CPU usage percentage; may exceed 100 percent across cores.
    /// The first sample after start is not representative.
    pub cpu_usage_percent: f32,
    /// Process memory in bytes.
    pub memory_bytes: u64,
    /// Process start time in seconds since the Unix epoch.
    /// Process identity is `(pid, start_time_secs)` because operating systems reuse PIDs.
    pub start_time_secs: u64,
}

impl ProcessSnapshot {
    /// Returns the stable identity pair because operating systems reuse PIDs.
    pub fn identity(&self) -> (u32, u64) {
        (self.pid, self.start_time_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::{MemorySnapshot, ProcessSnapshot, SystemSnapshot};

    #[test]
    fn zero_total_reports_zero() {
        let memory = MemorySnapshot {
            total_bytes: 0,
            used_bytes: 42,
        };

        assert_eq!(memory.usage_percent(), 0.0);
    }

    #[test]
    fn half_of_total_reports_fifty() {
        let memory = MemorySnapshot {
            total_bytes: 16_000,
            used_bytes: 8_000,
        };

        assert_eq!(memory.usage_percent(), 50.0);
    }

    #[test]
    fn non_round_ratio_reports_fraction() {
        let memory = MemorySnapshot {
            total_bytes: 3,
            used_bytes: 1,
        };

        assert!(memory.usage_percent() > 0.0 && memory.usage_percent() < 100.0);
    }

    #[test]
    fn overshoot_clamps_to_hundred() {
        let memory = MemorySnapshot {
            total_bytes: 3,
            used_bytes: 4,
        };

        assert_eq!(memory.usage_percent(), 100.0);
    }

    #[test]
    fn system_snapshot_round_trips() {
        let snapshot = SystemSnapshot {
            timestamp_ms: 1_700_000_000_123,
            cpu_usage_percent: 42.5,
            cpu_updated_at_ms: Some(1_700_000_000_123),
            memory: MemorySnapshot {
                total_bytes: 16_000,
                used_bytes: 8_000,
            },
            processes: vec![
                ProcessSnapshot {
                    pid: 42,
                    parent_pid: Some(1),
                    name: "worker".into(),
                    cpu_usage_percent: 125.0,
                    memory_bytes: 4_000,
                    start_time_secs: 1_699_999_999,
                },
                ProcessSnapshot {
                    pid: 43,
                    parent_pid: Some(42),
                    name: "child".into(),
                    cpu_usage_percent: 1.25,
                    memory_bytes: 2_000,
                    start_time_secs: 1_700_000_000,
                },
            ],
            processes_updated_at_ms: None,
        };

        let serialized = serde_json::to_string(&snapshot).unwrap();
        let deserialized: SystemSnapshot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, snapshot);
    }

    #[test]
    fn process_snapshot_round_trips_without_parent() {
        let process = ProcessSnapshot {
            pid: 42,
            parent_pid: None,
            name: "worker".into(),
            cpu_usage_percent: 0.0,
            memory_bytes: 0,
            start_time_secs: 1_700_000_000,
        };

        let serialized = serde_json::to_string(&process).unwrap();
        let deserialized: ProcessSnapshot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.parent_pid, None);
        assert_eq!(deserialized, process);
    }

    #[test]
    fn identity_combines_pid_and_start_time() {
        let process = ProcessSnapshot {
            pid: 42,
            parent_pid: None,
            name: "worker".into(),
            cpu_usage_percent: 0.0,
            memory_bytes: 0,
            start_time_secs: 1_700_000_000,
        };
        let reused_pid = ProcessSnapshot {
            start_time_secs: 1_700_000_001,
            ..process.clone()
        };

        assert_eq!(process.identity(), (42, 1_700_000_000));
        assert_ne!(process.identity(), reused_pid.identity());
        assert_eq!(process.identity(), process.clone().identity());
    }
}
