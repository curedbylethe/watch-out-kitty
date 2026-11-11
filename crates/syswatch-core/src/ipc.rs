use std::{
    env,
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

pub const SOCKET_PATH_ENV: &str = "SYSWATCH_SOCKET";
pub const SOCKET_FILE_NAME: &str = "syswatch.sock";

#[derive(Debug, PartialEq, Eq)]
pub enum SocketPathError {
    NotConfigured,
    RelativePath(PathBuf),
}

impl fmt::Display for SocketPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(
                f,
                "no socket path configured; set SYSWATCH_SOCKET to an absolute path, or XDG_RUNTIME_DIR, or HOME"
            ),
            Self::RelativePath(path) => {
                write!(f, "socket path must be absolute: {}", path.display())
            }
        }
    }
}

impl std::error::Error for SocketPathError {}

pub fn socket_path() -> Result<PathBuf, SocketPathError> {
    resolve_socket_path(
        env::var_os(SOCKET_PATH_ENV).as_deref(),
        env::var_os("XDG_RUNTIME_DIR").as_deref(),
        env::var_os("HOME").as_deref(),
    )
}

pub fn resolve_socket_path(
    explicit: Option<&OsStr>,
    runtime_dir: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, SocketPathError> {
    if let Some(value) = non_empty(explicit) {
        let path = PathBuf::from(value);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Err(SocketPathError::RelativePath(path))
        };
    }

    if let Some(value) = non_empty(runtime_dir) {
        let path = Path::new(value);
        if path.is_absolute() {
            return Ok(path.join(SOCKET_FILE_NAME));
        }
    }

    if let Some(value) = non_empty(home) {
        let path = Path::new(value);
        if path.is_absolute() {
            return Ok(path.join(".local/run").join(SOCKET_FILE_NAME));
        }
    }

    Err(SocketPathError::NotConfigured)
}

fn non_empty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> &OsStr {
        OsStr::new(value)
    }

    #[test]
    fn explicit_absolute_path_wins_over_both_defaults() {
        assert_eq!(
            resolve_socket_path(
                Some(os("/custom/socket")),
                Some(os("/run/user")),
                Some(os("/home/user"))
            ),
            Ok(PathBuf::from("/custom/socket"))
        );
    }

    #[test]
    fn relative_explicit_path_is_an_error() {
        assert_eq!(
            resolve_socket_path(Some(os("relative.sock")), None, None),
            Err(SocketPathError::RelativePath(PathBuf::from(
                "relative.sock"
            )))
        );
    }

    #[test]
    fn empty_explicit_path_is_ignored() {
        assert_eq!(
            resolve_socket_path(Some(os("")), Some(os("/run/user")), None),
            Ok(PathBuf::from("/run/user/syswatch.sock"))
        );
    }

    #[test]
    fn runtime_dir_wins_over_home() {
        assert_eq!(
            resolve_socket_path(None, Some(os("/run/user/42")), Some(os("/home/user"))),
            Ok(PathBuf::from("/run/user/42/syswatch.sock"))
        );
    }

    #[test]
    fn home_is_the_fallback_for_macos() {
        assert_eq!(
            resolve_socket_path(None, None, Some(os("/Users/alice"))),
            Ok(PathBuf::from("/Users/alice/.local/run/syswatch.sock"))
        );
    }

    #[test]
    fn relative_runtime_dir_falls_back_to_home() {
        assert_eq!(
            resolve_socket_path(None, Some(os("relative")), Some(os("/home/user"))),
            Ok(PathBuf::from("/home/user/.local/run/syswatch.sock"))
        );
    }

    #[test]
    fn no_sources_is_not_configured() {
        assert_eq!(
            resolve_socket_path(None, None, None),
            Err(SocketPathError::NotConfigured)
        );
    }

    #[test]
    fn resolved_default_ends_with_the_shared_file_name() {
        assert!(
            resolve_socket_path(None, Some(os("/run/user/42")), None)
                .unwrap()
                .ends_with(SOCKET_FILE_NAME)
        );
    }
}
