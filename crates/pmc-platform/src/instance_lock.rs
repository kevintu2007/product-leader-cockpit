//! One running PMC per profile (item ⑩; product owner 2026-09-21: a file
//! lock, no plugin). A second launch finds the lock held and stops before it
//! opens a window, a Ledger or the settings document — so the two-instance
//! settings race recorded on 2026-09-19 cannot start.
//!
//! The lock is `instance.lock` in the protected root, opened with no sharing
//! on Windows: the operating system refuses a second open with
//! `ERROR_SHARING_VIOLATION` and releases the hold when the process ends,
//! however it ends, so a crash never leaves a stale lock. The same rule the
//! backup registry and the host audit log use for their writers. Elsewhere
//! the file serialises nothing across processes (PMC ships for Windows only).

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::Path;

use crate::settings::ProtectedSettingsRoot;

/// The lock file's name inside the protected root.
pub const INSTANCE_LOCK_FILE_NAME: &str = "instance.lock";

/// The hold: this process is the one PMC for its profile as long as the
/// value lives. Keep it for the whole run; drop it only at exit.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

#[derive(Debug)]
pub enum InstanceLockError {
    /// Another PMC holds the lock: it is already running for this profile.
    AlreadyRunning,
    /// The lock file could not be opened for another reason.
    Io(io::Error),
}

impl fmt::Display for InstanceLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => formatter.write_str("another PMC is already running"),
            Self::Io(error) => write!(formatter, "instance lock: {error}"),
        }
    }
}

impl std::error::Error for InstanceLockError {}

/// A sharing conflict says only that someone holds the file. A running PMC
/// holds it for good; a virus scanner or an indexer holds it for a moment.
/// A few short retries tell the two apart without making a real second
/// launch wait noticeably.
const ATTEMPTS: u32 = 5;
const WAIT_MILLIS: u64 = 100;

/// A restart this PMC asked for (switching workspace, item ⑨): the new
/// process starts *before* the old one exits, so it may wait this long for
/// the old one's hold to go. The old one keeps its hold until it exits.
const RESTART_ATTEMPTS: u32 = 100;
/// A handover marker older than this is not a restart in progress.
const RESTART_MARKER_FRESH_MILLIS: i64 = 30_000;
/// The handover marker's name inside the protected root.
pub const RESTART_MARKER_FILE_NAME: &str = "restart-pending";

impl InstanceLock {
    /// Take the hold for `root`, or learn that another PMC has it. Half a
    /// second at most: a second launch should say so at once — unless this
    /// PMC's own restart is handing over (a fresh marker, left by
    /// [`Self::mark_restart`]), when it waits for the old process to exit.
    pub fn acquire(root: &ProtectedSettingsRoot) -> Result<Self, InstanceLockError> {
        Self::acquire_at(root.path())
    }

    fn acquire_at(directory: &Path) -> Result<Self, InstanceLockError> {
        let marker = directory.join(RESTART_MARKER_FILE_NAME);
        if restart_marker_is_fresh(&marker) {
            let taken = Self::acquire_in_with(directory, RESTART_ATTEMPTS);
            if taken.is_ok() {
                let _ = fs::remove_file(&marker);
            }
            return taken;
        }
        Self::acquire_in(directory)
    }

    /// Leave the handover marker for the restart this process is about to
    /// ask for. The hold itself stays until this process exits.
    pub fn mark_restart(root: &ProtectedSettingsRoot, now_millis: i64) -> io::Result<()> {
        fs::write(
            root.path().join(RESTART_MARKER_FILE_NAME),
            now_millis.to_string(),
        )
    }

    fn acquire_in(directory: &Path) -> Result<Self, InstanceLockError> {
        Self::acquire_in_with(directory, ATTEMPTS)
    }

    fn acquire_in_with(directory: &Path, attempts: u32) -> Result<Self, InstanceLockError> {
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let path = directory.join(INSTANCE_LOCK_FILE_NAME);
        for attempt in 1..=attempts {
            match options.open(&path) {
                Ok(file) => return Ok(Self { _file: file }),
                // ERROR_SHARING_VIOLATION: someone's open still stands.
                Err(error) if error.raw_os_error() == Some(32) => {
                    if attempt < attempts {
                        std::thread::sleep(std::time::Duration::from_millis(WAIT_MILLIS));
                    }
                }
                Err(error) => return Err(InstanceLockError::Io(error)),
            }
        }
        Err(InstanceLockError::AlreadyRunning)
    }
}

/// Whether a handover marker was written within the last half minute.
fn restart_marker_is_fresh(marker: &Path) -> bool {
    let Ok(text) = fs::read_to_string(marker) else {
        return false;
    };
    let Ok(written) = text.trim().parse::<i64>() else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        });
    (0..RESTART_MARKER_FRESH_MILLIS).contains(&now.saturating_sub(written))
}

#[cfg(test)]
mod tests {
    use super::*;

    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn scratch(name: &str) -> std::path::PathBuf {
        // Unique per test run without a process id: the nanosecond clock
        // plus a per-process sequence keeps concurrent runs apart.
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory =
            std::env::temp_dir().join(format!("pmc-instance-lock-{name}-{unique}-{sequence}"));
        let _ = fs::remove_dir_all(&directory);
        assert!(fs::create_dir_all(&directory).is_ok(), "scratch directory");
        directory
    }

    fn hold(directory: &Path) -> InstanceLock {
        match InstanceLock::acquire_in(directory) {
            Ok(lock) => lock,
            Err(error) => panic!("hold: {error}"),
        }
    }

    #[test]
    fn the_first_hold_succeeds_and_creates_the_lock_file() {
        let directory = scratch("first");
        let lock = hold(&directory);
        assert!(directory.join(INSTANCE_LOCK_FILE_NAME).is_file());
        drop(lock);
        let _ = fs::remove_dir_all(&directory);
    }

    #[cfg(windows)]
    #[test]
    fn a_second_hold_is_refused_while_the_first_lives_and_allowed_after() {
        let directory = scratch("second");
        let first = hold(&directory);
        assert!(matches!(
            InstanceLock::acquire_in(&directory),
            Err(InstanceLockError::AlreadyRunning)
        ));
        drop(first);
        let again = InstanceLock::acquire_in(&directory);
        assert!(again.is_ok(), "{again:?}");
        drop(again);
        let _ = fs::remove_dir_all(&directory);
    }

    // Only the Windows tests below write a timestamped marker.
    #[cfg(windows)]
    fn now_millis() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }

    /// A restart this PMC asked for waits for the old hold to go; the old
    /// process kept its hold the whole time.
    #[cfg(windows)]
    #[test]
    fn a_restart_handover_waits_for_the_old_hold_and_clears_its_marker() {
        let directory = scratch("handover");
        let old = hold(&directory);
        let marker = directory.join(RESTART_MARKER_FILE_NAME);
        assert!(fs::write(&marker, now_millis().to_string()).is_ok());
        let exiting = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(700));
            drop(old);
        });
        let new = InstanceLock::acquire_at(&directory);
        assert!(new.is_ok(), "{new:?}");
        assert!(!marker.exists());
        assert!(exiting.join().is_ok());
        drop(new);
        let _ = fs::remove_dir_all(&directory);
    }

    /// A stale marker is no handover: a second launch is told at once.
    #[cfg(windows)]
    #[test]
    fn a_stale_marker_does_not_make_a_second_launch_wait() {
        let directory = scratch("stale");
        let first = hold(&directory);
        let marker = directory.join(RESTART_MARKER_FILE_NAME);
        assert!(fs::write(&marker, (now_millis() - 60_000).to_string()).is_ok());
        assert!(matches!(
            InstanceLock::acquire_at(&directory),
            Err(InstanceLockError::AlreadyRunning)
        ));
        drop(first);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_directory_that_does_not_exist_is_an_io_error_not_a_running_instance() {
        let parent = scratch("missing");
        let directory = parent.join("absent");
        assert!(matches!(
            InstanceLock::acquire_in(&directory),
            Err(InstanceLockError::Io(_))
        ));
        let _ = fs::remove_dir_all(&parent);
    }
}
