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
    if command == args::Command::Help {
        println!("{}", args::USAGE);
        return;
    }
    let path = match syswatch_core::ipc::socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("syswatch: {error}");
            std::process::exit(1);
        }
    };
    let response = match client::request_status(&path, client::TIMEOUT) {
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
    match output::render_status(&response, now_ms) {
        Ok(status) => println!("{status}"),
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
