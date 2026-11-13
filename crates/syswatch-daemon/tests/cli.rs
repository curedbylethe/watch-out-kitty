#![cfg(unix)]

mod common;

use common::{
    CLIENT_TIMEOUT, Daemon, TempDir, field_u64, request, request_line, run_cli, socket_path,
    wait_until,
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
}

#[test]
fn status_command_does_not_disturb_the_daemon() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);

    assert!(run_cli(daemon.socket(), &["status"]).status.success());
    assert!(request(daemon.socket(), &request_line(RequestCommand::Status)).is_ok());
    assert!(daemon.socket().exists());
}

const STILL_COLLECTING: &str = "still collecting evidence: suggestions are not available until enough process history has been collected";

#[test]
fn suggestions_command_reports_still_collecting() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);

    let output = run_cli(daemon.socket(), &["suggestions"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(stdout.trim_end(), STILL_COLLECTING);
    for token in ["safe", "kill", "terminate", "orphan"] {
        assert!(!stdout.contains(token), "{token:?} in {stdout}");
    }
    assert!(output.stderr.is_empty());
}

#[test]
fn suggestions_command_reports_a_missing_daemon() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["suggestions"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot connect to daemon"), "{stderr}");
}

#[test]
fn suggestions_command_prints_usage_for_help() {
    let dir = TempDir::new();

    for spelling in ["--help", "-h", "help"] {
        let output = run_cli(&socket_path(&dir), &["suggestions", spelling]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{spelling}");
        assert!(stdout.contains("suggestions"), "{stdout}");
        assert!(!stdout.contains("--limit") && !stdout.contains("--sort"));
    }
}

#[test]
fn suggestions_command_rejects_unknown_arguments() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["suggestions", "--limit", "3"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr.contains("Usage: syswatch <COMMAND>"), "{stderr}");
}

#[test]
fn suggestions_command_leaves_the_daemon_serving() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);

    assert!(run_cli(daemon.socket(), &["suggestions"]).status.success());
    let status = request(daemon.socket(), &request_line(RequestCommand::Status)).unwrap();
    assert!(status.contains(r#""kind":"status""#), "{status}");
    assert!(daemon.socket().exists());
}
