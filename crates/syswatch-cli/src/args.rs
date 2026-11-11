pub const USAGE: &str = "Usage: syswatch <COMMAND>\n\nRead-only client for the syswatch daemon.\n\nCommands:\n  status  Show CPU, memory and process freshness from the running daemon\n  help    Show this help\n\nOptions:\n  -h, --help  Show this help";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Help,
}

pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let command = command.to_string_lossy();
    match command.as_ref() {
        "help" | "-h" | "--help" if args.next().is_none() => Ok(Command::Help),
        "status" if args.next().is_none() => Ok(Command::Status),
        "status" => Err(format!(
            "{}\n\nsyswatch: status accepts no arguments",
            USAGE
        )),
        other => Err(format!(
            "{}\n\nsyswatch: unrecognized command: {other}",
            USAGE
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, USAGE, parse};

    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn status_is_accepted() {
        assert_eq!(parse(args(&["status"])), Ok(Command::Status));
    }

    #[test]
    fn help_spellings_are_accepted() {
        for spelling in ["help", "-h", "--help"] {
            assert_eq!(parse(args(&[spelling])), Ok(Command::Help));
        }
    }

    #[test]
    fn missing_and_unknown_commands_show_usage() {
        assert!(parse(args(&[])).unwrap_err().contains(USAGE));
        assert!(
            parse(args(&["other"]))
                .unwrap_err()
                .contains("unrecognized command")
        );
    }

    #[test]
    fn status_rejects_extra_arguments() {
        assert!(
            parse(args(&["status", "extra"]))
                .unwrap_err()
                .contains("status accepts no arguments")
        );
    }
}
