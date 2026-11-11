#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    os::unix::net::UnixStream,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "watchout-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Daemon(Option<Child>);

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn request_status(socket: &std::path::Path) -> Option<String> {
    use std::io::{BufRead, BufReader, Write};

    let mut stream = UnixStream::connect(socket).ok()?;
    stream
        .write_all(b"{\"version\":1,\"command\":\"status\"}\n")
        .ok()?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).ok()?;
    Some(response)
}

#[test]
fn daemon_socket_lifecycle_uses_cached_protocol_and_cleans_up_on_sigterm() {
    let dir = TempDir::new();
    let socket = dir.0.join("s");
    let mut daemon = Daemon(Some(
        Command::new(env!("CARGO_BIN_EXE_syswatch-daemon"))
            .env("SYSWATCH_SOCKET", &socket)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));

    let deadline = Instant::now() + Duration::from_secs(5);
    let response = loop {
        if let Some(response) = request_status(&socket) {
            if response.contains("\"kind\":\"status\"") {
                break response;
            }
            assert!(response.contains("\"code\":\"unavailable\""));
        }
        assert!(Instant::now() < deadline, "daemon did not serve a snapshot");
        thread::sleep(Duration::from_millis(25));
    };
    assert!(response.contains("\"kind\":\"status\""));
    assert_eq!(
        fs::symlink_metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let second = Command::new(env!("CARGO_BIN_EXE_syswatch-daemon"))
        .env("SYSWATCH_SOCKET", &socket)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("another daemon"));
    assert!(
        request_status(&socket)
            .unwrap()
            .contains("\"kind\":\"status\"")
    );
    assert!(socket.exists());

    let pid = daemon.0.as_ref().unwrap().id().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if daemon.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "daemon did not stop on SIGTERM");
        thread::sleep(Duration::from_millis(25));
    }
    let child = daemon.0.take().unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("syswatch-daemon: stopped"));
    assert!(!socket.exists());
}
