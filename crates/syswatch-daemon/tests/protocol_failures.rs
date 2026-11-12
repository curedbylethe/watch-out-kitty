#![cfg(unix)]

mod common;

use common::{CLIENT_TIMEOUT, Daemon, TempDir, request, request_line, run_cli, socket_path};
use std::{
    io::{BufRead, Write},
    os::unix::net::UnixStream,
};
use syswatch_core::protocol::{MAX_REQUEST_BYTES, PROTOCOL_VERSION, RequestCommand};

#[test]
fn malformed_request_is_rejected_and_the_daemon_continues() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);

    let response = request(daemon.socket(), "not json\n").unwrap();
    assert!(response.contains("\"kind\":\"error\""));
    assert!(response.contains("\"code\":\"malformed_request\""));
    assert!(
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":")
    );
}

#[test]
fn oversized_request_is_rejected() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let mut stream = UnixStream::connect(daemon.socket()).unwrap();
    stream.set_read_timeout(Some(CLIENT_TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(CLIENT_TIMEOUT)).unwrap();
    stream
        .write_all(&vec![b'a'; MAX_REQUEST_BYTES + 20])
        .unwrap();
    let mut response = String::new();
    std::io::BufReader::new(stream)
        .read_line(&mut response)
        .unwrap();
    assert!(response.contains("\"kind\":\"error\""));
    assert!(response.contains("\"code\":\"malformed_request\""));
    assert!(
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":")
    );
}

#[test]
fn unsupported_protocol_version_is_reported() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let version = PROTOCOL_VERSION + 1;
    let response = request(
        daemon.socket(),
        &format!("{{\"version\":{version},\"command\":\"status\"}}\n"),
    )
    .unwrap();
    assert!(response.contains("\"code\":\"unsupported_version\""));
    assert!(response.contains(&version.to_string()));
    assert!(response.contains(&PROTOCOL_VERSION.to_string()));
}

#[test]
fn suggestions_version_mismatch_is_reported() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let response = request(
        daemon.socket(),
        "{\"version\":2,\"command\":\"suggestions\"}\n",
    )
    .unwrap();
    assert!(response.contains("\"code\":\"unsupported_version\""));
    assert!(response.contains("unsupported protocol version 2; this server speaks 1"));
}

#[test]
fn suggestions_silent_disconnect_is_ignored() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    drop(UnixStream::connect(daemon.socket()).unwrap());
    assert!(
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":")
    );
}

#[test]
fn suggestions_partial_line_disconnect_is_ignored() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let mut stream = UnixStream::connect(daemon.socket()).unwrap();
    stream
        .write_all(b"{\"version\":1,\"command\":\"suggestions\"")
        .unwrap();
    drop(stream);
    assert!(
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":")
    );
}

#[test]
fn unknown_command_is_rejected() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let response = request(
        daemon.socket(),
        &format!("{{\"version\":{PROTOCOL_VERSION},\"command\":\"kill\"}}\n"),
    )
    .unwrap();
    assert!(response.contains("\"code\":\"unsupported_command\""));
    assert!(response.contains("unsupported command \\\"kill\\\""));
}

#[test]
fn silent_client_is_disconnected_within_the_timeout() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let mut stream = UnixStream::connect(daemon.socket()).unwrap();
    stream.set_read_timeout(Some(CLIENT_TIMEOUT)).unwrap();
    let mut response = Vec::new();
    assert_eq!(std::io::Read::read(&mut stream, &mut response).unwrap(), 0);
    assert!(
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":")
    );
}

#[test]
fn partial_line_client_is_disconnected_within_the_timeout() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let mut stream = UnixStream::connect(daemon.socket()).unwrap();
    stream.set_read_timeout(Some(CLIENT_TIMEOUT)).unwrap();
    stream.set_write_timeout(Some(CLIENT_TIMEOUT)).unwrap();
    stream.write_all(b"{\"version\":1,").unwrap();
    let mut response = Vec::new();
    assert_eq!(std::io::Read::read(&mut stream, &mut response).unwrap(), 0);
    assert!(
        request(daemon.socket(), &request_line(RequestCommand::Status))
            .unwrap()
            .contains("\"kind\":")
    );
}

#[test]
fn cli_reports_a_missing_daemon() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);
    let output = run_cli(&socket, &["status"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot connect to daemon"));
}
