use syswatch_core::{
    findings::{FindingId, FindingKind},
    report::ProcessSort,
};

pub const USAGE: &str = "Usage: syswatch <COMMAND>

Client for the syswatch daemon. Only close can request a process action.

Commands:
  status  Show CPU, memory and process freshness from the running daemon
  top     Show cached processes (--sort memory|cpu, default memory; --limit N, default 10)
  suggestions  Show cached advisory findings for review; never acts on a process
  inspect  Explain one cached advisory finding (pid-start_time_secs-kind)
  close  Preview one finding, then request SIGTERM after you type yes
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

pub const CLOSE_USAGE: &str = "Usage: syswatch close <finding-reference>

Preview one advisory finding, then request one SIGTERM after you type yes.
Needs an interactive terminal. A requested signal is not a confirmed exit.

Arguments:
  <finding-reference>  pid-start_time_secs-kind, for example 42-1700000000-memory

Options:
  -h, --help           Show this help";

pub const INSPECT_USAGE: &str = "Usage: syswatch inspect <finding-reference>

Print the cached evidence for one advisory finding.

Arguments:
  <finding-reference>  pid-start_time_secs-kind, for example 42-1700000000-memory

Options:
  -h, --help           Show this help";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Top { sort: ProcessSort, limit: usize },
    TopHelp,
    Suggestions,
    SuggestionsHelp,
    Inspect { reference: FindingId },
    InspectHelp,
    Close { reference: FindingId },
    CloseHelp,
    Help,
}

fn usage_error(message: &str) -> String {
    format!("{USAGE}\n\nsyswatch: {message}")
}

fn close_usage_error(message: &str) -> String {
    format!("{CLOSE_USAGE}\n\nsyswatch: {message}")
}

pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let command = command.to_string_lossy();
    match command.as_ref() {
        "help" | "-h" | "--help" => parse_help(&command, args),
        "status" if args.next().is_none() => Ok(Command::Status),
        "status" => Err(usage_error("status accepts no arguments")),
        "top" => parse_top(args),
        "suggestions" => parse_suggestions(args),
        "inspect" => parse_inspect(args),
        "close" => parse_close(args),
        other => Err(usage_error(&format!("unrecognized command: {other}"))),
    }
}

fn parse_inspect(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Err(usage_error("inspect requires a finding reference"));
    };
    let first = first.to_string_lossy();
    if matches!(first.as_ref(), "-h" | "--help") {
        return match args.next() {
            None => Ok(Command::InspectHelp),
            Some(_) => Err(usage_error("inspect help accepts no arguments")),
        };
    }
    if args.next().is_some() {
        return Err(usage_error("inspect accepts one finding reference"));
    }
    parse_reference(&first)
        .map(|reference| Command::Inspect { reference })
        .map_err(|message| usage_error(&message))
}

fn parse_help(
    command: &str,
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Command, String> {
    let mut args = args.into_iter();
    match args.next() {
        None => Ok(Command::Help),
        Some(topic) if command == "help" && topic == "close" && args.next().is_none() => {
            Ok(Command::CloseHelp)
        }
        Some(_) => Err(usage_error(&format!("{command} accepts no arguments"))),
    }
}

fn parse_close(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Err(close_usage_error("close requires a finding reference"));
    };
    let first = first.to_string_lossy();
    if matches!(first.as_ref(), "-h" | "--help") {
        return match args.next() {
            None => Ok(Command::CloseHelp),
            Some(_) => Err(close_usage_error("close help accepts no arguments")),
        };
    }
    if args.next().is_some() {
        return Err(close_usage_error("close accepts one finding reference"));
    }
    parse_reference(&first)
        .map(|reference| Command::Close { reference })
        .map_err(|message| close_usage_error(&message))
}

/// Parses `<pid>-<start_time_secs>-<kind>`, the form `syswatch suggestions`
/// prints. Fields are checked in order; the first failure wins.
fn parse_reference(value: &str) -> Result<FindingId, String> {
    const SHAPE: &str = "invalid finding reference: expected pid-start_time_secs-kind";
    let fields: Vec<&str> = value.split('-').collect();
    let [pid, start, kind] = fields.as_slice() else {
        return Err(SHAPE.to_owned());
    };
    if fields.iter().any(|field| field.is_empty()) {
        return Err(SHAPE.to_owned());
    }
    let pid =
        parse_digits::<u32>(pid).ok_or("invalid finding reference: pid must be an integer")?;
    let start_time_secs = parse_digits::<u64>(start)
        .ok_or("invalid finding reference: start_time_secs must be an integer")?;
    let kind = match *kind {
        "cpu" => FindingKind::Cpu,
        "memory" => FindingKind::Memory,
        _ => return Err("invalid finding reference: kind must be cpu or memory".to_owned()),
    };
    Ok(FindingId {
        pid,
        start_time_secs,
        kind,
    })
}

/// Rejects signs and whitespace, which `str::parse` would otherwise accept.
fn parse_digits<T: std::str::FromStr>(field: &str) -> Option<T> {
    if field.bytes().all(|byte| byte.is_ascii_digit()) {
        field.parse().ok()
    } else {
        None
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
    use super::{CLOSE_USAGE, Command, INSPECT_USAGE, SUGGESTIONS_USAGE, TOP_USAGE, USAGE, parse};
    const USAGE_MARKER: &str = "syswatch:";
    use syswatch_core::{
        findings::{FindingId, FindingKind},
        report::ProcessSort,
    };

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

    fn memory_reference(pid: u32, start_time_secs: u64) -> FindingId {
        FindingId {
            pid,
            start_time_secs,
            kind: FindingKind::Memory,
        }
    }

    #[test]
    fn inspect_is_accepted() {
        assert_eq!(
            parse(args(&["inspect", "42-1700000000-memory"])),
            Ok(Command::Inspect {
                reference: memory_reference(42, 1_700_000_000)
            })
        );
    }

    #[test]
    fn inspect_accepts_leading_zeros() {
        assert_eq!(
            parse(args(&["inspect", "042-0005-cpu"])),
            Ok(Command::Inspect {
                reference: FindingId {
                    pid: 42,
                    start_time_secs: 5,
                    kind: FindingKind::Cpu,
                }
            })
        );
    }

    #[test]
    fn close_accepts_one_complete_reference() {
        assert_eq!(
            parse(args(&["close", "42-1-memory"])),
            Ok(Command::Close {
                reference: FindingId {
                    pid: 42,
                    start_time_secs: 1,
                    kind: FindingKind::Memory,
                },
            })
        );
    }

    #[test]
    fn close_rejects_selectors_and_flags_as_usage_errors() {
        for values in [
            &["close"][..],
            &["close", "42"],
            &["close", "42-1"],
            &["close", "42-1-cpu", "extra"],
            &["close", "--yes"],
            &["close", "1"],
        ] {
            let error = parse(args(values)).unwrap_err();
            assert!(error.contains(USAGE_MARKER), "{values:?}: {error}");
        }
    }

    #[test]
    fn close_help_is_recognized_by_flag_and_topic() {
        assert_eq!(parse(args(&["close", "--help"])), Ok(Command::CloseHelp));
        assert_eq!(parse(args(&["close", "-h"])), Ok(Command::CloseHelp));
        assert_eq!(parse(args(&["help", "close"])), Ok(Command::CloseHelp));
        assert!(CLOSE_USAGE.contains("<finding-reference>"));
        assert!(CLOSE_USAGE.contains("yes"));
    }

    #[test]
    fn inspect_help_is_recognized() {
        for spelling in ["-h", "--help"] {
            assert_eq!(
                parse(args(&["inspect", spelling])),
                Ok(Command::InspectHelp)
            );
        }
        let error = parse(args(&["inspect", "--help", "42-5-memory"])).unwrap_err();
        assert!(error.contains("inspect help accepts no arguments"));
        assert!(INSPECT_USAGE.contains("pid-start_time_secs-kind"));
    }

    #[test]
    fn inspect_accepts_every_printed_reference() {
        for kind in [FindingKind::Cpu, FindingKind::Memory] {
            let id = FindingId {
                pid: 42,
                start_time_secs: 1_700_000_000,
                kind,
            };
            assert_eq!(
                parse(args(&["inspect", &id.reference()])),
                Ok(Command::Inspect { reference: id })
            );
        }
    }

    #[test]
    fn inspect_requires_exactly_one_reference() {
        assert!(
            parse(args(&["inspect"]))
                .unwrap_err()
                .contains("inspect requires a finding reference")
        );
        assert!(
            parse(args(&["inspect", "42-5-memory", "43-5-memory"]))
                .unwrap_err()
                .contains("inspect accepts one finding reference")
        );
    }

    #[test]
    fn inspect_rejects_invalid_references() {
        const SHAPE: &str = "invalid finding reference: expected pid-start_time_secs-kind";
        let cases = [
            ("", SHAPE),
            ("42", SHAPE),
            ("42-5", SHAPE),
            ("42-5-memory-1", SHAPE),
            ("--", SHAPE),
            (
                "x-5-memory",
                "invalid finding reference: pid must be an integer",
            ),
            (
                "+42-5-memory",
                "invalid finding reference: pid must be an integer",
            ),
            (
                " 42-5-memory",
                "invalid finding reference: pid must be an integer",
            ),
            (
                "4294967296-5-memory",
                "invalid finding reference: pid must be an integer",
            ),
            (
                "42-18446744073709551616-memory",
                "invalid finding reference: start_time_secs must be an integer",
            ),
            (
                "42-5-disk",
                "invalid finding reference: kind must be cpu or memory",
            ),
            (
                "42-5-Memory",
                "invalid finding reference: kind must be cpu or memory",
            ),
        ];
        for (value, message) in cases {
            let error = parse(args(&["inspect", value])).unwrap_err();
            assert!(error.contains(message), "{value:?} gave {error}");
            assert!(error.contains(USAGE), "usage missing for {value:?}");
        }
    }

    #[test]
    fn usage_lists_inspect() {
        assert!(USAGE.lines().any(|line| line.contains("inspect")));
    }

    #[test]
    fn usage_lists_suggestions_without_action_flags() {
        assert!(USAGE.lines().any(|line| line.contains("suggestions")));
        assert!(SUGGESTIONS_USAGE.contains("--help"));
        assert!(!SUGGESTIONS_USAGE.contains("--limit"));
        assert!(!SUGGESTIONS_USAGE.contains("--sort"));
    }
}
