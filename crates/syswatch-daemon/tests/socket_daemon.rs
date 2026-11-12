#![cfg(unix)]

mod common;

use common::{Daemon, TempDir, poll, request, request_line, socket_path};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use syswatch_core::protocol::RequestCommand;

#[test]
fn daemon_socket_lifecycle_uses_cached_protocol_and_cleans_up_on_sigterm() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);
    let daemon = Daemon::start(&dir);
    assert_eq!(
        fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );

    let _response = poll(&daemon, RequestCommand::Status, |line| {
        line.contains("\"kind\":\"status\"")
    });
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
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":\"status\"")
    );
    assert!(socket.exists());

    let output = daemon.terminate();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("syswatch-daemon: stopped"));
    assert!(!socket.exists());
}
