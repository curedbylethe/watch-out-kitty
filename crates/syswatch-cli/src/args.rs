use syswatch_core::report::ProcessSort;

pub const USAGE: &str = "Usage: syswatch <COMMAND>

Read-only client for the syswatch daemon.

Commands:
  status  Show CPU, memory and process freshness from the running daemon
  top     Show cached processes (--sort memory|cpu, default memory; --limit N, default 10)
  suggestions  Show cached advisory findings for review; never acts on a process
  help    Show this help

Options:
  -h, --help  Show this help";

pub const TOP_USAGE: &str = "Usage: syswatch top [--sort memory|cpu] [--limit N]

Options:
  --sort memory|cpu  Process order (default: memory)
  --limit N           Maximum rows, positive integer (default: 10)
  -h, --help          Show this help";

pub const SUGGESTIONS_USAGE: &str = "Usage: syswatch suggestions

Show cached advisory findings for review. Findings are advisory only;
this command never acts on a process.

Options:
  -h, --help  Show this help";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Top { sort: ProcessSort, limit: usize },
    TopHelp,
    Suggestions,
    SuggestionsHelp,
    Help,
}

fn usage_error(message: &str) -> String {
    format!("{USAGE}\n\nsyswatch: {message}")
}

pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let command = command.to_string_lossy();
    match command.as_ref() {
        "help" | "-h" | "--help" if args.next().is_none() => Ok(Command::Help),
        "help" | "-h" | "--help" => Err(usage_error(&format!("{command} accepts no arguments"))),
        "status" if args.next().is_none() => Ok(Command::Status),
        "status" => Err(usage_error("status accepts no arguments")),
        "top" => parse_top(args),
        "suggestions" => parse_suggestions(args),
        other => Err(usage_error(&format!("unrecognized command: {other}"))),
    }
}

fn parse_suggestions(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Command, String> {
    let mut args = args.into_iter();
    match args.next() {
        None => Ok(Command::Suggestions),
        Some(argument)
            if matches!(
                argument.to_string_lossy().as_ref(),
                "-h" | "--help" | "help"
            ) && args.next().is_none() =>
        {
            Ok(Command::SuggestionsHelp)
        }
        Some(_) => Err(usage_error("suggestions accepts no arguments")),
    }
}

fn parse_top(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut sort = ProcessSort::Memory;
    let mut limit = 10;
    let mut sort_set = false;
    let mut limit_set = false;

    while let Some(argument) = args.next() {
        match argument.to_string_lossy().as_ref() {
            "-h" | "--help" if args.next().is_none() => return Ok(Command::TopHelp),
            "--sort" if !sort_set => {
                sort = match args.next().as_deref().map(|arg| arg.to_string_lossy()) {
                    Some(value) if value == "memory" => ProcessSort::Memory,
                    Some(value) if value == "cpu" => ProcessSort::Cpu,
                    _ => return Err(usage_error("top --sort must be memory or cpu")),
                };
                sort_set = true;
            }
            "--limit" if !limit_set => {
                let value = args
                    .next()
                    .ok_or_else(|| usage_error("top --limit requires N"))?;
                let value = value.to_string_lossy();
                limit = value
                    .parse()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or_else(|| usage_error("top --limit must be a positive integer"))?;
                limit_set = true;
            }
            "-h" | "--help" => return Err(usage_error("top help accepts no arguments")),
            "--sort" => return Err(usage_error("top accepts --sort only once")),
            "--limit" => return Err(usage_error("top accepts --limit only once")),
            other => return Err(usage_error(&format!("unexpected top argument: {other}"))),
        }
    }
    Ok(Command::Top { sort, limit })
}

#[cfg(test)]
mod tests {
    use super::{Command, SUGGESTIONS_USAGE, TOP_USAGE, USAGE, parse};
    use syswatch_core::report::ProcessSort;

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

    #[test]
    fn help_rejects_extra_arguments_as_help_usage() {
        assert!(
            parse(args(&["help", "extra"]))
                .unwrap_err()
                .contains("help accepts no arguments")
        );
    }

    #[test]
    fn top_defaults_and_accepts_options_in_either_order() {
        assert_eq!(
            parse(args(&["top"])),
            Ok(Command::Top {
                sort: ProcessSort::Memory,
                limit: 10
            })
        );
        assert_eq!(
            parse(args(&["top", "--sort", "cpu"])),
            Ok(Command::Top {
                sort: ProcessSort::Cpu,
                limit: 10
            })
        );
        assert_eq!(
            parse(args(&["top", "--limit", "5"])),
            Ok(Command::Top {
                sort: ProcessSort::Memory,
                limit: 5
            })
        );
        assert_eq!(
            parse(args(&["top", "--limit", "5", "--sort", "cpu"])),
            Ok(Command::Top {
                sort: ProcessSort::Cpu,
                limit: 5
            })
        );
    }

    #[test]
    fn top_rejects_invalid_and_duplicate_options() {
        for values in [
            &["top", "--limit", "0"][..],
            &["top", "--limit", "-1"],
            &["top", "--limit", "x"],
            &["top", "--limit", "18446744073709551616"],
            &["top", "--sort", "disk"],
            &["top", "--wat"],
            &["top", "--sort", "cpu", "--sort", "memory"],
            &["top", "extra"],
        ] {
            assert!(parse(args(values)).is_err(), "accepted {values:?}");
        }
    }

    #[test]
    fn top_help_documents_flags_defaults_and_values() {
        assert_eq!(parse(args(&["top", "--help"])), Ok(Command::TopHelp));
        assert!(TOP_USAGE.contains("--sort memory|cpu"));
        assert!(TOP_USAGE.contains("default: memory"));
        assert!(TOP_USAGE.contains("default: 10"));
    }

    #[test]
    fn suggestions_is_accepted() {
        assert_eq!(parse(args(&["suggestions"])), Ok(Command::Suggestions));
    }

    #[test]
    fn suggestions_help_is_accepted() {
        for spelling in ["--help", "-h", "help"] {
            assert_eq!(
                parse(args(&["suggestions", spelling])),
                Ok(Command::SuggestionsHelp)
            );
        }
    }

    #[test]
    fn suggestions_rejects_extra_arguments() {
        for values in [&["suggestions", "--limit", "3"][..], &["suggestions", "42"]] {
            let error = parse(args(values)).unwrap_err();
            assert!(error.contains("suggestions accepts no arguments"));
            assert!(error.contains(USAGE), "usage missing from {error}");
        }
    }

    #[test]
    fn usage_lists_suggestions_without_action_flags() {
        assert!(USAGE.lines().any(|line| line.contains("suggestions")));
        assert!(SUGGESTIONS_USAGE.contains("--help"));
        assert!(!SUGGESTIONS_USAGE.contains("--limit"));
        assert!(!SUGGESTIONS_USAGE.contains("--sort"));
    }
}
