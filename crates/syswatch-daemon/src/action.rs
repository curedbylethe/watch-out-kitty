//! Answers the close requests from the latest process scan.
//!
//! Every answer runs the preflight on the scan's evidence at request time.
//! `preview` sends nothing. `confirm` calls the sender at most once, and only
//! after the core action service admits the delivery-time observation.

use syswatch_core::FindingId;
use syswatch_core::action::{ActionRequest, SignalSender, execute};
use syswatch_core::collector::timestamp_ms;
use syswatch_core::preflight::{self, PreflightObservation, PreflightRequest, RequestedAction};
use syswatch_core::protocol::{ErrorCode, ProtocolError, RequestCommand, Response};

use crate::latest::ActionState;

/// Answers one protocol request. Monitoring commands are not accepted here.
pub fn answer(
    command: RequestCommand,
    finding: FindingId,
    state: Option<&ActionState<'_>>,
    max_observation_age_ms: u64,
    sender: &mut impl SignalSender,
) -> Response {
    // No scan with findings yet: nothing is advisory, so the identity is not found.
    let Some(state) = state else {
        return not_found(finding);
    };
    let now_ms = timestamp_ms();
    match command {
        RequestCommand::ClosePreview => preview(state, finding, now_ms, max_observation_age_ms),
        RequestCommand::CloseConfirm => {
            confirm(state, finding, now_ms, max_observation_age_ms, sender)
        }
        RequestCommand::Status | RequestCommand::Processes | RequestCommand::Suggestions => {
            Response::error(ProtocolError {
                code: ErrorCode::UnsupportedCommand,
                message: "not an action command".into(),
            })
        }
    }
}

fn preview(
    state: &ActionState<'_>,
    finding: FindingId,
    now_ms: u64,
    max_observation_age_ms: u64,
) -> Response {
    let observation = match observation(state, finding) {
        Ok(observation) => observation,
        Err(error) => return Response::error(error),
    };
    let assessment = preflight::assess(
        &PreflightRequest {
            finding,
            action: RequestedAction::Close,
            now_ms,
            max_observation_age_ms,
        },
        Some(&observation),
    );
    // A row with the same PID and another start time is a different process.
    let process = state
        .processes
        .iter()
        .find(|process| process.identity() == finding.identity())
        .cloned();
    Response::close_preview(assessment, process)
}

fn confirm(
    state: &ActionState<'_>,
    finding: FindingId,
    now_ms: u64,
    max_observation_age_ms: u64,
    sender: &mut impl SignalSender,
) -> Response {
    let observation = match observation(state, finding) {
        Ok(observation) => observation,
        Err(error) => return Response::error(error),
    };
    let assessment = preflight::assess(
        &PreflightRequest {
            finding,
            action: RequestedAction::Close,
            now_ms,
            max_observation_age_ms,
        },
        Some(&observation),
    );
    let request = ActionRequest {
        finding,
        now_ms,
        max_preflight_age_ms: max_observation_age_ms,
        max_observation_age_ms,
        caller_uid: caller_uid(),
    };
    Response::close_result(execute(&request, &assessment, &observation, sender))
}

/// Refuses an identity that is not a cached advisory finding, or a scan with
/// no monitor identity. The monitor is never invented.
fn observation<'a>(
    state: &ActionState<'a>,
    finding: FindingId,
) -> Result<PreflightObservation<'a>, ProtocolError> {
    // The raw findings, not the dismissed view: a dismissal is a display filter.
    if !state
        .findings
        .iter()
        .any(|candidate| candidate.id == finding)
    {
        return Err(not_found_error(finding));
    }
    let Some(monitor) = state.monitor else {
        return Err(unavailable(
            "monitor identity is not in the current process scan",
        ));
    };
    Ok(PreflightObservation {
        processes: state.processes,
        provenance: state.provenance,
        observed_at_ms: state.observed_at_ms,
        monitor,
    })
}

fn not_found(finding: FindingId) -> Response {
    Response::error(not_found_error(finding))
}

fn not_found_error(finding: FindingId) -> ProtocolError {
    ProtocolError {
        code: ErrorCode::NotFound,
        message: format!(
            "finding {} is not among the current advisory findings",
            finding.reference()
        ),
    }
}

fn unavailable(message: &str) -> ProtocolError {
    ProtocolError {
        code: ErrorCode::Unavailable,
        message: message.into(),
    }
}

fn caller_uid() -> u32 {
    // SAFETY: `getuid` takes no arguments and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use syswatch_core::action::{
        ActionDenial, ActionFailure, ActionOutcome, SignalDelivery, SignalSender,
    };
    use syswatch_core::collector::timestamp_ms;
    use syswatch_core::preflight::{PreflightConclusion, PreflightReason};
    use syswatch_core::protocol::{ErrorCode, RequestCommand, Response};
    use syswatch_core::provenance::{ParentContext, ProcessProvenance};
    use syswatch_core::{
        AdvisoryFinding, Confidence, EligibilityReason, FindingEvidence, FindingId, FindingKind,
        ProcessSnapshot,
    };

    use super::{answer, caller_uid};
    use crate::latest::ActionState;

    const TARGET: (u32, u64) = (42, 100);
    const MONITOR: (u32, u64) = (9_999, 9_999);
    const MAX_AGE_MS: u64 = 10_000;

    #[derive(Default)]
    struct FakeSender {
        calls: Vec<(u32, u64)>,
        reply: Option<SignalDelivery>,
    }

    impl SignalSender for FakeSender {
        fn request_stop(&mut self, target: (u32, u64)) -> SignalDelivery {
            self.calls.push(target);
            self.reply.unwrap_or(SignalDelivery::Requested)
        }
    }

    fn target_finding() -> FindingId {
        FindingId {
            pid: TARGET.0,
            start_time_secs: TARGET.1,
            kind: FindingKind::Cpu,
        }
    }

    fn advisory(id: FindingId) -> AdvisoryFinding {
        AdvisoryFinding {
            id,
            confidence: Confidence::High,
            observed_at_ms: 1_000,
            explanation: "pid 42 used CPU continuously".into(),
            evidence: FindingEvidence {
                sample_count: 2,
                span_ms: 120_000,
                cpu: None,
                memory: None,
            },
        }
    }

    fn row(pid: u32, start_time_secs: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: None,
            name: "worker".into(),
            cpu_usage_percent: 0.0,
            memory_bytes: 0,
            start_time_secs,
        }
    }

    fn provenance(pid: u32, start_time_secs: u64, owner_uid: Option<u32>) -> ProcessProvenance {
        ProcessProvenance {
            pid,
            start_time_secs,
            executable: None,
            program: Some("worker".into()),
            owner_uid,
            parent: ParentContext::NoParent,
        }
    }

    /// Owned scan data that a test borrows through [`ActionState`].
    struct Scan {
        processes: Vec<ProcessSnapshot>,
        provenance: BTreeMap<(u32, u64), ProcessProvenance>,
        findings: Vec<AdvisoryFinding>,
        observed_at_ms: u64,
        monitor: Option<(u32, u64)>,
    }

    impl Scan {
        fn healthy() -> Self {
            let rows = [TARGET, MONITOR]
                .map(|(pid, start)| provenance(pid, start, Some(caller_uid())))
                .map(|row| (row.identity(), row));
            Self {
                processes: vec![row(TARGET.0, TARGET.1), row(MONITOR.0, MONITOR.1)],
                provenance: BTreeMap::from(rows),
                findings: vec![advisory(target_finding())],
                observed_at_ms: timestamp_ms(),
                monitor: Some(MONITOR),
            }
        }

        fn state(&self) -> ActionState<'_> {
            ActionState {
                processes: &self.processes,
                provenance: &self.provenance,
                observed_at_ms: self.observed_at_ms,
                monitor: self.monitor,
                findings: &self.findings,
            }
        }
    }

    fn close(scan: &Scan, finding: FindingId, sender: &mut FakeSender) -> Option<ActionOutcome> {
        match answer(
            RequestCommand::CloseConfirm,
            finding,
            Some(&scan.state()),
            MAX_AGE_MS,
            sender,
        ) {
            Response::CloseResult { outcome, .. } => Some(outcome),
            _ => None,
        }
    }

    #[test]
    fn preview_names_the_allowed_identity_and_sends_nothing() {
        let scan = Scan::healthy();
        let response = answer(
            RequestCommand::ClosePreview,
            target_finding(),
            Some(&scan.state()),
            MAX_AGE_MS,
            &mut FakeSender::default(),
        );
        let Response::ClosePreview {
            assessment,
            process,
            ..
        } = response
        else {
            panic!("expected close preview");
        };
        assert_eq!(
            assessment.conclusion,
            PreflightConclusion::AllowedForConfirmation
        );
        assert_eq!(process.map(|row| row.name), Some("worker".into()));
    }

    #[test]
    fn confirm_requests_one_sigterm_for_the_full_identity() {
        let scan = Scan::healthy();
        let mut sender = FakeSender::default();
        assert_eq!(
            close(&scan, target_finding(), &mut sender),
            Some(ActionOutcome::SigtermRequested)
        );
        assert_eq!(sender.calls, [TARGET]);
    }

    #[test]
    fn monitor_target_is_denied_without_a_signal() {
        let mut scan = Scan::healthy();
        let finding = FindingId {
            pid: MONITOR.0,
            start_time_secs: MONITOR.1,
            kind: FindingKind::Cpu,
        };
        scan.findings = vec![advisory(finding)];
        let mut sender = FakeSender::default();
        let outcome = close(&scan, finding, &mut sender).unwrap();
        assert!(matches!(
            outcome,
            ActionOutcome::Denied { ref reasons }
                if reasons.contains(&ActionDenial::Preflight(PreflightReason::Ineligible(
                    EligibilityReason::MonitorProcess
                )))
        ));
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn privileged_owner_is_denied_without_a_signal() {
        let mut scan = Scan::healthy();
        scan.provenance
            .insert(TARGET, provenance(TARGET.0, TARGET.1, Some(0)));
        let mut sender = FakeSender::default();
        let outcome = close(&scan, target_finding(), &mut sender).unwrap();
        assert!(matches!(outcome, ActionOutcome::Denied { .. }));
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn reused_pid_is_denied_without_a_signal() {
        let mut scan = Scan::healthy();
        scan.processes = vec![row(TARGET.0, TARGET.1 + 1)];
        let mut sender = FakeSender::default();
        let outcome = close(&scan, target_finding(), &mut sender).unwrap();
        assert_eq!(
            outcome,
            ActionOutcome::Denied {
                reasons: vec![ActionDenial::Preflight(PreflightReason::IdentityReused)]
            }
        );
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn stale_observation_is_denied_without_a_signal() {
        let mut scan = Scan::healthy();
        scan.observed_at_ms = timestamp_ms() - MAX_AGE_MS - 1_000;
        let mut sender = FakeSender::default();
        let outcome = close(&scan, target_finding(), &mut sender).unwrap();
        assert!(matches!(
            outcome,
            ActionOutcome::Denied { ref reasons }
                if reasons.contains(&ActionDenial::Preflight(PreflightReason::StaleObservation))
        ));
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn identity_absent_from_findings_is_not_found() {
        let mut scan = Scan::healthy();
        scan.findings.clear();
        let mut sender = FakeSender::default();
        let response = answer(
            RequestCommand::CloseConfirm,
            target_finding(),
            Some(&scan.state()),
            MAX_AGE_MS,
            &mut sender,
        );
        let Response::Error { code, .. } = response else {
            panic!("expected error");
        };
        assert_eq!(code, ErrorCode::NotFound);
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn missing_monitor_identity_is_unavailable_without_a_signal() {
        let mut scan = Scan::healthy();
        scan.monitor = None;
        let mut sender = FakeSender::default();
        let response = answer(
            RequestCommand::CloseConfirm,
            target_finding(),
            Some(&scan.state()),
            MAX_AGE_MS,
            &mut sender,
        );
        let Response::Error { code, .. } = response else {
            panic!("expected error");
        };
        assert_eq!(code, ErrorCode::Unavailable);
        assert!(sender.calls.is_empty());
    }

    #[test]
    fn sender_refusal_is_a_failure_after_one_call() {
        let scan = Scan::healthy();
        let mut sender = FakeSender {
            calls: Vec::new(),
            reply: Some(SignalDelivery::PermissionDenied),
        };
        assert_eq!(
            close(&scan, target_finding(), &mut sender),
            Some(ActionOutcome::Failed {
                failure: ActionFailure::PermissionDenied
            })
        );
        assert_eq!(sender.calls.len(), 1);
    }

    #[test]
    fn no_scan_is_not_found_without_a_signal() {
        let mut sender = FakeSender::default();
        let response = answer(
            RequestCommand::ClosePreview,
            target_finding(),
            None,
            MAX_AGE_MS,
            &mut sender,
        );
        let Response::Error { code, .. } = response else {
            panic!("expected error");
        };
        assert_eq!(code, ErrorCode::NotFound);
        assert!(sender.calls.is_empty());
    }
}
