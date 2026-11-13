use syswatch_core::{
    action::{ActionDenial, ActionFailure, ActionOutcome},
    findings::{self, FindingId},
    preflight,
    protocol::{ErrorCode, Response},
    report::{self, ProcessSort, format_bytes, format_cpu_percent},
};

pub fn render_status(response: &Response, now_ms: u64) -> Result<String, String> {
    let Response::Status {
        aggregate,
        process_count,
        processes_updated_at_ms,
        ..
    } = response
    else {
        return Err(match response {
            Response::Error { code, message, .. } => {
                format!("daemon error ({}): {message}", error_code(code))
            }
            _ => "invalid response from daemon: expected status response".to_owned(),
        });
    };
    let cpu = if aggregate.cpu_updated_at_ms.is_some() {
        format_cpu_percent(aggregate.cpu_usage_percent)
    } else {
        "warming up".to_owned()
    };
    let memory = aggregate.memory.usage_percent();
    let processes = process_count.map_or_else(
        || "Processes: not yet sampled".to_owned(),
        |count| {
            let age = processes_updated_at_ms
                .map(|updated| format_age_ms(now_ms, updated))
                .unwrap_or_else(|| "unknown".to_owned());
            format!("Processes: {count} (updated {age} ago)")
        },
    );
    Ok(format!(
        "CPU: {cpu}\nMemory: {} / {} ({})\n{}\nSampled: {} ago",
        format_bytes(aggregate.memory.used_bytes),
        format_bytes(aggregate.memory.total_bytes),
        format_cpu_percent(memory),
        processes,
        format_age_ms(now_ms, aggregate.timestamp_ms),
    ))
}

pub fn render_top(
    response: &Response,
    sort: ProcessSort,
    limit: usize,
    now_ms: u64,
) -> Result<String, String> {
    match response {
        Response::Processes {
            processes_updated_at_ms,
            processes,
            ..
        } => Ok(format!(
            "{}\n{}",
            report::render_table(processes, sort, limit),
            report::format_process_age(now_ms, Some(*processes_updated_at_ms))
        )),
        Response::Error {
            code: ErrorCode::Unavailable,
            ..
        } => Ok(report::format_process_age(now_ms, None)),
        Response::Error { code, message, .. } => {
            Err(format!("daemon error ({}): {message}", error_code(code)))
        }
        _ => Err("invalid response from daemon: expected processes response".to_owned()),
    }
}

/// Renders cached findings. `processes` is the optional cached process reply
/// used only to look up names; a missing or foreign reply renders `<unknown>`.
pub fn render_suggestions(
    response: &Response,
    processes: Option<&Response>,
    now_ms: u64,
) -> Result<String, String> {
    match response {
        Response::Suggestions {
            evaluated_at_ms,
            processes_updated_at_ms,
            findings,
            ..
        } => {
            let mut findings = findings.clone();
            findings::sort(&mut findings);
            let mut lines = Vec::new();
            for finding in &findings {
                let id = finding.id;
                let name = match processes {
                    Some(Response::Processes {
                        processes: rows, ..
                    }) => rows
                        .iter()
                        .find(|row| (row.pid, row.start_time_secs) == id.identity())
                        .map_or_else(
                            || "<unknown>".to_owned(),
                            |row| report::sanitize_field(&row.name, 24),
                        ),
                    _ => "<unknown>".to_owned(),
                };
                lines.push(format!(
                    "{}  {}  {name}",
                    id.reference(),
                    finding.confidence,
                ));
                lines.push(format!(
                    "  {}",
                    report::sanitize_field(&finding.explanation, 160)
                ));
            }
            if findings.is_empty() {
                lines.push("(no current suggestions)".to_owned());
            }
            lines.push(format!(
                "findings evaluated {} ago; {}",
                format_age_ms(now_ms, *evaluated_at_ms),
                report::format_process_age(now_ms, Some(*processes_updated_at_ms)),
            ));
            Ok(lines.join("\n"))
        }
        Response::Error {
            code: ErrorCode::Unavailable,
            message,
            ..
        } => Ok(format!("still collecting evidence: {message}")),
        Response::Error { code, message, .. } => {
            Err(format!("daemon error ({}): {message}", error_code(code)))
        }
        _ => Err("invalid response from daemon: expected suggestions response".to_owned()),
    }
}

/// Renders one cached finding, selected by its full identity. The reply
/// carries no process name, argv, or path, so none is printed.
pub fn render_inspect(
    response: &Response,
    reference: FindingId,
    now_ms: u64,
) -> Result<String, String> {
    let finding = match response {
        Response::Suggestions { findings, .. } => findings
            .iter()
            .find(|finding| finding.id == reference)
            .ok_or_else(|| {
                format!(
                    "finding not found: {}; it may have expired",
                    reference.reference()
                )
            })?,
        Response::Error { code, message, .. } => {
            return Err(format!("daemon error ({}): {message}", error_code(code)));
        }
        _ => return Err("invalid response from daemon: expected suggestions response".to_owned()),
    };
    let evidence = &finding.evidence;
    let samples = if evidence.sample_count == 0 {
        "  samples: no retained samples".to_owned()
    } else {
        format!(
            "  samples: {} sample{} over {:.1}s",
            evidence.sample_count,
            plural(evidence.sample_count),
            evidence.span_ms as f64 / 1000.0,
        )
    };
    let cpu = match &evidence.cpu {
        None => "  CPU: not measured".to_owned(),
        Some(cpu) => format!(
            "  CPU: mean {}, max {}, {} valid sample{}",
            format_cpu_percent(cpu.mean_percent),
            format_cpu_percent(cpu.max_percent),
            cpu.valid_samples,
            plural(cpu.valid_samples),
        ),
    };
    let memory = match &evidence.memory {
        None => "  Memory: not measured".to_owned(),
        Some(memory) => format!(
            "  Memory: latest {}, max {}",
            format_bytes(memory.latest_bytes),
            format_bytes(memory.max_bytes),
        ),
    };
    let explanation = report::sanitize_line(&finding.explanation, 160);
    let explanation = if explanation.is_empty() {
        "  (none)".to_owned()
    } else {
        format!("  {explanation}")
    };
    Ok(format!(
        "Finding: {}\nKind: {}\nConfidence: {}\nObserved: {} ago ({})\n\nEvidence:\n{samples}\n{cpu}\n{memory}\n\nExplanation:\n{explanation}\n\nReview suggestion only, not proof of abandonment or permission to terminate.",
        reference.reference(),
        reference.kind,
        finding.confidence,
        format_age_ms(now_ms, finding.observed_at_ms),
        finding.observed_at_ms,
    ))
}

/// Empty suffix for exactly one item, `s` otherwise.
/// Renders the labelled preview for one close request. Nothing was executed.
pub fn render_close_preview(response: &Response) -> Result<String, String> {
    match response {
        Response::ClosePreview {
            assessment,
            process,
            ..
        } => Ok(format!(
            "Action preview\n{}",
            preflight::preview(assessment, process.as_ref())
        )),
        Response::Error { code, message, .. } => {
            Err(format!("daemon error ({}): {message}", error_code(code)))
        }
        _ => Err("invalid response from daemon: expected close preview response".to_owned()),
    }
}

/// Renders a close confirmation and its exit code. Only a requested SIGTERM
/// exits 0; the text never claims the process exited.
pub fn render_close_outcome(
    response: &Response,
    reference: FindingId,
) -> Result<(String, i32), String> {
    let outcome = match response {
        Response::CloseResult { outcome, .. } => outcome,
        Response::Error { code, message, .. } => {
            return Err(format!("daemon error ({}): {message}", error_code(code)));
        }
        _ => return Err("invalid response from daemon: expected close result response".to_owned()),
    };
    let reference = reference.reference();
    Ok(match outcome {
        ActionOutcome::SigtermRequested => (
            format!(
                "signal requested: SIGTERM\nprocess: {reference}\nThe daemon accepted the request. This is not confirmation that the process exited.\nRun `syswatch inspect {reference}` to check again."
            ),
            0,
        ),
        ActionOutcome::AlreadyExited => (
            "no signal sent: the process already exited\nNo signal was requested.".to_owned(),
            1,
        ),
        ActionOutcome::Denied { reasons } => {
            let mut lines = vec!["action denied: no signal was sent".to_owned()];
            lines.extend(
                reasons
                    .iter()
                    .map(|reason| format!("reason: {}", denial_text(reason))),
            );
            (lines.join("\n"), 1)
        }
        ActionOutcome::Failed { failure } => (
            format!(
                "action failed: no signal was confirmed\nreason: {}",
                failure_text(*failure)
            ),
            1,
        ),
        _ => ("action failed: no signal was confirmed".to_owned(), 1),
    })
}

fn denial_text(reason: &ActionDenial) -> String {
    match reason {
        ActionDenial::Preflight(reason) => preflight::reason_text(reason),
        ActionDenial::PreflightNotAllowed => "preflight did not allow the action".to_owned(),
        ActionDenial::PreflightTooOld => "preflight evidence is too old".to_owned(),
        ActionDenial::PreflightMismatch => "preflight was for another finding".to_owned(),
        ActionDenial::OwnerMismatch => "process is not owned by the current user".to_owned(),
        _ => "action refused".to_owned(),
    }
}

fn failure_text(failure: ActionFailure) -> &'static str {
    match failure {
        ActionFailure::PermissionDenied => "permission denied",
        ActionFailure::InvalidTarget => "invalid target",
        _ => "delivery failed",
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn error_code(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnsupportedVersion => "unsupported_version",
        ErrorCode::UnsupportedCommand => "unsupported_command",
        ErrorCode::MalformedRequest => "malformed_request",
        ErrorCode::Unavailable => "unavailable",
        ErrorCode::NotFound => "not_found",
    }
}

pub fn format_age_ms(now_ms: u64, then_ms: u64) -> String {
    format!("{:.1}s", now_ms.saturating_sub(then_ms) as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::{
        format_age_ms, render_close_outcome, render_close_preview, render_inspect, render_status,
        render_suggestions, render_top,
    };
    use syswatch_core::{
        MemorySnapshot,
        action::{ActionDenial, ActionFailure, ActionOutcome},
        findings::{
            AdvisoryFinding, Confidence, CpuActivity, FindingEvidence, FindingId, FindingKind,
            MemoryActivity,
        },
        preflight::{PreflightAssessment, PreflightConclusion, PreflightReason, RequestedAction},
        protocol::{ErrorCode, PROTOCOL_VERSION, ProtocolError, Response},
        report::ProcessSort,
        snapshot::{AggregateSnapshot, ProcessSnapshot},
    };

    fn close_finding() -> FindingId {
        FindingId {
            pid: 42,
            start_time_secs: 1,
            kind: FindingKind::Cpu,
        }
    }

    fn close_assessment(
        conclusion: PreflightConclusion,
        reasons: Vec<PreflightReason>,
    ) -> PreflightAssessment {
        PreflightAssessment {
            finding: close_finding(),
            action: RequestedAction::Close,
            conclusion,
            reasons,
            owner_uid: Some(501),
            observation_age_ms: Some(250),
        }
    }

    fn close_result(outcome: ActionOutcome) -> Response {
        Response::CloseResult {
            version: PROTOCOL_VERSION,
            outcome,
        }
    }

    fn process(pid: u32, name: &str, memory_bytes: u64, cpu_usage_percent: f32) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: None,
            name: name.to_owned(),
            cpu_usage_percent,
            memory_bytes,
            start_time_secs: 1,
        }
    }

    fn processes(processes: Vec<ProcessSnapshot>) -> Response {
        Response::Processes {
            version: PROTOCOL_VERSION,
            observed_at_ms: 1_700_000_000_123,
            processes_updated_at_ms: 1_700_000_000_023,
            processes,
        }
    }

    fn status(cpu_updated_at_ms: Option<u64>, process_count: Option<usize>) -> Response {
        Response::Status {
            version: PROTOCOL_VERSION,
            aggregate: AggregateSnapshot {
                timestamp_ms: 1_700_000_000_123,
                cpu_usage_percent: 42.5,
                cpu_updated_at_ms,
                memory: MemorySnapshot {
                    total_bytes: 17_179_869_184,
                    used_bytes: 8_589_934_592,
                },
            },
            process_count,
            processes_updated_at_ms: process_count.map(|_| 1_700_000_000_023),
        }
    }

    #[test]
    fn renders_status_measurements_and_ages() {
        assert_eq!(
            render_status(
                &status(Some(1_700_000_000_123), Some(312)),
                1_700_000_000_523
            )
            .unwrap(),
            concat!(
                "CPU: 42.5%\n",
                "Memory: 8.0 GiB / 16.0 GiB (50.0%)\n",
                "Processes: 312 (updated 0.5s ago)\n",
                "Sampled: 0.4s ago"
            )
        );
    }

    #[test]
    fn renders_warmup_and_unsampled_processes() {
        let rendered = render_status(&status(None, None), 1_700_000_000_523).unwrap();
        assert!(rendered.contains("CPU: warming up"));
        assert!(rendered.contains("Processes: not yet sampled"));
    }

    #[test]
    fn zero_total_memory_renders_zero_percent() {
        let mut response = status(Some(1), Some(0));
        if let Response::Status { aggregate, .. } = &mut response {
            aggregate.memory.total_bytes = 0;
            aggregate.memory.used_bytes = 42;
        }
        let rendered = render_status(&response, 1).unwrap();
        assert!(rendered.contains("Memory: 42 B / 0 B (0.0%)"));
    }

    #[test]
    fn age_clamps_future_values() {
        assert_eq!(format_age_ms(10, 11), "0.0s");
    }

    #[test]
    fn unexpected_response_kind_is_reported() {
        let response = Response::error(syswatch_core::protocol::ProtocolError {
            code: syswatch_core::protocol::ErrorCode::MalformedRequest,
            message: "bad".to_owned(),
        });
        assert_eq!(
            render_status(&response, 0).unwrap_err(),
            "daemon error (malformed_request): bad"
        );
    }

    #[test]
    fn top_renders_memory_order_and_equal_memory_by_pid() {
        let response = processes(vec![
            process(3, "three", 20, 1.0),
            process(2, "two", 20, 2.0),
            process(1, "one", 10, 3.0),
        ]);
        let rendered = render_top(&response, ProcessSort::Memory, 10, 1_700_000_000_523).unwrap();
        let rows = rendered.lines().skip(1).take(3).collect::<Vec<_>>();
        assert!(rows[0].starts_with("2"));
        assert!(rows[1].starts_with("3"));
        assert!(rows[2].starts_with("1"));
    }

    #[test]
    fn top_renders_cpu_order_and_applies_limit_before_age_line() {
        let response = processes(vec![
            process(1, "one", 100, 50.0),
            process(2, "two", 200, 125.0),
            process(3, "three", 300, 90.0),
        ]);
        let rendered = render_top(&response, ProcessSort::Cpu, 2, 1_700_000_000_523).unwrap();
        let rows = rendered.lines().skip(1).collect::<Vec<_>>();
        assert!(rows[0].starts_with("2"));
        assert!(rows[0].ends_with("125.0%"));
        assert!(rows[1].starts_with("3"));
        assert_eq!(rows[2], "process data: 0.5 s old");
    }

    #[test]
    fn top_renders_empty_dated_data() {
        assert_eq!(
            render_top(&processes(Vec::new()), ProcessSort::Memory, 10, 100).unwrap(),
            "(no processes observed)\nprocess data: 0.0 s old"
        );
    }

    #[test]
    fn top_renders_unavailable_data() {
        let unavailable = Response::error(ProtocolError {
            code: ErrorCode::Unavailable,
            message: "not sampled".to_owned(),
        });
        assert_eq!(
            render_top(&unavailable, ProcessSort::Cpu, 10, 100).unwrap(),
            "process data: unavailable"
        );
    }

    #[test]
    fn top_reports_daemon_errors_and_wrong_response_kind() {
        let error = Response::error(ProtocolError {
            code: ErrorCode::MalformedRequest,
            message: "bad".to_owned(),
        });
        assert_eq!(
            render_top(&error, ProcessSort::Memory, 10, 0).unwrap_err(),
            "daemon error (malformed_request): bad"
        );
        assert!(
            render_top(&status(Some(1), Some(1)), ProcessSort::Memory, 10, 0)
                .unwrap_err()
                .contains("expected processes response")
        );
    }

    #[test]
    fn top_renders_nonfinite_cpu_as_na_and_ranks_it_last() {
        let response = processes(vec![
            process(1, "nan", 0, f32::NAN),
            process(2, "finite", 0, 1.0),
        ]);
        let rendered = render_top(&response, ProcessSort::Cpu, 10, 1_700_000_000_023).unwrap();
        let rows = rendered.lines().skip(1).take(2).collect::<Vec<_>>();
        assert!(rows[0].starts_with("2"));
        assert!(rows[1].starts_with("1"));
        assert!(rows[1].ends_with("n/a"));
    }

    const NOW: u64 = 1_700_000_000_523;

    fn finding(
        pid: u32,
        kind: FindingKind,
        confidence: Confidence,
        explanation: &str,
    ) -> AdvisoryFinding {
        AdvisoryFinding {
            id: FindingId {
                pid,
                start_time_secs: 1,
                kind,
            },
            confidence,
            observed_at_ms: 1,
            explanation: explanation.to_owned(),
            evidence: FindingEvidence {
                sample_count: 0,
                span_ms: 0,
                cpu: None,
                memory: None,
            },
        }
    }

    fn suggestions(findings: Vec<AdvisoryFinding>) -> Response {
        Response::Suggestions {
            version: PROTOCOL_VERSION,
            evaluated_at_ms: 1_700_000_000_123,
            processes_updated_at_ms: 1_700_000_000_023,
            findings,
        }
    }

    const FOOTER: &str = "findings evaluated 0.4s ago; process data: 0.5 s old";

    fn inspect_finding(
        pid: u32,
        start: u64,
        kind: FindingKind,
        explanation: &str,
    ) -> AdvisoryFinding {
        let mut found = finding(pid, kind, Confidence::High, explanation);
        found.id.start_time_secs = start;
        found
    }

    fn reference(pid: u32, start: u64, kind: FindingKind) -> FindingId {
        FindingId {
            pid,
            start_time_secs: start,
            kind,
        }
    }

    #[test]
    fn inspect_renders_the_full_report() {
        let mut found = inspect_finding(
            42,
            1_700_000_000,
            FindingKind::Memory,
            "pid 42 held 512 MiB while idle; review this process",
        );
        found.observed_at_ms = 1_700_000_000_123;
        found.evidence = FindingEvidence {
            sample_count: 7,
            span_ms: 120_000,
            cpu: Some(CpuActivity {
                max_percent: 0.8,
                mean_percent: 0.4,
                valid_samples: 7,
            }),
            memory: Some(MemoryActivity {
                latest_bytes: 512 * 1024 * 1024,
                max_bytes: 600 * 1024 * 1024,
            }),
        };
        let rendered = render_inspect(
            &suggestions(vec![found]),
            reference(42, 1_700_000_000, FindingKind::Memory),
            1_700_000_004_323,
        )
        .unwrap();
        assert_eq!(
            rendered,
            [
                "Finding: 42-1700000000-memory",
                "Kind: memory",
                "Confidence: high",
                "Observed: 4.2s ago (1700000000123)",
                "",
                "Evidence:",
                "  samples: 7 samples over 120.0s",
                "  CPU: mean 0.4%, max 0.8%, 7 valid samples",
                "  Memory: latest 512.0 MiB, max 600.0 MiB",
                "",
                "Explanation:",
                "  pid 42 held 512 MiB while idle; review this process",
                "",
                "Review suggestion only, not proof of abandonment or permission to terminate.",
            ]
            .join("\n")
        );
    }

    #[test]
    fn inspect_selects_the_exact_identity() {
        let response = suggestions(vec![
            inspect_finding(42, 1, FindingKind::Memory, "first"),
            inspect_finding(42, 2, FindingKind::Memory, "second"),
            inspect_finding(42, 1, FindingKind::Cpu, "third"),
        ]);
        let rendered =
            render_inspect(&response, reference(42, 1, FindingKind::Memory), NOW).unwrap();
        assert!(rendered.starts_with("Finding: 42-1-memory\n"));
        assert!(rendered.contains("\n  first\n"));
        assert!(!rendered.contains("second") && !rendered.contains("third"));
    }

    #[test]
    fn inspect_reports_a_missing_finding() {
        let response = suggestions(vec![inspect_finding(42, 2, FindingKind::Memory, "other")]);
        assert_eq!(
            render_inspect(&response, reference(42, 1, FindingKind::Memory), NOW),
            Err("finding not found: 42-1-memory; it may have expired".to_owned())
        );
    }

    #[test]
    fn inspect_renders_missing_measurements() {
        let found = inspect_finding(42, 1, FindingKind::Cpu, "");
        let rendered = render_inspect(
            &suggestions(vec![found]),
            reference(42, 1, FindingKind::Cpu),
            NOW,
        )
        .unwrap();
        assert!(rendered.contains("\n  samples: no retained samples\n"));
        assert!(rendered.contains("\n  CPU: not measured\n"));
        assert!(rendered.contains("\n  Memory: not measured\n"));
        assert!(rendered.contains("\nExplanation:\n  (none)\n"));
        assert!(rendered.ends_with("permission to terminate."));
    }

    #[test]
    fn inspect_bounds_and_sanitizes_the_explanation() {
        let explanation = format!("\u{1b}[2J\né{}", "x".repeat(200));
        let found = inspect_finding(42, 1, FindingKind::Cpu, &explanation);
        let rendered = render_inspect(
            &suggestions(vec![found]),
            reference(42, 1, FindingKind::Cpu),
            NOW,
        )
        .unwrap();
        assert!(
            rendered
                .chars()
                .all(|c| c == '\n' || matches!(c, '\u{20}'..='\u{7e}'))
        );
        let line = rendered
            .lines()
            .find(|line| line.starts_with("  ?[2J"))
            .unwrap();
        assert!(line.ends_with("..."));
        assert_eq!(line.chars().count(), 2 + 160);
    }

    #[test]
    fn inspect_keeps_the_closing_advice_of_a_real_explanation() {
        // Same shape as the detector's memory explanation (detector.rs format string).
        let explanation = "pid 42 held 256 MiB memory while using most 0.8% CPU across 6 samples over 120 s; review process";
        let found = inspect_finding(42, 1, FindingKind::Memory, explanation);
        let rendered = render_inspect(
            &suggestions(vec![found]),
            reference(42, 1, FindingKind::Memory),
            NOW,
        )
        .unwrap();
        assert!(
            rendered.contains(&format!("\n  {explanation}\n")),
            "{rendered}"
        );
    }

    #[test]
    fn inspect_reports_daemon_errors_and_wrong_response_kind() {
        let unavailable = Response::error(ProtocolError {
            code: ErrorCode::Unavailable,
            message:
                "suggestions are not available until enough process history has been collected"
                    .to_owned(),
        });
        assert_eq!(
            render_inspect(&unavailable, reference(42, 1, FindingKind::Cpu), NOW),
            Err("daemon error (unavailable): suggestions are not available until enough process history has been collected".to_owned())
        );
        assert_eq!(
            render_inspect(&status(None, None), reference(42, 1, FindingKind::Cpu), NOW),
            Err("invalid response from daemon: expected suggestions response".to_owned())
        );
    }

    #[test]
    fn render_suggestions_lists_findings_with_reference_and_name() {
        let response = suggestions(vec![finding(
            42,
            FindingKind::Memory,
            Confidence::Medium,
            "pid 42 held 256 MiB",
        )]);
        let names = processes(vec![process(42, "worker", 1, 0.0)]);
        let rendered = render_suggestions(&response, Some(&names), NOW).unwrap();
        assert_eq!(
            rendered,
            format!("42-1-memory  medium  worker\n  pid 42 held 256 MiB\n{FOOTER}")
        );
    }

    #[test]
    fn render_suggestions_orders_by_shared_sort_key() {
        let response = suggestions(vec![
            finding(43, FindingKind::Memory, Confidence::Low, "later"),
            finding(42, FindingKind::Cpu, Confidence::Low, "earlier"),
        ]);
        let rendered = render_suggestions(&response, None, NOW).unwrap();
        let headers = rendered
            .lines()
            .filter(|line| !line.starts_with("  "))
            .collect::<Vec<_>>();
        assert!(headers[0].starts_with("42-1-cpu"));
        assert!(headers[1].starts_with("43-1-memory"));
    }

    #[test]
    fn render_suggestions_reports_an_empty_list() {
        assert_eq!(
            render_suggestions(&suggestions(Vec::new()), None, NOW).unwrap(),
            format!("(no current suggestions)\n{FOOTER}")
        );
    }

    #[test]
    fn render_suggestions_reports_still_collecting() {
        let response = Response::error(ProtocolError {
            code: ErrorCode::Unavailable,
            message:
                "suggestions are not available until enough process history has been collected"
                    .to_owned(),
        });
        assert_eq!(
            render_suggestions(&response, None, NOW).unwrap(),
            "still collecting evidence: suggestions are not available until enough process history has been collected"
        );
    }

    #[test]
    fn render_suggestions_reports_other_daemon_errors() {
        let response = Response::error(ProtocolError {
            code: ErrorCode::MalformedRequest,
            message: "bad".to_owned(),
        });
        assert_eq!(
            render_suggestions(&response, None, NOW).unwrap_err(),
            "daemon error (malformed_request): bad"
        );
    }

    #[test]
    fn render_suggestions_rejects_a_foreign_response() {
        assert_eq!(
            render_suggestions(&status(Some(1), Some(1)), None, NOW).unwrap_err(),
            "invalid response from daemon: expected suggestions response"
        );
    }

    #[test]
    fn render_suggestions_tolerates_a_missing_process_reply() {
        let response = suggestions(vec![finding(42, FindingKind::Cpu, Confidence::High, "x")]);
        let foreign = status(Some(1), Some(1));
        for names in [None, Some(&foreign)] {
            let rendered = render_suggestions(&response, names, NOW).unwrap();
            assert!(rendered.starts_with("42-1-cpu  high  <unknown>\n"));
        }
    }

    #[test]
    fn render_suggestions_sanitizes_names_and_explanations() {
        let response = suggestions(vec![finding(
            42,
            FindingKind::Cpu,
            Confidence::Low,
            &format!("a\nb{}", "y".repeat(200)),
        )]);
        let names = processes(vec![process(
            42,
            &format!("a\u{1b}{}", "x".repeat(40)),
            1,
            0.0,
        )]);
        let rendered = render_suggestions(&response, Some(&names), NOW).unwrap();
        let lines = rendered.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 3, "explanation must stay on one line");
        assert_eq!(
            lines[0],
            format!("42-1-cpu  low  a\u{fffd}{}", "x".repeat(22))
        );
        assert_eq!(lines[1].chars().count(), 2 + 160);
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn render_suggestions_keeps_closing_advice_of_real_explanation() {
        // Same shape as the detector's memory explanation (detector.rs format string).
        let explanation = "pid 4294967295 held 9999 MiB of memory while using at most 100.0% CPU across 9999 samples over 9999 s; review this process";
        let response = suggestions(vec![finding(
            42,
            FindingKind::Memory,
            Confidence::Medium,
            explanation,
        )]);
        let rendered = render_suggestions(&response, None, NOW).unwrap();
        assert_eq!(
            rendered.lines().nth(1),
            Some(format!("  {explanation}").as_str())
        );
    }

    #[test]
    fn render_suggestions_keeps_stale_findings_and_reports_their_age() {
        let response = Response::Suggestions {
            version: PROTOCOL_VERSION,
            evaluated_at_ms: NOW - 600_000,
            processes_updated_at_ms: NOW - 600_000,
            findings: vec![finding(
                42,
                FindingKind::Memory,
                Confidence::Medium,
                "pid 42 held 256 MiB",
            )],
        };
        let names = processes(vec![process(42, "worker", 1, 0.0)]);
        let rendered = render_suggestions(&response, Some(&names), NOW).unwrap();
        assert!(rendered.starts_with("42-1-memory  medium  worker\n"));
        assert!(rendered.ends_with("findings evaluated 600.0s ago; process data: 600.0 s old"));
    }

    #[test]
    fn render_suggestions_reports_data_age_and_future_timestamps() {
        let response = Response::Suggestions {
            version: PROTOCOL_VERSION,
            evaluated_at_ms: NOW + 10,
            processes_updated_at_ms: 1_700_000_000_023,
            findings: Vec::new(),
        };
        assert!(
            render_suggestions(&response, None, NOW)
                .unwrap()
                .ends_with("findings evaluated 0.0s ago; process data: 0.5 s old")
        );
    }

    #[test]
    fn render_suggestions_never_mentions_actions() {
        let response = suggestions(vec![finding(
            42,
            FindingKind::Memory,
            Confidence::High,
            "held memory",
        )]);
        let rendered = render_suggestions(&response, None, NOW).unwrap();
        for word in ["safe", "kill", "terminate", "signal", "stale", "orphan"] {
            assert!(!rendered.contains(word), "{word} in {rendered}");
        }
    }

    #[test]
    fn close_preview_labels_the_allowed_block_and_asks_for_confirmation() {
        let response = Response::ClosePreview {
            version: PROTOCOL_VERSION,
            assessment: close_assessment(PreflightConclusion::AllowedForConfirmation, vec![]),
            process: Some(process(42, "worker", 0, 0.0)),
        };
        assert_eq!(
            render_close_preview(&response).unwrap(),
            "Action preview\nprocess: 42-1-cpu (worker)\nowner uid: 501\naction: close\nverdict: allowed for confirmation\nobservation age: 250 ms\nnothing was executed; no signal was sent\na separate explicit confirmation is required before any action"
        );
    }

    #[test]
    fn close_preview_of_a_stale_assessment_states_the_reason_without_confirmation() {
        let response = Response::ClosePreview {
            version: PROTOCOL_VERSION,
            assessment: close_assessment(
                PreflightConclusion::NeedsFreshEvidence,
                vec![PreflightReason::StaleObservation],
            ),
            process: None,
        };
        assert_eq!(
            render_close_preview(&response).unwrap(),
            "Action preview\nprocess: 42-1-cpu (<unknown>)\nowner uid: 501\naction: close\nverdict: needs fresh evidence\nreason: observation is older than the allowed age\nobservation age: 250 ms\nnothing was executed; no signal was sent"
        );
    }

    #[test]
    fn close_sigterm_request_is_exit_zero_and_not_a_confirmed_exit() {
        let (text, code) = render_close_outcome(
            &close_result(ActionOutcome::SigtermRequested),
            close_finding(),
        )
        .unwrap();
        assert_eq!(code, 0);
        assert_eq!(
            text,
            "signal requested: SIGTERM\nprocess: 42-1-cpu\nThe daemon accepted the request. This is not confirmation that the process exited.\nRun `syswatch inspect 42-1-cpu` to check again."
        );
    }

    #[test]
    fn close_already_exited_and_denied_and_failed_are_exit_one() {
        let (text, code) =
            render_close_outcome(&close_result(ActionOutcome::AlreadyExited), close_finding())
                .unwrap();
        assert_eq!(code, 1);
        assert_eq!(
            text,
            "no signal sent: the process already exited\nNo signal was requested."
        );

        let denied = close_result(ActionOutcome::Denied {
            reasons: vec![ActionDenial::PreflightTooOld, ActionDenial::OwnerMismatch],
        });
        let (text, code) = render_close_outcome(&denied, close_finding()).unwrap();
        assert_eq!(code, 1);
        assert_eq!(
            text,
            "action denied: no signal was sent\nreason: preflight evidence is too old\nreason: process is not owned by the current user"
        );

        let failed = close_result(ActionOutcome::Failed {
            failure: ActionFailure::PermissionDenied,
        });
        let (text, code) = render_close_outcome(&failed, close_finding()).unwrap();
        assert_eq!(code, 1);
        assert_eq!(
            text,
            "action failed: no signal was confirmed\nreason: permission denied"
        );
    }

    #[test]
    fn close_daemon_errors_are_reported_not_rendered() {
        let error = Response::error(ProtocolError {
            code: ErrorCode::NotFound,
            message: "finding 42-1-cpu is not among the current advisory findings".into(),
        });
        assert_eq!(
            render_close_outcome(&error, close_finding()).unwrap_err(),
            "daemon error (not_found): finding 42-1-cpu is not among the current advisory findings"
        );
        assert_eq!(
            render_close_preview(&error).unwrap_err(),
            "daemon error (not_found): finding 42-1-cpu is not among the current advisory findings"
        );
    }
}
