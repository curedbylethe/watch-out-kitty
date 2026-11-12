//! Advisory findings describe evidence and invite inspection.
//!
//! A confidence level describes evidence coverage. No level asserts that a
//! process is safe or safe to terminate.

use crate::history::ProcessRecord;

/// Machine-readable evidence retained for an advisory finding.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FindingEvidence {
    /// Number of retained process observations.
    pub sample_count: usize,
    /// Time span across retained observations, in milliseconds.
    pub span_ms: u64,
    /// Aggregate CPU activity, when any retained sample has valid CPU data.
    pub cpu: Option<CpuActivity>,
    /// Aggregate memory activity, when at least one observation is retained.
    pub memory: Option<MemoryActivity>,
}

/// Aggregate CPU activity from valid retained samples.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CpuActivity {
    /// Maximum valid CPU usage, in percent; values may exceed 100 on multicore systems.
    pub max_percent: f32,
    /// Mean valid CPU usage, in percent; values may exceed 100 on multicore systems.
    pub mean_percent: f32,
    /// Number of finite CPU samples included in the aggregate.
    pub valid_samples: usize,
}

/// Aggregate memory activity from retained observations.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryActivity {
    /// Memory usage at the newest observation, in bytes.
    pub latest_bytes: u64,
    /// Maximum memory usage across retained observations, in bytes.
    pub max_bytes: u64,
}

impl FindingEvidence {
    /// Aggregates only the samples currently retained for a process.
    pub fn from_record(record: &ProcessRecord) -> Self {
        let observations = record.observations();
        let sample_count = observations.len();
        let timestamps = || {
            observations
                .iter()
                .map(|observation| observation.timestamp_ms)
        };
        let span_ms = timestamps()
            .max()
            .zip(timestamps().min())
            .map_or(0, |(max, min)| max - min);

        let mut valid_cpu = observations
            .iter()
            .filter_map(|observation| observation.cpu_usage_percent)
            .filter(|value| value.is_finite());
        let cpu = valid_cpu.next().map(|first| {
            let (sum, max, count) = valid_cpu
                .fold((f64::from(first), first, 1), |(sum, max, count), value| {
                    (sum + f64::from(value), max.max(value), count + 1)
                });
            CpuActivity {
                max_percent: max,
                mean_percent: (sum / count as f64) as f32,
                valid_samples: count,
            }
        });

        let memory = observations
            .iter()
            .max_by_key(|observation| observation.timestamp_ms)
            .zip(
                observations
                    .iter()
                    .map(|observation| observation.memory_bytes)
                    .max(),
            )
            .map(|(latest, max_bytes)| MemoryActivity {
                latest_bytes: latest.memory_bytes,
                max_bytes,
            });

        Self {
            sample_count,
            span_ms,
            cpu,
            memory,
        }
    }
}

/// Kind of advisory finding.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// CPU activity finding.
    Cpu,
    /// Memory activity finding.
    Memory,
}

/// Stable identity for one finding about one process and kind.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct FindingId {
    /// Operating system process identifier.
    pub pid: u32,
    /// Process start time in seconds since the Unix epoch.
    pub start_time_secs: u64,
    /// Kind of finding.
    pub kind: FindingKind,
}

impl FindingId {
    /// Returns process identity, which includes start time because operating
    /// systems reuse PIDs.
    pub fn identity(&self) -> (u32, u64) {
        (self.pid, self.start_time_secs)
    }
}

/// Strength of evidence supporting a finding.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Fewer than two retained observations support the finding, or no valid
    /// measurement exists for its kind.
    Low,
    /// At least two retained observations support the finding, but one or more
    /// retained observations lack a valid measurement for its kind.
    Medium,
    /// Every retained observation has a valid measurement for the finding kind,
    /// and at least two observations are retained.
    High,
}

/// Advisory description of evidence about one process.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AdvisoryFinding {
    /// Stable identity for this process and finding kind.
    pub id: FindingId,
    /// Strength of evidence supporting the finding.
    pub confidence: Confidence,
    /// Timestamp of the newest observation supporting the finding, in
    /// milliseconds since the Unix epoch.
    pub observed_at_ms: u64,
    /// Human-readable explanation naming the process, kind, and observation
    /// window.
    pub explanation: String,
    /// Typed aggregates from retained process observations.
    pub evidence: FindingEvidence,
}

/// Advisory findings evaluated for one process scan.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SuggestionsSnapshot {
    /// Evaluation time in milliseconds since the Unix epoch.
    pub evaluated_at_ms: u64,
    /// Process scan time in milliseconds since the Unix epoch.
    pub processes_updated_at_ms: u64,
    /// Findings from the evaluation.
    pub findings: Vec<AdvisoryFinding>,
}

/// Sorts findings by stable ID, then observation time.
pub fn sort(findings: &mut [AdvisoryFinding]) {
    findings.sort_by_key(|finding| (finding.id, finding.observed_at_ms));
}

#[cfg(test)]
mod tests {
    use super::{AdvisoryFinding, Confidence, FindingEvidence, FindingId, FindingKind, sort};
    use crate::history::{HistoryLimits, ProcessHistory};
    use crate::snapshot::ProcessSnapshot;

    fn process(pid: u32, start_time_secs: u64, cpu: f32, memory: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: Some(1),
            name: "worker".into(),
            cpu_usage_percent: cpu,
            memory_bytes: memory,
            start_time_secs,
        }
    }

    fn record(
        samples: &[(u64, f32, u64)],
        max_age_ms: u64,
        max_observations: usize,
    ) -> crate::history::ProcessRecord {
        let mut history = ProcessHistory::with_limits(HistoryLimits {
            max_processes: 10,
            max_observations_per_process: max_observations,
            max_age_ms,
        });
        for &(timestamp, cpu, memory) in samples {
            history.record_scan(timestamp, &[process(42, 1_700_000_000, cpu, memory)]);
        }
        history.get((42, 1_700_000_000)).unwrap().clone()
    }

    fn finding(
        pid: u32,
        start_time_secs: u64,
        kind: FindingKind,
        observed_at_ms: u64,
    ) -> AdvisoryFinding {
        AdvisoryFinding {
            id: FindingId {
                pid,
                start_time_secs,
                kind,
            },
            confidence: Confidence::Medium,
            observed_at_ms,
            explanation: "pid 42 (worker) used CPU continuously over the retained window".into(),
            evidence: FindingEvidence {
                sample_count: 0,
                span_ms: 0,
                cpu: None,
                memory: None,
            },
        }
    }

    #[test]
    fn finding_id_is_deterministic_for_identical_inputs() {
        let id = FindingId {
            pid: 42,
            start_time_secs: 1_700_000_000,
            kind: FindingKind::Cpu,
        };
        let mut findings = [finding(42, 1_700_000_000, FindingKind::Cpu, 1)];
        let mut repeated = [finding(42, 1_700_000_000, FindingKind::Cpu, 1)];

        assert_eq!(id, repeated[0].id);
        sort(&mut findings);
        sort(&mut repeated);
        assert_eq!(findings, repeated);
    }

    #[test]
    fn pid_reuse_yields_a_different_finding_id() {
        let old = FindingId {
            pid: 42,
            start_time_secs: 1,
            kind: FindingKind::Cpu,
        };
        let new = FindingId {
            pid: 42,
            start_time_secs: 2,
            kind: FindingKind::Cpu,
        };

        assert_ne!(old, new);
        assert_ne!(old.cmp(&new), std::cmp::Ordering::Equal);
        assert_eq!(old.identity(), (42, 1));
    }

    #[test]
    fn finding_kind_changes_the_finding_id() {
        let cpu = FindingId {
            pid: 42,
            start_time_secs: 1,
            kind: FindingKind::Cpu,
        };
        let memory = FindingId {
            kind: FindingKind::Memory,
            ..cpu
        };

        assert_ne!(cpu, memory);
    }

    #[test]
    fn finding_round_trips() {
        let finding = finding(42, 1_700_000_000, FindingKind::Cpu, 1_700_000_000_123);
        let serialized = serde_json::to_string(&finding).unwrap();
        let deserialized: AdvisoryFinding = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, finding);
        assert_eq!(deserialized.explanation, finding.explanation);
    }

    #[test]
    fn finding_json_shape_is_stable() {
        let finding = finding(42, 1_700_000_000, FindingKind::Cpu, 1_700_000_000_123);

        assert_eq!(
            serde_json::to_string(&finding).unwrap(),
            r#"{"id":{"pid":42,"start_time_secs":1700000000,"kind":"cpu"},"confidence":"medium","observed_at_ms":1700000000123,"explanation":"pid 42 (worker) used CPU continuously over the retained window","evidence":{"sample_count":0,"span_ms":0,"cpu":null,"memory":null}}"#
        );
    }

    #[test]
    fn kind_and_confidence_use_stable_wire_names() {
        assert_eq!(serde_json::to_string(&FindingKind::Cpu).unwrap(), "\"cpu\"");
        assert_eq!(
            serde_json::to_string(&FindingKind::Memory).unwrap(),
            "\"memory\""
        );
        assert_eq!(serde_json::to_string(&Confidence::Low).unwrap(), "\"low\"");
        assert_eq!(
            serde_json::to_string(&Confidence::Medium).unwrap(),
            "\"medium\""
        );
        assert_eq!(
            serde_json::to_string(&Confidence::High).unwrap(),
            "\"high\""
        );

        for kind in [FindingKind::Cpu, FindingKind::Memory] {
            assert_eq!(
                serde_json::from_str::<FindingKind>(&serde_json::to_string(&kind).unwrap())
                    .unwrap(),
                kind
            );
        }
        for confidence in [Confidence::Low, Confidence::Medium, Confidence::High] {
            assert_eq!(
                serde_json::from_str::<Confidence>(&serde_json::to_string(&confidence).unwrap())
                    .unwrap(),
                confidence
            );
        }
    }

    #[test]
    fn sort_is_deterministic_across_repeated_runs() {
        let inputs = [
            finding(43, 1, FindingKind::Cpu, 3),
            finding(42, 1, FindingKind::Memory, 2),
            finding(42, 1, FindingKind::Cpu, 2),
            finding(42, 1, FindingKind::Cpu, 1),
        ];
        let mut first = inputs.clone();
        let mut second = [
            inputs[2].clone(),
            inputs[0].clone(),
            inputs[3].clone(),
            inputs[1].clone(),
        ];

        sort(&mut first);
        sort(&mut second);

        assert_eq!(first, second);
        assert_eq!(first[0].observed_at_ms, 1);
        assert_eq!(first[1].observed_at_ms, 2);
        assert_eq!(first[2].id.kind, FindingKind::Memory);
    }

    #[test]
    fn explanation_survives_round_trip() {
        let explanation = "pid 42 (worker) used CPU continuously over the retained window";
        let finding = finding(42, 1_700_000_000, FindingKind::Cpu, 1);
        let serialized = serde_json::to_string(&finding).unwrap();
        let deserialized: AdvisoryFinding = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.explanation, explanation);
    }

    #[test]
    fn evidence_aggregates_valid_cpu_samples() {
        let record = record(
            &[(0, f32::NAN, 20), (50, 65.0, 40), (200, 95.0, 60)],
            1_000,
            10,
        );
        let evidence = FindingEvidence::from_record(&record);

        let cpu = evidence.cpu.unwrap();
        assert_eq!(cpu.valid_samples, 2);
        assert_eq!(cpu.max_percent, 95.0);
        assert_eq!(cpu.mean_percent, 80.0);
    }

    #[test]
    fn evidence_ignores_cold_and_invalid_cpu_samples() {
        let record = record(&[(0, f32::NAN, 10)], 1_000, 10);
        assert_eq!(FindingEvidence::from_record(&record).cpu, None);
    }

    #[test]
    fn evidence_without_observations_is_missing() {
        let record = record(&[(0, 50.0, 10)], 1_000, 0);
        let evidence = FindingEvidence::from_record(&record);

        assert_eq!(evidence.sample_count, 0);
        assert_eq!(evidence.span_ms, 0);
        assert_eq!(evidence.cpu, None);
        assert_eq!(evidence.memory, None);
    }

    #[test]
    fn evidence_span_uses_retained_samples_only() {
        let record = record(&[(0, 50.0, 10), (50, 55.0, 20), (200, 60.0, 30)], 10, 10);
        let evidence = FindingEvidence::from_record(&record);

        assert_eq!(record.first_seen_ms, 0);
        assert_eq!(evidence.sample_count, 1);
        assert_eq!(evidence.span_ms, 0);
    }

    #[test]
    fn evidence_memory_latest_uses_greatest_timestamp() {
        let record = record(&[(200, 50.0, 20), (100, 55.0, 80)], 1_000, 10);
        let evidence = FindingEvidence::from_record(&record);
        let memory = evidence.memory.unwrap();

        assert_eq!(evidence.span_ms, 100);
        assert_eq!(memory.latest_bytes, 20);
        assert_eq!(memory.max_bytes, 80);
    }

    #[test]
    fn evidence_round_trips_with_missing_measurements() {
        let mut finding = finding(42, 1_700_000_000, FindingKind::Cpu, 10);
        finding.evidence.sample_count = 3;
        finding.evidence.span_ms = 200;
        let serialized = serde_json::to_string(&finding).unwrap();
        assert!(serialized.contains("\"cpu\":null"));

        let decoded: AdvisoryFinding = serde_json::from_str(&serialized).unwrap();
        assert_eq!(decoded, finding);
    }

    #[test]
    fn evidence_json_shape_is_stable() {
        let mut finding = finding(42, 1_700_000_000, FindingKind::Cpu, 1_700_000_000_200);
        finding.evidence = FindingEvidence::from_record(&record(
            &[(0, f32::NAN, 8_192), (100, 65.0, 4_096), (200, 95.0, 6_144)],
            1_000,
            10,
        ));

        assert_eq!(
            serde_json::to_string(&finding).unwrap(),
            r#"{"id":{"pid":42,"start_time_secs":1700000000,"kind":"cpu"},"confidence":"medium","observed_at_ms":1700000000200,"explanation":"pid 42 (worker) used CPU continuously over the retained window","evidence":{"sample_count":3,"span_ms":200,"cpu":{"max_percent":95.0,"mean_percent":80.0,"valid_samples":2},"memory":{"latest_bytes":6144,"max_bytes":8192}}}"#
        );
    }

    #[test]
    fn pid_reuse_keeps_evidence_separate() {
        let mut history = ProcessHistory::new();
        history.record_scan(100, &[process(42, 1, 50.0, 10)]);
        history.record_scan(200, &[process(42, 2, 75.0, 20)]);
        let old = history.get((42, 1)).unwrap();
        let new = history.get((42, 2)).unwrap();

        assert_eq!(
            FindingEvidence::from_record(old)
                .memory
                .unwrap()
                .latest_bytes,
            10
        );
        assert_eq!(
            FindingEvidence::from_record(new)
                .memory
                .unwrap()
                .latest_bytes,
            20
        );
        assert_ne!(old.identity(), new.identity());
    }
}
