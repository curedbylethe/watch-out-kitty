use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use sysinfo::{MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::provenance::{ParentContext, ProcessProvenance};
use crate::snapshot::{AggregateSnapshot, MemorySnapshot, ProcessSnapshot, SystemSnapshot};

/// Stateful collector for aggregate CPU and physical-memory usage.
pub struct Collector {
    system: System,
    last_cpu_refresh: Option<Instant>,
    last_cpu_usage_percent: f32,
    last_cpu_sample_ms: Option<u64>,
    last_processes_updated_ms: Option<u64>,
    cached_processes: Vec<ProcessSnapshot>,
    provenance: BTreeMap<(u32, u64), ProcessProvenance>,
    #[cfg(test)]
    process_refresh_calls: usize,
}

impl Collector {
    /// Creates a collector without taking a CPU sample.
    pub fn new() -> Result<Self, sysinfo::Error> {
        Ok(Self {
            system: System::new()?,
            last_cpu_refresh: None,
            last_cpu_usage_percent: 0.0,
            last_cpu_sample_ms: None,
            last_processes_updated_ms: None,
            cached_processes: Vec::new(),
            provenance: BTreeMap::new(),
            #[cfg(test)]
            process_refresh_calls: 0,
        })
    }

    /// Refreshes memory every call and CPU after the minimum interval, without enumerating processes.
    ///
    /// The first call primes CPU counters and reports `0.0`; a later call at
    /// least `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` later reports CPU usage.
    /// Calls inside that interval reuse the prior CPU value. On macOS,
    /// occupied RAM does not equal memory pressure.
    pub fn refresh_aggregate(&mut self) -> AggregateSnapshot {
        let observed_at_ms = timestamp_ms();
        let now = Instant::now();
        let cpu_refresh_due = cpu_refresh_due(self.last_cpu_refresh, now);

        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

        if cpu_refresh_due {
            self.system.refresh_cpu_usage();
            if self.last_cpu_refresh.is_some() {
                self.last_cpu_usage_percent = self.system.global_cpu_usage();
                self.last_cpu_sample_ms = Some(observed_at_ms);
            }
            self.last_cpu_refresh = Some(Instant::now());
        }

        AggregateSnapshot {
            timestamp_ms: observed_at_ms,
            cpu_usage_percent: self.last_cpu_usage_percent,
            cpu_updated_at_ms: self.last_cpu_sample_ms,
            memory: MemorySnapshot {
                total_bytes: self.system.total_memory(),
                used_bytes: self.system.used_memory(),
            },
        }
    }

    /// Refreshes process observations and returns the cached slice.
    pub fn refresh_processes(&mut self) -> &[ProcessSnapshot] {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_user(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .without_tasks(),
        );
        #[cfg(test)]
        {
            self.process_refresh_calls += 1;
        }

        let (processes, provenance_rows): (Vec<_>, Vec<_>) = self
            .system
            .processes()
            .values()
            .map(|process| {
                let pid = process.pid().as_u32();
                let start_time_secs = process.start_time();
                (
                    process_snapshot(
                        pid,
                        process.parent().map(Pid::as_u32),
                        process.name(),
                        process.cpu_usage(),
                        process.memory(),
                        start_time_secs,
                    ),
                    RawProvenance {
                        pid,
                        start_time_secs,
                        executable: process.exe().map(Path::to_path_buf),
                        cmd: process.cmd().to_vec(),
                        owner_uid: process.user_id().map(|uid| **uid),
                        parent_pid: process.parent().map(Pid::as_u32),
                    },
                )
            })
            .unzip();
        self.cached_processes = processes;
        self.provenance = build_provenance(provenance_rows);
        self.last_processes_updated_ms = Some(timestamp_ms());
        &self.cached_processes
    }

    /// Returns the latest process observations without refreshing them.
    pub fn cached_processes(&self) -> &[ProcessSnapshot] {
        &self.cached_processes
    }

    /// Returns when the cached process observations were collected.
    pub fn processes_updated_at_ms(&self) -> Option<u64> {
        self.last_processes_updated_ms
    }

    /// Returns provenance from the latest completed process scan.
    ///
    /// The map is empty before the first refresh. An absent identity means the
    /// process exited or was never observed.
    pub fn provenance(&self) -> &BTreeMap<(u32, u64), ProcessProvenance> {
        &self.provenance
    }

    /// Returns provenance for one process identity from the latest scan.
    pub fn provenance_for(&self, identity: (u32, u64)) -> Option<&ProcessProvenance> {
        self.provenance.get(&identity)
    }

    /// Refreshes aggregate metrics and processes in one call.
    ///
    /// CPU warm-up and interval behavior match [`Self::refresh_aggregate`]. The
    /// first per-process CPU sample may not be representative.
    pub fn refresh(&mut self) -> SystemSnapshot {
        let aggregate = self.refresh_aggregate();
        let processes = self.refresh_processes().to_vec();
        snapshot_at(
            aggregate.timestamp_ms,
            aggregate.cpu_usage_percent,
            aggregate.cpu_updated_at_ms,
            aggregate.memory,
            processes,
            self.last_processes_updated_ms,
        )
    }

    #[cfg(test)]
    fn process_refresh_calls(&self) -> usize {
        self.process_refresh_calls
    }
}

struct RawProvenance {
    pid: u32,
    start_time_secs: u64,
    executable: Option<PathBuf>,
    cmd: Vec<OsString>,
    owner_uid: Option<u32>,
    parent_pid: Option<u32>,
}

fn build_provenance(rows: Vec<RawProvenance>) -> BTreeMap<(u32, u64), ProcessProvenance> {
    let identities: BTreeMap<_, _> = rows
        .iter()
        .map(|row| (row.pid, (row.pid, row.start_time_secs)))
        .collect();

    rows.into_iter()
        .map(|row| {
            let parent = match row.parent_pid {
                Some(pid) => identities
                    .get(&pid)
                    .map(|identity| ParentContext::Resolved {
                        pid: identity.0,
                        start_time_secs: identity.1,
                    })
                    .unwrap_or(ParentContext::PidOnly(pid)),
                None => ParentContext::NoParent,
            };
            let process = ProcessProvenance {
                pid: row.pid,
                start_time_secs: row.start_time_secs,
                executable: row.executable,
                program: crate::provenance::program_label(row.cmd.first().map(OsString::as_os_str)),
                owner_uid: row.owner_uid,
                parent,
            };
            (process.identity(), process)
        })
        .collect()
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
    cpu_updated_at_ms: Option<u64>,
    memory: MemorySnapshot,
    processes: Vec<ProcessSnapshot>,
    processes_updated_at_ms: Option<u64>,
) -> SystemSnapshot {
    SystemSnapshot {
        timestamp_ms,
        cpu_usage_percent,
        cpu_updated_at_ms,
        memory,
        processes,
        processes_updated_at_ms,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::process::Command;
    use std::{
        ffi::OsStr,
        time::{Duration, Instant},
    };

    use super::{Collector, cpu_refresh_due, process_snapshot, snapshot_at};
    use crate::snapshot::{MemorySnapshot, ProcessSnapshot};

    fn raw_provenance(
        pid: u32,
        start_time_secs: u64,
        parent_pid: Option<u32>,
        cmd: &[&str],
    ) -> super::RawProvenance {
        super::RawProvenance {
            pid,
            start_time_secs,
            executable: None,
            cmd: cmd.iter().map(Into::into).collect(),
            owner_uid: None,
            parent_pid,
        }
    }

    #[test]
    fn provenance_is_empty_before_the_first_process_refresh() {
        let collector = Collector::new().unwrap();
        assert!(collector.provenance().is_empty());
    }

    #[test]
    fn aggregate_refresh_does_not_change_provenance() {
        let mut collector = Collector::new().unwrap();
        collector.refresh_processes();
        let provenance = collector.provenance().clone();

        collector.refresh_aggregate();

        assert_eq!(collector.provenance(), &provenance);
    }

    #[test]
    fn parent_identity_resolves_within_one_scan() {
        use crate::provenance::ParentContext;

        let provenance = super::build_provenance(vec![
            raw_provenance(10, 100, None, &["parent"]),
            raw_provenance(11, 101, Some(10), &["child"]),
        ]);

        assert_eq!(
            provenance[&(11, 101)].parent,
            ParentContext::Resolved {
                pid: 10,
                start_time_secs: 100
            }
        );
    }

    #[test]
    fn missing_parent_is_pid_only() {
        use crate::provenance::ParentContext;

        let provenance =
            super::build_provenance(vec![raw_provenance(11, 101, Some(10), &["child"])]);

        assert_eq!(provenance[&(11, 101)].parent, ParentContext::PidOnly(10));
    }

    #[test]
    fn no_parent_is_distinct_from_pid_only() {
        use crate::provenance::ParentContext;

        let provenance = super::build_provenance(vec![raw_provenance(11, 101, None, &["child"])]);

        assert_eq!(provenance[&(11, 101)].parent, ParentContext::NoParent);
    }

    #[test]
    fn pid_reuse_creates_separate_provenance_entries() {
        let provenance = super::build_provenance(vec![
            raw_provenance(11, 101, None, &["old"]),
            raw_provenance(11, 202, None, &["new"]),
        ]);

        assert_eq!(provenance.len(), 2);
        assert!(provenance.contains_key(&(11, 101)));
        assert!(provenance.contains_key(&(11, 202)));
    }

    #[test]
    fn unavailable_fields_remain_none() {
        let provenance = super::build_provenance(vec![raw_provenance(11, 101, None, &[])]);
        let process = &provenance[&(11, 101)];

        assert_eq!(process.executable, None);
        assert_eq!(process.owner_uid, None);
        assert_eq!(process.program, None);
    }

    #[test]
    fn command_arguments_are_not_stored() {
        let provenance = super::build_provenance(vec![raw_provenance(
            11,
            101,
            None,
            &["worker", "--token", "hunter2"],
        )]);
        let debug = format!("{provenance:?}");

        assert_eq!(provenance[&(11, 101)].program.as_deref(), Some("worker"));
        assert!(!debug.contains("--token"));
        assert!(!debug.contains("hunter2"));
    }

    #[test]
    fn build_provenance_is_deterministic() {
        let rows = || {
            vec![
                raw_provenance(11, 101, Some(10), &["child"]),
                raw_provenance(10, 100, None, &["parent"]),
            ]
        };

        assert_eq!(
            super::build_provenance(rows()),
            super::build_provenance(rows())
        );
    }

    #[cfg(unix)]
    #[test]
    fn live_scan_enriches_the_current_process() {
        let mut collector = Collector::new().unwrap();
        let current = std::process::id();
        let start_time_secs = collector
            .refresh_processes()
            .iter()
            .find(|process| process.pid == current)
            .map(|process| process.start_time_secs)
            .unwrap();

        assert!(
            collector
                .provenance_for((current, start_time_secs))
                .is_some()
        );
    }

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
        let snapshot = snapshot_at(
            123,
            42.5,
            Some(123),
            memory.clone(),
            processes.clone(),
            Some(124),
        );

        assert_eq!(snapshot.timestamp_ms, 123);
        assert_eq!(snapshot.cpu_usage_percent, 42.5);
        assert_eq!(snapshot.cpu_updated_at_ms, Some(123));
        assert_eq!(snapshot.memory, memory);
        assert_eq!(snapshot.processes, processes);
        assert_eq!(snapshot.processes_updated_at_ms, Some(124));
    }

    #[test]
    fn collector_refresh_returns_snapshot_with_processes() {
        let mut collector = Collector::new().unwrap();

        let first = collector.refresh();
        assert_eq!(first.cpu_usage_percent, 0.0);
        assert_eq!(first.cpu_updated_at_ms, None);
        assert!(first.processes_updated_at_ms.is_some());
        assert!(
            first
                .processes
                .iter()
                .any(|process| process.pid == std::process::id())
        );
        assert_eq!(collector.process_refresh_calls(), 1);
        assert!(
            collector
                .refresh()
                .processes
                .iter()
                .any(|process| process.pid == std::process::id())
        );
        assert_eq!(collector.process_refresh_calls(), 2);
    }

    #[test]
    fn aggregate_refresh_does_not_enumerate_processes() {
        let mut collector = Collector::new().unwrap();

        collector.refresh_aggregate();
        collector.refresh_aggregate();

        assert_eq!(collector.process_refresh_calls(), 0);
        assert!(collector.cached_processes().is_empty());
        assert_eq!(collector.processes_updated_at_ms(), None);
    }

    #[test]
    fn aggregate_refresh_preserves_cached_processes() {
        let mut collector = Collector::new().unwrap();
        collector.refresh();
        let identities: Vec<_> = collector
            .cached_processes()
            .iter()
            .map(ProcessSnapshot::identity)
            .collect();
        let updated_at = collector.processes_updated_at_ms();
        let calls = collector.process_refresh_calls();

        collector.refresh_aggregate();

        assert_eq!(
            collector
                .cached_processes()
                .iter()
                .map(ProcessSnapshot::identity)
                .collect::<Vec<_>>(),
            identities
        );
        assert_eq!(collector.processes_updated_at_ms(), updated_at);
        assert_eq!(collector.process_refresh_calls(), calls);
    }

    #[test]
    fn aggregate_refresh_preserves_cpu_warm_up() {
        let mut collector = Collector::new().unwrap();
        assert_eq!(collector.refresh_aggregate().cpu_updated_at_ms, None);
        assert_eq!(collector.refresh_aggregate().cpu_updated_at_ms, None);

        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(1));
        let sample = collector.refresh_aggregate();
        assert!(sample.cpu_updated_at_ms.is_some());
        assert_eq!(sample.cpu_updated_at_ms, Some(sample.timestamp_ms));
        assert!(sample.cpu_usage_percent.is_finite());
    }

    #[test]
    fn process_timestamp_changes_only_on_process_refresh() {
        let mut collector = Collector::new().unwrap();
        assert_eq!(collector.processes_updated_at_ms(), None);
        collector.refresh_aggregate();
        assert_eq!(collector.processes_updated_at_ms(), None);

        collector.refresh_processes();
        let updated_at = collector.processes_updated_at_ms();
        assert!(updated_at.is_some());
        collector.refresh_aggregate();
        assert_eq!(collector.processes_updated_at_ms(), updated_at);
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

    #[cfg(unix)]
    #[test]
    fn collector_refresh_tolerates_process_exit() {
        let mut collector = Collector::new().unwrap();
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();

        assert!(
            collector
                .refresh_processes()
                .iter()
                .any(|process| process.pid == pid)
        );
        child.kill().unwrap();
        child.wait().unwrap();

        assert!(
            !collector
                .refresh_processes()
                .iter()
                .any(|process| process.pid == pid)
        );
    }
}
