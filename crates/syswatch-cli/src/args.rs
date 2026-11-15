use syswatch_core::{
    findings::{FindingId, FindingKind},
    report::ProcessSort,
};

pub const USAGE: &str = "Usage: wok <COMMAND> [OPTIONS]

Client for the wok daemon. Only close can request a process action.

Commands:
  status         Show CPU, memory and process freshness from the running daemon
  top            Show cached processes, ordered by memory or CPU
  suggestions    Show cached advisory findings for review
  inspect        Explain one cached advisory finding
  close          Preview one finding, then request SIGTERM after you type yes
  update         Manage the update notice printed by status
  version        Print the version and exit
  help           Show this help

Options:
  -h, --help     Show this help
  -V, --version  Print the version and exit

Exit codes:
  0  Success
  1  Runtime failure, such as a missing daemon
  2  Usage error";

pub const STATUS_USAGE: &str = "Usage: wok status

Show CPU, memory and process freshness from the running daemon.

Options:
  -h, --help  Show this help";

pub const VERSION_USAGE: &str = "Usage: wok version

Print the version and exit.

Options:
  -h, --help  Show this help";

pub const VERSION: &str = concat!("wok ", env!("CARGO_PKG_VERSION"));

pub const TOP_USAGE: &str = "Usage: wok top [--sort memory|cpu] [--limit N]

Show cached processes, ordered by memory or CPU.

Options:
  --sort memory|cpu  Process order (default: memory)
  --limit N          Maximum rows, positive integer (default: 10)
  -h, --help         Show this help";

pub const SUGGESTIONS_USAGE: &str = "Usage: wok suggestions

Show cached advisory findings for review. Findings are advisory only;
this command never acts on a process.

Options:
  -h, --help  Show this help";

pub const CLOSE_USAGE: &str = "Usage: wok close <finding-reference>

Preview one advisory finding, then request one SIGTERM after you type yes.
Needs an interactive terminal. A requested signal is not a confirmed exit.

Arguments:
  <finding-reference>  pid-start_time_secs-kind, for example 42-1700000000-memory

Options:
  -h, --help           Show this help";

pub const INSPECT_USAGE: &str = "Usage: wok inspect <finding-reference>

Print the cached evidence for one advisory finding.

Arguments:
  <finding-reference>  pid-start_time_secs-kind, for example 42-1700000000-memory

Options:
  -h, --help           Show this help";

pub const UPDATE_USAGE: &str = "Usage: wok update <COMMAND>

Sub-commands for the update notice printed by status. Dismissal is durable
across restarts until a newer version appears.

Commands:
  dismiss  Silence the current update notice

Options:
  -h, --help  Show this help";

pub const UPDATE_DISMISS_USAGE: &str = "Usage: wok update dismiss

Silence the current update notice. The dismissal is durable across restarts
until a newer version appears.

Options:
  -h, --help  Show this help";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    StatusHelp,
    Top { sort: ProcessSort, limit: usize },
    TopHelp,
    Suggestions,
    SuggestionsHelp,
    Inspect { reference: FindingId },
    InspectHelp,
    Close { reference: FindingId },
    CloseHelp,
    UpdateDismiss,
    UpdateDismissHelp,
    UpdateHelp,
    Version,
    VersionHelp,
    Help,
}

fn usage_error(message: &str) -> String {
    format!("{USAGE}\n\nwok: {message}")
}

fn close_usage_error(message: &str) -> String {
    format!("{CLOSE_USAGE}\n\nwok: {message}")
}

fn update_usage_error(message: &str) -> String {
    format!("{UPDATE_USAGE}\n\nwok: {message}")
}

pub fn parse(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let command = command.to_string_lossy();
    match command.as_ref() {
        "help" | "-h" | "--help" => parse_help(&command, args),
        "status" => match args.next() {
            None => Ok(Command::Status),
            Some(argument)
                if matches!(argument.to_string_lossy().as_ref(), "-h" | "--help")
                    && args.next().is_none() =>
            {
                Ok(Command::StatusHelp)
            }
            Some(_) => Err(usage_error("status accepts no arguments")),
        },
        "version" => match args.next() {
            None => Ok(Command::Version),
            Some(argument)
                if matches!(argument.to_string_lossy().as_ref(), "-h" | "--help")
                    && args.next().is_none() =>
            {
                Ok(Command::VersionHelp)
            }
            Some(_) => Err(usage_error("version accepts no arguments")),
        },
        "-V" | "--version" if args.next().is_none() => Ok(Command::Version),
        "-V" | "--version" => Err(usage_error("version accepts no arguments")),
        "top" => parse_top(args),
        "suggestions" => parse_suggestions(args),
        "inspect" => parse_inspect(args),
        "close" => parse_close(args),
        "update" => parse_update(args),
        other => Err(usage_error(&format!("unrecognized command: {other}"))),
    }
}

fn parse_update(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Err(update_usage_error("update requires a sub-command"));
    };
    let first = first.to_string_lossy();
    match first.as_ref() {
        "-h" | "--help" if args.next().is_none() => Ok(Command::UpdateHelp),
        "-h" | "--help" => Err(update_usage_error("update help accepts no arguments")),
        "dismiss" => match args.next() {
            None => Ok(Command::UpdateDismiss),
            Some(argument)
                if matches!(argument.to_string_lossy().as_ref(), "-h" | "--help")
                    && args.next().is_none() =>
            {
                Ok(Command::UpdateDismissHelp)
            }
            Some(_) => Err(update_usage_error("update dismiss accepts no arguments")),
        },
        other => Err(update_usage_error(&format!(
            "unrecognized update sub-command: {other}"
        ))),
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
    let Some(topic) = args.next() else {
        return Ok(Command::Help);
    };
    if command != "help" {
        return Err(usage_error(&format!("{command} accepts no arguments")));
    }
    if args.next().is_some() {
        return Err(usage_error(
            "help accepts one command; expected help [<COMMAND>]",
        ));
    }
    match topic.to_string_lossy().as_ref() {
        "status" => Ok(Command::StatusHelp),
        "top" => Ok(Command::TopHelp),
        "suggestions" => Ok(Command::SuggestionsHelp),
        "inspect" => Ok(Command::InspectHelp),
        "close" => Ok(Command::CloseHelp),
        "update" => Ok(Command::UpdateHelp),
        "version" => Ok(Command::VersionHelp),
        "help" | "-h" | "--help" => Ok(Command::Help),
        other => Err(usage_error(&format!(
            "unrecognized help topic: {other}; expected help [<COMMAND>]"
        ))),
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

/// Parses `<pid>-<start_time_secs>-<kind>`, the form `wok suggestions`
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
            if matches!(argument.to_string_lossy().as_ref(), "-h" | "--help")
                && args.next().is_none() =>
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
    use super::{
        CLOSE_USAGE, Command, INSPECT_USAGE, STATUS_USAGE, SUGGESTIONS_USAGE, TOP_USAGE,
        UPDATE_DISMISS_USAGE, UPDATE_USAGE, USAGE, VERSION, VERSION_USAGE, parse,
    };
    const USAGE_MARKER: &str = "wok:";
    /// Every block a help spelling can print, top-level first.
    const BLOCKS: [&str; 9] = [
        USAGE,
        STATUS_USAGE,
        TOP_USAGE,
        SUGGESTIONS_USAGE,
        INSPECT_USAGE,
        CLOSE_USAGE,
        UPDATE_USAGE,
        UPDATE_DISMISS_USAGE,
        VERSION_USAGE,
    ];
    use syswatch_core::{
        findings::{FindingId, FindingKind},
        report::ProcessSort,
    };

    fn args(values: &[&str]) -> Vec<std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from).collect()
    }

    /// Splits a help block into `(header, indented rows)` pairs. A header is a
    /// column-zero line ending in `:`; prose paragraphs are skipped.
    fn sections(block: &str) -> Vec<(&str, Vec<&str>)> {
        let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
        for line in block.lines() {
            if line.starts_with("  ") {
                let (_, rows) = sections.last_mut().expect("indented row outside a section");
                rows.push(line);
            } else if line.ends_with(':') {
                sections.push((line, Vec::new()));
            }
        }
        sections
    }

    /// Rows of one section, empty when the block has no such section.
    fn section<'a>(block: &'a str, header: &str) -> Vec<&'a str> {
        sections(block)
            .into_iter()
            .find(|(name, _)| *name == header)
            .map(|(_, rows)| rows)
            .unwrap_or_default()
    }

    /// Index where the description column starts: the first run of two or more
    /// spaces after the two-space indent and the command or flag name.
    fn description_column(row: &str) -> usize {
        let bytes = row.as_bytes();
        assert!(
            bytes.len() > 3 && bytes[0] == b' ' && bytes[1] == b' ' && bytes[2] != b' ',
            "row must start with exactly two spaces: {row:?}"
        );
        let mut index = 2;
        while index + 1 < bytes.len() {
            if bytes[index] == b' ' && bytes[index + 1] == b' ' {
                break;
            }
            index += 1;
        }
        while index < bytes.len() && bytes[index] == b' ' {
            index += 1;
        }
        assert!(index < bytes.len(), "row has no description: {row:?}");
        index
    }

    #[test]
    fn every_command_prints_its_own_block() {
        for (values, expected) in [
            (&["status", "-h"][..], Command::StatusHelp),
            (&["status", "--help"], Command::StatusHelp),
            (&["top", "-h"], Command::TopHelp),
            (&["top", "--help"], Command::TopHelp),
            (&["suggestions", "-h"], Command::SuggestionsHelp),
            (&["suggestions", "--help"], Command::SuggestionsHelp),
            (&["inspect", "-h"], Command::InspectHelp),
            (&["inspect", "--help"], Command::InspectHelp),
            (&["close", "-h"], Command::CloseHelp),
            (&["close", "--help"], Command::CloseHelp),
            (&["update", "-h"], Command::UpdateHelp),
            (&["update", "--help"], Command::UpdateHelp),
            (&["update", "dismiss", "-h"], Command::UpdateDismissHelp),
            (&["update", "dismiss", "--help"], Command::UpdateDismissHelp),
            (&["version", "-h"], Command::VersionHelp),
            (&["version", "--help"], Command::VersionHelp),
        ] {
            assert_eq!(parse(args(values)), Ok(expected), "{values:?}");
        }
    }

    #[test]
    fn every_block_documents_its_flags() {
        let blocks: [(&str, &[&str]); 9] = [
            (USAGE, &["-h, --help", "-V, --version"]),
            (STATUS_USAGE, &["-h, --help"]),
            (TOP_USAGE, &["--sort memory|cpu", "--limit N", "-h, --help"]),
            (SUGGESTIONS_USAGE, &["-h, --help"]),
            (INSPECT_USAGE, &["-h, --help"]),
            (CLOSE_USAGE, &["-h, --help"]),
            (UPDATE_USAGE, &["-h, --help"]),
            (UPDATE_DISMISS_USAGE, &["-h, --help"]),
            (VERSION_USAGE, &["-h, --help"]),
        ];
        for (block, flags) in blocks {
            for flag in flags {
                assert!(block.contains(flag), "{flag:?} missing from {block}");
            }
        }
    }

    #[test]
    fn reference_commands_document_their_argument() {
        for block in [INSPECT_USAGE, CLOSE_USAGE] {
            assert!(
                section(block, "Arguments:")
                    .iter()
                    .any(|row| row.contains("<finding-reference>")),
                "{block}"
            );
        }
        for block in [
            USAGE,
            STATUS_USAGE,
            TOP_USAGE,
            SUGGESTIONS_USAGE,
            UPDATE_USAGE,
            UPDATE_DISMISS_USAGE,
            VERSION_USAGE,
        ] {
            assert!(!block.contains("Arguments:"), "{block}");
        }
    }

    #[test]
    fn every_synopsis_follows_one_shape() {
        for block in BLOCKS {
            let synopsis = block.lines().next().expect("block is not empty");
            assert!(synopsis.starts_with("Usage: wok "), "{synopsis:?}");
            let description = block.lines().nth(2).expect("block has a description line");
            assert!(!description.is_empty(), "{block}");
            assert!(!description.ends_with(':'), "no description in {block}");
            for row in section(block, "Commands:") {
                let name = row.split_whitespace().next().expect("row names a command");
                assert!(!name.starts_with('-'), "{row:?}");
                assert!(!row.contains("--"), "flag detail in command row {row:?}");
            }
        }
    }

    #[test]
    fn every_section_row_is_aligned() {
        for block in BLOCKS {
            let all = sections(block);
            assert!(!all.is_empty(), "{block}");
            for (header, rows) in all {
                let column = description_column(rows[0]);
                for row in rows {
                    assert!(row.starts_with("  "), "{row:?}");
                    assert_eq!(
                        description_column(row),
                        column,
                        "misaligned row {row:?} in {header}"
                    );
                }
            }
        }
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
        for values in [&["status", "extra"][..], &["status", "--help", "extra"]] {
            assert!(
                parse(args(values))
                    .unwrap_err()
                    .contains("status accepts no arguments"),
                "{values:?}"
            );
        }
    }

    #[test]
    fn version_spellings_are_accepted() {
        for spelling in ["version", "-V", "--version"] {
            assert_eq!(parse(args(&[spelling])), Ok(Command::Version), "{spelling}");
        }
    }

    #[test]
    fn version_rejects_extra_arguments() {
        for values in [
            &["version", "extra"][..],
            &["version", "--help", "extra"],
            &["-V", "extra"],
            &["--version", "extra"],
        ] {
            let error = parse(args(values)).unwrap_err();
            assert!(error.contains("version accepts no arguments"), "{error}");
            assert!(error.contains(USAGE), "usage missing from {error}");
        }
    }

    #[test]
    fn version_text_carries_the_crate_version() {
        assert_eq!(
            VERSION.strip_prefix("wok "),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn lowercase_v_is_an_unknown_argument() {
        let error = parse(args(&["-v"])).unwrap_err();
        assert!(error.contains("unrecognized command: -v"), "{error}");
        assert!(error.contains(USAGE), "usage missing from {error}");
    }

    #[test]
    fn no_block_documents_lowercase_v() {
        for block in BLOCKS {
            assert!(!block.contains("-v,"), "{block}");
        }
    }

    #[test]
    fn help_rejects_an_unknown_topic_or_a_second_argument() {
        let error = parse(args(&["help", "extra"])).unwrap_err();
        assert!(error.contains("unrecognized help topic: extra"), "{error}");
        assert!(error.contains("help [<COMMAND>]"), "{error}");
        let error = parse(args(&["help", "status", "extra"])).unwrap_err();
        assert!(error.contains("help [<COMMAND>]"), "{error}");
    }

    #[test]
    fn help_answers_both_help_spellings() {
        for values in [&["help", "-h"][..], &["help", "--help"]] {
            assert_eq!(parse(args(values)), Ok(Command::Help), "{values:?}");
        }
    }

    #[test]
    fn help_topic_resolves_every_command() {
        for (topic, expected) in [
            ("status", Command::StatusHelp),
            ("top", Command::TopHelp),
            ("suggestions", Command::SuggestionsHelp),
            ("inspect", Command::InspectHelp),
            ("close", Command::CloseHelp),
            ("update", Command::UpdateHelp),
            ("version", Command::VersionHelp),
            ("help", Command::Help),
        ] {
            assert_eq!(parse(args(&["help", topic])), Ok(expected), "{topic}");
        }
    }

    #[test]
    fn the_bare_word_help_is_rejected_after_every_command() {
        for values in [
            &["status", "help"][..],
            &["top", "help"],
            &["suggestions", "help"],
            &["inspect", "help"],
            &["close", "help"],
            &["version", "help"],
            &["update", "dismiss", "help"],
        ] {
            let error = parse(args(values)).unwrap_err();
            assert!(error.contains(USAGE_MARKER), "{values:?}: {error}");
        }
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
        for spelling in ["--help", "-h"] {
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
    fn update_dismiss_is_accepted_and_listed_in_usage() {
        assert_eq!(
            parse(args(&["update", "dismiss"])),
            Ok(Command::UpdateDismiss)
        );
        assert!(
            section(USAGE, "Commands:")
                .iter()
                .any(|row| row.split_whitespace().next() == Some("update")),
            "{USAGE}"
        );
        assert!(UPDATE_USAGE.contains("Usage: wok update <COMMAND>"));
        assert!(
            section(UPDATE_USAGE, "Commands:")
                .iter()
                .any(|row| row.split_whitespace().next() == Some("dismiss")),
            "{UPDATE_USAGE}"
        );
        assert_eq!(
            parse(args(&["update", "dismiss", "--help"])),
            Ok(Command::UpdateDismissHelp)
        );
    }

    #[test]
    fn update_help_is_recognized() {
        for spelling in ["-h", "--help"] {
            assert_eq!(parse(args(&["update", spelling])), Ok(Command::UpdateHelp));
        }
    }

    #[test]
    fn update_rejects_missing_extra_and_unknown_sub_commands() {
        let error = parse(args(&["update"])).unwrap_err();
        assert!(error.contains(UPDATE_USAGE), "{error}");
        assert!(error.contains("update requires a sub-command"), "{error}");

        for values in [
            &["update", "dismiss", "extra"][..],
            &["update", "dismiss", "--help", "extra"],
            &["update", "--help", "dismiss"],
            &["update", "check"],
        ] {
            let error = parse(args(values)).unwrap_err();
            assert!(error.contains(UPDATE_USAGE), "{values:?}: {error}");
        }
        assert!(
            parse(args(&["update", "check"]))
                .unwrap_err()
                .contains("unrecognized update sub-command: check")
        );
        assert!(
            parse(args(&["update", "dismiss", "extra"]))
                .unwrap_err()
                .contains("update dismiss accepts no arguments")
        );
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
