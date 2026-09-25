//! The durable operation record of the sample workspace's reset and delete
//! (item ⑨; the accepted sample-workspace amendment §8).
//!
//! Both replace or remove the whole sample folder with renames, and a crash
//! can fall between any two of them. So the operation, its phase and the
//! names of the sibling folders it uses live here — outside the sample
//! folder, in the protected root beside the restore and Vault-change records
//! — written and synced before anything on disk moves. Startup reads it and
//! finishes or rolls back deterministically.
//!
//! This module only stores and locks, exactly as the authority control
//! record does. The rules belong to the application service, which runs them
//! inside [`SampleControlStore::update`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

const FORMAT: &str = "pmc-sample-control/v1";
const LOCK_ATTEMPTS: u32 = 100;
const LOCK_WAIT_MILLIS: u64 = 20;
/// Enough history to answer a repeated request; older outcomes are dropped.
const FINISHED_KEPT: usize = 16;

/// Where a reset stands. Each phase is recorded before the step it names.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetPhase {
    /// A fresh sample is being built in the staging folder.
    Building,
    /// The staging folder holds a verified sample; the current one is about
    /// to be renamed aside.
    Built,
    /// The current folder was renamed aside; the staging folder is about to
    /// be renamed into place.
    SwappedAside,
}

/// Where an approved delete stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletePhase {
    /// Approved; the folder is about to be renamed aside.
    Approved,
    /// Renamed aside; what remains is removing it.
    RenamedAside,
}

/// What a delete preview bound (§8): re-derived and compared whole at
/// approval.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SampleDeleteBinding {
    /// Always `delete_sample_workspace`: a payload for another operation
    /// can never approve this one.
    pub operation: String,
    /// SHA-256 of this profile's sample folder location — which sample,
    /// without a path in the record's facts.
    pub workspace_identity_sha256: String,
    /// Always `irreversible_live_unaffected`: the effect the person approves.
    pub effect: String,
    pub seed_id: String,
    pub seed_version: u32,
    /// SHA-256 over every directory and file in the sample folder: each
    /// relative path, and for a file its length and content digest, sorted.
    pub inventory_sha256: String,
    pub has_ledger: bool,
    pub has_vault: bool,
    pub has_generated_files: bool,
    pub settings_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedSampleDelete {
    pub prepared_intent_id: String,
    pub payload_sha256: String,
    pub binding: SampleDeleteBinding,
    pub prepared_at_millis: i64,
    pub expires_at_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SampleOperation {
    Reset {
        operation_id: String,
        phase: ResetPhase,
        started_at_millis: i64,
        /// SHA-256 of the manifest the verified staging sample carries,
        /// recorded with `Built`: at startup only that sample counts as
        /// installed.
        #[serde(default)]
        built_manifest_sha256: Option<String>,
    },
    DeletePrepared {
        prepared: PreparedSampleDelete,
    },
    Deleting {
        prepared: PreparedSampleDelete,
        idempotency_id: String,
        receipt_id: String,
        phase: DeletePhase,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleOutcome {
    /// The sample was replaced by a fresh one.
    Reset,
    /// A reset was interrupted before the fresh sample was in place; the
    /// previous sample is back.
    ResetRolledBack,
    /// The sample folder is gone.
    Deleted,
    /// An approved delete was interrupted before anything moved; the sample
    /// is still there.
    NotDeleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SampleTerminal {
    /// The reset's operation id, or the delete's approval idempotency id.
    pub operation_id: String,
    /// The delete preview this outcome answers; None for a reset. A
    /// repeated approval must name the same one.
    #[serde(default)]
    pub prepared_intent_id: Option<String>,
    /// The approval receipt a delete was executed under, kept with its
    /// outcome; None for a reset.
    #[serde(default)]
    pub receipt_id: Option<String>,
    pub outcome: SampleOutcome,
    pub completed_at_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SampleControl {
    pub format: String,
    pub active: Option<SampleOperation>,
    pub finished: Vec<SampleTerminal>,
    /// Sibling folders a finished operation created and could not remove
    /// yet (their names only). The next start removes exactly these —
    /// never a folder merely named like one — and one that stays here is
    /// reported rather than hidden.
    #[serde(default)]
    pub pending_cleanup: Vec<String>,
}

impl Default for SampleControl {
    fn default() -> Self {
        Self {
            format: FORMAT.to_owned(),
            active: None,
            finished: Vec::new(),
            pending_cleanup: Vec::new(),
        }
    }
}

impl SampleControl {
    /// Record a terminal outcome and clear the active operation.
    pub fn finish(&mut self, terminal: SampleTerminal) {
        self.active = None;
        self.finished.push(terminal);
        if self.finished.len() > FINISHED_KEPT {
            let excess = self.finished.len() - FINISHED_KEPT;
            self.finished.drain(..excess);
        }
    }

    /// The outcome already recorded under this operation id, if any.
    #[must_use]
    pub fn terminal_of(&self, operation_id: &str) -> Option<&SampleTerminal> {
        self.finished
            .iter()
            .rev()
            .find(|terminal| terminal.operation_id == operation_id)
    }
}

#[derive(Debug)]
pub enum SampleControlError {
    /// The document exists but is not one this version reads.
    Unreadable,
    /// Another writer held it for too long; nothing was written.
    Busy,
    Io(io::Error),
}

impl std::fmt::Display for SampleControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("sample control record is unreadable"),
            Self::Busy => formatter.write_str("sample control record is held by another writer"),
            Self::Io(error) => write!(formatter, "sample control record I/O failed: {error}"),
        }
    }
}

impl std::error::Error for SampleControlError {}

#[derive(Clone, Debug)]
pub struct SampleControlStore {
    path: PathBuf,
}

impl SampleControlStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The record, or an empty one when none was written yet.
    pub fn load(&self) -> Result<SampleControl, SampleControlError> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let control: SampleControl =
                    serde_json::from_slice(&bytes).map_err(|_| SampleControlError::Unreadable)?;
                if control.format != FORMAT {
                    return Err(SampleControlError::Unreadable);
                }
                Ok(control)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SampleControl::default()),
            Err(error) => Err(SampleControlError::Io(error)),
        }
    }

    /// Read, change and durably replace the record under the exclusive lock.
    /// `change` decides; an `Err` from it writes nothing. The new document is
    /// synced before it replaces the old, so once this returns `Ok` the
    /// change survives a crash.
    pub fn update<R, E>(
        &self,
        change: impl FnOnce(&mut SampleControl) -> Result<R, E>,
    ) -> Result<Result<R, E>, SampleControlError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(SampleControlError::Io)?;
        }
        let _guard = self.lock()?;
        let mut control = self.load()?;
        let decided = match change(&mut control) {
            Ok(decided) => decided,
            Err(refused) => return Ok(Err(refused)),
        };
        let bytes = serde_json::to_vec_pretty(&control)
            .map_err(|error| SampleControlError::Io(io::Error::other(error)))?;
        let mut file = AtomicWriteFile::open(&self.path).map_err(SampleControlError::Io)?;
        file.write_all(&bytes).map_err(SampleControlError::Io)?;
        file.commit().map_err(SampleControlError::Io)?;
        Ok(Ok(decided))
    }

    fn lock(&self) -> Result<File, SampleControlError> {
        let mut name = self.path.as_os_str().to_owned();
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
                Err(error) => return Err(SampleControlError::Io(error)),
            }
        }
        Err(SampleControlError::Busy)
    }
}
