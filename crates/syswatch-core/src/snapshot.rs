/// System measurements captured at one point in time.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SystemSnapshot {
    /// Sample time in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Aggregate CPU usage across all cores, from 0 to 100 percent.
    pub cpu_usage_percent: f32,
    /// Physical memory measurements reported by the operating system.
    pub memory: MemorySnapshot,
    /// Process observations; empty when process collection was not performed.
    pub processes: Vec<ProcessSnapshot>,
}

/// Physical memory measurements reported by the operating system.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemorySnapshot {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Occupied physical memory in bytes; this is not memory pressure on macOS.
    pub used_bytes: u64,
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
