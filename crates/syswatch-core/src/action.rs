//! Guarded SIGTERM request service.
//!
//! [`execute`] takes one finding identity, an admitted preflight result, and a
//! delivery-time observation. It runs [`preflight::assess`] again on the
//! observation and calls the sender at most once. The only action it can
//! request is SIGTERM.
//!
//! Limits:
//! - The admitted preflight is a ticket, not permission. The delivery-time
//!   assessment decides. The gap between the two is not time-bounded here,
//!   beyond the recorded observation age, so a caller must bound it.
//! - The check-to-signal window is not closed. Between the delivery assessment
//!   and the kernel applying the signal, the process may exit and its PID may
//!   be reused. The kernel applies the signal to whichever process holds the
//!   PID at delivery time.
//! - [`ActionOutcome::SigtermRequested`] means the signal was requested. It
//!   does not show that the process exited.
//! - No state survives a call. There is no retry, escalation, or confirmation.
//! - [`UnixSignalSender`] is the only sender. It is Unix-only and sends one
//!   SIGTERM per call, with no retry, escalation, or privilege change.

use crate::findings::FindingId;
use crate::preflight::{
    self, PreflightAssessment, PreflightConclusion, PreflightObservation, PreflightReason,
    PreflightRequest, RequestedAction,
};

/// Caller's request to stop one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionRequest {
    /// Full identity of the selected finding. The only process selector.
    pub finding: FindingId,
    /// Current time in milliseconds, on the same clock as the observation.
    pub now_ms: u64,
    /// Largest age, in milliseconds, of the evidence behind the admitted preflight
    /// when that preflight ran. It does not bound the time since the preflight ran.
    pub max_preflight_age_ms: u64,
    /// Largest delivery-time observation age in milliseconds that still counts as fresh.
    pub max_observation_age_ms: u64,
    /// UID of the process making the request. Only the caller's own processes may be
    /// signalled. The caller must supply its real UID; this crate does not read it.
    pub caller_uid: u32,
}

/// Result of one delivery attempt by a [`SignalSender`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalDelivery {
    /// The operating system accepted the signal request.
    Requested,
    /// The target was already gone when the request was made.
    AlreadyExited,
    /// The operating system refused the request for permission reasons.
    PermissionDenied,
    /// The target was refused before any request was made.
    InvalidTarget,
    /// Any other delivery failure.
    Failed,
}

/// Injectable delivery seam for the SIGTERM request.
///
/// [`execute`] calls `request_stop` at most once per call, and only after every
/// rule passed. The target is the validated `(pid, start_time_secs)` identity.
pub trait SignalSender {
    /// Requests SIGTERM for the target. Returns what the operating system reported.
    fn request_stop(&mut self, target: (u32, u64)) -> SignalDelivery;
}

/// Explicit reason why [`execute`] refused to send.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ActionDenial {
    /// A reason from the preflight assessment, copied in order.
    Preflight(PreflightReason),
    /// The admitted preflight was not allowed and gave no reason.
    PreflightNotAllowed,
    /// The admitted preflight has no age, or its age is over the limit.
    PreflightTooOld,
    /// The admitted preflight is for a different finding identity.
    PreflightMismatch,
    /// The target's owner UID is not the caller's UID.
    OwnerMismatch,
}

/// Failure reported by the sender after a permitted request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ActionFailure {
    /// The operating system refused the request for permission reasons.
    PermissionDenied,
    /// The target was refused before any request was made.
    InvalidTarget,
    /// Any other delivery failure.
    DeliveryFailed,
}

/// Typed result of [`execute`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    /// The sender accepted a SIGTERM request. No exit is promised or observed.
    SigtermRequested,
    /// The process was not in the delivery observation, or the sender reports it gone.
    AlreadyExited,
    /// Nothing was sent. Lists every reason, in order.
    Denied {
        /// Reasons the request was refused.
        reasons: Vec<ActionDenial>,
    },
    /// The sender was called and the request failed.
    Failed {
        /// Why the request failed.
        failure: ActionFailure,
    },
}

/// Runs the guarded SIGTERM rules in a fixed order and returns one outcome.
///
/// `preflight` is the earlier admission. `observation` is the delivery-time
/// scan and decides whether the sender is called.
pub fn execute(
    request: &ActionRequest,
    preflight: &PreflightAssessment,
    observation: &PreflightObservation<'_>,
    sender: &mut impl SignalSender,
) -> ActionOutcome {
    // No wildcard: a new action must be mapped here, never sent as SIGTERM.
    match preflight.action {
        RequestedAction::Close => {}
    }
    if preflight.finding != request.finding {
        return denied(vec![ActionDenial::PreflightMismatch]);
    }
    if preflight.conclusion != PreflightConclusion::AllowedForConfirmation {
        let mut reasons: Vec<ActionDenial> = preflight
            .reasons
            .iter()
            .cloned()
            .map(ActionDenial::Preflight)
            .collect();
        if reasons.is_empty() {
            reasons.push(ActionDenial::PreflightNotAllowed);
        }
        return denied(reasons);
    }
    if preflight
        .observation_age_ms
        .is_none_or(|age| age > request.max_preflight_age_ms)
    {
        return denied(vec![ActionDenial::PreflightTooOld]);
    }

    let fresh = preflight::assess(
        &PreflightRequest {
            finding: request.finding,
            action: RequestedAction::Close,
            now_ms: request.now_ms,
            max_observation_age_ms: request.max_observation_age_ms,
        },
        Some(observation),
    );
    if fresh.reasons == [PreflightReason::Exited] {
        return ActionOutcome::AlreadyExited;
    }
    if fresh.conclusion != PreflightConclusion::AllowedForConfirmation {
        return denied(
            fresh
                .reasons
                .into_iter()
                .map(ActionDenial::Preflight)
                .collect(),
        );
    }

    if fresh.owner_uid != Some(request.caller_uid) {
        return denied(vec![ActionDenial::OwnerMismatch]);
    }

    match sender.request_stop(request.finding.identity()) {
        SignalDelivery::Requested => ActionOutcome::SigtermRequested,
        SignalDelivery::AlreadyExited => ActionOutcome::AlreadyExited,
        SignalDelivery::PermissionDenied => ActionOutcome::Failed {
            failure: ActionFailure::PermissionDenied,
        },
        SignalDelivery::InvalidTarget => ActionOutcome::Failed {
            failure: ActionFailure::InvalidTarget,
        },
        SignalDelivery::Failed => ActionOutcome::Failed {
            failure: ActionFailure::DeliveryFailed,
        },
    }
}

fn denied(reasons: Vec<ActionDenial>) -> ActionOutcome {
    ActionOutcome::Denied { reasons }
}

/// Sends SIGTERM to one process with `kill(2)`.
///
/// Platform limit: the kernel applies the signal to whichever process holds
/// the PID when `kill` runs. macOS has no `pidfd`, so a process exit and PID
/// reuse between the caller's check and the call cannot be detected. This
/// sender does not claim identity-safe delivery.
#[cfg(unix)]
pub struct UnixSignalSender;

#[cfg(unix)]
impl SignalSender for UnixSignalSender {
    fn request_stop(&mut self, target: (u32, u64)) -> SignalDelivery {
        // The start time is not passed to the kernel: `kill(2)` takes a PID only.
        let (pid, _start_time_secs) = target;
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return SignalDelivery::InvalidTarget;
        };
        // Below 2, `kill` signals a process group or every process: refuse first.
        if pid < 2 {
            return SignalDelivery::InvalidTarget;
        }
        // SAFETY: `kill` is thread-safe and takes no pointers. `pid` is a
        // positive PID (at least 2), so no process group or broadcast is targeted.
        let result = unsafe { libc::kill(pid, libc::SIGTERM) };
        classify_delivery(result, std::io::Error::last_os_error().raw_os_error())
    }
}

/// Maps the `kill` return value and errno to one delivery outcome.
#[cfg(unix)]
fn classify_delivery(result: libc::c_int, errno: Option<i32>) -> SignalDelivery {
    if result == 0 {
        return SignalDelivery::Requested;
    }
    match errno {
        Some(libc::ESRCH) => SignalDelivery::AlreadyExited,
        Some(libc::EPERM) => SignalDelivery::PermissionDenied,
        _ => SignalDelivery::Failed,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::eligibility::EligibilityReason;
    use crate::provenance::{ParentContext, ProcessProvenance};
    use crate::snapshot::ProcessSnapshot;

    const NOW_MS: u64 = 10_000;
    const OBSERVED_MS: u64 = 9_000;
    const MAX_AGE_MS: u64 = 5_000;
    /// A monitor identity no test process uses.
    const MONITOR: (u32, u64) = (9_999, 9_999);

    /// Owned copy of one synthetic scan, borrowed as a `PreflightObservation`.
    struct Scan {
        processes: Vec<ProcessSnapshot>,
        provenance: BTreeMap<(u32, u64), ProcessProvenance>,
        observed_at_ms: u64,
    }

    impl Scan {
        /// One process at `(pid, start_time_secs)` owned by `owner_uid`.
        fn one(pid: u32, start_time_secs: u64, owner_uid: u32, observed_at_ms: u64) -> Self {
            Scan {
                processes: vec![ProcessSnapshot {
                    pid,
                    parent_pid: None,
                    name: "worker".to_string(),
                    cpu_usage_percent: 0.0,
                    memory_bytes: 0,
                    start_time_secs,
                }],
                provenance: BTreeMap::from([(
                    (pid, start_time_secs),
                    ProcessProvenance {
                        pid,
                        start_time_secs,
                        executable: None,
                        program: None,
                        owner_uid: Some(owner_uid),
                        parent: ParentContext::NoParent,
                    },
                )]),
                observed_at_ms,
            }
        }

        /// Scan with no process and no provenance.
        fn empty(observed_at_ms: u64) -> Self {
            Scan {
                processes: Vec::new(),
                provenance: BTreeMap::new(),
                observed_at_ms,
            }
        }

        fn observation(&self) -> PreflightObservation<'_> {
            PreflightObservation {
                processes: &self.processes,
                provenance: &self.provenance,
                observed_at_ms: self.observed_at_ms,
                monitor: MONITOR,
            }
        }
    }

    struct FakeSender {
        calls: Vec<(u32, u64)>,
        next: SignalDelivery,
    }

    impl SignalSender for FakeSender {
        fn request_stop(&mut self, target: (u32, u64)) -> SignalDelivery {
            self.calls.push(target);
            self.next
        }
    }

    fn sender(next: SignalDelivery) -> FakeSender {
        FakeSender {
            calls: Vec::new(),
            next,
        }
    }

    fn finding(pid: u32, start_time_secs: u64) -> FindingId {
        FindingId {
            pid,
            start_time_secs,
            kind: crate::findings::FindingKind::Cpu,
        }
    }

    /// Runs `preflight::assess` for `target` and checks the allowed-iff-no-reasons invariant.
    fn assess_scan(target: FindingId, scan: Option<&Scan>) -> PreflightAssessment {
        let observation = scan.map(Scan::observation);
        let result = preflight::assess(
            &PreflightRequest {
                finding: target,
                action: RequestedAction::Close,
                now_ms: NOW_MS,
                max_observation_age_ms: MAX_AGE_MS,
            },
            observation.as_ref(),
        );
        let allowed = result.conclusion == PreflightConclusion::AllowedForConfirmation;
        assert_eq!(allowed, result.reasons.is_empty(), "{result:?}");
        result
    }

    /// Calls `execute` for `target` against the delivery observation of `delivery`.
    fn run(
        target: FindingId,
        admitted: &PreflightAssessment,
        delivery: &Scan,
        sender: &mut FakeSender,
    ) -> ActionOutcome {
        let request = ActionRequest {
            finding: target,
            now_ms: NOW_MS,
            max_preflight_age_ms: MAX_AGE_MS,
            max_observation_age_ms: MAX_AGE_MS,
            caller_uid: 501,
        };
        execute(&request, admitted, &delivery.observation(), sender)
    }

    fn denied(reasons: Vec<ActionDenial>) -> ActionOutcome {
        ActionOutcome::Denied { reasons }
    }

    #[test]
    fn allowed_identity_requests_sigterm_once() {
        let scan = Scan::one(42, 1, 501, OBSERVED_MS);
        let admitted = assess_scan(finding(42, 1), Some(&scan));
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &scan, &mut sender),
            ActionOutcome::SigtermRequested
        );
        assert_eq!(sender.calls, vec![(42, 1)]);
    }

    #[test]
    fn privileged_admitted_identity_is_denied_without_send() {
        let scan = Scan::one(42, 1, 0, OBSERVED_MS);
        let admitted = assess_scan(finding(42, 1), Some(&scan));
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &scan, &mut sender),
            denied(vec![
                ActionDenial::Preflight(PreflightReason::Ineligible(
                    EligibilityReason::PrivilegedOwner,
                )),
                ActionDenial::Preflight(PreflightReason::Ineligible(
                    EligibilityReason::SystemOwner,
                )),
            ])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn missing_observation_admission_is_denied_without_send() {
        let admitted = assess_scan(finding(42, 1), None);
        let delivery = Scan::one(42, 1, 501, OBSERVED_MS);
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &delivery, &mut sender),
            denied(vec![ActionDenial::Preflight(
                PreflightReason::NoObservation
            )])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn admitted_other_identity_is_mismatch_without_send() {
        let delivery = Scan::one(42, 1, 501, OBSERVED_MS);
        let others = [
            finding(43, 1),
            finding(42, 2),
            FindingId {
                kind: crate::findings::FindingKind::Memory,
                ..finding(42, 1)
            },
        ];
        for other in others {
            let other_scan = Scan::one(other.pid, other.start_time_secs, 501, OBSERVED_MS);
            let admitted = assess_scan(other, Some(&other_scan));
            let mut sender = sender(SignalDelivery::Requested);
            assert_eq!(
                run(finding(42, 1), &admitted, &delivery, &mut sender),
                denied(vec![ActionDenial::PreflightMismatch])
            );
            assert!(sender.calls.is_empty());
        }
    }

    #[test]
    fn admitted_age_over_limit_or_missing_is_too_old_without_send() {
        let scan = Scan::one(42, 1, 501, OBSERVED_MS);
        for age in [Some(MAX_AGE_MS + 1), None] {
            let mut admitted = assess_scan(finding(42, 1), Some(&scan));
            admitted.observation_age_ms = age;
            let mut sender = sender(SignalDelivery::Requested);
            assert_eq!(
                run(finding(42, 1), &admitted, &scan, &mut sender),
                denied(vec![ActionDenial::PreflightTooOld])
            );
            assert!(sender.calls.is_empty());
        }
    }

    #[test]
    fn admitted_age_at_limit_still_requests_sigterm() {
        let scan = Scan::one(42, 1, 501, OBSERVED_MS);
        let mut admitted = assess_scan(finding(42, 1), Some(&scan));
        admitted.observation_age_ms = Some(MAX_AGE_MS);
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &scan, &mut sender),
            ActionOutcome::SigtermRequested
        );
        assert_eq!(sender.calls, vec![(42, 1)]);
    }

    #[test]
    fn delivery_without_row_is_already_exited_without_send() {
        let admitted = assess_scan(finding(42, 1), Some(&Scan::one(42, 1, 501, OBSERVED_MS)));
        let delivery = Scan::empty(OBSERVED_MS);
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &delivery, &mut sender),
            ActionOutcome::AlreadyExited
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn delivery_with_other_start_time_is_identity_reused_without_send() {
        let admitted = assess_scan(finding(42, 1), Some(&Scan::one(42, 1, 501, OBSERVED_MS)));
        let delivery = Scan::one(42, 2, 501, OBSERVED_MS);
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &delivery, &mut sender),
            denied(vec![ActionDenial::Preflight(
                PreflightReason::IdentityReused
            )])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn delivery_owner_uid_zero_is_denied_without_send() {
        let admitted = assess_scan(finding(42, 1), Some(&Scan::one(42, 1, 501, OBSERVED_MS)));
        let delivery = Scan::one(42, 1, 0, OBSERVED_MS);
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &delivery, &mut sender),
            denied(vec![
                ActionDenial::Preflight(PreflightReason::Ineligible(
                    EligibilityReason::PrivilegedOwner,
                )),
                ActionDenial::Preflight(PreflightReason::Ineligible(
                    EligibilityReason::SystemOwner,
                )),
            ])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn other_user_owner_is_owner_mismatch_without_send() {
        let scan = Scan::one(42, 1, 502, OBSERVED_MS);
        let admitted = assess_scan(finding(42, 1), Some(&scan));
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &scan, &mut sender),
            denied(vec![ActionDenial::OwnerMismatch])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn denied_admission_without_reasons_falls_back_without_send() {
        let admitted = PreflightAssessment {
            finding: finding(42, 1),
            action: RequestedAction::Close,
            conclusion: PreflightConclusion::Denied,
            reasons: Vec::new(),
            owner_uid: Some(501),
            observation_age_ms: Some(0),
        };
        let delivery = Scan::one(42, 1, 501, OBSERVED_MS);
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 1), &admitted, &delivery, &mut sender),
            denied(vec![ActionDenial::PreflightNotAllowed])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn delivery_stale_or_future_dated_is_stale_without_send() {
        let admitted = assess_scan(finding(42, 1), Some(&Scan::one(42, 1, 501, OBSERVED_MS)));
        for observed_at_ms in [NOW_MS - MAX_AGE_MS - 1, NOW_MS + 1] {
            let delivery = Scan::one(42, 1, 501, observed_at_ms);
            let mut sender = sender(SignalDelivery::Requested);
            assert_eq!(
                run(finding(42, 1), &admitted, &delivery, &mut sender),
                denied(vec![ActionDenial::Preflight(
                    PreflightReason::StaleObservation
                )])
            );
            assert!(sender.calls.is_empty());
        }
    }

    #[test]
    fn zero_start_time_is_unverifiable_without_send() {
        let scan = Scan::one(42, 0, 501, OBSERVED_MS);
        let admitted = assess_scan(finding(42, 0), Some(&scan));
        let mut sender = sender(SignalDelivery::Requested);
        assert_eq!(
            run(finding(42, 0), &admitted, &scan, &mut sender),
            denied(vec![ActionDenial::Preflight(
                PreflightReason::UnverifiableIdentity
            )])
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn sender_delivery_maps_to_outcome() {
        let scan = Scan::one(42, 1, 501, OBSERVED_MS);
        let admitted = assess_scan(finding(42, 1), Some(&scan));
        let cases = [
            (SignalDelivery::Requested, ActionOutcome::SigtermRequested),
            (SignalDelivery::AlreadyExited, ActionOutcome::AlreadyExited),
            (
                SignalDelivery::PermissionDenied,
                ActionOutcome::Failed {
                    failure: ActionFailure::PermissionDenied,
                },
            ),
            (
                SignalDelivery::InvalidTarget,
                ActionOutcome::Failed {
                    failure: ActionFailure::InvalidTarget,
                },
            ),
            (
                SignalDelivery::Failed,
                ActionOutcome::Failed {
                    failure: ActionFailure::DeliveryFailed,
                },
            ),
        ];
        for (next, expected) in cases {
            let mut sender = sender(next);
            assert_eq!(run(finding(42, 1), &admitted, &scan, &mut sender), expected);
            assert_eq!(sender.calls, vec![(42, 1)]);
        }
    }

    #[test]
    fn repeated_identical_calls_send_each_time() {
        let scan = Scan::one(42, 1, 501, OBSERVED_MS);
        let admitted = assess_scan(finding(42, 1), Some(&scan));
        let mut sender = sender(SignalDelivery::Requested);
        let first = run(finding(42, 1), &admitted, &scan, &mut sender);
        let second = run(finding(42, 1), &admitted, &scan, &mut sender);
        assert_eq!(first, second);
        assert_eq!(first, ActionOutcome::SigtermRequested);
        assert_eq!(sender.calls, vec![(42, 1), (42, 1)]);
    }

    #[test]
    fn denial_then_allowed_call_sends_once() {
        let privileged = Scan::one(42, 1, 0, OBSERVED_MS);
        let denied_admission = assess_scan(finding(42, 1), Some(&privileged));
        let allowed = Scan::one(42, 1, 501, OBSERVED_MS);
        let allowed_admission = assess_scan(finding(42, 1), Some(&allowed));
        let mut sender = sender(SignalDelivery::Requested);
        run(finding(42, 1), &denied_admission, &privileged, &mut sender);
        assert_eq!(
            run(finding(42, 1), &allowed_admission, &allowed, &mut sender),
            ActionOutcome::SigtermRequested
        );
        assert_eq!(sender.calls, vec![(42, 1)]);
    }

    #[cfg(unix)]
    #[test]
    fn classifier_maps_kernel_answer() {
        assert_eq!(classify_delivery(0, None), SignalDelivery::Requested);
        assert_eq!(
            classify_delivery(0, Some(libc::EPERM)),
            SignalDelivery::Requested
        );
        assert_eq!(
            classify_delivery(-1, Some(libc::ESRCH)),
            SignalDelivery::AlreadyExited
        );
        assert_eq!(
            classify_delivery(-1, Some(libc::EPERM)),
            SignalDelivery::PermissionDenied
        );
        assert_eq!(
            classify_delivery(-1, Some(libc::EINVAL)),
            SignalDelivery::Failed
        );
        assert_eq!(classify_delivery(-1, None), SignalDelivery::Failed);
    }

    #[cfg(unix)]
    #[test]
    fn unix_guard_refuses_before_syscall() {
        let mut unix = UnixSignalSender;
        assert_eq!(unix.request_stop((0, 1)), SignalDelivery::InvalidTarget);
        assert_eq!(unix.request_stop((1, 1)), SignalDelivery::InvalidTarget);
        assert_eq!(
            unix.request_stop((u32::MAX, 1)),
            SignalDelivery::InvalidTarget
        );
    }
}
