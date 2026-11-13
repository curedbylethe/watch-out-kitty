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
        args::Command::Status => syswatch_core::protocol::RequestCommand::Status,
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
        args::Command::TopHelp => {
            println!("{}", args::TOP_USAGE);
            return;
        }
    };
    let path = match syswatch_core::ipc::socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("syswatch: {error}");
            std::process::exit(1);
        }
    };
    let response = match client::request(&path, request_command, None, client::TIMEOUT) {
        Ok(response) => response,
        Err(error) => {
            eprintln!("syswatch: {error}");
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
        args::Command::TopHelp => return,
        args::Command::SuggestionsHelp => return,
        args::Command::InspectHelp => return,
        args::Command::Close { .. } | args::Command::CloseHelp => return,
    };
    match rendered {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("syswatch: {error}");
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
        eprintln!("syswatch: close requires an interactive terminal; no action was requested");
        return 1;
    }
    let path = match syswatch_core::ipc::socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("syswatch: {error}");
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
            eprintln!("syswatch: {error}");
            return 1;
        }
    };
    let text = match output::render_close_preview(&preview) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("syswatch: {error}");
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
        "Review the evidence first: syswatch inspect {}",
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
                eprintln!("syswatch: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("syswatch: {error}");
            eprintln!(
                "syswatch: the daemon may have received the request; run `syswatch inspect {}` to check",
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

#[cfg(test)]
mod tests {
    use super::confirmed;

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
    eprintln!("syswatch: unsupported platform");
    std::process::exit(1);
}
