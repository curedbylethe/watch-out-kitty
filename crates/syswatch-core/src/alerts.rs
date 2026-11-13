//! Alert eligibility decides which advisory findings deserve an alert.
//!
//! State is keyed by complete [`FindingId`] and holds only the identities of
//! the latest call's findings. The policy reads no clock, takes no lock and
//! performs no I/O; the caller supplies `now_ms`.

use std::collections::BTreeMap;

use crate::findings::{AdvisoryFinding, Confidence, FindingId};

/// Minimum gap between alerts for one unchanged finding, in milliseconds.
///
/// Equals the default detector evidence window. It is a constant, not configuration.
pub const DEFAULT_ALERT_COOLDOWN_MS: u64 = 600_000;

/// Reason an alert event was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertReason {
    /// The finding identity was not alerted before, or reappeared after absence.
    New,
    /// Confidence rose, or memory maximum at least doubled, since the last alert.
    EvidenceChanged,
    /// The cooldown elapsed since the last alert.
    CooldownExpired,
}

/// Decision that one finding should alert now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertEvent {
    /// Identity of the finding that should alert.
    pub id: FindingId,
    /// Observation time of the finding, in milliseconds since the Unix epoch.
    pub observed_at_ms: u64,
    /// Why the alert was emitted.
    pub reason: AlertReason,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    last_seen_ms: u64,
    last_alerted_ms: u64,
    alerted_confidence: Confidence,
    alerted_memory_max_bytes: Option<u64>,
}

impl Entry {
    fn alerted(now_ms: u64, finding: &AdvisoryFinding) -> Self {
        Self {
            last_seen_ms: now_ms,
            last_alerted_ms: now_ms,
            alerted_confidence: finding.confidence,
            alerted_memory_max_bytes: memory_max(finding),
        }
    }
}

fn memory_max(finding: &AdvisoryFinding) -> Option<u64> {
    finding
        .evidence
        .memory
        .as_ref()
        .map(|memory| memory.max_bytes)
}

/// Alert eligibility state for advisory findings.
#[derive(Debug, Default)]
pub struct AlertPolicy {
    entries: BTreeMap<FindingId, Entry>,
}

impl AlertPolicy {
    /// Evaluates one scan of findings at `now_ms` and returns alert events in
    /// [`FindingId`] order.
    ///
    /// An identity absent from `findings` is dropped, so its reappearance is
    /// `New`. A `now_ms` before the last evaluation restarts the cooldown and
    /// emits nothing.
    pub fn evaluate(&mut self, findings: &[AdvisoryFinding], now_ms: u64) -> Vec<AlertEvent> {
        let mut previous = std::mem::take(&mut self.entries);
        let current = findings
            .iter()
            .map(|finding| (finding.id, finding))
            .collect::<BTreeMap<_, _>>();
        let mut events = Vec::new();

        for (id, finding) in current {
            let entry = match previous.remove(&id) {
                None => {
                    events.push(alert(id, finding, AlertReason::New));
                    Entry::alerted(now_ms, finding)
                }
                Some(entry) if now_ms < entry.last_seen_ms => Entry::alerted(now_ms, finding),
                Some(entry) => match reason(&entry, finding, now_ms) {
                    Some(reason) => {
                        events.push(alert(id, finding, reason));
                        Entry::alerted(now_ms, finding)
                    }
                    None => Entry {
                        last_seen_ms: now_ms,
                        ..entry
                    },
                },
            };
            self.entries.insert(id, entry);
        }

        events
    }

    /// Number of identities currently held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when no identity is held.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn alert(id: FindingId, finding: &AdvisoryFinding, reason: AlertReason) -> AlertEvent {
    AlertEvent {
        id,
        observed_at_ms: finding.observed_at_ms,
        reason,
    }
}

fn reason(entry: &Entry, finding: &AdvisoryFinding, now_ms: u64) -> Option<AlertReason> {
    if is_material_change(entry, finding) {
        Some(AlertReason::EvidenceChanged)
    } else if now_ms.saturating_sub(entry.last_alerted_ms) >= DEFAULT_ALERT_COOLDOWN_MS {
        Some(AlertReason::CooldownExpired)
    } else {
        None
    }
}

/// Material change is a confidence increase, or a memory maximum at least
/// double the alerted one. A zero alerted maximum is material only when the
/// current maximum is positive. Decreases and other drift are not material.
fn is_material_change(entry: &Entry, finding: &AdvisoryFinding) -> bool {
    if finding.confidence > entry.alerted_confidence {
        return true;
    }
    match (entry.alerted_memory_max_bytes, memory_max(finding)) {
        (Some(alerted), Some(current)) => current > 0 && current >= alerted.saturating_mul(2),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{AlertEvent, AlertPolicy, AlertReason, DEFAULT_ALERT_COOLDOWN_MS};
    use crate::findings::{
        AdvisoryFinding, Confidence, FindingEvidence, FindingId, FindingKind, MemoryActivity,
    };

    const T: u64 = 1_700_000_000_000;
    const COOLDOWN: u64 = DEFAULT_ALERT_COOLDOWN_MS;

    fn finding(pid: u32, kind: FindingKind, confidence: Confidence) -> AdvisoryFinding {
        AdvisoryFinding {
            id: FindingId {
                pid,
                start_time_secs: 1,
                kind,
            },
            confidence,
            observed_at_ms: T,
            explanation: "test finding".into(),
            evidence: FindingEvidence {
                sample_count: 2,
                span_ms: 120_000,
                cpu: None,
                memory: None,
            },
        }
    }

    fn with_memory(mut finding: AdvisoryFinding, max_bytes: u64) -> AdvisoryFinding {
        finding.evidence.memory = Some(MemoryActivity {
            latest_bytes: max_bytes,
            max_bytes,
        });
        finding
    }

    fn reasons(events: &[AlertEvent]) -> Vec<AlertReason> {
        events.iter().map(|event| event.reason).collect()
    }

    #[test]
    fn repeated_unchanged_scans_alert_once() {
        let mut policy = AlertPolicy::default();
        let scan = [finding(42, FindingKind::Cpu, Confidence::High)];
        assert_eq!(reasons(&policy.evaluate(&scan, T)), [AlertReason::New]);
        assert!(policy.evaluate(&scan, T + 5_000).is_empty());
        assert!(policy.evaluate(&scan, T + 10_000).is_empty());
    }

    #[test]
    fn cooldown_expires_exactly_at_the_boundary() {
        let mut policy = AlertPolicy::default();
        let scan = [finding(42, FindingKind::Cpu, Confidence::High)];
        policy.evaluate(&scan, T);
        assert!(policy.evaluate(&scan, T + COOLDOWN - 1).is_empty());
        assert_eq!(
            reasons(&policy.evaluate(&scan, T + COOLDOWN)),
            [AlertReason::CooldownExpired]
        );
    }

    #[test]
    fn confidence_increase_inside_cooldown_alerts() {
        let mut policy = AlertPolicy::default();
        policy.evaluate(&[finding(42, FindingKind::Cpu, Confidence::Medium)], T);
        let raised = [finding(42, FindingKind::Cpu, Confidence::High)];
        assert_eq!(
            reasons(&policy.evaluate(&raised, T + 5_000)),
            [AlertReason::EvidenceChanged]
        );
    }

    #[test]
    fn memory_doubling_alerts_and_decrease_does_not() {
        let mut policy = AlertPolicy::default();
        let base = with_memory(finding(42, FindingKind::Memory, Confidence::High), 100);
        policy.evaluate(std::slice::from_ref(&base), T);
        let grown = with_memory(finding(42, FindingKind::Memory, Confidence::High), 199);
        assert!(policy.evaluate(&[grown], T + 1).is_empty());
        let doubled = with_memory(finding(42, FindingKind::Memory, Confidence::High), 200);
        assert_eq!(
            reasons(&policy.evaluate(&[doubled], T + 2)),
            [AlertReason::EvidenceChanged]
        );
        let shrunk = with_memory(finding(42, FindingKind::Memory, Confidence::High), 50);
        assert!(policy.evaluate(&[shrunk], T + 3).is_empty());
    }

    #[test]
    fn zero_memory_baseline_is_material_only_when_it_becomes_positive() {
        let mut policy = AlertPolicy::default();
        let zero = with_memory(finding(42, FindingKind::Memory, Confidence::High), 0);
        policy.evaluate(std::slice::from_ref(&zero), T);
        assert!(
            policy
                .evaluate(std::slice::from_ref(&zero), T + 1)
                .is_empty()
        );
        let positive = with_memory(finding(42, FindingKind::Memory, Confidence::High), 1);
        assert_eq!(
            reasons(&policy.evaluate(&[positive], T + 2)),
            [AlertReason::EvidenceChanged]
        );
    }

    #[test]
    fn sample_and_span_drift_do_not_alert() {
        let mut policy = AlertPolicy::default();
        let base = finding(42, FindingKind::Cpu, Confidence::High);
        policy.evaluate(std::slice::from_ref(&base), T);
        let mut drifted = base;
        drifted.evidence.sample_count = 9;
        drifted.evidence.span_ms = 999_000;
        assert!(policy.evaluate(&[drifted], T + 5_000).is_empty());
    }

    #[test]
    fn disappearance_then_reappearance_is_new() {
        let mut policy = AlertPolicy::default();
        let scan = [finding(42, FindingKind::Cpu, Confidence::High)];
        policy.evaluate(&scan, T);
        assert!(policy.evaluate(&[], T + 5_000).is_empty());
        assert!(policy.is_empty());
        assert_eq!(
            reasons(&policy.evaluate(&scan, T + 10_000)),
            [AlertReason::New]
        );
    }

    #[test]
    fn reused_pid_with_new_start_time_is_a_different_identity() {
        let mut policy = AlertPolicy::default();
        policy.evaluate(&[finding(42, FindingKind::Cpu, Confidence::High)], T);
        let mut reused = finding(42, FindingKind::Cpu, Confidence::High);
        reused.id.start_time_secs = 2;
        let events = policy.evaluate(std::slice::from_ref(&reused), T + 5_000);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, reused.id);
        assert_eq!(events[0].reason, AlertReason::New);
        assert_eq!(policy.len(), 1);
    }

    #[test]
    fn clock_regression_emits_nothing_and_measures_cooldown_from_regressed_time() {
        let mut policy = AlertPolicy::default();
        let scan = [finding(42, FindingKind::Cpu, Confidence::High)];
        policy.evaluate(&scan, T + COOLDOWN);
        assert!(policy.evaluate(&scan, T).is_empty());
        assert!(policy.evaluate(&scan, T + COOLDOWN - 1).is_empty());
        assert_eq!(
            reasons(&policy.evaluate(&scan, T + COOLDOWN)),
            [AlertReason::CooldownExpired]
        );
    }

    #[test]
    fn events_follow_finding_id_order_for_unsorted_input() {
        let mut policy = AlertPolicy::default();
        let events = policy.evaluate(
            &[
                finding(43, FindingKind::Cpu, Confidence::High),
                finding(42, FindingKind::Cpu, Confidence::High),
            ],
            T,
        );
        let pids = events.iter().map(|event| event.id.pid).collect::<Vec<_>>();
        assert_eq!(pids, [42, 43]);
    }

    #[test]
    fn state_holds_only_the_latest_identities() {
        let mut policy = AlertPolicy::default();
        let a = finding(42, FindingKind::Cpu, Confidence::High);
        let b = finding(43, FindingKind::Cpu, Confidence::High);
        policy.evaluate(&[a.clone(), b], T);
        assert_eq!(policy.len(), 2);
        policy.evaluate(&[a], T + 1);
        assert_eq!(policy.len(), 1);
    }

    #[test]
    fn kind_agnostic_cpu_finding_alerts_once_then_at_cooldown() {
        let mut policy = AlertPolicy::default();
        let scan = [finding(42, FindingKind::Cpu, Confidence::Low)];
        assert_eq!(reasons(&policy.evaluate(&scan, T)), [AlertReason::New]);
        assert!(policy.evaluate(&scan, T + 1).is_empty());
        assert_eq!(
            reasons(&policy.evaluate(&scan, T + COOLDOWN)),
            [AlertReason::CooldownExpired]
        );
    }
}
