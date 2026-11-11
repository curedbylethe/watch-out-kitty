//! Versioned, transport-agnostic monitoring request and response types.
//!
//! Messages use one UTF-8 JSON value per line. See the repository README for
//! the wire examples and framing contract.

use serde::{Deserialize, Serialize};

use crate::snapshot::{AggregateSnapshot, ProcessSnapshot, SystemSnapshot};

/// Protocol version supported by this module.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum request frame size in bytes.
pub const MAX_REQUEST_BYTES: usize = 4096;

const MALFORMED_MESSAGE: &str =
    "request is not a JSON object with integer version and string command";

/// One client request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Protocol version spoken by the client.
    pub version: u32,
    /// Requested read-only operation.
    pub command: RequestCommand,
}

/// Supported read-only operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestCommand {
    /// Request aggregate system status.
    Status,
    /// Request the latest process list.
    Processes,
}

/// Stable machine-readable protocol error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The client requested a protocol version this server does not support.
    UnsupportedVersion,
    /// The request named an unsupported command.
    UnsupportedCommand,
    /// The request frame or JSON value is malformed.
    MalformedRequest,
    /// Requested data has not been collected yet.
    Unavailable,
}

/// Structured protocol error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    /// Stable machine-readable error category.
    pub code: ErrorCode,
    /// Human-readable explanation.
    pub message: String,
}

/// A protocol response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    /// Aggregate system measurements and process freshness.
    Status {
        /// Protocol version used by this response.
        version: u32,
        /// Aggregate CPU and memory measurements.
        aggregate: AggregateSnapshot,
        /// Number of processes when process data has been sampled.
        process_count: Option<usize>,
        /// Time when process observations were last collected.
        processes_updated_at_ms: Option<u64>,
    },
    /// Latest process observations.
    Processes {
        /// Protocol version used by this response.
        version: u32,
        /// Time of aggregate observation in milliseconds since Unix epoch.
        observed_at_ms: u64,
        /// Time when process observations were last collected.
        processes_updated_at_ms: u64,
        /// Processes in collector order.
        processes: Vec<ProcessSnapshot>,
    },
    /// Error response for a rejected request.
    Error {
        /// Protocol version used by this response.
        version: u32,
        /// Stable machine-readable error category.
        code: ErrorCode,
        /// Human-readable explanation.
        message: String,
    },
}

impl Response {
    /// Build a status response from a system snapshot.
    pub fn status(snapshot: &SystemSnapshot) -> Self {
        Self::Status {
            version: PROTOCOL_VERSION,
            aggregate: AggregateSnapshot {
                timestamp_ms: snapshot.timestamp_ms,
                cpu_usage_percent: snapshot.cpu_usage_percent,
                cpu_updated_at_ms: snapshot.cpu_updated_at_ms,
                memory: snapshot.memory.clone(),
            },
            process_count: snapshot
                .processes_updated_at_ms
                .map(|_| snapshot.processes.len()),
            processes_updated_at_ms: snapshot.processes_updated_at_ms,
        }
    }

    /// Build a process-list response, or an unavailable error before the first scan.
    pub fn processes(snapshot: &SystemSnapshot) -> Self {
        let Some(processes_updated_at_ms) = snapshot.processes_updated_at_ms else {
            return Self::error(ProtocolError {
                code: ErrorCode::Unavailable,
                message: "process data has not been collected yet".into(),
            });
        };

        Self::Processes {
            version: PROTOCOL_VERSION,
            observed_at_ms: snapshot.timestamp_ms,
            processes_updated_at_ms,
            processes: snapshot.processes.clone(),
        }
    }

    /// Build an error response from a protocol error.
    pub fn error(error: ProtocolError) -> Self {
        Self::Error {
            version: PROTOCOL_VERSION,
            code: error.code,
            message: error.message,
        }
    }
}

/// Serialize one protocol value without adding its line terminator.
pub fn encode_line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

/// Parse and validate one request frame.
pub fn decode_request(line: &str) -> Result<Request, ProtocolError> {
    if line.is_empty() || line.len() > MAX_REQUEST_BYTES || line.contains(['\n', '\r']) {
        return Err(malformed_request());
    }

    #[derive(Deserialize)]
    struct RawRequest {
        version: u32,
        command: String,
    }

    let raw: RawRequest = serde_json::from_str(line).map_err(|_| malformed_request())?;

    if raw.version != PROTOCOL_VERSION {
        return Err(ProtocolError {
            code: ErrorCode::UnsupportedVersion,
            message: format!(
                "unsupported protocol version {}; this server speaks {}",
                raw.version, PROTOCOL_VERSION
            ),
        });
    }

    let command = match raw.command.as_str() {
        "status" => RequestCommand::Status,
        "processes" => RequestCommand::Processes,
        _ => {
            let command: String = raw.command.chars().take(64).collect();
            return Err(ProtocolError {
                code: ErrorCode::UnsupportedCommand,
                message: format!("unsupported command \"{command}\""),
            });
        }
    };

    Ok(Request {
        version: raw.version,
        command,
    })
}

fn malformed_request() -> ProtocolError {
    ProtocolError {
        code: ErrorCode::MalformedRequest,
        message: MALFORMED_MESSAGE.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ErrorCode, PROTOCOL_VERSION, ProtocolError, Request, RequestCommand, Response,
        decode_request, encode_line,
    };
    use crate::snapshot::{MemorySnapshot, ProcessSnapshot, SystemSnapshot};

    fn snapshot(
        cpu_updated_at_ms: Option<u64>,
        processes_updated_at_ms: Option<u64>,
        processes: Vec<ProcessSnapshot>,
    ) -> SystemSnapshot {
        SystemSnapshot {
            timestamp_ms: 1_700_000_000_123,
            cpu_usage_percent: if cpu_updated_at_ms.is_some() {
                42.5
            } else {
                0.0
            },
            cpu_updated_at_ms,
            memory: MemorySnapshot {
                total_bytes: 17_179_869_184,
                used_bytes: 8_589_934_592,
            },
            processes,
            processes_updated_at_ms,
        }
    }

    fn process(name: &str, pid: u32) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: Some(1),
            name: name.into(),
            cpu_usage_percent: 12.5,
            memory_bytes: 4096,
            start_time_secs: 1_700_000_000,
        }
    }

    #[test]
    fn request_round_trips() {
        for (command, json) in [
            (
                RequestCommand::Status,
                r#"{"version":1,"command":"status"}"#,
            ),
            (
                RequestCommand::Processes,
                r#"{"version":1,"command":"processes"}"#,
            ),
        ] {
            let request = Request {
                version: PROTOCOL_VERSION,
                command,
            };
            assert_eq!(serde_json::to_string(&request).unwrap(), json);
            assert_eq!(serde_json::from_str::<Request>(json).unwrap(), request);
            assert_eq!(decode_request(json).unwrap(), request);
        }
    }

    #[test]
    fn unknown_command_is_rejected() {
        for command in ["kill", "terminate", "restart", "snapshot"] {
            let error =
                decode_request(&format!(r#"{{"version":1,"command":"{command}"}}"#)).unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedCommand);
        }
    }

    #[test]
    fn unsupported_version_names_both_versions() {
        for version in [0, 2] {
            let error = decode_request(&format!(r#"{{"version":{version},"command":"status"}}"#))
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UnsupportedVersion);
            assert!(error.message.contains(&version.to_string()));
            assert!(error.message.contains(&PROTOCOL_VERSION.to_string()));
        }
    }

    #[test]
    fn malformed_input_never_panics() {
        for line in [
            "",
            "{",
            "[]",
            "null",
            r#"{"version":1}"#,
            r#"{"command":"status"}"#,
            "not json",
            "{\"version\":1,\"command\":\"status\"}\n{}",
            "{\"version\":1,\"command\":\"status\"}\r{}",
        ] {
            assert_eq!(
                decode_request(line).unwrap_err().code,
                ErrorCode::MalformedRequest
            );
        }
        assert_eq!(
            decode_request(&"x".repeat(4097)).unwrap_err().code,
            ErrorCode::MalformedRequest
        );
    }

    #[test]
    fn unknown_command_message_is_bounded() {
        let error = decode_request(&format!(
            r#"{{"version":1,"command":"{}"}}"#,
            "x".repeat(3000)
        ))
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedCommand);
        assert!(error.message.len() < 200);

        let command = format!("{}{}", "x".repeat(63), "é".repeat(100));
        let error =
            decode_request(&format!(r#"{{"version":1,"command":"{command}"}}"#)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedCommand);
    }

    #[test]
    fn encoded_line_has_no_newline() {
        let response = Response::error(ProtocolError {
            code: ErrorCode::MalformedRequest,
            message: "a\nb".into(),
        });
        let line = encode_line(&response).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), response);
    }

    #[test]
    fn error_response_wire_shape_is_stable() {
        let cases = [
            (
                ProtocolError {
                    code: ErrorCode::UnsupportedVersion,
                    message: "unsupported protocol version 2; this server speaks 1".into(),
                },
                r#"{"kind":"error","version":1,"code":"unsupported_version","message":"unsupported protocol version 2; this server speaks 1"}"#,
            ),
            (
                ProtocolError {
                    code: ErrorCode::UnsupportedCommand,
                    message: "unsupported command \"kill\"".into(),
                },
                r#"{"kind":"error","version":1,"code":"unsupported_command","message":"unsupported command \"kill\""}"#,
            ),
            (
                ProtocolError {
                    code: ErrorCode::MalformedRequest,
                    message: "request is not a JSON object with integer version and string command"
                        .into(),
                },
                r#"{"kind":"error","version":1,"code":"malformed_request","message":"request is not a JSON object with integer version and string command"}"#,
            ),
            (
                ProtocolError {
                    code: ErrorCode::Unavailable,
                    message: "process data has not been collected yet".into(),
                },
                r#"{"kind":"error","version":1,"code":"unavailable","message":"process data has not been collected yet"}"#,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(encode_line(&Response::error(error)).unwrap(), expected);
        }
    }

    #[test]
    fn status_wire_shape_is_stable() {
        let cases = [
            (
                snapshot(
                    Some(1_700_000_000_123),
                    Some(1_700_000_000_100),
                    vec![process("worker", 42), process("kitty", 43)],
                ),
                r#"{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":42.5,"cpu_updated_at_ms":1700000000123,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":2,"processes_updated_at_ms":1700000000100}"#,
            ),
            (
                snapshot(None, None, vec![]),
                r#"{"kind":"status","version":1,"aggregate":{"timestamp_ms":1700000000123,"cpu_usage_percent":0.0,"cpu_updated_at_ms":null,"memory":{"total_bytes":17179869184,"used_bytes":8589934592}},"process_count":null,"processes_updated_at_ms":null}"#,
            ),
        ];

        for (snapshot, expected) in cases {
            let response = Response::status(&snapshot);
            let line = encode_line(&response).unwrap();
            assert_eq!(line, expected);
            assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), response);
        }
    }

    #[test]
    fn processes_wire_shape_is_stable() {
        let snapshot = snapshot(
            Some(1_700_000_000_123),
            Some(1_700_000_000_100),
            vec![process("worker", 42)],
        );
        let response = Response::processes(&snapshot);
        let expected = r#"{"kind":"processes","version":1,"observed_at_ms":1700000000123,"processes_updated_at_ms":1700000000100,"processes":[{"pid":42,"parent_pid":1,"name":"worker","cpu_usage_percent":12.5,"memory_bytes":4096,"start_time_secs":1700000000}]}"#;
        let line = encode_line(&response).unwrap();
        assert_eq!(line, expected);
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), response);
    }

    #[test]
    fn never_sampled_processes_reports_unavailable() {
        let response = Response::processes(&snapshot(None, None, vec![]));
        assert_eq!(
            encode_line(&response).unwrap(),
            r#"{"kind":"error","version":1,"code":"unavailable","message":"process data has not been collected yet"}"#
        );
    }

    #[test]
    fn empty_scan_is_not_unavailable() {
        let response = Response::processes(&snapshot(None, Some(1_700_000_000_100), vec![]));
        assert_eq!(
            encode_line(&response).unwrap(),
            r#"{"kind":"processes","version":1,"observed_at_ms":1700000000123,"processes_updated_at_ms":1700000000100,"processes":[]}"#
        );
    }

    #[test]
    fn process_count_is_known_only_when_sampled() {
        for (updated_at_ms, processes, expected) in [
            (Some(1), vec![], Some(0)),
            (
                Some(1),
                vec![process("worker", 42), process("kitty", 43)],
                Some(2),
            ),
            (None, vec![process("worker", 42)], None),
        ] {
            let response = Response::status(&snapshot(None, updated_at_ms, processes));
            let line = encode_line(&response).unwrap();
            assert!(line.contains(&format!(
                "\"process_count\":{}",
                expected.map_or("null".into(), |count| count.to_string())
            )));
        }
    }

    #[test]
    fn warm_up_cpu_serializes_as_null() {
        let line = encode_line(&Response::status(&snapshot(None, None, vec![]))).unwrap();
        assert!(line.contains("\"cpu_updated_at_ms\":null"));
        assert!(!line.contains("\"cpu_updated_at_ms\":0"));
    }

    #[test]
    fn response_carries_no_filesystem_path_or_control_verb() {
        let snapshot = snapshot(
            Some(1_700_000_000_123),
            Some(1_700_000_000_100),
            vec![
                process("worker", 42),
                process("kitty", 43),
                process("loginwindow", 44),
            ],
        );
        for response in [Response::status(&snapshot), Response::processes(&snapshot)] {
            let line = encode_line(&response).unwrap();
            for forbidden in [
                "/Users",
                "/home",
                "/private",
                "\"path\"",
                "\"command\"",
                "\"signal\"",
                "\"kill\"",
            ] {
                assert!(!line.contains(forbidden), "found {forbidden} in {line}");
            }
        }
    }

    #[test]
    fn process_name_with_newline_round_trips() {
        let response = Response::processes(&snapshot(
            None,
            Some(1_700_000_000_100),
            vec![process("worker\n\"猫", 42)],
        ));
        let line = encode_line(&response).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), response);
    }
}
