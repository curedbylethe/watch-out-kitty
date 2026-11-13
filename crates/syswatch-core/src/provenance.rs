//! Internal advisory process metadata. It is not part of `ProcessSnapshot`,
//! the read-only protocol, or any rendered report.
//!
//! `None` means denied or unavailable; platform APIs do not distinguish them,
//! so this module does not invent a reason. An identity absent from the latest
//! scan means exited or never observed; `ProcessHistory::disappeared_at_ms`
//! records exit (`history.rs:411`). A zero `start_time_secs` is unique only per
//! PID and is not stable across PID reuse. Such an identity is insufficient for
//! advisory findings and eligibility; the process stays in ordinary telemetry.
//! Full `argv` is not retained in
//! `ProcessProvenance` or serialized; the collector's system backend owns its
//! process data. Environment variables are not collected.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const MAX_PROGRAM_CHARS: usize = 64;

/// Best-effort executable, command, owner, and parent context for one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessProvenance {
    /// Operating system process identifier.
    pub pid: u32,
    /// Process start time in seconds since the Unix epoch.
    pub start_time_secs: u64,
    /// Full executable path when reported by the platform.
    pub executable: Option<PathBuf>,
    /// Lossy-decoded file name of `argv[0]`, capped at 64 characters.
    pub program: Option<String>,
    /// Numeric owning user ID on Unix.
    pub owner_uid: Option<u32>,
    /// Best-effort parent identity or context.
    pub parent: ParentContext,
}

impl ProcessProvenance {
    /// Returns the process identity pair because operating systems reuse PIDs.
    pub fn identity(&self) -> (u32, u64) {
        (self.pid, self.start_time_secs)
    }
}

/// Parent information, preserving the difference between identity and PID alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentContext {
    /// Parent was present in the same scan and its identity is known.
    Resolved { pid: u32, start_time_secs: u64 },
    /// Parent PID was reported, but parent was absent from the scan.
    PidOnly(u32),
    /// Operating system reported no parent.
    NoParent,
}

/// Returns a capped, lossy-decoded file name from `argv[0]`.
pub fn program_label(argv0: Option<&OsStr>) -> Option<String> {
    let filename = Path::new(argv0?).file_name()?;
    let label: String = filename
        .to_string_lossy()
        .chars()
        .take(MAX_PROGRAM_CHARS)
        .collect();
    (!label.is_empty()).then_some(label)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{ParentContext, ProcessProvenance, program_label};

    #[test]
    fn program_label_uses_the_file_name() {
        assert_eq!(
            program_label(Some(OsStr::new("/usr/bin/python3"))),
            Some("python3".into())
        );
    }

    #[test]
    fn program_label_accepts_a_bare_name() {
        assert_eq!(program_label(Some(OsStr::new("node"))), Some("node".into()));
    }

    #[test]
    fn program_label_is_none_for_empty_input() {
        assert_eq!(program_label(None), None);
        assert_eq!(program_label(Some(OsStr::new(""))), None);
    }

    #[test]
    fn program_label_is_capped() {
        let name = "a".repeat(80);
        assert_eq!(program_label(Some(OsStr::new(&name))).unwrap().len(), 64);
    }

    #[cfg(unix)]
    #[test]
    fn program_label_is_lossy_for_non_utf8() {
        use std::os::unix::ffi::OsStrExt;

        assert_eq!(
            program_label(Some(OsStr::from_bytes(b"/bin/a\xff"))),
            Some("a�".into())
        );
    }

    #[test]
    fn parent_context_distinguishes_missing_parent_from_resolved_identity() {
        assert_ne!(
            ParentContext::PidOnly(7),
            ParentContext::Resolved {
                pid: 7,
                start_time_secs: 2
            }
        );
        assert_ne!(ParentContext::PidOnly(7), ParentContext::NoParent);
    }

    #[test]
    fn identity_matches_the_record_key() {
        let process = ProcessProvenance {
            pid: 42,
            start_time_secs: 99,
            executable: None,
            program: None,
            owner_uid: None,
            parent: ParentContext::NoParent,
        };
        assert_eq!(process.identity(), (42, 99));
    }
}
