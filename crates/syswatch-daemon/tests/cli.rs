#![cfg(unix)]

mod common;

use common::{
    CLIENT_TIMEOUT, Daemon, TempDir, field_u64, request, request_line, run_cli, wait_until,
};
use std::time::Duration;
use syswatch_core::protocol::RequestCommand;

#[test]
fn status_command_prints_the_cached_telemetry() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let status = request_line(RequestCommand::Status);

    assert!(wait_until(CLIENT_TIMEOUT + Duration::from_secs(3), || {
        request(daemon.socket(), &status)
            .ok()
            .and_then(|line| field_u64(&line, "cpu_updated_at_ms"))
            .is_some()
    }));

    let output = run_cli(daemon.socket(), &["status"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(!stdout.is_empty());
    for token in ["CPU:", "Memory:", "Processes:", "Sampled:", "%"] {
        assert!(stdout.contains(token), "missing {token:?} in {stdout}");
    }
    assert!(
        [" B", " KiB", " MiB", " GiB", " TiB"]
            .iter()
            .any(|unit| stdout.contains(unit)),
        "missing binary memory unit in {stdout}"
    );
    assert!(!stdout.contains("error"));
}

#[test]
fn status_command_uses_the_daemon_socket() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let socket = daemon.socket().to_owned();

    assert!(run_cli(&socket, &["status"]).status.success());
    let _ = daemon.terminate();
    assert!(!socket.exists());

    let output = run_cli(&socket, &["status"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
}

#[test]
fn status_command_does_not_disturb_the_daemon() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);

    assert!(run_cli(daemon.socket(), &["status"]).status.success());
    assert!(request(daemon.socket(), &request_line(RequestCommand::Status)).is_ok());
    assert!(daemon.socket().exists());
}
