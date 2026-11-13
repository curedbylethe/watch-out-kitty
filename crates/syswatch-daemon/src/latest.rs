use syswatch_core::{AggregateSnapshot, FindingId, SuggestionsSnapshot, SystemSnapshot};

use crate::{dismissal::DismissedFindings, sampler::ProcessScan};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cached {
    pub snapshot: Option<SystemSnapshot>,
    pub suggestions: Option<SuggestionsSnapshot>,
}

#[derive(Default)]
pub struct LatestSnapshot {
    aggregate: Option<AggregateSnapshot>,
    processes: Option<ProcessScan>,
    dismissed: DismissedFindings,
}

impl LatestSnapshot {
    pub fn set_aggregate(&mut self, aggregate: AggregateSnapshot) {
        self.aggregate = Some(aggregate);
    }
    pub fn set_processes(&mut self, processes: ProcessScan) {
        self.dismissed
            .retain_observed(processes.processes.iter().map(|process| process.identity()));
        self.processes = Some(processes);
    }

    pub fn dismiss(&mut self, id: FindingId) {
        self.dismissed.dismiss(id);
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

    /// Serves the cached suggestions without dismissed findings. The raw scan
    /// keeps every finding.
    pub fn cached(&self) -> Cached {
        Cached {
            snapshot: self.snapshot(),
            suggestions: self
                .processes
                .as_ref()
                .and_then(|scan| scan.suggestions.as_ref())
                .map(|suggestions| {
                    let mut visible = suggestions.clone();
                    visible
                        .findings
                        .retain(|finding| !self.dismissed.is_dismissed(&finding.id));
                    visible
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cached, LatestSnapshot};
    use crate::sampler::ProcessScan;
    use syswatch_core::{
        AdvisoryFinding, AggregateSnapshot, Confidence, FindingEvidence, FindingId, FindingKind,
        MemorySnapshot, ProcessSnapshot, SuggestionsSnapshot,
    };

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

    fn process(pid: u32, start_time_secs: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: Some(1),
            name: "worker".into(),
            cpu_usage_percent: 0.0,
            memory_bytes: 1,
            start_time_secs,
        }
    }

    fn finding(pid: u32, start_time_secs: u64) -> AdvisoryFinding {
        AdvisoryFinding {
            id: FindingId {
                pid,
                start_time_secs,
                kind: FindingKind::Cpu,
            },
            confidence: Confidence::High,
            observed_at_ms: 20,
            explanation: "pid used CPU continuously".into(),
            evidence: FindingEvidence {
                sample_count: 2,
                span_ms: 120_000,
                cpu: None,
                memory: None,
            },
        }
    }

    fn scan(processes: Vec<ProcessSnapshot>, findings: Vec<AdvisoryFinding>) -> ProcessScan {
        ProcessScan {
            processes,
            observed_at_ms: 20,
            suggestions: Some(SuggestionsSnapshot {
                evaluated_at_ms: 20,
                processes_updated_at_ms: 20,
                findings,
            }),
        }
    }

    fn visible_ids(latest: &LatestSnapshot) -> Vec<(u32, u64)> {
        latest
            .cached()
            .suggestions
            .unwrap()
            .findings
            .iter()
            .map(|finding| (finding.id.pid, finding.id.start_time_secs))
            .collect()
    }

    #[test]
    fn dismissed_finding_is_hidden_but_stays_in_the_raw_scan() {
        let mut latest = LatestSnapshot::default();
        latest.set_processes(scan(
            vec![process(42, 1), process(43, 1)],
            vec![finding(42, 1), finding(43, 1)],
        ));
        latest.dismiss(finding(42, 1).id);
        assert_eq!(visible_ids(&latest), vec![(43, 1)]);
        let raw = latest
            .processes
            .as_ref()
            .unwrap()
            .suggestions
            .as_ref()
            .unwrap();
        assert_eq!(raw.findings.len(), 2);
    }

    #[test]
    fn dismissal_survives_repeated_scans_and_expires_on_absence() {
        let mut latest = LatestSnapshot::default();
        let both = scan(
            vec![process(42, 1), process(43, 1)],
            vec![finding(42, 1), finding(43, 1)],
        );
        latest.set_processes(both.clone());
        latest.dismiss(finding(42, 1).id);
        latest.set_processes(both);
        assert_eq!(visible_ids(&latest), vec![(43, 1)]);

        latest.set_processes(scan(vec![process(43, 1)], vec![finding(43, 1)]));
        latest.set_processes(scan(
            vec![process(42, 1), process(43, 1)],
            vec![finding(42, 1), finding(43, 1)],
        ));
        assert_eq!(visible_ids(&latest), vec![(42, 1), (43, 1)]);
    }

    #[test]
    fn reused_pid_is_visible_and_old_identity_is_not_dismissed() {
        let mut latest = LatestSnapshot::default();
        latest.set_processes(scan(vec![process(42, 1)], vec![finding(42, 1)]));
        latest.dismiss(finding(42, 1).id);
        latest.set_processes(scan(vec![process(42, 2)], vec![finding(42, 2)]));
        assert_eq!(visible_ids(&latest), vec![(42, 2)]);
        assert!(!latest.dismissed.is_dismissed(&finding(42, 1).id));
    }

    #[test]
    fn missing_suggestions_stay_none_after_dismissal() {
        let mut latest = LatestSnapshot::default();
        latest.set_processes(ProcessScan {
            processes: vec![process(42, 1)],
            observed_at_ms: 20,
            suggestions: None,
        });
        latest.dismiss(finding(42, 1).id);
        assert!(latest.cached().suggestions.is_none());
    }

    #[test]
    fn concurrent_dismissals_and_reads_leave_consistent_final_state() {
        use std::sync::{Arc, Mutex};

        let findings = (1..=8).map(|pid| finding(pid, 1)).collect::<Vec<_>>();
        let processes = (1..=8).map(|pid| process(pid, 1)).collect::<Vec<_>>();
        let mut seeded = LatestSnapshot::default();
        seeded.set_processes(scan(processes, findings));
        let shared = Arc::new(Mutex::new(seeded));

        let writers = (0..2).map(|slot| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                for pid in [slot * 2 + 1, slot * 2 + 2] {
                    shared.lock().unwrap().dismiss(finding(pid, 1).id);
                }
            })
        });
        let readers = (0..2).map(|_| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let _ = shared.lock().unwrap().cached();
                }
            })
        });
        for handle in writers.chain(readers) {
            handle.join().unwrap();
        }

        let latest = shared.lock().unwrap();
        assert_eq!(visible_ids(&latest), vec![(5, 1), (6, 1), (7, 1), (8, 1)]);
        let raw = latest
            .processes
            .as_ref()
            .unwrap()
            .suggestions
            .as_ref()
            .unwrap();
        assert_eq!(raw.findings.len(), 8);
    }
}
