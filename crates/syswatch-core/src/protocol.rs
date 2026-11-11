//! Versioned, transport-agnostic monitoring request and response types.
//!
//! Messages use one UTF-8 JSON value per line. See the repository README for
//! the wire examples and framing contract.

use serde::{Deserialize, Serialize};

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
}
