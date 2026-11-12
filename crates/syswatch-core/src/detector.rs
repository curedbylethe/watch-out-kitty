//! History-only candidates are not findings or eligibility decisions.
//! They do not establish that a process is idle, abandoned, or safe to terminate.

use std::collections::BTreeMap;

use crate::eligibility::{self, EligibilityState};
use crate::findings::{self, AdvisoryFinding, Confidence, FindingEvidence, FindingId, FindingKind};
use crate::history::ProcessHistory;
use crate::provenance::ProcessProvenance;

/// Default minimum memory, in bytes (256 MiB).
pub const DEFAULT_MIN_MEMORY_BYTES: u64 = 268_435_456;
/// Default maximum CPU usage percentage across valid samples.
pub const DEFAULT_MAX_IDLE_CPU_PERCENT: f32 = 1.0;
/// Default minimum count of valid observations.
pub const DEFAULT_MIN_VALID_OBSERVATIONS: usize = 6;
/// Default minimum span of valid observations, in milliseconds.
pub const DEFAULT_MIN_SPAN_MS: u64 = 120_000;
/// Default maximum age of the latest scan, in milliseconds.
pub const DEFAULT_MAX_STALENESS_MS: u64 = 30_000;

/// Thresholds for history-only candidate evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Minimum latest memory in bytes.
    pub min_memory_bytes: u64,
    /// Maximum allowed CPU usage percentage in every valid sample.
    pub max_idle_cpu_percent: f32,
    /// Minimum count of valid CPU observations.
    pub min_valid_observations: usize,
    /// Minimum span of valid CPU observations in milliseconds.
    pub min_span_ms: u64,
    /// Maximum age of the latest scan in milliseconds.
    pub max_staleness_ms: u64,
}
impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            min_memory_bytes: DEFAULT_MIN_MEMORY_BYTES,
            max_idle_cpu_percent: DEFAULT_MAX_IDLE_CPU_PERCENT,
            min_valid_observations: DEFAULT_MIN_VALID_OBSERVATIONS,
            min_span_ms: DEFAULT_MIN_SPAN_MS,
            max_staleness_ms: DEFAULT_MAX_STALENESS_MS,
        }
    }
}
/// A history-only candidate, without any protection or eligibility assessment.
#[derive(Debug, Clone, PartialEq)]
pub struct IdleMemoryCandidate {
    /// Process identity, including start time.
    pub identity: (u32, u64),
    /// Newest valid CPU observation timestamp in milliseconds.
    pub observed_at_ms: u64,
    /// Memory bytes in the newest supporting observation.
    pub latest_memory_bytes: u64,
    /// Maximum CPU percentage across valid samples.
    pub max_idle_cpu_percent: f32,
    /// Number of valid CPU samples.
    pub valid_samples: usize,
    /// Span of valid CPU samples in milliseconds.
    pub span_ms: u64,
}
/// Returns candidates based only on retained observations and thresholds.
pub fn idle_memory_candidates(
    history: &ProcessHistory,
    now_ms: u64,
    config: &DetectorConfig,
) -> Vec<IdleMemoryCandidate> {
    if config.min_valid_observations < 2 || config.min_span_ms == 0 {
        return vec![];
    }
    let mut candidates = Vec::new();
    for record in history.iter() {
        if record.disappeared_at_ms.is_some()
            || now_ms.saturating_sub(record.last_seen_ms) > config.max_staleness_ms
        {
            continue;
        }
        let observations: Vec<_> = record
            .observations()
            .iter()
            .filter(|o| o.timestamp_ms <= now_ms)
            .collect();
        let Some(latest) = observations.iter().max_by_key(|o| o.timestamp_ms) else {
            continue;
        };
        if now_ms.saturating_sub(latest.timestamp_ms) > config.max_staleness_ms {
            continue;
        }
        let latest_memory = latest.memory_bytes;
        let mut idle = Vec::new();
        let mut busy = false;
        for observation in observations {
            if let Some(cpu) = observation.cpu_usage_percent {
                if !cpu.is_finite()
                    || cpu
                        .partial_cmp(&config.max_idle_cpu_percent)
                        .is_none_or(|order| order.is_gt())
                {
                    busy = true;
                    break;
                }
                idle.push((observation.timestamp_ms, cpu));
            }
        }
        if busy
            || idle.len() < config.min_valid_observations
            || latest_memory < config.min_memory_bytes
        {
            continue;
        }
        idle.sort_by_key(|sample| sample.0);
        if now_ms.saturating_sub(idle.last().unwrap().0) > config.max_staleness_ms {
            continue;
        }
        let span_ms = idle.last().unwrap().0.saturating_sub(idle[0].0);
        if span_ms < config.min_span_ms {
            continue;
        }
        candidates.push(IdleMemoryCandidate {
            identity: record.identity(),
            observed_at_ms: idle.last().unwrap().0,
            latest_memory_bytes: latest_memory,
            max_idle_cpu_percent: idle
                .iter()
                .map(|sample| sample.1)
                .fold(f32::NEG_INFINITY, f32::max),
            valid_samples: idle.len(),
            span_ms,
        });
    }
    candidates
}

/// Returns advisory findings for eligible possible idle memory consumers.
pub fn detect_idle_memory_consumers(
    history: &ProcessHistory,
    now_ms: u64,
    provenance: &BTreeMap<(u32, u64), ProcessProvenance>,
    config: &DetectorConfig,
) -> Vec<AdvisoryFinding> {
    let monitor = eligibility::monitor_identity(provenance);
    let mut findings = Vec::new();
    for candidate in idle_memory_candidates(history, now_ms, config) {
        if eligibility::assess(candidate.identity, provenance, monitor).state
            != EligibilityState::Eligible
        {
            continue;
        }
        let Some(record) = history.get(candidate.identity) else {
            continue;
        };
        let evidence = FindingEvidence::from_record(record);
        let confidence = if evidence
            .cpu
            .as_ref()
            .is_some_and(|cpu| cpu.valid_samples == evidence.sample_count)
        {
            Confidence::High
        } else {
            Confidence::Medium
        };
        findings.push(AdvisoryFinding {
            id: FindingId {
                pid: candidate.identity.0,
                start_time_secs: candidate.identity.1,
                kind: FindingKind::Memory,
            },
            confidence,
            observed_at_ms: candidate.observed_at_ms,
            explanation: format!(
                "pid {} held {} MiB of memory while using at most {:.1}% CPU across {} samples over {} s; review this process",
                candidate.identity.0,
                candidate.latest_memory_bytes / 1_048_576,
                candidate.max_idle_cpu_percent,
                candidate.valid_samples,
                candidate.span_ms / 1000,
            ),
            evidence,
        });
    }
    findings::sort(&mut findings);
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::ProcessSnapshot;
    fn process(cpu: f32, memory: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            pid: 42,
            parent_pid: None,
            name: "worker".into(),
            cpu_usage_percent: cpu,
            memory_bytes: memory,
            start_time_secs: 1,
        }
    }
    fn history(samples: &[(u64, f32, u64)]) -> ProcessHistory {
        let mut h = ProcessHistory::new();
        for &(time, cpu, memory) in samples {
            h.record_scan(time, &[process(cpu, memory)]);
        }
        h
    }
    fn samples(cpu: f32, memory: u64) -> Vec<(u64, f32, u64)> {
        vec![
            (0, cpu, memory),
            (1, cpu, memory),
            (24_001, cpu, memory),
            (48_001, cpu, memory),
            (72_001, cpu, memory),
            (96_001, cpu, memory),
            (120_001, cpu, memory),
        ]
    }
    fn candidates(h: &ProcessHistory, now: u64) -> Vec<IdleMemoryCandidate> {
        idle_memory_candidates(h, now, &DetectorConfig::default())
    }
    #[test]
    fn memory_threshold_is_inclusive() {
        let c = DetectorConfig::default();
        assert_eq!(
            candidates(&history(&samples(0.0, c.min_memory_bytes)), 120_001).len(),
            1
        );
        assert!(candidates(&history(&samples(0.0, c.min_memory_bytes - 1)), 120_001).is_empty());
    }

    #[test]
    fn idle_cpu_threshold_is_inclusive() {
        let mut s = samples(1.0, DEFAULT_MIN_MEMORY_BYTES);
        assert_eq!(candidates(&history(&s), 120_001).len(), 1);
        s[3].1 = 1.01;
        assert!(candidates(&history(&s), 120_001).is_empty());
    }
    #[test]
    fn span_floor_is_inclusive() {
        let mut s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES);
        assert_eq!(candidates(&history(&s), 120_001).len(), 1);
        s[6].0 -= 1;
        assert!(candidates(&history(&s), 120_001).is_empty());
    }
    #[test]
    fn observation_floor_is_inclusive() {
        let mut s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES);
        assert_eq!(candidates(&history(&s), 120_001).len(), 1);
        s.remove(2);
        assert!(candidates(&history(&s), 120_001).is_empty());
    }
    #[test]
    fn varying_idle_cpu_still_qualifies() {
        let mut s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES);
        for (sample, cpu) in s[1..].iter_mut().zip([0.0, 0.4, 0.9, 0.2, 0.0, 0.6]) {
            sample.1 = cpu;
        }
        assert_eq!(
            candidates(&history(&s), 120_001)[0].max_idle_cpu_percent,
            0.9
        );
    }
    #[test]
    fn high_memory_active_process_is_not_idle() {
        let mut s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES + 1);
        s[3].1 = 12.5;
        assert!(candidates(&history(&s), 120_001).is_empty());
    }
    #[test]
    fn old_process_with_insufficient_observations_is_rejected() {
        let s = [
            (0, 0.0, DEFAULT_MIN_MEMORY_BYTES),
            (120_000, 0.0, DEFAULT_MIN_MEMORY_BYTES),
            (240_000, 0.0, DEFAULT_MIN_MEMORY_BYTES),
        ];
        assert!(candidates(&history(&s), 240_000).is_empty());
    }
    #[test]
    fn warm_up_samples_are_ignored() {
        let s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES);
        assert_eq!(candidates(&history(&s), 120_001)[0].valid_samples, 6);
        let s = samples(f32::NAN, DEFAULT_MIN_MEMORY_BYTES);
        assert!(candidates(&history(&s), 120_001).is_empty());
    }
    #[test]
    fn outdated_observations_are_rejected() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        assert_eq!(candidates(&h, 150_001).len(), 1);
        assert!(candidates(&h, 150_002).is_empty());
    }
    #[test]
    fn invalid_cpu_samples_do_not_refresh_idle_evidence() {
        let mut h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        h.record_scan(150_001, &[process(f32::NAN, DEFAULT_MIN_MEMORY_BYTES)]);
        assert_eq!(candidates(&h, 150_001).len(), 1);
        h.record_scan(150_002, &[process(f32::NAN, DEFAULT_MIN_MEMORY_BYTES)]);
        assert!(candidates(&h, 150_002).is_empty());
    }
    #[test]
    fn disappeared_record_is_rejected() {
        let mut h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        h.record_scan(120_002, &[]);
        assert!(candidates(&h, 120_002).is_empty());
    }
    #[test]
    fn future_dated_observation_is_ignored() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        assert!(candidates(&h, 120_000).is_empty());
    }
    #[test]
    fn future_scan_cannot_make_old_supporting_observations_fresh() {
        let mut h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        h.record_scan(200_000, &[process(0.0, DEFAULT_MIN_MEMORY_BYTES)]);
        assert!(candidates(&h, 160_002).is_empty());
    }

    #[test]
    fn non_monotonic_history_is_evaluated_by_timestamp() {
        let mut s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES);
        s.swap(2, 6);
        let result = candidates(&history(&s), 120_001);
        assert_eq!(result[0].span_ms, 120_000);
    }
    #[test]
    fn degenerate_config_produces_nothing() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        for n in [0, 1] {
            let c = DetectorConfig {
                min_valid_observations: n,
                ..Default::default()
            };
            assert!(idle_memory_candidates(&h, 120_001, &c).is_empty());
        }
        let c = DetectorConfig {
            min_span_ms: 0,
            ..Default::default()
        };
        assert!(idle_memory_candidates(&h, 120_001, &c).is_empty());
    }
    #[test]
    fn repeated_evaluation_is_deterministic() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        assert_eq!(candidates(&h, 120_001), candidates(&h, 120_001));
    }
    use crate::history::HistoryLimits;
    use crate::provenance::ParentContext;

    fn provenance(
        pid: u32,
        start: u64,
        owner: Option<u32>,
        parent: ParentContext,
        program: &str,
    ) -> ProcessProvenance {
        ProcessProvenance {
            pid,
            start_time_secs: start,
            executable: None,
            program: Some(program.into()),
            owner_uid: owner,
            parent,
        }
    }

    fn scan(rows: Vec<ProcessProvenance>) -> BTreeMap<(u32, u64), ProcessProvenance> {
        rows.into_iter().map(|row| (row.identity(), row)).collect()
    }

    fn eligible(pid: u32, start: u64) -> ProcessProvenance {
        provenance(pid, start, Some(501), ParentContext::NoParent, "worker")
    }

    fn detect(
        h: &ProcessHistory,
        rows: &BTreeMap<(u32, u64), ProcessProvenance>,
    ) -> Vec<AdvisoryFinding> {
        detect_idle_memory_consumers(h, 120_001, rows, &DetectorConfig::default())
    }

    #[test]
    fn eligible_idle_consumer_yields_one_finding() {
        let h = history(&samples(0.8, DEFAULT_MIN_MEMORY_BYTES));
        let rows = scan(vec![eligible(42, 1)]);
        let result = detect(&h, &rows);
        assert_eq!(result.len(), 1);
        let finding = &result[0];
        assert_eq!(finding.id.identity(), (42, 1));
        assert_eq!(finding.id.kind, FindingKind::Memory);
        assert_eq!(finding.observed_at_ms, 120_001);
        assert_eq!(
            finding.explanation,
            "pid 42 held 256 MiB of memory while using at most 0.8% CPU across 6 samples over 120 s; review this process"
        );
        assert_eq!(finding.evidence.sample_count, 7);
        assert_eq!(finding.evidence.span_ms, 120_001);
    }

    #[test]
    fn protected_owner_is_excluded() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        for uid in [0, 499] {
            let rows = scan(vec![provenance(
                42,
                1,
                Some(uid),
                ParentContext::NoParent,
                "worker",
            )]);
            assert!(detect(&h, &rows).is_empty());
        }
    }

    #[test]
    fn missing_provenance_is_excluded() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        assert!(detect(&h, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn monitor_process_is_excluded() {
        let pid = std::process::id();
        let mut h = ProcessHistory::new();
        for (time, cpu, memory) in samples(0.0, DEFAULT_MIN_MEMORY_BYTES) {
            let mut p = process(cpu, memory);
            p.pid = pid;
            h.record_scan(time, &[p]);
        }
        assert!(detect(&h, &scan(vec![eligible(pid, 1)])).is_empty());
    }

    #[test]
    fn active_terminal_chain_is_excluded() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        let rows = scan(vec![
            provenance(
                42,
                1,
                Some(501),
                ParentContext::Resolved {
                    pid: 50,
                    start_time_secs: 1,
                },
                "worker",
            ),
            provenance(50, 1, Some(501), ParentContext::NoParent, "zsh"),
        ]);
        assert!(detect(&h, &rows).is_empty());
    }

    #[test]
    fn standalone_terminal_program_is_excluded() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        for label in ["zsh", "bash", "kitty", "tmux"] {
            let rows = scan(vec![provenance(
                42,
                1,
                Some(501),
                ParentContext::NoParent,
                label,
            )]);
            assert!(detect(&h, &rows).is_empty(), "{label}");
        }
    }

    #[test]
    fn unknown_ancestry_is_excluded() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        let rows = scan(vec![provenance(
            42,
            1,
            Some(501),
            ParentContext::PidOnly(50),
            "worker",
        )]);
        assert!(detect(&h, &rows).is_empty());
    }

    #[test]
    fn pid_reuse_does_not_inherit_a_finding() {
        let mut h = ProcessHistory::new();
        for (time, cpu, memory) in samples(0.0, DEFAULT_MIN_MEMORY_BYTES) {
            let mut p = process(cpu, memory);
            p.start_time_secs = 1000;
            h.record_scan(time, &[p]);
        }
        let old = detect(&h, &scan(vec![eligible(42, 1000)]));
        assert_eq!(old[0].id.start_time_secs, 1000);
        let mut p = process(12.5, DEFAULT_MIN_MEMORY_BYTES);
        p.start_time_secs = 2000;
        h.record_scan(120_002, &[p.clone()]);
        h.record_scan(120_003, &[p]);
        let new = detect_idle_memory_consumers(
            &h,
            120_003,
            &scan(vec![eligible(42, 2000)]),
            &DetectorConfig::default(),
        );
        assert!(new.is_empty());
        assert_ne!(
            old[0].id,
            FindingId {
                pid: 42,
                start_time_secs: 2000,
                kind: FindingKind::Memory
            }
        );
    }

    #[test]
    fn high_memory_active_process_yields_no_finding() {
        let mut s = samples(0.0, DEFAULT_MIN_MEMORY_BYTES + 1);
        s[3].1 = 12.5;
        assert!(detect(&history(&s), &scan(vec![eligible(42, 1)])).is_empty());
    }

    #[test]
    fn confidence_is_medium_with_a_warm_up_sample_and_high_when_all_are_valid() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        let rows = scan(vec![eligible(42, 1)]);
        assert_eq!(detect(&h, &rows)[0].confidence, Confidence::Medium);
        let mut h = ProcessHistory::with_limits(HistoryLimits {
            max_observations_per_process: 6,
            ..Default::default()
        });
        for i in 0..7 {
            h.record_scan(i * 24_000, &[process(0.0, DEFAULT_MIN_MEMORY_BYTES)]);
        }
        assert_eq!(
            detect_idle_memory_consumers(&h, 144_000, &rows, &DetectorConfig::default())[0]
                .confidence,
            Confidence::High
        );
    }

    #[test]
    fn explanation_never_claims_safety() {
        let h = history(&samples(0.8, DEFAULT_MIN_MEMORY_BYTES));
        let finding = &detect(&h, &scan(vec![eligible(42, 1)]))[0];
        assert!(finding.explanation.contains("review this process"));
        for word in ["safe", "stale", "orphan", "kill"] {
            assert!(!finding.explanation.contains(word));
        }
        assert!(finding.explanation.contains("256 MiB"));
        assert!(
            finding
                .explanation
                .contains("0.8% CPU across 6 samples over 120 s")
        );
    }

    #[test]
    fn findings_are_ordered_by_identity() {
        let mut h = ProcessHistory::new();
        for (time, cpu, memory) in samples(0.0, DEFAULT_MIN_MEMORY_BYTES) {
            let p = process(cpu, memory);
            let mut other = p.clone();
            other.pid = 43;
            h.record_scan(time, &[other, p]);
        }
        let result = detect(&h, &scan(vec![eligible(43, 1), eligible(42, 1)]));
        assert_eq!(
            result.iter().map(|f| f.id.pid).collect::<Vec<_>>(),
            [42, 43]
        );
    }

    #[test]
    fn findings_repeated_evaluation_is_deterministic() {
        let h = history(&samples(0.0, DEFAULT_MIN_MEMORY_BYTES));
        let rows = scan(vec![eligible(42, 1)]);
        assert_eq!(detect(&h, &rows), detect(&h, &rows));
    }

    #[test]
    fn empty_history_yields_no_findings() {
        assert!(detect(&ProcessHistory::new(), &scan(vec![eligible(42, 1)])).is_empty());
    }
}
