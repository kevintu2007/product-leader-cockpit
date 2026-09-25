//! The durable control record of a Product Vault authority-path change (item
//! ⑦; DG3 Vault-root amendment §3 and §6.5, accepted 2026-09-23).
//!
//! Changing which folder the Live workspace reads Evidence from is H2b (DG0:
//! "Change Product Vault / Product Ledger authority path"). The settings
//! document is the committed pointer, but a settings write cannot be atomic
//! with the backup that is its recovery evidence or with the audit events
//! that record it, so the Prepared Intent, the approval receipt, the phase
//! and the previous value live here: one JSON document per workspace in the
//! protected root, read and replaced whole under an exclusive lock file (the
//! backup registry's rule), synced before it is renamed into place.
//!
//! This module only stores and locks. The rules — what a preview must still
//! match, who may claim an intent, what each phase allows — belong to the
//! application service, which runs them inside [`AuthorityControlStore::update`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

const FORMAT: &str = "pmc-authority-control/v1";
const LOCK_ATTEMPTS: u32 = 100;
const LOCK_WAIT_MILLIS: u64 = 20;

/// One Evidence reference as the preview bound it: the identity, the version
/// it was read at, and the fingerprint that proved the file under the
/// proposed folder holds the same content. A reference with no pinned
/// fingerprint cannot be bound and blocks the change (§3.2), so every entry
/// here has one.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundEvidence {
    pub evidence_id: String,
    pub version: u64,
    pub fingerprint_algorithm: String,
    pub fingerprint_digest: String,
}

/// The exact preview the person approves (the H2b Prepared Intent).
///
/// No absolute path reaches the webview from any of this: only the folders'
/// own names and the counts. The proposed folder itself is kept because the
/// approval, not the preview, is what applies it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedVaultRootChange {
    pub prepared_intent_id: String,
    /// SHA-256 over the canonical preview payload the person approves.
    pub payload_sha256: String,
    /// The folder that will be recorded, and its own name for the sheet.
    pub proposed_root: PathBuf,
    pub proposed_folder_name: String,
    /// What is being replaced: `None` when no Vault was set.
    pub previous_root: Option<PathBuf>,
    pub previous_folder_name: Option<String>,
    /// The settings document this preview read; approval requires it
    /// unchanged, so a folder chosen elsewhere cannot be overwritten blind.
    pub settings_revision: u64,
    /// The Ledger this preview read; approval requires it unchanged, so an
    /// Evidence created, pinned or superseded since invalidates the preview.
    pub ledger_revision: u64,
    /// Every Evidence reference the preview resolved, sorted by id.
    pub bound_evidence: Vec<BoundEvidence>,
    /// The verified Operational Backup that is this change's recovery
    /// evidence (§3.2), and when it verified.
    pub recovery_archive_id: String,
    pub recovery_verified_at_millis: i64,
    /// The short code the person types after the fixed phrase (§3.3).
    pub confirmation_code: String,
    pub prepared_at_millis: i64,
    pub expires_at_millis: i64,
}

/// How far an approved change has got. Each phase is written before the step
/// it names is taken, so a restart knows what may have happened.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultRootChangePhase {
    /// Approved and claimed; the settings still hold the previous folder.
    Approved,
    /// About to write the settings document.
    Committing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthorityOperation {
    Prepared {
        prepared: PreparedVaultRootChange,
    },
    Executing {
        prepared: PreparedVaultRootChange,
        idempotency_id: String,
        receipt_id: String,
        approved_at_millis: i64,
        phase: VaultRootChangePhase,
    },
}

/// What a finished change did. A change either happened or did not: unlike a
/// restore, nothing is moved on disk, so there is no half-replaced state and
/// no rollback beyond leaving the settings as they were.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultRootChangeOutcome {
    /// The settings now name the proposed folder.
    Changed,
    /// Nothing was changed: the preview no longer matched, the write was
    /// refused, or the approval was interrupted before it reached the write.
    /// There is no third outcome: the settings are written once, last, and
    /// the next start reads them to learn which of these two happened rather
    /// than assuming from how far the record got.
    NotChanged,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VaultRootChangeTerminal {
    pub idempotency_id: String,
    pub prepared_intent_id: String,
    pub outcome: VaultRootChangeOutcome,
    /// The folder's own name before and after, for the audit trail and for a
    /// person who wants to put it back.
    pub previous_folder_name: Option<String>,
    pub folder_name: String,
    pub recovery_archive_id: String,
    pub completed_at_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityControl {
    pub format: String,
    pub active: Option<AuthorityOperation>,
    pub finished: Vec<VaultRootChangeTerminal>,
}

impl Default for AuthorityControl {
    fn default() -> Self {
        Self {
            format: FORMAT.to_owned(),
            active: None,
            finished: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum AuthorityControlError {
    /// The document exists but is not one this version reads.
    Unreadable,
    /// Another writer held it for too long; nothing was written.
    Busy,
    Io(io::Error),
}

impl std::fmt::Display for AuthorityControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("authority control record is unreadable"),
            Self::Busy => formatter.write_str("authority control record is held by another writer"),
            Self::Io(error) => write!(formatter, "authority control record I/O failed: {error}"),
        }
    }
}

impl std::error::Error for AuthorityControlError {}

#[derive(Clone, Debug)]
pub struct AuthorityControlStore {
    path: PathBuf,
}

impl AuthorityControlStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The record, or an empty one when none was written yet.
    pub fn load(&self) -> Result<AuthorityControl, AuthorityControlError> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let control: AuthorityControl = serde_json::from_slice(&bytes)
                    .map_err(|_| AuthorityControlError::Unreadable)?;
                if control.format != FORMAT {
                    return Err(AuthorityControlError::Unreadable);
                }
                Ok(control)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(AuthorityControl::default())
            }
            Err(error) => Err(AuthorityControlError::Io(error)),
        }
    }

    /// Read, change and durably replace the record under the exclusive lock.
    /// `change` decides; an `Err` from it writes nothing. The new document is
    /// synced before it replaces the old, so once this returns `Ok` the
    /// change survives a crash.
    pub fn update<R, E>(
        &self,
        change: impl FnOnce(&mut AuthorityControl) -> Result<R, E>,
    ) -> Result<Result<R, E>, AuthorityControlError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(AuthorityControlError::Io)?;
        }
        let _guard = self.lock()?;
        let mut control = self.load()?;
        let decided = match change(&mut control) {
            Ok(decided) => decided,
            Err(refused) => return Ok(Err(refused)),
        };
        let bytes = serde_json::to_vec_pretty(&control)
            .map_err(|error| AuthorityControlError::Io(io::Error::other(error)))?;
        let mut file = AtomicWriteFile::open(&self.path).map_err(AuthorityControlError::Io)?;
        file.write_all(&bytes).map_err(AuthorityControlError::Io)?;
        file.commit().map_err(AuthorityControlError::Io)?;
        Ok(Ok(decided))
    }

    fn lock(&self) -> Result<File, AuthorityControlError> {
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
                Err(error) => return Err(AuthorityControlError::Io(error)),
            }
        }
        Err(AuthorityControlError::Busy)
    }
}
