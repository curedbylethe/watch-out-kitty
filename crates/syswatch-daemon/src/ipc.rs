use std::{
    fs, io,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use syswatch_core::FindingId;
use syswatch_core::protocol::{
    ErrorCode, MAX_REQUEST_BYTES, ProtocolError, RequestCommand, Response, decode_request,
    encode_line,
};

use crate::latest::Cached;

const READ_TIMEOUT: Duration = Duration::from_millis(500);
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);
const ACCEPT_POLL: Duration = Duration::from_millis(25);

pub struct ServerHandle {
    path: PathBuf,
    identity: (u64, u64),
    shutdown: Arc<AtomicBool>,
    accept_thread: JoinHandle<()>,
}

impl ServerHandle {
    pub fn shutdown(self) -> io::Result<()> {
        self.shutdown.store(true, Ordering::SeqCst);
        self.accept_thread
            .join()
            .map_err(|_| io::Error::other("socket server thread panicked"))?;

        match fs::symlink_metadata(&self.path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && (metadata.dev(), metadata.ino()) == self.identity =>
            {
                fs::remove_file(self.path)
            }
            Ok(_) => Err(io::Error::other(
                "socket path changed; refusing to remove it",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Starts the socket server. `latest` serves monitoring requests. `action`
/// answers the close requests for one complete finding identity.
pub fn start<F, A>(path: PathBuf, latest: F, action: A) -> io::Result<ServerHandle>
where
    F: Fn() -> Cached + Send + Sync + 'static,
    A: Fn(RequestCommand, FindingId) -> Response + Send + Sync + 'static,
{
    if !path.is_absolute() || path.parent().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path must be absolute and have a parent",
        ));
    }
    let parent = path.parent().expect("absolute socket path has parent");
    match fs::metadata(parent) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        Err(error) => return Err(error),
    }
    let metadata = fs::metadata(parent)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("socket parent is not a directory: {}", parent.display()),
        ));
    }
    let mode = metadata.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "socket directory {} has unsafe mode {:o}",
                parent.display(),
                mode & 0o777
            ),
        ));
    }

    // std exposes only a blocking connect probe; the private directory bounds exposure.
    match UnixStream::connect(&path) {
        Ok(stream) => {
            drop(stream);
            return Err(in_use(&path));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_socket() {
                fs::remove_file(&path)?;
            } else {
                return Err(in_use(&path));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(in_use(&path)),
        Err(error) => return Err(error),
    }

    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::AddrInUse | io::ErrorKind::AlreadyExists
            ) =>
        {
            return Err(in_use(&path));
        }
        Err(error) => return Err(error),
    };
    // The 0700 parent protects the short interval before this chmod takes effect.
    let identity = match (|| {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&path)?;
        listener.set_nonblocking(true)?;
        Ok((metadata.dev(), metadata.ino()))
    })() {
        Ok(identity) => identity,
        Err(error) => {
            drop(listener);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
    };

    let shutdown = Arc::new(AtomicBool::new(false));
    let server_shutdown = Arc::clone(&shutdown);
    let latest = Arc::new(latest);
    let action = Arc::new(action);
    let accept_thread = match thread::Builder::new()
        .name("syswatch-socket".into())
        .spawn(move || accept_loop(listener, server_shutdown, latest, action))
    {
        Ok(thread) => thread,
        Err(error) => {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
    };
    Ok(ServerHandle {
        path,
        identity,
        shutdown,
        accept_thread,
    })
}

fn in_use(path: &std::path::Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AddrInUse,
        format!("another daemon is already serving {}", path.display()),
    )
}

fn accept_loop<F, A>(
    listener: UnixListener,
    shutdown: Arc<AtomicBool>,
    latest: Arc<F>,
    action: Arc<A>,
) where
    F: Fn() -> Cached + Send + Sync + 'static,
    A: Fn(RequestCommand, FindingId) -> Response + Send + Sync + 'static,
{
    let mut handlers: Vec<JoinHandle<()>> = Vec::new();
    while !shutdown.load(Ordering::SeqCst) {
        let mut index = 0;
        while index < handlers.len() {
            if handlers[index].is_finished() {
                let _ = handlers.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let latest = Arc::clone(&latest);
                let action = Arc::clone(&action);
                if let Ok(handler) = thread::Builder::new()
                    .name("syswatch-socket-client".into())
                    .spawn(move || handle_client(stream, &*latest, &*action))
                {
                    handlers.push(handler);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("syswatch-daemon: socket accept failed: {error}");
                thread::sleep(ACCEPT_POLL);
            }
        }
    }
    drop(listener);
    for handler in handlers {
        let _ = handler.join();
    }
}

struct DeadlineReader<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(buf)
    }
}

fn handle_client<F, A>(mut stream: UnixStream, latest: &F, action: &A)
where
    F: Fn() -> Cached,
    A: Fn(RequestCommand, FindingId) -> Response,
{
    if stream.set_nonblocking(false).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    let mut line = String::new();
    let reader = DeadlineReader {
        stream: &stream,
        deadline: Instant::now() + READ_TIMEOUT,
    };
    let read = BufReader::new(reader)
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_line(&mut line);
    let response = match read {
        Ok(0) => return,
        Ok(_) => {
            if line.ends_with('\n') {
                line.pop();
            }
            match decode_request(&line) {
                Ok(request) => match request.command {
                    RequestCommand::Status => {
                        let cached = latest();
                        cached
                            .snapshot
                            .as_ref()
                            .map_or_else(snapshot_unavailable, Response::status)
                    }
                    RequestCommand::Processes => {
                        let cached = latest();
                        cached
                            .snapshot
                            .as_ref()
                            .map_or_else(snapshot_unavailable, Response::processes)
                    }
                    RequestCommand::Suggestions => {
                        let cached = latest();
                        Response::suggestions(cached.suggestions.as_ref())
                    }
                    RequestCommand::ClosePreview | RequestCommand::CloseConfirm => {
                        match request.finding {
                            Some(finding) => action(request.command, finding),
                            // decode_request rejects this pairing; kept as a typed reply.
                            None => Response::error(ProtocolError {
                                code: ErrorCode::MalformedRequest,
                                message: "close requests require a finding object".into(),
                            }),
                        }
                    }
                },
                Err(error) => Response::error(error),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            Response::error(ProtocolError {
                code: ErrorCode::MalformedRequest,
                message: "request is not valid UTF-8".into(),
            })
        }
        Err(_) => return,
    };
    // Transport contract allows one request per connection.
    if let Ok(mut line) = encode_line(&response) {
        line.push('\n');
        let _ = stream.write_all(line.as_bytes());
    }
}

fn snapshot_unavailable() -> Response {
    Response::error(ProtocolError {
        code: ErrorCode::Unavailable,
        message: "no snapshot has been collected yet".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use syswatch_core::{MemorySnapshot, ProcessSnapshot, SuggestionsSnapshot, SystemSnapshot};

    fn no_action(_: RequestCommand, _: FindingId) -> Response {
        Response::error(ProtocolError {
            code: ErrorCode::UnsupportedCommand,
            message: "no action in this test".into(),
        })
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = PathBuf::from("/tmp").join(format!(
                "si{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join("s")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            timestamp_ms: 123,
            cpu_usage_percent: 1.5,
            cpu_updated_at_ms: Some(123),
            memory: MemorySnapshot {
                total_bytes: 100,
                used_bytes: 50,
            },
            processes: vec![ProcessSnapshot {
                pid: 42,
                parent_pid: Some(1),
                name: "worker".into(),
                cpu_usage_percent: 2.0,
                memory_bytes: 10,
                start_time_secs: 10,
            }],
            processes_updated_at_ms: Some(122),
        }
    }

    fn request(path: &std::path::Path, input: &str) -> String {
        let mut stream = UnixStream::connect(path).unwrap();
        stream.write_all(input.as_bytes()).unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        response
    }

    fn server(path: PathBuf) -> ServerHandle {
        start(
            path,
            || Cached {
                snapshot: Some(snapshot()),
                suggestions: None,
            },
            no_action,
        )
        .unwrap()
    }

    #[test]
    fn serves_the_cached_snapshot_as_status() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let line = request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\n");
        assert!(line.contains("\"kind\":\"status\""));
        assert!(line.contains("\"timestamp_ms\":123"));
        server.shutdown().unwrap();
    }

    #[test]
    fn handles_delayed_request_after_accept() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let mut stream = UnixStream::connect(dir.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        stream
            .write_all(b"{\"version\":1,\"command\":\"status\"}\n")
            .unwrap();
        let mut response = String::new();
        assert!(BufReader::new(stream).read_line(&mut response).unwrap() > 0);
        assert!(response.contains("\"kind\":\"status\""));
        server.shutdown().unwrap();
    }

    #[test]
    fn serves_processes_from_the_cached_snapshot() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let line = request(&dir.socket(), "{\"version\":1,\"command\":\"processes\"}\n");
        assert!(line.contains("\"kind\":\"processes\""));
        assert!(line.contains("\"name\":\"worker\""));
        server.shutdown().unwrap();
    }

    #[test]
    fn answers_unavailable_before_the_first_snapshot() {
        let dir = TempDir::new();
        let server = start(dir.socket(), Cached::default, no_action).unwrap();
        for command in ["status", "processes"] {
            let line = request(
                &dir.socket(),
                &format!("{{\"version\":1,\"command\":\"{command}\"}}\n"),
            );
            assert!(line.contains("\"code\":\"unavailable\""));
            assert!(line.contains("no snapshot has been collected yet"));
        }
        server.shutdown().unwrap();
    }

    #[test]
    fn suggestions_are_served_from_the_cached_source() {
        let dir = TempDir::new();
        let server = start(
            dir.socket(),
            || Cached {
                snapshot: None,
                suggestions: Some(SuggestionsSnapshot {
                    evaluated_at_ms: 7,
                    processes_updated_at_ms: 6,
                    findings: vec![],
                }),
            },
            no_action,
        )
        .unwrap();
        let line = request(
            &dir.socket(),
            "{\"version\":1,\"command\":\"suggestions\"}\n",
        );
        assert!(line.contains("\"kind\":\"suggestions\""));
        assert!(line.contains("\"findings\":[]"));
        let line = request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\n");
        assert!(line.contains("no snapshot has been collected yet"));
        server.shutdown().unwrap();
    }

    #[test]
    fn suggestions_report_their_own_unavailable_state() {
        let dir = TempDir::new();
        let server = start(dir.socket(), Cached::default, no_action).unwrap();
        let line = request(
            &dir.socket(),
            "{\"version\":1,\"command\":\"suggestions\"}\n",
        );
        assert!(line.contains("\"code\":\"unavailable\""));
        assert!(line.contains(
            "suggestions are not available until enough process history has been collected"
        ));
        assert!(!line.contains("no snapshot has been collected yet"));
        server.shutdown().unwrap();
    }

    #[test]
    fn reads_the_source_once_per_suggestions_request() {
        let dir = TempDir::new();
        let reads = Arc::new(AtomicUsize::new(0));
        let server_reads = Arc::clone(&reads);
        let server = start(
            dir.socket(),
            move || {
                server_reads.fetch_add(1, Ordering::SeqCst);
                Cached::default()
            },
            no_action,
        )
        .unwrap();
        request(
            &dir.socket(),
            "{\"version\":1,\"command\":\"suggestions\"}\n",
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        server.shutdown().unwrap();
    }

    #[test]
    fn reads_the_source_once_per_request_without_storing_it() {
        let dir = TempDir::new();
        let reads = Arc::new(AtomicUsize::new(0));
        let server_reads = Arc::clone(&reads);
        let server = start(
            dir.socket(),
            move || {
                server_reads.fetch_add(1, Ordering::SeqCst);
                Cached {
                    snapshot: Some(snapshot()),
                    suggestions: None,
                }
            },
            no_action,
        )
        .unwrap();
        for _ in 0..2 {
            assert!(
                request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\n")
                    .contains("\"timestamp_ms\":123")
            );
        }
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        server.shutdown().unwrap();
    }

    #[test]
    fn rejects_a_second_server_without_removing_the_live_socket() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        assert_eq!(
            start(
                dir.socket(),
                || Cached {
                    snapshot: Some(snapshot()),
                    suggestions: None
                },
                no_action
            )
            .err()
            .unwrap()
            .kind(),
            io::ErrorKind::AddrInUse
        );
        assert!(
            request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\n")
                .contains("\"kind\":\"status\"")
        );
        assert!(dir.socket().exists());
        server.shutdown().unwrap();
    }

    #[test]
    fn reclaims_a_stale_socket_file() {
        let dir = TempDir::new();
        drop(UnixListener::bind(dir.socket()).unwrap());
        let server = server(dir.socket());
        assert!(
            request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\n")
                .contains("\"kind\":\"status\"")
        );
        server.shutdown().unwrap();
    }

    #[test]
    fn refuses_to_remove_a_regular_file() {
        let dir = TempDir::new();
        fs::write(dir.socket(), "keep").unwrap();
        assert!(
            start(
                dir.socket(),
                || Cached {
                    snapshot: Some(snapshot()),
                    suggestions: None
                },
                no_action
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(dir.socket()).unwrap(), "keep");
    }

    #[test]
    fn creates_the_socket_with_owner_only_permissions() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        assert_eq!(
            fs::metadata(dir.socket()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&dir.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        server.shutdown().unwrap();
    }

    #[test]
    fn rejects_a_group_readable_directory() {
        let dir = TempDir::new();
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            start(
                dir.socket(),
                || Cached {
                    snapshot: Some(snapshot()),
                    suggestions: None
                },
                no_action
            )
            .err()
            .unwrap()
            .kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn bounds_a_client_that_sends_nothing() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let stream = UnixStream::connect(dir.socket()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut response = String::new();
        assert_eq!(BufReader::new(stream).read_line(&mut response).unwrap(), 0);
        server.shutdown().unwrap();
    }

    #[test]
    fn bounds_slow_drip_client_shutdown() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let mut client = UnixStream::connect(dir.socket()).unwrap();
        client.write_all(b"{").unwrap();
        let keep_writing = Arc::new(AtomicBool::new(true));
        let writer_flag = Arc::clone(&keep_writing);
        let writer = thread::spawn(move || {
            while writer_flag.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(100));
                if client.write_all(b" ").is_err() {
                    break;
                }
            }
        });
        let (finished, done) = std::sync::mpsc::channel();
        let shutdown = thread::spawn(move || finished.send(server.shutdown()).unwrap());
        let timely = done.recv_timeout(Duration::from_millis(900));
        let completed_timely = timely.is_ok();
        keep_writing.store(false, Ordering::SeqCst);
        writer.join().unwrap();
        let result = match timely {
            Ok(result) => result,
            Err(_) => done.recv_timeout(Duration::from_secs(2)).unwrap(),
        };
        shutdown.join().unwrap();
        assert!(completed_timely, "slow client delayed server shutdown");
        result.unwrap();
    }

    #[test]
    fn rejects_an_oversized_request() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let line = request(
            &dir.socket(),
            &format!("{}\n", "x".repeat(MAX_REQUEST_BYTES + 100)),
        );
        assert!(line.contains("\"code\":\"malformed_request\""));
        server.shutdown().unwrap();
    }

    #[test]
    fn rejects_malformed_json() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        assert!(request(&dir.socket(), "not json\n").contains("\"code\":\"malformed_request\""));
        server.shutdown().unwrap();
    }

    #[test]
    fn rejects_crlf_request_lines() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let response = request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\r\n");
        assert!(response.contains("\"code\":\"malformed_request\""));
        server.shutdown().unwrap();
    }

    #[test]
    fn handles_concurrent_clients() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let path = dir.socket();
        let clients: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || request(&path, "{\"version\":1,\"command\":\"status\"}\n"))
            })
            .collect();
        for client in clients {
            assert!(client.join().unwrap().contains("\"kind\":\"status\""));
        }
        server.shutdown().unwrap();
    }

    #[test]
    fn handles_sequential_clients() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        for _ in 0..2 {
            assert!(
                request(&dir.socket(), "{\"version\":1,\"command\":\"status\"}\n")
                    .contains("\"kind\":\"status\"")
            );
        }
        server.shutdown().unwrap();
    }

    #[test]
    fn closes_after_one_response() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        let mut stream = UnixStream::connect(dir.socket()).unwrap();
        stream
            .write_all(
                b"{\"version\":1,\"command\":\"status\"}\n{\"version\":1,\"command\":\"status\"}\n",
            )
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        server.shutdown().unwrap();
    }

    #[test]
    fn shutdown_removes_the_owned_socket() {
        let dir = TempDir::new();
        server(dir.socket()).shutdown().unwrap();
        assert!(!dir.socket().exists());
        let second = server(dir.socket());
        second.shutdown().unwrap();
    }

    #[test]
    fn does_not_unlink_a_replaced_socket() {
        let dir = TempDir::new();
        let server = server(dir.socket());
        fs::remove_file(dir.socket()).unwrap();
        fs::write(dir.socket(), "keep").unwrap();
        assert!(server.shutdown().is_err());
        assert_eq!(fs::read_to_string(dir.socket()).unwrap(), "keep");
    }
}
