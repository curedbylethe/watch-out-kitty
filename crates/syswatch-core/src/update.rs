//! Update notice: release feed parsing, one fixed `curl` attempt, and the
//! bounded dismissal state.
//!
//! The check is read-only and offline-tolerant: every failure path returns
//! `None`, nothing is printed, and one call performs exactly one attempt.

use std::{
    env,
    ffi::OsStr,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// Release feed read by the update check.
pub const UPDATE_FEED_URL: &str =
    "https://api.github.com/repos/curedbylethe/watch-out-kitty/releases/latest";

/// Set to any non-empty value to disable the update check.
pub const UPDATE_DISABLE_ENV: &str = "SYSWATCH_DISABLE_UPDATE_CHECK";

/// Absolute path override for the update state file.
pub const UPDATE_STATE_ENV: &str = "SYSWATCH_UPDATE_STATE";

/// File name of the update state file.
pub const UPDATE_STATE_FILE_NAME: &str = "update.json";

/// Only key accepted in the update state file.
pub const STATE_VERSION_KEY: &str = "version";

/// Maximum accepted feed body size in bytes.
const MAX_FEED_BYTES: usize = 64 * 1024;

/// Maximum accepted state file size in bytes.
const MAX_STATE_BYTES: usize = 1024;

/// Fixed curl arguments: no cookie, token, query string or machine data.
const CURL_ARGS: [&str; 8] = [
    "-fsS",
    "--max-time",
    "3",
    "-A",
    "syswatch",
    "-H",
    "Accept: application/vnd.github+json",
    UPDATE_FEED_URL,
];

/// A notice at or below a dismissed version stays hidden.
pub fn is_dismissed(dismissed: Option<Version>, noticed: Version) -> bool {
    dismissed.is_some_and(|version| version >= noticed)
}

/// One parsed semantic version. Ordering is numeric per component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// Major version.
    pub major: u64,
    /// Minor version.
    pub minor: u64,
    /// Patch version.
    pub patch: u64,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parses a release tag. Accepts `vMAJOR.MINOR.PATCH` and `MAJOR.MINOR.PATCH`.
// shortcut: vMAJOR.MINOR.PATCH only; add pre-release ordering when a pre-release tag is published
pub fn parse_tag(tag: &str) -> Option<Version> {
    let body = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = body.split('.');
    let major = numeric(parts.next()?)?;
    let minor = numeric(parts.next()?)?;
    let patch = numeric(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(Version {
        major,
        minor,
        patch,
    })
}

fn numeric(part: &str) -> Option<u64> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

/// One release newer than the running version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Parsed release version.
    pub version: Version,
    /// Release page URL from the feed.
    pub url: String,
}

/// Parses a release feed body. A missing field, a wrong JSON type, an empty
/// body or a body above [`MAX_FEED_BYTES`] yields `None`.
pub fn parse_release(body: &str) -> Option<Release> {
    if body.is_empty() || body.len() > MAX_FEED_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let version = parse_tag(value.get("tag_name")?.as_str()?)?;
    let url = value.get("html_url")?.as_str()?.to_owned();
    Some(Release { version, url })
}

/// Runs one feed request. The implementation owns every failure decision.
pub trait CommandRunner {
    /// Runs the request with `args` and returns its body, or `None` on any
    /// failure.
    fn run(&self, args: &[&str]) -> Option<String>;
}

/// Production runner: one `curl` subprocess, no shell.
#[derive(Debug, Default, Clone, Copy)]
pub struct CurlRunner;

impl CommandRunner for CurlRunner {
    fn run(&self, args: &[&str]) -> Option<String> {
        let output = Command::new("curl").args(args).output().ok()?;
        if !output.status.success() || output.stdout.is_empty() {
            return None;
        }
        String::from_utf8(output.stdout).ok()
    }
}

/// Reads the feed once and returns a release only when it is strictly newer
/// than `current_version`. An unparseable running version yields `None`.
pub fn check<R: CommandRunner + ?Sized>(runner: &R, current_version: &str) -> Option<Release> {
    let current = parse_tag(current_version)?;
    let release = parse_release(&runner.run(&CURL_ARGS)?)?;
    (release.version > current).then_some(release)
}

/// Reports whether the update check is switched off. Any non-empty value of
/// [`UPDATE_DISABLE_ENV`] disables it.
pub fn disabled() -> bool {
    env::var_os(UPDATE_DISABLE_ENV).is_some_and(|value| !value.is_empty())
}

#[derive(Debug, PartialEq, Eq)]
pub enum StatePathError {
    NotConfigured,
    RelativePath(PathBuf),
}

impl fmt::Display for StatePathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(
                f,
                "no update state path configured; set SYSWATCH_UPDATE_STATE to an absolute path, or XDG_STATE_HOME, or HOME"
            ),
            Self::RelativePath(path) => {
                write!(f, "update state path must be absolute: {}", path.display())
            }
        }
    }
}

impl std::error::Error for StatePathError {}

/// Resolves the update state file path.
pub fn state_path() -> Result<PathBuf, StatePathError> {
    resolve_state_path(
        env::var_os(UPDATE_STATE_ENV).as_deref(),
        env::var_os("XDG_STATE_HOME").as_deref(),
        env::var_os("HOME").as_deref(),
    )
}

/// Resolves the state path from its three inputs in precedence order.
pub fn resolve_state_path(
    explicit: Option<&OsStr>,
    state_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, StatePathError> {
    if let Some(value) = non_empty(explicit) {
        let path = PathBuf::from(value);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Err(StatePathError::RelativePath(path))
        };
    }

    if let Some(value) = non_empty(state_home) {
        let path = Path::new(value);
        if path.is_absolute() {
            return Ok(path.join("syswatch").join(UPDATE_STATE_FILE_NAME));
        }
    }

    if let Some(value) = non_empty(home) {
        let path = Path::new(value);
        if path.is_absolute() {
            return Ok(path
                .join(".local/state/syswatch")
                .join(UPDATE_STATE_FILE_NAME));
        }
    }

    Err(StatePathError::NotConfigured)
}

fn non_empty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

/// Loads the dismissed version. An absent, unreadable, oversized or malformed
/// file means "no dismissal".
pub fn load_state(path: &Path) -> Option<Version> {
    let body = fs::read(path).ok()?;
    if body.is_empty() || body.len() > MAX_STATE_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    parse_tag(value.get(STATE_VERSION_KEY)?.as_str()?)
}

/// Writes the dismissed version as one key, then renames it into place. A
/// missing parent directory is created with mode `0700`.
pub fn save_state(path: &Path, version: &Version) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "update state path has no parent",
        )
    })?;
    create_private_dir(parent)?;
    let body = serde_json::json!({ STATE_VERSION_KEY: version.to_string() }).to_string();
    let staging = path.with_extension("tmp");
    fs::write(&staging, body)?;
    fs::rename(&staging, path)
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const FEED: &str = r#"{"tag_name":"v0.2.0","html_url":"https://example.test/releases/0.2.0"}"#;

    struct FakeRunner {
        response: Option<&'static str>,
        calls: Cell<usize>,
        args: RefCell<Vec<String>>,
    }

    impl FakeRunner {
        fn answering(response: Option<&'static str>) -> Self {
            Self {
                response,
                calls: Cell::new(0),
                args: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, args: &[&str]) -> Option<String> {
            self.calls.set(self.calls.get() + 1);
            self.args
                .borrow_mut()
                .extend(args.iter().map(|arg| (*arg).to_owned()));
            self.response.map(str::to_owned)
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        env::temp_dir().join(format!("syswatch-update-{}-{name}", std::process::id()))
    }

    #[test]
    fn tag_parse_accepts_only_plain_semver() {
        let expected = Version {
            major: 0,
            minor: 2,
            patch: 0,
        };
        assert_eq!(parse_tag("v0.2.0"), Some(expected));
        assert_eq!(parse_tag("0.2.0"), Some(expected));
        for tag in ["V0.2.0", "v0.2", "v0.2.0-rc1", "v0.2.0 ", "", "v1.2.3.4"] {
            assert_eq!(parse_tag(tag), None, "tag {tag:?} must not parse");
        }
    }

    #[test]
    fn version_ordering_is_numeric_per_component() {
        let older = parse_tag("0.1.9").expect("0.1.9 parses");
        let newer = parse_tag("0.1.10").expect("0.1.10 parses");
        assert!(newer > older);
        assert!(parse_tag("0.2.0").expect("0.2.0 parses") > newer);
        assert!(
            parse_tag("1.0.0").expect("1.0.0 parses") > parse_tag("0.9.9").expect("0.9.9 parses")
        );
    }

    #[test]
    fn feed_parse_takes_tag_and_url() {
        let release = parse_release(FEED).expect("two-field document parses");
        assert_eq!(
            release.version,
            Version {
                major: 0,
                minor: 2,
                patch: 0,
            }
        );
        assert_eq!(release.url, "https://example.test/releases/0.2.0");
    }

    #[test]
    fn feed_parse_rejects_missing_field_and_wrong_type() {
        assert_eq!(parse_release("{}"), None);
        assert_eq!(
            parse_release(r#"{"tag_name":5,"html_url":"https://example.test"}"#),
            None
        );
        assert_eq!(parse_release(r#"{"tag_name":"v0.2.0"}"#), None);
        assert_eq!(parse_release("[]"), None);
    }

    #[test]
    fn feed_parse_rejects_empty_and_oversized_body() {
        assert_eq!(parse_release(""), None);
        assert_eq!(parse_release(&"x".repeat(MAX_FEED_BYTES + 1)), None);
    }

    #[test]
    fn check_returns_notice_only_for_strictly_newer_release() {
        let runner = FakeRunner::answering(Some(FEED));
        assert_eq!(
            check(&runner, "0.1.9").map(|release| release.version),
            Some(Version {
                major: 0,
                minor: 2,
                patch: 0,
            })
        );
        let runner = FakeRunner::answering(Some(FEED));
        assert_eq!(
            check(&runner, "0.2.0"),
            None,
            "equal version must not notify"
        );
        let runner = FakeRunner::answering(Some(FEED));
        assert_eq!(
            check(&runner, "0.3.0"),
            None,
            "older release must not notify"
        );
        let runner = FakeRunner::answering(Some(FEED));
        assert_eq!(check(&runner, "not-a-version"), None);
        assert_eq!(
            runner.calls.get(),
            0,
            "bad running version costs no request"
        );
    }

    #[test]
    fn check_failure_returns_none_after_one_attempt() {
        let runner = FakeRunner::answering(None);
        assert_eq!(check(&runner, "0.1.0"), None);
        assert_eq!(runner.calls.get(), 1);
    }

    #[test]
    fn check_ignores_error_and_empty_bodies() {
        for body in [
            "",
            r#"{"message":"Not Found","documentation_url":"https://docs.github.com"}"#,
            r#"{"message":"API rate limit exceeded for 1.2.3.4"}"#,
        ] {
            let runner = FakeRunner::answering(Some(body));
            assert_eq!(check(&runner, "0.1.0"), None, "body {body:?}");
            assert_eq!(runner.calls.get(), 1);
        }
    }

    #[test]
    fn curl_arguments_are_fixed() {
        let runner = FakeRunner::answering(Some(FEED));
        let _ = check(&runner, "0.1.0");
        let expected = [
            "-fsS",
            "--max-time",
            "3",
            "-A",
            "syswatch",
            "-H",
            "Accept: application/vnd.github+json",
            UPDATE_FEED_URL,
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(*runner.args.borrow(), expected);
    }

    #[test]
    fn state_path_resolution_prefers_an_absolute_override() {
        let override_path = Path::new("/tmp/syswatch-update-override.json");
        assert_eq!(
            resolve_state_path(
                Some(override_path.as_os_str()),
                Some(OsStr::new("/state")),
                Some(OsStr::new("/home"))
            ),
            Ok(override_path.to_path_buf())
        );
        assert_eq!(
            resolve_state_path(None, Some(OsStr::new("/state")), Some(OsStr::new("/home"))),
            Ok(PathBuf::from("/state/syswatch/update.json"))
        );
        assert_eq!(
            resolve_state_path(None, None, Some(OsStr::new("/home"))),
            Ok(PathBuf::from("/home/.local/state/syswatch/update.json"))
        );
        assert_eq!(
            resolve_state_path(Some(OsStr::new("relative.json")), None, None),
            Err(StatePathError::RelativePath(PathBuf::from("relative.json")))
        );
        assert_eq!(
            resolve_state_path(None, Some(OsStr::new("")), None),
            Err(StatePathError::NotConfigured)
        );
    }

    #[test]
    fn state_round_trip_writes_one_key() {
        let dir = temp_dir("round-trip");
        let path = dir.join(UPDATE_STATE_FILE_NAME);
        let version = Version {
            major: 0,
            minor: 2,
            patch: 0,
        };
        save_state(&path, &version).expect("state saves");
        assert_eq!(load_state(&path), Some(version));

        let body = fs::read_to_string(&path).expect("state file is readable");
        let value: serde_json::Value = serde_json::from_str(&body).expect("state file is JSON");
        let object = value.as_object().expect("state file is an object");
        assert_eq!(object.len(), 1);
        assert_eq!(
            object.get(STATE_VERSION_KEY),
            Some(&serde_json::Value::from("0.2.0"))
        );
        assert!(!path.with_extension("tmp").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dir)
                .expect("state directory exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_state_ignores_oversized_file() {
        let dir = temp_dir("oversized");
        fs::create_dir_all(&dir).expect("temp dir creates");
        let path = dir.join(UPDATE_STATE_FILE_NAME);
        fs::write(&path, "x".repeat(MAX_STATE_BYTES + 1)).expect("oversized file writes");
        assert_eq!(load_state(&path), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_state_ignores_malformed_file() {
        let dir = temp_dir("malformed");
        fs::create_dir_all(&dir).expect("temp dir creates");
        let path = dir.join(UPDATE_STATE_FILE_NAME);
        for body in [
            "",
            "not json",
            r#"{"version":5}"#,
            r#"{"other":"0.2.0"}"#,
            r#"{"version":"v0.2"}"#,
        ] {
            fs::write(&path, body).expect("malformed file writes");
            assert_eq!(load_state(&path), None, "body {body:?} must not load");
        }
        assert_eq!(load_state(&dir.join("absent.json")), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
