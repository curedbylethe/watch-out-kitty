#![cfg(unix)]

mod common;

use common::{
    Daemon, TempDir, field_f64, field_is_null, field_u64, line_contains, request, request_line,
    wait_until,
};
use std::{thread, time::Duration};
use syswatch_core::protocol::RequestCommand;

fn response(daemon: &Daemon, command: RequestCommand, kind: &str) -> String {
    let line = request_line(command);
    let mut found = None;
    assert!(
        wait_until(Duration::from_secs(5), || {
            if let Ok(reply) = request(daemon.socket(), &line)
                && line_contains(&reply, &format!("\"kind\":\"{kind}\""))
            {
                found = Some(reply);
                true
            } else {
                false
            }
        }),
        "daemon did not return {kind} response"
    );
    found.unwrap()
}

#[test]
fn status_response_carries_the_aggregate_and_freshness() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let line = response(&daemon, RequestCommand::Status, "status");
    assert!(line_contains(&line, "\"version\":1"));
    assert!(field_u64(&line, "timestamp_ms").is_some());
    let cpu = field_f64(&line, "cpu_usage_percent").unwrap();
    assert!(cpu.is_finite() && (0.0..=100.0).contains(&cpu));
    let total = field_u64(&line, "total_bytes").unwrap();
    let used = field_u64(&line, "used_bytes").unwrap();
    assert!(total > 0);
    assert!(used <= total);
    assert!(field_u64(&line, "process_count").is_some() || field_is_null(&line, "process_count"));
    assert!(
        field_u64(&line, "processes_updated_at_ms").is_some()
            || field_is_null(&line, "processes_updated_at_ms")
    );
}

#[test]
fn status_freshens_after_an_aggregate_interval() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let first = response(&daemon, RequestCommand::Status, "status");
    let first_timestamp = field_u64(&first, "timestamp_ms").unwrap();
    let mut fresh = None;
    assert!(
        wait_until(Duration::from_secs(5), || {
            if let Ok(line) = request(daemon.socket(), &request_line(RequestCommand::Status))
                && line_contains(&line, "\"kind\":\"status\"")
                && field_u64(&line, "timestamp_ms").is_some_and(|time| time > first_timestamp)
            {
                fresh = Some(line);
                true
            } else {
                false
            }
        }),
        "aggregate did not refresh"
    );
    let line = fresh.unwrap();
    let cpu_updated = field_u64(&line, "cpu_updated_at_ms").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i128;
    assert!((-5_000..=60_000).contains(&(now - cpu_updated as i128)));
}

#[test]
fn process_list_returns_the_cached_scan() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let mut found = None;
    let mut last = String::new();
    assert!(
        wait_until(Duration::from_secs(5), || {
            if let Ok(line) = request(daemon.socket(), &request_line(RequestCommand::Processes))
                && line_contains(&line, "\"kind\":\"processes\"")
            {
                last = line;
                let valid = process_entries(&last)
                    .any(|entry| field_u64(entry, "start_time_secs").is_some_and(|time| time > 0));
                if valid {
                    found = Some(last.clone());
                }
                valid
            } else {
                false
            }
        }),
        "process scan did not contain a started process: {last}"
    );
    let line = found.unwrap();
    assert!(field_u64(&line, "observed_at_ms").unwrap() > 0);
    assert!(field_u64(&line, "processes_updated_at_ms").unwrap() > 0);
    let process = process_entries(&line)
        .find(|entry| field_u64(entry, "start_time_secs").is_some_and(|time| time > 0))
        .unwrap();
    assert!(field_u64(process, "pid").unwrap() > 0);
    assert!(process.contains("\"name\":"));
    assert!(field_u64(process, "memory_bytes").is_some());
    assert!(field_f64(process, "cpu_usage_percent").unwrap().is_finite());
}

fn process_entries(line: &str) -> impl Iterator<Item = &str> {
    line.split_once("\"processes\":[")
        .map(|(_, entries)| entries.split("},{"))
        .into_iter()
        .flatten()
}

#[test]
fn several_sequential_clients_are_served() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    for index in 0..8 {
        let line = request(daemon.socket(), &request_line(RequestCommand::Status)).unwrap();
        assert!(line.contains("\"kind\":"), "client {index} got {line}");
        if index == 7 {
            assert!(field_u64(&line, "timestamp_ms").is_some());
        }
    }
}

#[test]
fn concurrent_clients_are_served() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| {
                let line = request(daemon.socket(), &request_line(RequestCommand::Status)).unwrap();
                assert!(line.contains("\"kind\":"));
            });
        }
    });
}

#[test]
fn process_and_status_agree_on_freshness_fields() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let line = request_line(RequestCommand::Status);
    let mut sampled = None;
    assert!(
        wait_until(Duration::from_secs(5), || {
            if let Ok(reply) = request(daemon.socket(), &line)
                && line_contains(&reply, "\"kind\":\"status\"")
                && field_u64(&reply, "processes_updated_at_ms").is_some()
            {
                sampled = Some(reply);
                true
            } else {
                false
            }
        }),
        "daemon did not return a status response with process data"
    );
    let status = sampled.unwrap();
    let processes = response(&daemon, RequestCommand::Processes, "processes");
    let status_time = field_u64(&status, "processes_updated_at_ms").unwrap();
    let process_time = field_u64(&processes, "processes_updated_at_ms").unwrap();
    assert!((status_time.abs_diff(process_time)) <= 60_000);
    assert!(!field_is_null(&status, "processes_updated_at_ms"));
}
