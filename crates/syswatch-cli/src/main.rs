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
        args::Command::Status => syswatch_core::protocol::RequestCommand::Status,
        args::Command::Top { .. } => syswatch_core::protocol::RequestCommand::Processes,
        args::Command::Suggestions => syswatch_core::protocol::RequestCommand::Suggestions,
        args::Command::SuggestionsHelp => {
            println!("{}", args::SUGGESTIONS_USAGE);
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
    let response = match client::request(&path, request_command, client::TIMEOUT) {
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
                    client::TIMEOUT,
                )
                .ok()
            })
            .flatten();
            output::render_suggestions(&response, processes.as_ref(), now_ms)
        }
        args::Command::Help => return,
        args::Command::TopHelp => return,
        args::Command::SuggestionsHelp => return,
    };
    match rendered {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("syswatch: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("syswatch: unsupported platform");
    std::process::exit(1);
}
