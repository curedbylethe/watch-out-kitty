//! In-memory record of dismissed advisory findings.
//!
//! Entries are keyed by the complete finding identity. An entry expires when
//! its `(pid, start_time_secs)` is absent from a process sample, so a reused PID
//! is a different identity and never inherits a dismissal. State lives only as
//! long as the daemon process. No request path calls `dismiss` in this scope.
//! The detector never builds a finding with a zero start time, so no guard is
//! needed here.

use std::collections::{BTreeMap, BTreeSet};

use syswatch_core::FindingId;

/// Equals the detector's process ceiling: `ProcessHistory` keeps at most 1024
/// records, so the detector sees at most 1024 processes at one tick.
pub const MAX_DISMISSED_FINDINGS: usize = 1024;

#[derive(Debug, Default)]
pub struct DismissedFindings {
    entries: BTreeMap<FindingId, u64>,
    next_ordinal: u64,
}

impl DismissedFindings {
    /// Records a dismissal. Re-dismissing keeps the original insertion ordinal.
    /// At capacity, the earliest dismissal is evicted.
    pub fn dismiss(&mut self, id: FindingId) {
        if self.entries.contains_key(&id) {
            return;
        }
        if self.entries.len() == MAX_DISMISSED_FINDINGS {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, ordinal)| **ordinal)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(id, self.next_ordinal);
        self.next_ordinal += 1;
    }

    pub fn is_dismissed(&self, id: &FindingId) -> bool {
        self.entries.contains_key(id)
    }

    /// Drops every dismissal whose `(pid, start_time_secs)` is not in `observed`.
    pub fn retain_observed(&mut self, observed: impl IntoIterator<Item = (u32, u64)>) {
        let observed: BTreeSet<(u32, u64)> = observed.into_iter().collect();
        self.entries
            .retain(|id, _| observed.contains(&id.identity()));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{DismissedFindings, MAX_DISMISSED_FINDINGS};
    use syswatch_core::{FindingId, FindingKind};

    fn id(pid: u32, start_time_secs: u64, kind: FindingKind) -> FindingId {
        FindingId {
            pid,
            start_time_secs,
            kind,
        }
    }

    #[test]
    fn fresh_registry_is_empty() {
        let registry = DismissedFindings::default();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn only_the_exact_identity_is_dismissed() {
        let mut registry = DismissedFindings::default();
        registry.dismiss(id(42, 100, FindingKind::Memory));
        assert!(registry.is_dismissed(&id(42, 100, FindingKind::Memory)));
        assert!(!registry.is_dismissed(&id(42, 200, FindingKind::Memory)));
        assert!(!registry.is_dismissed(&id(42, 100, FindingKind::Cpu)));
    }

    #[test]
    fn dismissing_twice_is_a_no_op() {
        let mut registry = DismissedFindings::default();
        registry.dismiss(id(42, 100, FindingKind::Memory));
        registry.dismiss(id(42, 100, FindingKind::Memory));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn retain_observed_keeps_observed_and_drops_absent() {
        let mut registry = DismissedFindings::default();
        registry.dismiss(id(1, 10, FindingKind::Cpu));
        registry.dismiss(id(2, 20, FindingKind::Memory));
        registry.retain_observed([(1, 10)]);
        assert!(registry.is_dismissed(&id(1, 10, FindingKind::Cpu)));
        assert!(!registry.is_dismissed(&id(2, 20, FindingKind::Memory)));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn reused_pid_does_not_inherit_dismissal() {
        let mut registry = DismissedFindings::default();
        registry.dismiss(id(42, 100, FindingKind::Memory));
        registry.retain_observed([(42, 200)]);
        assert!(registry.is_empty());
        assert!(!registry.is_dismissed(&id(42, 200, FindingKind::Memory)));
    }

    #[test]
    fn capacity_evicts_the_earliest_dismissal() {
        let mut registry = DismissedFindings::default();
        for pid in 0..=MAX_DISMISSED_FINDINGS as u32 {
            registry.dismiss(id(pid, 1, FindingKind::Cpu));
        }
        assert_eq!(registry.len(), MAX_DISMISSED_FINDINGS);
        assert!(!registry.is_dismissed(&id(0, 1, FindingKind::Cpu)));
        assert!(registry.is_dismissed(&id(1, 1, FindingKind::Cpu)));
        assert!(registry.is_dismissed(&id(MAX_DISMISSED_FINDINGS as u32, 1, FindingKind::Cpu)));
    }
}
