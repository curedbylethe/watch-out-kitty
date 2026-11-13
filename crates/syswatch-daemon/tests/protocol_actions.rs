#![cfg(unix)]

mod common;

use common::{Daemon, TempDir, request};

/// A PID no test host uses, so the identity is absent from every scan.
const ABSENT: &str = r#""finding":{"pid":4000000,"start_time_secs":1,"kind":"cpu"}"#;

fn send(daemon: &Daemon, line: &str) -> String {
    request(daemon.socket(), &format!("{line}\n")).expect("daemon answers")
}

#[test]
fn close_preview_for_an_absent_identity_is_not_found() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let line = send(
        &daemon,
        &format!(r#"{{"version":1,"command":"close_preview",{ABSENT}}}"#),
    );
    assert!(line.contains(r#""kind":"error""#), "{line}");
    assert!(line.contains(r#""code":"not_found""#), "{line}");
}

#[test]
fn close_confirm_for_an_absent_identity_is_not_found() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let line = send(
        &daemon,
        &format!(r#"{{"version":1,"command":"close_confirm",{ABSENT}}}"#),
    );
    assert!(line.contains(r#""code":"not_found""#), "{line}");
}

#[test]
fn close_preview_without_a_finding_is_malformed() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let line = send(&daemon, r#"{"version":1,"command":"close_preview"}"#);
    assert!(line.contains(r#""code":"malformed_request""#), "{line}");
}

#[test]
fn status_with_a_finding_is_malformed() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let line = send(
        &daemon,
        &format!(r#"{{"version":1,"command":"status",{ABSENT}}}"#),
    );
    assert!(line.contains(r#""code":"malformed_request""#), "{line}");
}
