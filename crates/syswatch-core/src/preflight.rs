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
