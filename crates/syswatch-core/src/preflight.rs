//! Dry-run preflight for a future process action.
//!
//! A preflight decides only whether a finding identity may be offered to a
//! later explicit confirmation step. `AllowedForConfirmation` is not
//! permission: this module sends no signal and performs no process action.
//!
//! The module performs no lookup. Callers supply the scanned processes and
//! provenance as plain data, so tests can simulate exits, PID reuse, and
//! ownership changes without a live system.

use std::collections::BTreeMap;

use crate::eligibility::{self, EligibilityReason, EligibilityState};
use crate::findings::FindingId;
use crate::provenance::ProcessProvenance;
use crate::snapshot::ProcessSnapshot;

/// Action named by a preflight request.
///
/// This is vocabulary only. It has no executor, no signal number, and no
/// mapping to a system call. Naming an action here does not authorize it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestedAction {
    /// The planned `syswatch close` action.
    Close,
}

/// Input to [`assess`]. The finding identity is the only process selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreflightRequest {
    /// Full identity of the process the caller wants to act on.
    pub finding: FindingId,
    /// Action the caller intends.
    pub action: RequestedAction,
    /// Current time in milliseconds, on the same clock as `observed_at_ms`.
    pub now_ms: u64,
    /// Largest observation age in milliseconds that still counts as fresh.
    pub max_observation_age_ms: u64,
}

/// Process data supplied by the caller as the basis for an assessment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreflightObservation<'a> {
    /// Processes from one scan.
    pub processes: &'a [ProcessSnapshot],
    /// Provenance from the same scan, keyed by `(pid, start_time_secs)`.
    pub provenance: &'a BTreeMap<(u32, u64), ProcessProvenance>,
    /// When the scan was taken, in milliseconds.
    pub observed_at_ms: u64,
}

/// Outcome of a preflight assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightConclusion {
    /// The identity may be offered to a later confirmation step. Grants no permission.
    AllowedForConfirmation,
    /// The identity must not be offered. See the reasons.
    Denied,
    /// The evidence is missing or too old to decide. Observe again before deciding.
    NeedsFreshEvidence,
}

/// Explicit reason behind a non-allowed conclusion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightReason {
    /// The caller supplied no observation.
    NoObservation,
    /// The observation is older than `max_observation_age_ms`.
    StaleObservation,
    /// No process with the requested PID is in the observation.
    Exited,
    /// The requested PID is live, but with a different start time.
    IdentityReused,
    /// The requested start time is zero, so the identity cannot be verified.
    UnverifiableIdentity,
    /// Eligibility protected or could not verify the process. Carries the eligibility code.
    Ineligible(EligibilityReason),
}

/// Typed result of [`assess`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightAssessment {
    /// Requested finding identity, echoed from the request.
    pub finding: FindingId,
    /// Requested action, echoed from the request.
    pub action: RequestedAction,
    /// Outcome of the assessment.
    pub conclusion: PreflightConclusion,
    /// Empty only when `conclusion` is `AllowedForConfirmation`.
    pub reasons: Vec<PreflightReason>,
    /// Owner UID from the observation's provenance row for the requested identity, if any.
    pub owner_uid: Option<u32>,
    /// Observation age in milliseconds. `None` only when no observation was supplied.
    pub observation_age_ms: Option<u64>,
}

/// Assesses whether a finding identity may be offered to a later confirmation step.
///
/// Rules run in a fixed order: zero start time, missing observation, stale
/// observation, PID lookup, then eligibility.
pub fn assess(
    request: &PreflightRequest,
    observation: Option<&PreflightObservation<'_>>,
) -> PreflightAssessment {
    let (conclusion, reasons) = decide(request, observation);
    PreflightAssessment {
        finding: request.finding,
        action: request.action,
        conclusion,
        reasons,
        owner_uid: observation
            .and_then(|observation| observation.provenance.get(&request.finding.identity()))
            .and_then(|provenance| provenance.owner_uid),
        observation_age_ms: observation
            .map(|observation| request.now_ms.saturating_sub(observation.observed_at_ms)),
    }
}

fn decide(
    request: &PreflightRequest,
    observation: Option<&PreflightObservation<'_>>,
) -> (PreflightConclusion, Vec<PreflightReason>) {
    let finding = request.finding;
    if finding.start_time_secs == 0 {
        return (
            PreflightConclusion::Denied,
            vec![PreflightReason::UnverifiableIdentity],
        );
    }
    let Some(observation) = observation else {
        return (
            PreflightConclusion::NeedsFreshEvidence,
            vec![PreflightReason::NoObservation],
        );
    };
    if request.now_ms.saturating_sub(observation.observed_at_ms) > request.max_observation_age_ms {
        return (
            PreflightConclusion::NeedsFreshEvidence,
            vec![PreflightReason::StaleObservation],
        );
    }
    // PID equality alone never validates identity; start time must match too.
    let Some(row) = observation
        .processes
        .iter()
        .find(|row| row.pid == finding.pid)
    else {
        return (PreflightConclusion::Denied, vec![PreflightReason::Exited]);
    };
    if row.start_time_secs != finding.start_time_secs {
        return (
            PreflightConclusion::Denied,
            vec![PreflightReason::IdentityReused],
        );
    }

    let monitor = eligibility::monitor_identity(observation.provenance);
    let assessment = eligibility::assess(finding.identity(), observation.provenance, monitor);
    match assessment.state {
        EligibilityState::Eligible => (PreflightConclusion::AllowedForConfirmation, Vec::new()),
        // Unknown is missing evidence, not permission, so it denies too.
        EligibilityState::Protected | EligibilityState::Unknown => (
            PreflightConclusion::Denied,
            assessment
                .reasons
                .into_iter()
                .map(PreflightReason::Ineligible)
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::eligibility::{MAX_ANCESTRY_DEPTH, SYSTEM_UID_THRESHOLD};
    use crate::findings::FindingKind;
    use crate::provenance::ParentContext;

    const NOW_MS: u64 = 10_000;
    const OBSERVED_MS: u64 = 9_000;
    const MAX_AGE_MS: u64 = 5_000;

    fn finding(pid: u32, start_time_secs: u64) -> FindingId {
        FindingId {
            pid,
            start_time_secs,
            kind: FindingKind::Cpu,
        }
    }

    fn process(pid: u32, start_time_secs: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: None,
            name: "worker".to_string(),
            cpu_usage_percent: 0.0,
            memory_bytes: 0,
            start_time_secs,
        }
    }

    fn provenance(
        pid: u32,
        start_time_secs: u64,
        owner_uid: Option<u32>,
        parent: ParentContext,
        program: Option<&str>,
    ) -> ProcessProvenance {
        ProcessProvenance {
            pid,
            start_time_secs,
            executable: None,
            program: program.map(str::to_string),
            owner_uid,
            parent,
        }
    }

    fn scan(rows: Vec<ProcessProvenance>) -> BTreeMap<(u32, u64), ProcessProvenance> {
        rows.into_iter().map(|row| (row.identity(), row)).collect()
    }

    fn request(finding: FindingId) -> PreflightRequest {
        PreflightRequest {
            finding,
            action: RequestedAction::Close,
            now_ms: NOW_MS,
            max_observation_age_ms: MAX_AGE_MS,
        }
    }

    /// Runs `assess` and checks the allowed-iff-no-reasons invariant.
    fn assess_with(
        target: FindingId,
        observation: Option<&PreflightObservation<'_>>,
    ) -> PreflightAssessment {
        let result = assess(&request(target), observation);
        let allowed = result.conclusion == PreflightConclusion::AllowedForConfirmation;
        assert_eq!(allowed, result.reasons.is_empty(), "{result:?}");
        result
    }

    fn assess_at(
        target: FindingId,
        processes: &[ProcessSnapshot],
        rows: Vec<ProcessProvenance>,
        observed_at_ms: u64,
    ) -> PreflightAssessment {
        let provenance = scan(rows);
        let observation = PreflightObservation {
            processes,
            provenance: &provenance,
            observed_at_ms,
        };
        assess_with(target, Some(&observation))
    }

    /// Assesses finding (42, 1) against a fresh observation.
    fn assess_rows(
        processes: &[ProcessSnapshot],
        rows: Vec<ProcessProvenance>,
    ) -> PreflightAssessment {
        assess_at(finding(42, 1), processes, rows, OBSERVED_MS)
    }

    /// Assesses finding (42, 1) whose only process row is the given provenance row.
    fn assess_row(row: ProcessProvenance) -> PreflightAssessment {
        assess_rows(&[process(42, 1)], vec![row])
    }

    #[test]
    fn eligible_user_process_is_allowed() {
        let result = assess_row(provenance(42, 1, Some(501), ParentContext::NoParent, None));
        assert_eq!(
            result.conclusion,
            PreflightConclusion::AllowedForConfirmation
        );
        assert!(result.reasons.is_empty());
        assert_eq!(result.owner_uid, Some(501));
        assert_eq!(result.observation_age_ms, Some(NOW_MS - OBSERVED_MS));
        assert_eq!(result.finding, finding(42, 1));
        assert_eq!(result.action, RequestedAction::Close);
    }

    #[test]
    fn missing_observation_needs_fresh_evidence() {
        let result = assess_with(finding(42, 1), None);
        assert_eq!(result.conclusion, PreflightConclusion::NeedsFreshEvidence);
        assert_eq!(result.reasons, vec![PreflightReason::NoObservation]);
        assert_eq!(result.owner_uid, None);
        assert_eq!(result.observation_age_ms, None);
    }

    #[test]
    fn observation_age_equal_to_bound_is_not_stale() {
        let result = assess_at(
            finding(42, 1),
            &[process(42, 1)],
            vec![provenance(42, 1, Some(501), ParentContext::NoParent, None)],
            NOW_MS - MAX_AGE_MS,
        );
        assert_eq!(
            result.conclusion,
            PreflightConclusion::AllowedForConfirmation
        );
        assert_eq!(result.observation_age_ms, Some(MAX_AGE_MS));
    }

    #[test]
    fn observation_one_millisecond_over_bound_is_stale() {
        let result = assess_at(
            finding(42, 1),
            &[process(42, 1)],
            vec![provenance(42, 1, Some(501), ParentContext::NoParent, None)],
            NOW_MS - MAX_AGE_MS - 1,
        );
        assert_eq!(result.conclusion, PreflightConclusion::NeedsFreshEvidence);
        assert_eq!(result.reasons, vec![PreflightReason::StaleObservation]);
        assert_eq!(result.observation_age_ms, Some(MAX_AGE_MS + 1));
    }

    #[test]
    fn zero_start_time_is_unverifiable() {
        let result = assess_at(
            finding(42, 0),
            &[process(42, 0)],
            vec![provenance(42, 0, Some(501), ParentContext::NoParent, None)],
            OBSERVED_MS,
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(result.reasons, vec![PreflightReason::UnverifiableIdentity]);

        // Zero start outranks the missing-observation rule.
        let without_observation = assess_with(finding(42, 0), None);
        assert_eq!(without_observation.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            without_observation.reasons,
            vec![PreflightReason::UnverifiableIdentity]
        );
    }

    #[test]
    fn missing_process_row_is_exited() {
        let result = assess_rows(
            &[],
            vec![provenance(42, 1, Some(501), ParentContext::NoParent, None)],
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(result.reasons, vec![PreflightReason::Exited]);
    }

    #[test]
    fn same_pid_with_different_start_time_is_identity_reused() {
        let result = assess_rows(
            &[process(42, 2)],
            vec![
                provenance(42, 1, Some(501), ParentContext::NoParent, None),
                provenance(42, 2, Some(501), ParentContext::NoParent, None),
            ],
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(result.reasons, vec![PreflightReason::IdentityReused]);
    }

    #[test]
    fn monitor_process_is_denied() {
        let pid = std::process::id();
        let result = assess_at(
            finding(pid, 1),
            &[process(pid, 1)],
            vec![provenance(pid, 1, Some(501), ParentContext::NoParent, None)],
            OBSERVED_MS,
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::MonitorProcess
            )]
        );
    }

    #[test]
    fn monitor_ancestor_is_denied() {
        let monitor_pid = std::process::id();
        let result = assess_rows(
            &[process(42, 1)],
            vec![
                provenance(
                    42,
                    1,
                    Some(501),
                    ParentContext::Resolved {
                        pid: monitor_pid,
                        start_time_secs: 7,
                    },
                    None,
                ),
                provenance(monitor_pid, 7, Some(501), ParentContext::NoParent, None),
            ],
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::MonitorAncestor
            )]
        );
    }

    #[test]
    fn root_owner_is_denied() {
        let result = assess_row(provenance(42, 1, Some(0), ParentContext::NoParent, None));
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        // UID 0 is also below SYSTEM_UID_THRESHOLD, so eligibility reports both.
        assert_eq!(
            result.reasons,
            vec![
                PreflightReason::Ineligible(EligibilityReason::PrivilegedOwner),
                PreflightReason::Ineligible(EligibilityReason::SystemOwner),
            ]
        );
    }

    #[test]
    fn system_owner_is_denied() {
        let result = assess_row(provenance(
            42,
            1,
            Some(SYSTEM_UID_THRESHOLD - 1),
            ParentContext::NoParent,
            None,
        ));
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(EligibilityReason::SystemOwner)]
        );
    }

    #[test]
    fn unknown_owner_is_denied() {
        let result = assess_row(provenance(42, 1, None, ParentContext::NoParent, None));
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(result.owner_uid, None);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(EligibilityReason::OwnerUnknown)]
        );
    }

    #[test]
    fn own_terminal_program_is_denied() {
        let result = assess_row(provenance(
            42,
            1,
            Some(501),
            ParentContext::NoParent,
            Some("zsh"),
        ));
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::OwnTerminalProgram
            )]
        );
    }

    #[test]
    fn ancestor_terminal_program_is_denied() {
        let result = assess_rows(
            &[process(42, 1)],
            vec![
                provenance(
                    42,
                    1,
                    Some(501),
                    ParentContext::Resolved {
                        pid: 7,
                        start_time_secs: 9,
                    },
                    Some("worker"),
                ),
                provenance(7, 9, Some(501), ParentContext::NoParent, Some("zsh")),
            ],
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::ActiveTerminal
            )]
        );
    }

    #[test]
    fn pid_only_parent_is_denied_as_incomplete_ancestry() {
        let result = assess_row(provenance(
            42,
            1,
            Some(501),
            ParentContext::PidOnly(7),
            None,
        ));
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::AncestryIncomplete
            )]
        );
    }

    #[test]
    fn missing_provenance_row_is_denied() {
        let result = assess_rows(&[process(42, 1)], Vec::new());
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::ProvenanceUnavailable
            )]
        );
    }

    #[test]
    fn ancestry_cycle_is_denied() {
        let result = assess_rows(
            &[process(42, 1)],
            vec![
                provenance(
                    42,
                    1,
                    Some(501),
                    ParentContext::Resolved {
                        pid: 7,
                        start_time_secs: 9,
                    },
                    None,
                ),
                provenance(
                    7,
                    9,
                    Some(501),
                    ParentContext::Resolved {
                        pid: 42,
                        start_time_secs: 1,
                    },
                    None,
                ),
            ],
        );
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::AncestryCycle
            )]
        );
    }

    #[test]
    fn ancestry_depth_limit_is_denied() {
        // One more validated link than MAX_ANCESTRY_DEPTH allows.
        let depth = MAX_ANCESTRY_DEPTH + 2;
        let rows: Vec<ProcessProvenance> = (0..depth)
            .map(|index| {
                let parent = if index + 1 < depth {
                    ParentContext::Resolved {
                        pid: 1000 + index + 1,
                        start_time_secs: 1,
                    }
                } else {
                    ParentContext::NoParent
                };
                provenance(1000 + index, 1, Some(501), parent, None)
            })
            .collect();
        let result = assess_at(finding(1000, 1), &[process(1000, 1)], rows, OBSERVED_MS);
        assert_eq!(result.conclusion, PreflightConclusion::Denied);
        assert_eq!(
            result.reasons,
            vec![PreflightReason::Ineligible(
                EligibilityReason::AncestryTooDeep
            )]
        );
    }

    #[test]
    fn identical_input_gives_identical_assessment() {
        let processes = [process(42, 1)];
        let provenance_map = scan(vec![provenance(
            42,
            1,
            Some(501),
            ParentContext::NoParent,
            Some("zsh"),
        )]);
        let observation = PreflightObservation {
            processes: &processes,
            provenance: &provenance_map,
            observed_at_ms: OBSERVED_MS,
        };
        let first = assess(&request(finding(42, 1)), Some(&observation));
        let second = assess(&request(finding(42, 1)), Some(&observation));
        assert_eq!(first, second);
    }
}
