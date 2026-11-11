use syswatch_core::{
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
    use super::{format_age_ms, render_status, render_top};
    use syswatch_core::{
        MemorySnapshot,
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
}
