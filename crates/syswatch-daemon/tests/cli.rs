#![cfg(unix)]

mod common;

use common::{
    CLIENT_TIMEOUT, Daemon, TempDir, cli_binary, field_u64, request, request_line, run_cli,
    socket_path, wait_for_exit, wait_until,
};
use std::{
    path::Path,
    process::{Command, Output, Stdio},
    time::Duration,
};
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

    for spelling in ["--help", "-h"] {
        let output = run_cli(&socket_path(&dir), &["suggestions", spelling]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{spelling}");
        assert!(stdout.contains("suggestions"), "{stdout}");
        assert!(!stdout.contains("--limit") && !stdout.contains("--sort"));
    }
}

#[test]
fn suggestions_rejects_the_bare_help_word() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["suggestions", "help"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr.contains("Usage: wok <COMMAND>"), "{stderr}");
}

#[test]
fn suggestions_command_rejects_unknown_arguments() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["suggestions", "--limit", "3"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr.contains("Usage: wok <COMMAND>"), "{stderr}");
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

#[test]
fn inspect_reports_the_unavailable_state() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);

    let output = run_cli(daemon.socket(), &["inspect", "42-1700000000-memory"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        stderr.contains(
            "wok: daemon error (unavailable): suggestions are not available until enough process history has been collected"
        ),
        "{stderr}"
    );
}

#[test]
fn inspect_reports_a_missing_daemon() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["inspect", "42-1700000000-memory"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot connect to daemon"), "{stderr}");
}

#[test]
fn inspect_rejects_a_malformed_reference_before_connecting() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["inspect", "42-5"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage: wok"), "{stderr}");
    assert!(
        stderr.contains("invalid finding reference: expected pid-start_time_secs-kind"),
        "{stderr}"
    );
    assert!(!stderr.contains("cannot connect"), "{stderr}");
}

#[test]
fn inspect_help_exits_zero() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["inspect", "--help"]);
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Usage: wok inspect"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn close_rejects_selectors_and_flags_before_connecting() {
    let dir = TempDir::new();
    for args in [&["close"][..], &["close", "42"], &["close", "--yes"]] {
        let output = run_cli(&socket_path(&dir), args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Usage: wok close"), "{stderr}");
        assert!(!stderr.contains("cannot connect"), "{stderr}");
    }
}

#[test]
fn close_without_a_terminal_aborts_before_any_request() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["close", "42-1-cpu"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("wok: close requires an interactive terminal; no action was requested"),
        "{stderr}"
    );
    assert!(!stderr.contains("cannot connect"), "{stderr}");
    assert!(output.stdout.is_empty());
}

#[test]
fn version_command_prints_the_binary_version_without_a_daemon() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);

    for spelling in ["version", "-V", "--version"] {
        let output = run_cli(&socket, &[spelling]);
        assert!(output.status.success(), "{spelling}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("wok {}\n", env!("CARGO_PKG_VERSION")),
            "{spelling}"
        );
        assert!(output.stderr.is_empty(), "{spelling}");
    }
}

#[test]
fn close_help_exits_zero() {
    let dir = TempDir::new();

    for args in [&["close", "--help"][..], &["help", "close"]] {
        let output = run_cli(&socket_path(&dir), args);
        assert!(output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("Usage: wok close <finding-reference>"),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

/// One row per command: the argv before the help spelling, the block's first
/// line, and the flags that block must document. `help` has no row of its own;
/// its block is the usage block checked by the byte-identity test.
const HELP_BLOCKS: [(&[&str], &str, &[&str]); 8] = [
    (&["status"], "Usage: wok status", &["-h", "--help"]),
    (
        &["top"],
        "Usage: wok top [--sort memory|cpu] [--limit N]",
        &["--sort", "--limit", "-h", "--help"],
    ),
    (
        &["suggestions"],
        "Usage: wok suggestions",
        &["-h", "--help"],
    ),
    (
        &["inspect"],
        "Usage: wok inspect <finding-reference>",
        &["<finding-reference>", "-h", "--help"],
    ),
    (
        &["close"],
        "Usage: wok close <finding-reference>",
        &["<finding-reference>", "-h", "--help"],
    ),
    (
        &["update"],
        "Usage: wok update <COMMAND>",
        &["dismiss", "-h", "--help"],
    ),
    (
        &["update", "dismiss"],
        "Usage: wok update dismiss",
        &["-h", "--help"],
    ),
    (&["version"], "Usage: wok version", &["-h", "--help"]),
];

#[test]
fn every_command_answers_both_help_spellings_with_its_own_block() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);

    for (prefix, first_line, flags) in HELP_BLOCKS {
        for spelling in ["-h", "--help"] {
            let mut args = prefix.to_vec();
            args.push(spelling);
            let output = run_cli(&socket, &args);
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert_eq!(output.status.code(), Some(0), "{args:?}");
            assert!(output.stderr.is_empty(), "{args:?}: {:?}", output.stderr);
            assert!(stdout.starts_with(first_line), "{args:?}: {stdout}");
            for flag in flags {
                assert!(stdout.contains(flag), "{args:?} lacks {flag:?}: {stdout}");
            }
        }
    }
}

#[test]
fn the_three_top_level_help_spellings_print_the_same_block() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);

    let outputs: Vec<Output> = ["--help", "-h", "help"]
        .into_iter()
        .map(|spelling| run_cli(&socket, &[spelling]))
        .collect();
    for output in &outputs {
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    }
    assert_eq!(outputs[0].stdout, outputs[1].stdout);
    assert_eq!(outputs[0].stdout, outputs[2].stdout);

    let stdout = String::from_utf8_lossy(&outputs[0].stdout);
    for section in [
        "Usage: wok <COMMAND>",
        "Commands:",
        "Options:",
        "Exit codes:",
    ] {
        assert!(stdout.contains(section), "missing {section:?} in {stdout}");
    }
    for code in ["0  Success", "1  Runtime failure", "2  Usage error"] {
        assert!(stdout.contains(code), "missing {code:?} in {stdout}");
    }
}

#[test]
fn help_topic_prints_that_commands_block() {
    let dir = TempDir::new();
    let socket = socket_path(&dir);

    for (prefix, first_line, _) in HELP_BLOCKS {
        let [name] = prefix else { continue };
        let output = run_cli(&socket, &["help", *name]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(0), "{name}");
        assert!(output.stderr.is_empty(), "{name}: {:?}", output.stderr);
        assert!(stdout.starts_with(first_line), "{name}: {stdout}");
    }

    let output = run_cli(&socket, &["help", "help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    assert!(stdout.starts_with("Usage: wok <COMMAND>"), "{stdout}");
}

#[test]
fn the_lowercase_v_is_an_unknown_command() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["-v"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert!(stderr.contains("unrecognized command: -v"), "{stderr}");
    assert!(stderr.contains("Usage: wok <COMMAND>"), "{stderr}");
    assert!(stderr.contains("Exit codes:"), "{stderr}");
}

/// Runs the CLI with the update state path pointed at a temporary file.
fn run_cli_with_state(socket: &Path, state: &Path, args: &[&str]) -> Output {
    let child = Command::new(cli_binary())
        .args(args)
        .env("WOK_SOCKET", socket)
        .env("WOK_UPDATE_STATE", state)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wok CLI");
    wait_for_exit(child, Duration::from_secs(5))
}

const UPDATE_USAGE_FIRST_LINE: &str = "Usage: wok update <COMMAND>";

#[test]
fn update_without_a_sub_command_prints_its_usage() {
    let dir = TempDir::new();

    let output = run_cli(&socket_path(&dir), &["update"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(stderr.contains(UPDATE_USAGE_FIRST_LINE), "{stderr}");
    assert!(stderr.contains("update requires a sub-command"), "{stderr}");
    assert!(!stderr.contains("cannot connect"), "{stderr}");
}

#[test]
fn update_help_exits_zero() {
    let dir = TempDir::new();

    for spelling in ["--help", "-h"] {
        let output = run_cli(&socket_path(&dir), &["update", spelling]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{spelling}");
        assert!(stdout.contains(UPDATE_USAGE_FIRST_LINE), "{stdout}");
        assert!(stdout.contains("dismiss"), "{stdout}");
        assert!(output.stderr.is_empty(), "{spelling}");
    }
}

#[test]
fn update_rejects_unknown_and_extra_sub_commands() {
    let dir = TempDir::new();

    for args in [
        &["update", "check"][..],
        &["update", "dismiss", "extra"],
        &["update", "--help", "dismiss"],
    ] {
        let output = run_cli(&socket_path(&dir), args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            stderr.contains(UPDATE_USAGE_FIRST_LINE),
            "{args:?}: {stderr}"
        );
        assert!(!stderr.contains("cannot connect"), "{args:?}: {stderr}");
    }
}

#[test]
fn update_dismiss_without_a_notice_writes_nothing() {
    let dir = TempDir::new();
    let daemon = Daemon::start(&dir);
    let state = dir.path().join("state/update.json");

    let output = run_cli_with_state(daemon.socket(), &state, &["update", "dismiss"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert_eq!(stdout.trim_end(), "no update to dismiss");
    assert!(output.stderr.is_empty());
    assert!(!state.exists(), "{} must not be written", state.display());
    assert!(
        !state.parent().expect("state path has a parent").exists(),
        "the state directory must not be created"
    );
    assert!(daemon.socket().exists());
}
