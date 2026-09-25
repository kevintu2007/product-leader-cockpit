//! The host audit log of ADR 0012: events the host performs outside the
//! Product Ledger — the named S7 bootstrap, each Operational Backup run and,
//! with S7-B1, each restore. A restore replaces the Ledger, so these events
//! cannot live inside it.
//!
//! One append-only JSON-lines file in the protected app-data root. Appends
//! are serialized by an exclusive lock file (the registry's rule), each event
//! is one newline-terminated record followed by `sync_all`, and a torn last
//! line left by a crash is closed with a newline before the next append so
//! it never swallows a later event. Nothing is rewritten or removed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FORMAT: &str = "pmc-host-audit/v1";
const LOCK_ATTEMPTS: u32 = 100;
const LOCK_WAIT_MILLIS: u64 = 20;

/// The codes this version writes (ADR 0012 §3).
pub const BOOTSTRAP_EMPTY_AUTHORITY: &str = "backup.bootstrap_empty_authority";
pub const BACKUP_COMPLETED: &str = "backup.completed";
pub const BACKUP_FAILED: &str = "backup.failed";
/// A Product Vault authority-path change (item ⑦, H2b). Its facts are the
/// intent and receipt ids, the counts, the revisions bound and the outcome —
/// never a path (ADR 0011) and never a folder the person did not name.
pub const VAULT_ROOT_PREPARED: &str = "authority.vault_root_prepared";
pub const VAULT_ROOT_REJECTED: &str = "authority.vault_root_rejected";
pub const VAULT_ROOT_CHANGED: &str = "authority.vault_root_changed";
pub const VAULT_ROOT_NOT_CHANGED: &str = "authority.vault_root_not_changed";
/// The sample workspace's reset and delete (item ⑨; sample-workspace
/// amendment §8). The audit log survives the deletion it records. Facts are
/// operation, intent and receipt ids and the seed version — never a path.
pub const SAMPLE_RESET: &str = "sample.reset";
pub const SAMPLE_RESET_ROLLED_BACK: &str = "sample.reset_rolled_back";
pub const SAMPLE_DELETE_PREPARED: &str = "sample.delete_prepared";
pub const SAMPLE_DELETE_REJECTED: &str = "sample.delete_rejected";
pub const SAMPLE_DELETED: &str = "sample.deleted";
pub const SAMPLE_NOT_DELETED: &str = "sample.not_deleted";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditWorkspace {
    Live,
    Training,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Succeeded,
    Failed,
    Refused,
}

/// One event. `facts` is a closed, non-secret set per code: never a path, a
/// passphrase, a record's content or a person's name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostAuditEvent {
    pub format: String,
    pub event_id: String,
    pub occurred_at_millis: i64,
    pub workspace: AuditWorkspace,
    pub event_code: String,
    pub outcome: AuditOutcome,
    #[serde(default)]
    pub facts: Vec<(String, String)>,
}

impl HostAuditEvent {
    #[must_use]
    pub fn new(
        event_id: String,
        occurred_at_millis: i64,
        workspace: AuditWorkspace,
        event_code: &str,
        outcome: AuditOutcome,
        facts: Vec<(String, String)>,
    ) -> Self {
        Self {
            format: FORMAT.to_owned(),
            event_id,
            occurred_at_millis,
            workspace,
            event_code: event_code.to_owned(),
            outcome,
            facts,
        }
    }
}

#[derive(Debug)]
pub enum HostAuditError {
    /// Another writer held the log for too long; nothing was written.
    Busy,
    Io(io::Error),
}

impl std::fmt::Display for HostAuditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => formatter.write_str("host audit log is held by another writer"),
            Self::Io(error) => write!(formatter, "host audit log I/O failed: {error}"),
        }
    }
}

impl std::error::Error for HostAuditError {}

#[derive(Clone, Debug)]
pub struct HostAuditLog {
    path: PathBuf,
}

impl HostAuditLog {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Append one event and flush it before returning.
    pub fn append(&self, event: &HostAuditEvent) -> Result<(), HostAuditError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(HostAuditError::Io)?;
        }
        let _guard = lock(&self.path)?;
        let mut line = serde_json::to_vec(event)
            .map_err(|error| HostAuditError::Io(io::Error::other(error)))?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&self.path)
            .map_err(HostAuditError::Io)?;
        if ends_unterminated(&mut file).map_err(HostAuditError::Io)? {
            line.insert(0, b'\n');
        }
        file.write_all(&line).map_err(HostAuditError::Io)?;
        file.sync_all().map_err(HostAuditError::Io)
    }

    /// Every complete event, in order. Torn or unreadable lines are skipped.
    pub fn events(&self) -> Result<Vec<HostAuditEvent>, HostAuditError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(HostAuditError::Io(error)),
        };
        Ok(bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<HostAuditEvent>(line).ok())
            .filter(|event| event.format == FORMAT)
            .collect())
    }

    /// Whether any complete event with this code was recorded for this
    /// workspace.
    pub fn has_event(
        &self,
        workspace: AuditWorkspace,
        event_code: &str,
    ) -> Result<bool, HostAuditError> {
        Ok(self
            .events()?
            .iter()
            .any(|event| event.workspace == workspace && event.event_code == event_code))
    }
}

fn ends_unterminated(file: &mut File) -> io::Result<bool> {
    let length = file.seek(SeekFrom::End(0))?;
    if length == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(length - 1))?;
    let mut last = [0_u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

/// An exclusive hold on `<log>.lock`, as the backup registry takes one.
fn lock(path: &Path) -> Result<File, HostAuditError> {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let lock_path = PathBuf::from(name);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    for _ in 0..LOCK_ATTEMPTS {
        match options.open(&lock_path) {
            Ok(file) => return Ok(file),
            // ERROR_SHARING_VIOLATION: another writer holds it.
            Err(error) if error.raw_os_error() == Some(32) => {
                std::thread::sleep(std::time::Duration::from_millis(LOCK_WAIT_MILLIS));
            }
            Err(error) => return Err(HostAuditError::Io(error)),
        }
    }
    Err(HostAuditError::Busy)
}
