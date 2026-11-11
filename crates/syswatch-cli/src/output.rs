use syswatch_core::{
    protocol::{ErrorCode, Response},
    report::{format_bytes, format_cpu_percent},
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
    use super::{format_age_ms, render_status};
    use syswatch_core::{
        MemorySnapshot,
        protocol::{PROTOCOL_VERSION, Response},
        snapshot::AggregateSnapshot,
    };

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
}
