//! Versioned, transport-agnostic monitoring request and response types.
//!
//! Messages use one UTF-8 JSON value per line. See the repository README for
//! the wire examples and framing contract.

use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};

use crate::action::ActionOutcome;
use crate::findings::{self, AdvisoryFinding, FindingId, SuggestionsSnapshot};
use crate::preflight::PreflightAssessment;
use crate::snapshot::{AggregateSnapshot, ProcessSnapshot, SystemSnapshot};

/// Protocol version supported by this module.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum request frame size in bytes.
pub const MAX_REQUEST_BYTES: usize = 4096;

/// Maximum number of findings in a suggestions response.
pub const MAX_SUGGESTIONS: usize = 64;

const MALFORMED_MESSAGE: &str =
    "request is not a JSON object with integer version and string command";

/// One client request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Protocol version spoken by the client.
    pub version: u32,
    /// Requested operation.
    pub command: RequestCommand,
    /// Complete finding identity. Required by the close commands and rejected by
    /// every other command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding: Option<FindingId>,
}

/// Supported operations. The monitoring commands are read-only; the close
/// commands are the only action requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestCommand {
    /// Request aggregate system status.
    Status,
    /// Request the latest process list.
    Processes,
    /// Request the latest advisory findings.
    Suggestions,
    /// Preview a fresh preflight for one finding identity. Sends no signal.
    #[serde(rename = "close_preview")]
    ClosePreview,
    /// Ask the daemon to request one SIGTERM for one finding identity, after
    /// revalidating at delivery.
    #[serde(rename = "close_confirm")]
    CloseConfirm,
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
    /// The finding identity is not among the current advisory findings.
    NotFound,
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
    /// Latest evaluated advisory findings.
    Suggestions {
        /// Protocol version used by this response.
        version: u32,
        /// Evaluation time in milliseconds since the Unix epoch.
        evaluated_at_ms: u64,
        /// Process scan time in milliseconds since the Unix epoch.
        processes_updated_at_ms: u64,
        /// Findings sorted by identity and observation time.
        findings: Vec<AdvisoryFinding>,
    },
    /// Fresh preflight for one finding identity, with its matching process row.
    ClosePreview {
        /// Protocol version used by this response.
        version: u32,
        /// Typed preflight result computed by the daemon.
        assessment: PreflightAssessment,
        /// Process row with the requested identity, when it is in the scan.
        process: Option<ProcessSnapshot>,
    },
    /// Result of a close confirmation. A requested signal is not an exit.
    CloseResult {
        /// Protocol version used by this response.
        version: u32,
        /// Typed action outcome.
        outcome: ActionOutcome,
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

    /// Build an advisory response, or an unavailable error before evaluation.
    pub fn suggestions(snapshot: Option<&SuggestionsSnapshot>) -> Self {
        let Some(snapshot) = snapshot else {
            return Self::error(ProtocolError {
                code: ErrorCode::Unavailable,
                message:
                    "suggestions are not available until enough process history has been collected"
                        .into(),
            });
        };
        let mut findings = snapshot.findings.clone();
        findings::sort(&mut findings);
        findings.truncate(MAX_SUGGESTIONS);
        Self::Suggestions {
            version: PROTOCOL_VERSION,
            evaluated_at_ms: snapshot.evaluated_at_ms,
            processes_updated_at_ms: snapshot.processes_updated_at_ms,
            findings,
        }
    }

    /// Build a close preview response.
    pub fn close_preview(
        assessment: PreflightAssessment,
        process: Option<ProcessSnapshot>,
    ) -> Self {
        Self::ClosePreview {
            version: PROTOCOL_VERSION,
            assessment,
            process,
        }
    }

    /// Build a close result response.
    pub fn close_result(outcome: ActionOutcome) -> Self {
        Self::CloseResult {
            version: PROTOCOL_VERSION,
            outcome,
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
///
/// Transport readers must cap buffered input at `MAX_REQUEST_BYTES + 1` bytes
/// and return `malformed_request` if no LF appears within that bound.
pub fn decode_request(line: &str) -> Result<Request, ProtocolError> {
    if line.is_empty() || line.len() > MAX_REQUEST_BYTES || line.contains(['\n', '\r']) {
        return Err(malformed_request());
    }

    #[derive(Deserialize)]
    struct Head {
        version: u32,
    }

    let head: Head = serde_json::from_str(line).map_err(|_| malformed_request())?;

    if head.version != PROTOCOL_VERSION {
        return Err(ProtocolError {
            code: ErrorCode::UnsupportedVersion,
            message: format!(
                "unsupported protocol version {}; this server speaks {}",
                head.version, PROTOCOL_VERSION
            ),
        });
    }

    #[derive(Deserialize)]
    struct RawRequest {
        version: u32,
        command: String,
        #[serde(default)]
        finding: Option<FindingId>,
    }

    let raw: RawRequest = serde_json::from_str(line).map_err(|_| malformed_request())?;
    let deserializer: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
        raw.command.as_str().into_deserializer();
    let command = match RequestCommand::deserialize(deserializer) {
        Ok(command) => command,
        Err(_) => {
            let command: String = raw.command.chars().take(64).collect();
            return Err(ProtocolError {
                code: ErrorCode::UnsupportedCommand,
                message: format!("unsupported command \"{command}\""),
            });
        }
    };

    let needs_finding = matches!(
        command,
        RequestCommand::ClosePreview | RequestCommand::CloseConfirm
    );
    match (needs_finding, raw.finding.is_some()) {
        (true, false) => Err(ProtocolError {
            code: ErrorCode::MalformedRequest,
            message: "close requests require a finding object".into(),
        }),
        (false, true) => Err(ProtocolError {
            code: ErrorCode::MalformedRequest,
            message: "only close requests accept a finding object".into(),
        }),
        _ => Ok(Request {
            version: raw.version,
            command,
            finding: raw.finding,
        }),
    }
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
                finding: None,
            };
            assert_eq!(serde_json::to_string(&request).unwrap(), json);
            assert_eq!(serde_json::from_str::<Request>(json).unwrap(), request);
            assert_eq!(decode_request(json).unwrap(), request);
        }
    }

    fn finding(pid: u32) -> crate::AdvisoryFinding {
        use crate::{
            AdvisoryFinding, Confidence, CpuActivity, FindingEvidence, FindingId, FindingKind,
            MemoryActivity,
        };
        AdvisoryFinding {
            id: FindingId {
                pid,
                start_time_secs: 5,
                kind: FindingKind::Memory,
            },
            confidence: Confidence::Medium,
            observed_at_ms: 6,
            explanation: "Observed idle memory use".into(),
            evidence: FindingEvidence {
                sample_count: 2,
                span_ms: 4,
                cpu: Some(CpuActivity {
                    max_percent: 1.0,
                    mean_percent: 0.5,
                    valid_samples: 2,
                }),
                memory: Some(MemoryActivity {
                    latest_bytes: 100,
                    max_bytes: 200,
                }),
            },
        }
    }

    fn suggestions(findings: Vec<crate::AdvisoryFinding>) -> crate::SuggestionsSnapshot {
        crate::SuggestionsSnapshot {
            evaluated_at_ms: 7,
            processes_updated_at_ms: 6,
            findings,
        }
    }

    #[test]
    fn suggestions_request_round_trips() {
        let json = r#"{"version":1,"command":"suggestions"}"#;
        let request = decode_request(json).unwrap();
        assert_eq!(request.command, RequestCommand::Suggestions);
        assert_eq!(encode_line(&request).unwrap(), json);
    }

    #[test]
    fn suggestions_wire_shape_is_stable() {
        let response = Response::suggestions(Some(&suggestions(vec![finding(42)])));
        let line = encode_line(&response).unwrap();
        assert_eq!(
            line,
            r#"{"kind":"suggestions","version":1,"evaluated_at_ms":7,"processes_updated_at_ms":6,"findings":[{"id":{"pid":42,"start_time_secs":5,"kind":"memory"},"confidence":"medium","observed_at_ms":6,"explanation":"Observed idle memory use","evidence":{"sample_count":2,"span_ms":4,"cpu":{"max_percent":1.0,"mean_percent":0.5,"valid_samples":2},"memory":{"latest_bytes":100,"max_bytes":200}}}]}"#
        );
        assert_eq!(serde_json::from_str::<Response>(&line).unwrap(), response);
    }

    #[test]
    fn empty_findings_list_is_not_unavailable() {
        assert_eq!(
            encode_line(&Response::suggestions(Some(&suggestions(vec![])))).unwrap(),
            r#"{"kind":"suggestions","version":1,"evaluated_at_ms":7,"processes_updated_at_ms":6,"findings":[]}"#
        );
    }

    #[test]
    fn never_evaluated_suggestions_reports_unavailable() {
        assert_eq!(
            encode_line(&Response::suggestions(None)).unwrap(),
            r#"{"kind":"error","version":1,"code":"unavailable","message":"suggestions are not available until enough process history has been collected"}"#
        );
    }

    #[test]
    fn suggestions_reply_is_bounded() {
        let input = suggestions((1..=65).rev().map(finding).collect());
        let Response::Suggestions { findings, .. } = Response::suggestions(Some(&input)) else {
            panic!("expected suggestions")
        };
        assert_eq!(findings.len(), 64);
        assert_eq!(findings.first().unwrap().id.pid, 1);
        assert_eq!(findings.last().unwrap().id.pid, 64);
        assert!(findings.windows(2).all(
            |pair| (pair[0].id, pair[0].observed_at_ms) <= (pair[1].id, pair[1].observed_at_ms)
        ));
    }

    #[test]
    fn suggestions_are_additive_within_protocol_version_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
        assert_eq!(
            decode_request(r#"{"version":1,"command":"suggestions"}"#)
                .unwrap()
                .command,
            RequestCommand::Suggestions
        );
        let error = decode_request(r#"{"version":2,"command":"suggestions"}"#).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedVersion);
        assert!(error.message.contains('2') && error.message.contains('1'));
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
    fn unsupported_version_precedes_future_command_shape() {
        let error = decode_request(r#"{"version":2,"command":{"name":"status"}}"#).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedVersion);
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
        for response in [
            Response::status(&snapshot),
            Response::processes(&snapshot),
            Response::suggestions(Some(&suggestions(vec![finding(42)]))),
        ] {
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

    fn close_finding() -> crate::FindingId {
        crate::FindingId {
            pid: 42,
            start_time_secs: 1_700_000_000,
            kind: crate::FindingKind::Cpu,
        }
    }

    fn close_assessment() -> crate::preflight::PreflightAssessment {
        use crate::preflight::{PreflightAssessment, PreflightConclusion, RequestedAction};
        PreflightAssessment {
            finding: close_finding(),
            action: RequestedAction::Close,
            conclusion: PreflightConclusion::Denied,
            reasons: vec![crate::preflight::PreflightReason::Ineligible(
                crate::eligibility::EligibilityReason::MonitorProcess,
            )],
            owner_uid: Some(501),
            observation_age_ms: Some(120),
        }
    }

    #[test]
    fn close_requests_round_trip_with_pinned_wire_strings() {
        let finding = r#""finding":{"pid":42,"start_time_secs":1700000000,"kind":"cpu"}"#;
        for (command, name) in [
            (RequestCommand::ClosePreview, "close_preview"),
            (RequestCommand::CloseConfirm, "close_confirm"),
        ] {
            let json = format!(r#"{{"version":1,"command":"{name}",{finding}}}"#);
            let request = Request {
                version: PROTOCOL_VERSION,
                command,
                finding: Some(close_finding()),
            };
            assert_eq!(encode_line(&request).unwrap(), json);
            assert_eq!(decode_request(&json).unwrap(), request);
        }
    }

    #[test]
    fn close_request_without_a_finding_is_malformed() {
        for name in ["close_preview", "close_confirm"] {
            for json in [
                format!(r#"{{"version":1,"command":"{name}"}}"#),
                format!(r#"{{"version":1,"command":"{name}","finding":null}}"#),
            ] {
                let error = decode_request(&json).unwrap_err();
                assert_eq!(error.code, ErrorCode::MalformedRequest, "{json}");
                assert!(error.message.contains("finding"), "{json}");
            }
        }
    }

    #[test]
    fn monitoring_request_with_a_finding_is_malformed() {
        let finding = r#""finding":{"pid":42,"start_time_secs":1700000000,"kind":"cpu"}"#;
        let json = format!(r#"{{"version":1,"command":"status",{finding}}}"#);
        assert_eq!(
            decode_request(&json).unwrap_err().code,
            ErrorCode::MalformedRequest
        );
    }

    #[test]
    fn close_commands_keep_version_and_unknown_command_rules() {
        let finding = r#""finding":{"pid":42,"start_time_secs":1700000000,"kind":"cpu"}"#;
        let version_two = format!(r#"{{"version":2,"command":"close_confirm",{finding}}}"#);
        assert_eq!(
            decode_request(&version_two).unwrap_err().code,
            ErrorCode::UnsupportedVersion
        );
        let unknown = format!(r#"{{"version":1,"command":"close_all",{finding}}}"#);
        assert_eq!(
            decode_request(&unknown).unwrap_err().code,
            ErrorCode::UnsupportedCommand
        );
    }

    #[test]
    fn close_responses_use_pinned_kinds_and_round_trip() {
        use crate::action::{ActionDenial, ActionOutcome};
        use crate::eligibility::EligibilityReason;
        use crate::preflight::PreflightReason;

        let preview = Response::close_preview(close_assessment(), None);
        let json = encode_line(&preview).unwrap();
        assert!(json.starts_with(r#"{"kind":"close_preview","version":1,"assessment":{"#));
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), preview);

        let result = Response::close_result(ActionOutcome::SigtermRequested);
        let json = encode_line(&result).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"close_result","version":1,"outcome":"sigterm_requested"}"#
        );
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), result);

        let denied = Response::close_result(ActionOutcome::Denied {
            reasons: vec![ActionDenial::Preflight(PreflightReason::Ineligible(
                EligibilityReason::MonitorProcess,
            ))],
        });
        let json = encode_line(&denied).unwrap();
        assert!(json.contains(
            r#""outcome":{"denied":{"reasons":[{"preflight":{"ineligible":"monitor_process"}}]}}"#
        ));
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), denied);
    }

    #[test]
    fn not_found_has_a_stable_wire_string() {
        let json = encode_line(&ErrorCode::NotFound).unwrap();
        assert_eq!(json, r#""not_found""#);
    }
}
