#![cfg(unix)]

mod common;

use common::{
    Daemon, TempDir, file_identity, other_socket_path, request, request_line, socket_path,
    spawn_daemon, stale_socket, wait_for_exit,
};
use std::{fs, io, os::unix::net::UnixStream, process::Output, time::Duration};
use syswatch_core::protocol::RequestCommand;

fn status_line() -> String {
    request_line(RequestCommand::Status)
}

fn failed_daemon_output(socket: &std::path::Path) -> Output {
    let child = spawn_daemon(socket).expect("spawn syswatch-daemon");
    wait_for_exit(child, Duration::from_secs(5))
}

#[test]
fn second_daemon_is_refused_and_the_live_socket_survives() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let socket = daemon.socket().to_owned();
    let identity = file_identity(&socket).expect("daemon socket exists");

    let second = failed_daemon_output(&socket);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("another daemon"));
    assert!(
        request(&socket, &status_line())
            .unwrap()
            .contains("\"kind\":\"status\"")
    );
    assert_eq!(file_identity(&socket), Some(identity));

    let output = daemon.terminate();
    assert!(output.status.success());
    assert!(!socket.exists());
}

#[test]
fn stale_socket_file_is_reclaimed() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);
    stale_socket(&socket);
    assert!(
        common::wait_until(Duration::from_secs(5), || {
            match UnixStream::connect(&socket) {
                Ok(stream) => {
                    drop(stream);
                    false
                }
                Err(error) => error.kind() == io::ErrorKind::ConnectionRefused,
            }
        }),
        "stale socket did not settle to connection refused"
    );

    let daemon = Daemon::start(&dir);
    assert!(
        request(&socket, &status_line())
            .unwrap()
            .contains("\"kind\":\"status\"")
    );
    let output = daemon.terminate();
    assert!(output.status.success());
    assert!(!socket.exists());
}

#[test]
fn foreign_file_at_the_socket_path_is_not_removed() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);
    let contents = b"owned by this test";
    fs::write(&socket, contents).unwrap();

    let output = failed_daemon_output(&socket);
    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    assert_eq!(fs::read(&socket).unwrap(), contents);
    assert!(fs::metadata(&socket).unwrap().is_file());
    assert!(UnixStream::connect(&socket).is_err());
}

#[test]
fn unrelated_socket_is_left_alone() {
    let dir = TempDir::new();
    let other = other_socket_path(&dir);
    stale_socket(&other);
    let identity = file_identity(&other).expect("unrelated socket exists");

    let daemon = Daemon::start(&dir);
    assert!(
        request(daemon.socket(), &status_line())
            .unwrap()
            .contains("\"kind\":\"status\"")
    );
    let output = daemon.terminate();
    assert!(output.status.success());

    assert_eq!(file_identity(&other), Some(identity));
    assert!(UnixStream::connect(&other).is_err());
}
