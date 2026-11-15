//! Out-of-band release check. One detached thread performs one bounded attempt
//! at start and then once per interval; a failure is silent and never retried
//! before the next interval.

use std::{
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::Duration,
};

use syswatch_core::UpdateNotice;
use syswatch_core::update::{self, CurlRunner, UPDATE_DISABLE_ENV, Version};

use crate::latest::LatestSnapshot;

/// Delay between two release check attempts.
pub const DEFAULT_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Remembers the last version printed, so a version prompts at most once in the
/// daemon's lifetime.
#[derive(Default)]
pub struct Notifier {
    last: Option<Version>,
}

impl Notifier {
    /// Reports whether `version` is newer than every version already printed
    /// and newer than the durable dismissal, if any.
    pub fn should_notify(&mut self, version: &Version, dismissed: Option<Version>) -> bool {
        if update::is_dismissed(dismissed, *version) {
            return false;
        }
        if self.last.is_some_and(|last| last >= *version) {
            return false;
        }
        self.last = Some(*version);
        true
    }
}

/// Starts the check thread. Nothing runs when the check is disabled or the
/// dismissal state path cannot be resolved. The thread is detached: it is never
/// joined and never delays shutdown.
pub fn start(latest: Arc<Mutex<LatestSnapshot>>) {
    if update::disabled() || update::state_path().is_err() {
        return;
    }
    let _ = thread::Builder::new()
        .name("update-check".into())
        .spawn(move || run(latest));
}

fn run(latest: Arc<Mutex<LatestSnapshot>>) {
    let mut notifier = Notifier::default();
    loop {
        if let Some(release) = update::check(&CurlRunner, env!("CARGO_PKG_VERSION")) {
            let dismissed = update::state_path()
                .ok()
                .and_then(|path| update::load_state(&path));
            if notifier.should_notify(&release.version, dismissed) {
                eprintln!(
                    "wok-daemon: version {} is available at {}; set {}=1 to silence",
                    release.version, release.url, UPDATE_DISABLE_ENV
                );
            }
            let notice = UpdateNotice {
                latest_version: release.version.to_string(),
                url: release.url,
            };
            latest
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_update(Some(notice));
        }
        thread::sleep(DEFAULT_UPDATE_CHECK_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::Notifier;
    use syswatch_core::update::Version;

    fn version(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn notifier_prints_once_per_version_and_again_for_a_newer_one() {
        let mut notifier = Notifier::default();
        assert!(notifier.should_notify(&version(1, 2, 0), None));
        assert!(!notifier.should_notify(&version(1, 2, 0), None));
        assert!(!notifier.should_notify(&version(1, 1, 9), None));
        assert!(notifier.should_notify(&version(1, 3, 0), None));
        assert!(!notifier.should_notify(&version(1, 3, 0), None));
    }

    #[test]
    fn notifier_stays_silent_for_a_dismissed_version_after_a_restart() {
        let mut notifier = Notifier::default();
        assert!(!notifier.should_notify(&version(1, 2, 0), Some(version(1, 2, 0))));
        assert!(!notifier.should_notify(&version(1, 2, 0), Some(version(1, 3, 0))));
        assert!(notifier.should_notify(&version(1, 2, 0), Some(version(1, 1, 9))));
        assert!(notifier.should_notify(&version(1, 3, 0), Some(version(1, 2, 0))));
    }

    #[test]
    fn a_suppressed_version_still_prints_once_the_dismissal_is_gone() {
        let mut notifier = Notifier::default();
        assert!(!notifier.should_notify(&version(1, 2, 0), Some(version(1, 2, 0))));
        assert!(notifier.should_notify(&version(1, 2, 0), None));
    }
}
