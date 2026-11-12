//! Advisory findings describe evidence and invite inspection.
//!
//! A confidence level describes evidence coverage. No level asserts that a
//! process is safe or safe to terminate.

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
    /// Evidence is missing, covers one sample, or covers too short a window to
    /// separate a transient spike from sustained activity.
    Low,
    /// Several valid samples support the finding, but the window is short
    /// relative to retained history or the samples are irregular.
    Medium,
    /// Valid samples span the retained observation window with no unexplained
    /// gaps and consistent measurements.
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
}

/// Sorts findings by stable ID, then observation time.
pub fn sort(findings: &mut [AdvisoryFinding]) {
    findings.sort_by_key(|finding| (finding.id, finding.observed_at_ms));
}

#[cfg(test)]
mod tests {
    use super::{AdvisoryFinding, Confidence, FindingId, FindingKind, sort};

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
            r#"{"id":{"pid":42,"start_time_secs":1700000000,"kind":"cpu"},"confidence":"medium","observed_at_ms":1700000000123,"explanation":"pid 42 (worker) used CPU continuously over the retained window"}"#
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
    fn explanation_is_preserved() {
        let explanation = "pid 42 (worker) used CPU continuously over the retained window";
        let finding = finding(42, 1_700_000_000, FindingKind::Cpu, 1);
        let serialized = serde_json::to_string(&finding).unwrap();
        let deserialized: AdvisoryFinding = serde_json::from_str(&serialized).unwrap();

        assert!(!deserialized.explanation.is_empty());
        assert!(deserialized.explanation.contains("pid 42"));
        assert_eq!(deserialized.explanation, explanation);
    }
}
