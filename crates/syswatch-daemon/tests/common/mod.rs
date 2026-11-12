#![allow(dead_code)]

use std::{
    fs,
    io::{self, BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use syswatch_core::protocol::{PROTOCOL_VERSION, Request, RequestCommand, encode_line};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub const DAEMON_READY: Duration = Duration::from_secs(5);
pub const POLL: Duration = Duration::from_millis(25);
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "watchout-e2e-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::SeqCst)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("create temporary test directory");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn socket_path(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("s");
    let length = path.as_os_str().len();
    assert!(
        length < 100,
        "socket path is {length} bytes; must be under 100 bytes"
    );
    path
}

pub fn other_socket_path(dir: &TempDir) -> PathBuf {
    dir.path().join("other.sock")
}

pub fn request_line(command: RequestCommand) -> String {
    format!(
        "{}\n",
        encode_line(&Request {
            version: PROTOCOL_VERSION,
            command,
        })
        .unwrap()
    )
}

pub fn request(socket: &Path, line: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    stream.write_all(line.as_bytes())?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(response.trim_end_matches('\n').to_owned())
}

pub fn field_u64(line: &str, key: &str) -> Option<u64> {
    let start = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    line[start..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

pub fn field_f64(line: &str, key: &str) -> Option<f64> {
    let start = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    line[start..]
        .trim_start()
        .split(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .next()?
        .parse()
        .ok()
}

pub fn field_is_null(line: &str, key: &str) -> bool {
    line.contains(&format!("\"{key}\":null"))
}

pub fn line_contains(line: &str, needle: &str) -> bool {
    line.contains(needle)
}

pub fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL);
    }
}

pub fn spawn_daemon(socket: &Path) -> io::Result<Child> {
    Command::new(env!("CARGO_BIN_EXE_syswatch-daemon"))
        .env("SYSWATCH_SOCKET", socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

pub struct Daemon {
    child: Option<Child>,
    socket: PathBuf,
}

impl Daemon {
    pub fn start(dir: &TempDir) -> Self {
        let socket = socket_path(dir);
        let child = spawn_daemon(&socket).expect("spawn syswatch-daemon");
        let mut daemon = Self {
            child: Some(child),
            socket,
        };
        let line = request_line(RequestCommand::Status);
        let mut last_error = String::new();
        if !wait_until(DAEMON_READY, || match request(&daemon.socket, &line) {
            Ok(response)
                if response.contains("\"kind\":\"status\"")
                    || response.contains("\"code\":\"unavailable\"") =>
            {
                true
            }
            Ok(response) => {
                last_error = response;
                false
            }
            Err(error) => {
                last_error = error.to_string();
                false
            }
        }) {
            let mut child = daemon.child.take().unwrap();
            let _ = child.kill();
            let output = child.wait_with_output().expect("collect daemon output");
            panic!(
                "daemon did not become ready: {last_error}; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        daemon
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
    pub fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().unwrap()
    }
    pub fn child_id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    pub fn terminate(mut self) -> Output {
        let pid = self.child_id().to_string();
        assert!(
            Command::new("kill")
                .args(["-TERM", &pid])
                .status()
                .unwrap()
                .success()
        );
        if !wait_until(DAEMON_READY, || {
            self.child_mut().try_wait().unwrap().is_some()
        }) {
            let child = self.child.as_mut().unwrap();
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not stop on SIGTERM");
        }
        self.child.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn cli_binary() -> PathBuf {
    let path = std::env::current_exe().expect("current test executable");
    let binary = path
        .parent()
        .and_then(Path::parent)
        .expect("test executable under target/debug/deps")
        .join("syswatch");
    assert!(
        binary.is_file(),
        "syswatch binary missing at {}; run cargo build --workspace",
        binary.display()
    );
    binary
}

pub fn stale_socket(path: &Path) {
    drop(UnixListener::bind(path).expect("bind stale socket"));
}

pub fn file_identity(path: &Path) -> Option<(u64, u64)> {
    fs::symlink_metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}

pub fn run_cli(socket: &Path, args: &[&str]) -> Output {
    let child = Command::new(cli_binary())
        .args(args)
        .env("SYSWATCH_SOCKET", socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syswatch CLI");
    wait_for_exit(child, Duration::from_secs(5))
}

pub fn wait_for_exit(mut child: Child, deadline: Duration) -> Output {
    let until = Instant::now() + deadline;
    loop {
        if child.try_wait().expect("poll child").is_some() {
            return child.wait_with_output().expect("collect child output");
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .expect("collect timed-out child output");
            panic!(
                "child exceeded {deadline:?}; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(POLL);
    }
}
