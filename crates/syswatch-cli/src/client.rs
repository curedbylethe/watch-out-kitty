use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};
use syswatch_core::protocol::{self, Request, RequestCommand, Response};

pub const TIMEOUT: Duration = Duration::from_millis(1000);
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub enum ClientError {
    Connect(String),
    Write(String),
    Read(String),
    Timeout(String),
    ProtocolMismatch { found: u32, expected: u32 },
    InvalidResponse(String),
    Encode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(message)
            | Self::Write(message)
            | Self::Read(message)
            | Self::InvalidResponse(message)
            | Self::Encode(message) => f.write_str(message),
            Self::Timeout(message) => f.write_str(message),
            Self::ProtocolMismatch { found, expected } => write!(
                f,
                "protocol mismatch: daemon speaks version {found}, client speaks version {expected}"
            ),
        }
    }
}

pub fn request_status(path: &Path, timeout: Duration) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(path).map_err(|error| {
        let message = if error.kind() == io::ErrorKind::NotFound {
            format!("cannot connect to daemon at {}: no such file or directory; syswatch-daemon running?", path.display())
        } else {
            format!("cannot connect daemon at {}: {error}", path.display())
        };
        ClientError::Connect(message)
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|error| {
            ClientError::Write(format!("cannot configure daemon connection: {error}"))
        })?;

    let mut request = protocol::encode_line(&Request {
        version: protocol::PROTOCOL_VERSION,
        command: RequestCommand::Status,
    })
    .map_err(|error| ClientError::Encode(format!("cannot encode request: {error}")))?;
    request.push('\n');
    stream
        .write_all(request.as_bytes())
        .map_err(|error| io_error(error, "send", path, timeout))?;

    let deadline = Instant::now() + timeout;
    let read_interval = timeout.min(Duration::from_millis(10));
    if read_interval.is_zero() {
        return Err(timeout_error(timeout, path));
    }
    stream
        .set_read_timeout(Some(read_interval))
        .map_err(|error| ClientError::Read(format!("cannot set daemon read timeout: {error}")))?;
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    while bytes.len() <= MAX_RESPONSE_BYTES {
        if deadline.saturating_duration_since(Instant::now()).is_zero() {
            return Err(timeout_error(timeout, path));
        }
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                let chunk = &chunk[..count];
                if let Some(end) = chunk.iter().position(|byte| *byte == b'\n') {
                    bytes.extend_from_slice(&chunk[..=end]);
                    break;
                }
                bytes.extend_from_slice(chunk);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(error) => return Err(io_error(error, "read", path, timeout)),
        }
    }
    if bytes.is_empty() {
        return Err(ClientError::InvalidResponse(format!(
            "invalid response from daemon at {}: daemon closed the connection without a response",
            path.display()
        )));
    }
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ClientError::InvalidResponse(format!(
            "invalid response from daemon at {}: response exceeds {MAX_RESPONSE_BYTES} bytes",
            path.display()
        )));
    }
    if bytes.last() != Some(&b'\n') {
        return Err(ClientError::InvalidResponse(format!(
            "invalid response from daemon at {}: missing line ending",
            path.display()
        )));
    }
    bytes.pop();
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    let response: Response = serde_json::from_slice(&bytes).map_err(|error| {
        ClientError::InvalidResponse(format!(
            "invalid response from daemon at {}: {error}",
            path.display()
        ))
    })?;
    let version = match &response {
        Response::Status { version, .. }
        | Response::Processes { version, .. }
        | Response::Error { version, .. } => *version,
    };
    if version != protocol::PROTOCOL_VERSION {
        return Err(ClientError::ProtocolMismatch {
            found: version,
            expected: protocol::PROTOCOL_VERSION,
        });
    }
    Ok(response)
}

fn io_error(error: io::Error, action: &str, path: &Path, timeout: Duration) -> ClientError {
    if matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ) {
        timeout_error(timeout, path)
    } else {
        let message = match action {
            "send" => format!(
                "cannot send request to daemon at {}: {error}",
                path.display()
            ),
            _ => format!(
                "cannot read response from daemon at {}: {error}",
                path.display()
            ),
        };
        if action == "send" {
            ClientError::Write(message)
        } else {
            ClientError::Read(message)
        }
    }
}

fn timeout_error(timeout: Duration, path: &Path) -> ClientError {
    ClientError::Timeout(format!(
        "timed out after {} ms waiting for daemon at {}",
        timeout.as_millis(),
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::BufRead,
        os::unix::net::UnixListener,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Socket(std::path::PathBuf);

    impl Socket {
        fn new() -> (Self, UnixListener) {
            let path = std::env::temp_dir().join(format!(
                "syswatch-cli-{}-{}.sock",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let listener = UnixListener::bind(&path).unwrap();
            (Self(path), listener)
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn sends_one_status_request_and_decodes_response() {
        let (_socket, listener) = Socket::new();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            io::BufReader::new(&mut stream)
                .read_line(&mut request)
                .unwrap();
            assert_eq!(request, "{\"version\":1,\"command\":\"status\"}\n");
            stream.write_all(b"{\"kind\":\"error\",\"version\":1,\"code\":\"unavailable\",\"message\":\"warming\"}\n").unwrap();
        });
        let response = request_status(&_socket.0, Duration::from_secs(1)).unwrap();
        assert!(matches!(response, Response::Error { .. }));
        server.join().unwrap();
    }

    #[test]
    fn missing_socket_has_daemon_hint() {
        let path =
            std::env::temp_dir().join(format!("syswatch-cli-missing-{}.sock", std::process::id()));
        let error = request_status(&path, TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("syswatch-daemon running?"));
    }

    #[test]
    fn timeout_is_reported() {
        let (socket, listener) = Socket::new();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(150));
        });
        let error = request_status(&socket.0, Duration::from_millis(30)).unwrap_err();
        assert!(matches!(&error, ClientError::Timeout(_)));
        assert!(error.to_string().contains(&socket.0.display().to_string()));
        server.join().unwrap();
    }

    #[test]
    fn timeout_applies_to_the_whole_response() {
        let (socket, listener) = Socket::new();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            io::BufReader::new(&mut stream)
                .read_line(&mut request)
                .unwrap();
            let response =
                protocol::encode_line(&Response::error(syswatch_core::protocol::ProtocolError {
                    code: syswatch_core::protocol::ErrorCode::Unavailable,
                    message: "later".to_owned(),
                }))
                .unwrap()
                    + "\n";
            for byte in response.bytes() {
                if stream.write_all(&[byte]).is_err() {
                    break;
                }
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(10));
            }
        });
        let error = request_status(&socket.0, Duration::from_millis(60)).unwrap_err();
        assert!(matches!(error, ClientError::Timeout(_)));
        server.join().unwrap();
    }

    #[test]
    fn malformed_and_unterminated_response_are_rejected() {
        for response in [b"not json\n".as_slice(), b"{}"] {
            let (_socket, listener) = Socket::new();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 64];
                let _ = stream.read(&mut request);
                stream.write_all(response).unwrap();
            });
            assert!(matches!(
                request_status(&_socket.0, Duration::from_secs(1)),
                Err(ClientError::InvalidResponse(_))
            ));
            server.join().unwrap();
        }
    }

    #[test]
    fn closed_connection_reports_missing_response() {
        let (socket, listener) = Socket::new();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            io::BufReader::new(&mut stream)
                .read_line(&mut request)
                .unwrap();
        });
        let error = request_status(&socket.0, Duration::from_secs(1)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("closed the connection without a response")
        );
        server.join().unwrap();
    }

    #[test]
    fn rejects_oversized_response() {
        let (_socket, listener) = Socket::new();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 64];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(&vec![b'x'; MAX_RESPONSE_BYTES + 1]);
        });
        assert!(matches!(
            request_status(&_socket.0, Duration::from_secs(1)),
            Err(ClientError::InvalidResponse(_))
        ));
        server.join().unwrap();
    }

    #[test]
    fn rejects_protocol_version_mismatch() {
        let (_socket, listener) = Socket::new();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 64];
            let _ = stream.read(&mut request);
            stream.write_all(b"{\"kind\":\"error\",\"version\":2,\"code\":\"unavailable\",\"message\":\"x\"}\n").unwrap();
        });
        assert!(matches!(
            request_status(&_socket.0, Duration::from_secs(1)),
            Err(ClientError::ProtocolMismatch { found: 2, .. })
        ));
        server.join().unwrap();
    }
}
