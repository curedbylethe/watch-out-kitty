use syswatch_core::{
    findings::{self, Confidence, FindingKind},
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
                    "{}-{}-{}  {}  {name}",
                    id.pid,
                    id.start_time_secs,
                    kind_name(id.kind),
                    confidence_name(finding.confidence),
                ));
                lines.push(format!(
                    "  {}",
                    report::sanitize_field(&finding.explanation, 100)
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

fn kind_name(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cpu => "cpu",
        FindingKind::Memory => "memory",
        _ => "unknown",
    }
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

fn error_code(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::UnsupportedVersion => "unsupported_version",
        ErrorCode::UnsupportedCommand => "unsupported_command",
        ErrorCode::MalformedRequest => "malformed_request",
        ErrorCode::Unavailable => "unavailable",
    }
}

pub fn format_age_ms(now_ms: u64, then_ms: u64) -> String {
    format!("{:.1}s", now_ms.saturating_sub(then_ms) as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::{format_age_ms, render_status, render_suggestions, render_top};
    use syswatch_core::{
        MemorySnapshot,
        findings::{AdvisoryFinding, Confidence, FindingEvidence, FindingId, FindingKind},
        protocol::{ErrorCode, PROTOCOL_VERSION, ProtocolError, Response},
        report::ProcessSort,
        snapshot::{AggregateSnapshot, ProcessSnapshot},
    };

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
        assert_eq!(lines[1].chars().count(), 2 + 100);
        assert!(!rendered.contains('\u{1b}'));
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
}
