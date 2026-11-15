#[cfg(unix)]
mod args;
#[cfg(unix)]
mod client;
#[cfg(unix)]
mod output;

#[cfg(unix)]
fn main() {
    use std::time::SystemTime;

    let command = match args::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let request_command = match &command {
        args::Command::Close { reference } => std::process::exit(run_close(*reference)),
        args::Command::CloseHelp => {
            println!("{}", args::CLOSE_USAGE);
            return;
        }
        args::Command::UpdateHelp => {
            println!("{}", args::UPDATE_USAGE);
            return;
        }
        args::Command::StatusHelp => {
            println!("{}", args::STATUS_USAGE);
            return;
        }
        args::Command::UpdateDismissHelp => {
            println!("{}", args::UPDATE_DISMISS_USAGE);
            return;
        }
        args::Command::VersionHelp => {
            println!("{}", args::VERSION_USAGE);
            return;
        }
        args::Command::Status => syswatch_core::protocol::RequestCommand::Status,
        args::Command::UpdateDismiss => syswatch_core::protocol::RequestCommand::Status,
        args::Command::Top { .. } => syswatch_core::protocol::RequestCommand::Processes,
        args::Command::Suggestions => syswatch_core::protocol::RequestCommand::Suggestions,
        args::Command::Inspect { .. } => syswatch_core::protocol::RequestCommand::Suggestions,
        args::Command::SuggestionsHelp => {
            println!("{}", args::SUGGESTIONS_USAGE);
            return;
        }
        args::Command::InspectHelp => {
            println!("{}", args::INSPECT_USAGE);
            return;
        }
        args::Command::Help => {
            println!("{}", args::USAGE);
            return;
        }
        args::Command::Version => {
            println!("{}", args::VERSION);
            return;
        }
        args::Command::TopHelp => {
            println!("{}", args::TOP_USAGE);
            return;
        }
    };
    let path = match syswatch_core::ipc::socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("wok: {error}");
            std::process::exit(1);
        }
    };
    let response = match client::request(&path, request_command, None, client::TIMEOUT) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("wok: {error}");
            std::process::exit(1);
        }
    };
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let rendered = match command {
        args::Command::Status => output::render_status(&response, now_ms),
        args::Command::UpdateDismiss => dismiss_update(&response),
        args::Command::Top { sort, limit } => output::render_top(&response, sort, limit, now_ms),
        args::Command::Suggestions => {
            // Best-effort name join; a failure only leaves names as <unknown>.
            let processes = matches!(
                response,
                syswatch_core::protocol::Response::Suggestions { .. }
            )
            .then(|| {
                client::request(
                    &path,
                    syswatch_core::protocol::RequestCommand::Processes,
                    None,
                    client::TIMEOUT,
                )
                .ok()
            })
            .flatten();
            output::render_suggestions(&response, processes.as_ref(), now_ms)
        }
        args::Command::Inspect { reference } => {
            output::render_inspect(&response, reference, now_ms)
        }
        args::Command::Help => return,
        args::Command::Version => return,
        args::Command::VersionHelp => return,
        args::Command::StatusHelp => return,
        args::Command::TopHelp => return,
        args::Command::SuggestionsHelp => return,
        args::Command::InspectHelp => return,
        args::Command::UpdateHelp => return,
        args::Command::UpdateDismissHelp => return,
        args::Command::Close { .. } | args::Command::CloseHelp => return,
    };
    match rendered {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("wok: {error}");
            std::process::exit(1);
        }
    }
}

/// Interactive close: preview, one line of confirmation, then one request.
/// Returns the exit code. Nothing is sent before the terminal check passes.
#[cfg(unix)]
fn run_close(reference: syswatch_core::findings::FindingId) -> i32 {
    use std::io::{BufRead, IsTerminal, Write};
    use syswatch_core::{
        preflight::PreflightConclusion,
        protocol::{RequestCommand, Response},
    };

    if !std::io::stdin().is_terminal() {
        eprintln!("wok: close requires an interactive terminal; no action was requested");
        return 1;
    }
    let path = match syswatch_core::ipc::socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("wok: {error}");
            return 1;
        }
    };
    let preview = match client::request(
        &path,
        RequestCommand::ClosePreview,
        Some(reference),
        client::TIMEOUT,
    ) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("wok: {error}");
            return 1;
        }
    };
    let text = match output::render_close_preview(&preview) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("wok: {error}");
            return 1;
        }
    };
    println!("{text}");
    let allowed = matches!(
        preview,
        Response::ClosePreview { ref assessment, .. }
            if assessment.conclusion == PreflightConclusion::AllowedForConfirmation
    );
    if !allowed {
        println!("aborted: no action was requested");
        return 1;
    }

    println!();
    println!(
        "Review the evidence first: wok inspect {}",
        reference.reference()
    );
    print!(
        "Type yes to request SIGTERM for {}: ",
        reference.reference()
    );
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line);
    if !(matches!(read, Ok(bytes) if bytes > 0) && confirmed(&line)) {
        // End of input leaves the prompt without a line break.
        if matches!(read, Ok(0)) {
            println!();
        }
        println!("aborted: no action was requested");
        return 1;
    }

    match client::request(
        &path,
        RequestCommand::CloseConfirm,
        Some(reference),
        client::TIMEOUT,
    ) {
        Ok(response) => match output::render_close_outcome(&response, reference) {
            Ok((text, code)) => {
                println!("{text}");
                code
            }
            Err(error) => {
                eprintln!("wok: {error}");
                1
            }
        },
        // Connect and encode fail before any byte is sent, so no hint for them.
        Err(error @ (client::ClientError::Connect(_) | client::ClientError::Encode(_))) => {
            eprintln!("wok: {error}");
            1
        }
        Err(error) => {
            eprintln!("wok: {error}");
            eprintln!(
                "wok: the daemon may have received the request; run `wok inspect {}` to check",
                reference.reference()
            );
            1
        }
    }
}

/// Accepts exactly `yes` after the line terminator is stripped.
#[cfg(unix)]
fn confirmed(line: &str) -> bool {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    line == "yes"
}

/// Writes the dismissal for the notice in a status reply and returns the line
/// to print. Without a notice it writes nothing and resolves no state path.
#[cfg(unix)]
fn dismiss_update(response: &syswatch_core::protocol::Response) -> Result<String, String> {
    let notice = match response {
        syswatch_core::protocol::Response::Status { update, .. } => update.as_ref(),
        _ => return Err("invalid response from daemon: expected status response".to_owned()),
    };
    let Some(notice) = notice else {
        return Ok(NO_UPDATE_TO_DISMISS.to_owned());
    };
    let version = syswatch_core::update::parse_tag(&notice.latest_version).ok_or_else(|| {
        format!(
            "invalid update version from daemon: {}",
            notice.latest_version
        )
    })?;
    let path = syswatch_core::update::state_path().map_err(|error| error.to_string())?;
    dismiss_at(&path, version)
}

/// Prints when the daemon reported no newer release.
#[cfg(unix)]
const NO_UPDATE_TO_DISMISS: &str = "no update to dismiss";

/// Writes one version to the state file and returns the confirmation line.
#[cfg(unix)]
fn dismiss_at(
    path: &std::path::Path,
    version: syswatch_core::update::Version,
) -> Result<String, String> {
    syswatch_core::update::save_state(path, &version).map_err(|error| error.to_string())?;
    Ok(format!("dismissed update notice for {version}"))
}

#[cfg(test)]
mod tests {
    use super::{confirmed, dismiss_at, dismiss_update};
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    use syswatch_core::{
        AggregateSnapshot, MemorySnapshot,
        protocol::{PROTOCOL_VERSION, Response},
        update::{STATE_VERSION_KEY, Version, load_state},
    };

    fn status_with(update: Option<syswatch_core::UpdateNotice>) -> Response {
        Response::Status {
            version: PROTOCOL_VERSION,
            aggregate: AggregateSnapshot {
                timestamp_ms: 0,
                cpu_usage_percent: 0.0,
                cpu_updated_at_ms: None,
                memory: MemorySnapshot {
                    total_bytes: 0,
                    used_bytes: 0,
                },
            },
            process_count: None,
            processes_updated_at_ms: None,
            build_version: None,
            update,
        }
    }

    fn notice(latest_version: &str) -> syswatch_core::UpdateNotice {
        syswatch_core::UpdateNotice {
            latest_version: latest_version.to_owned(),
            url: "https://example.invalid/releases".to_owned(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "syswatch-cli-dismiss-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir creates");
        dir
    }

    #[test]
    fn dismiss_without_a_notice_says_so() {
        assert_eq!(
            dismiss_update(&status_with(None)).unwrap(),
            "no update to dismiss"
        );
    }

    #[test]
    fn dismiss_rejects_an_unparseable_daemon_version() {
        let error = dismiss_update(&status_with(Some(notice("not-a-version")))).unwrap_err();
        assert!(
            error.contains("invalid update version from daemon: not-a-version"),
            "{error}"
        );
    }

    #[test]
    fn dismiss_writes_one_key_and_one_confirmation_line() {
        let dir = temp_dir("write");
        let path = dir.join("update.json");
        let version = Version {
            major: 1,
            minor: 2,
            patch: 3,
        };
        assert_eq!(
            dismiss_at(&path, version).unwrap(),
            "dismissed update notice for 1.2.3"
        );
        assert_eq!(load_state(&path), Some(version));
        let body = fs::read_to_string(&path).expect("state file is readable");
        let value: serde_json::Value = serde_json::from_str(&body).expect("state file is JSON");
        let object = value.as_object().expect("state file is an object");
        assert_eq!(object.len(), 1);
        assert_eq!(object.get(STATE_VERSION_KEY), Some(&"1.2.3".into()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dismiss_at_reports_an_unwritable_path() {
        let dir = temp_dir("unwritable");
        let blocking: &Path = &dir.join("file");
        fs::write(blocking, b"x").expect("blocking file writes");
        let error = dismiss_at(
            &blocking.join("update.json"),
            Version {
                major: 1,
                minor: 0,
                patch: 0,
            },
        )
        .unwrap_err();
        assert!(!error.is_empty(), "{error}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_exact_yes_confirms() {
        for line in ["yes\n", "yes", "yes\r\n"] {
            assert!(confirmed(line), "{line:?}");
        }
        for line in [
            "y\n", "YES\n", "\n", " yes\n", "yes \n", "", "yes\n\n", "no\n",
        ] {
            assert!(!confirmed(line), "{line:?}");
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("wok: unsupported platform");
    std::process::exit(1);
}
