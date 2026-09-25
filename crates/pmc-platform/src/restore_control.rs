//! The durable control record of an Operational Restore (S7-B1; DG3 restore
//! amendment §6, decided by the 8e design, product owner 2026-09-22; format
//! v2 for the restore-unopened amendment §6, accepted 2026-09-23).
//!
//! A restore replaces the Ledger that would normally hold its Prepared Intent
//! and approval receipt, so they live here instead: one JSON document per
//! workspace in the protected root, read and replaced whole under an
//! exclusive lock file (the backup registry's rule), each replacement synced
//! before it is renamed into place. It holds at most one active operation —
//! `Prepared`, then `Executing` with its phase — and the outcomes of finished
//! ones, keyed by idempotency id.
//!
//! Format v2 names what a restore replaces in one of two ways — a Ledger PMC
//! could inspect, or, for one it cannot, the exact file set by content — and
//! the recovery evidence in one of two kinds: a verified Operational Backup,
//! or a preservation copy of those exact files. A v1 document (the file name
//! stays `restore-control-live-v1.json`, so there is never a second record to
//! disagree with) is read as the v2 it means and written as v2 on the next
//! change; anything else is unreadable and never overwritten.
//!
//! This module only stores and locks. The rules — who may claim a Prepared
//! Intent, what a digest must match, what each phase allows — belong to the
//! restore service, which runs them inside [`RestoreControlStore::update`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const FORMAT_V1: &str = "pmc-restore-control/v1";
const FORMAT: &str = "pmc-restore-control/v2";
const FILE_SET_FORMAT: &str = "pmc-ledger-file-set/v1";
const LOCK_ATTEMPTS: u32 = 100;
const LOCK_WAIT_MILLIS: u64 = 20;

/// What the current Ledger was when the restore was prepared, as PMC could
/// inspect it; approval requires it unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerBinding {
    pub schema_version: u32,
    pub ledger_revision: u64,
    pub inventory_sha256: String,
    pub content_sha256: String,
}

/// One of the files a SQLite Ledger is made of.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerFileMember {
    Ledger,
    Wal,
    Shm,
}

impl LedgerFileMember {
    /// The fixed order members are bound and moved in.
    pub const ALL: [Self; 3] = [Self::Ledger, Self::Wal, Self::Shm];

    /// The suffix after the Ledger's own file name.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Ledger => "",
            Self::Wal => "-wal",
            Self::Shm => "-shm",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Ledger => "ledger",
            Self::Wal => "wal",
            Self::Shm => "shm",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileMemberBinding {
    pub member: LedgerFileMember,
    pub length: u64,
    pub sha256: String,
}

/// A Ledger's files bound by content (restore-unopened amendment §3.6, §6):
/// which members were present, each one's length and SHA-256, and one digest
/// over all of it. No modification time is part of it: content is the
/// authority. Made only by [`FileSetBinding::new`], so the digest always
/// matches the members.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileSetBinding {
    format: String,
    members: Vec<FileMemberBinding>,
    aggregate_sha256: String,
}

impl FileSetBinding {
    /// The binding of these members, in the fixed order whatever order they
    /// come in. `None` when the main Ledger file is missing or a member
    /// appears twice.
    #[must_use]
    pub fn new(mut members: Vec<FileMemberBinding>) -> Option<Self> {
        members.sort_by_key(|member| member.member);
        if members.first().map(|first| first.member) != Some(LedgerFileMember::Ledger)
            || members
                .windows(2)
                .any(|pair| pair[0].member == pair[1].member)
        {
            return None;
        }
        let aggregate_sha256 = aggregate(&members);
        Some(Self {
            format: FILE_SET_FORMAT.to_owned(),
            members,
            aggregate_sha256,
        })
    }

    #[must_use]
    pub fn members(&self) -> &[FileMemberBinding] {
        &self.members
    }

    #[must_use]
    pub fn aggregate_sha256(&self) -> &str {
        &self.aggregate_sha256
    }

    /// Whether a stored binding is one [`Self::new`] could have made: the
    /// format, the order and the digest all agree.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.format == FILE_SET_FORMAT
            && Self::new(self.members.clone()).is_some_and(|remade| remade == *self)
    }
}

fn aggregate(members: &[FileMemberBinding]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{FILE_SET_FORMAT}\n"));
    for member in members {
        hasher.update(format!(
            "{}\t{}\t{}\n",
            member.member.name(),
            member.length,
            member.sha256
        ));
    }
    format!("{:x}", hasher.finalize())
}

/// What a restore replaces (restore-unopened amendment §6).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CurrentSourceBinding {
    /// A Ledger PMC could inspect: open, or closed and waiting for its
    /// upgrade.
    Inspectable { binding: LedgerBinding },
    /// A Ledger PMC cannot inspect, bound by its exact files.
    Opaque { files: FileSetBinding },
}

/// Why the current Ledger was not open when the restore was prepared.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnopenedReason {
    UpgradeRequired,
    UnsupportedOld,
    OpenFailed,
    RestoreRecoveryRequired,
}

/// What the restore keeps of the workspace it replaces (restore-unopened
/// amendment §1). The webview is told the kind and a display name only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryEvidence {
    /// A verified Operational Backup, in the backup registry.
    OperationalBackup { archive_id: String },
    /// A preservation copy of the exact Ledger files: re-read and matched
    /// byte for byte, never an Operational Backup.
    PreservationCopy {
        preservation_id: String,
        /// Its file name in the backup folder; host-only.
        file_name: String,
        /// The files it holds.
        files: FileSetBinding,
    },
}

impl RecoveryEvidence {
    /// The stable identity audits and System Health name it by.
    #[must_use]
    pub fn identity(&self) -> &str {
        match self {
            Self::OperationalBackup { archive_id } => archive_id,
            Self::PreservationCopy {
                preservation_id, ..
            } => preservation_id,
        }
    }
}

/// A checked archive and the exact preview built from it (the H2b Prepared
/// Intent).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedRestore {
    pub prepared_intent_id: String,
    /// SHA-256 of the canonical preview payload the person approves.
    pub payload_sha256: String,
    pub archive_id: String,
    pub archive_container_sha256: String,
    /// `created_at` from the archive's manifest (RFC 3339, UTC).
    pub archive_created_at: String,
    pub archive_schema_version: u32,
    pub archive_ledger_revision: u64,
    pub archive_record_count: u64,
    pub snapshot_sha256: String,
    /// SHA-256 of the archive's `settings.json` and authority-inventory
    /// members as checked; both are verified again right before use.
    pub settings_member_sha256: String,
    pub inventory_member_sha256: String,
    pub current: CurrentSourceBinding,
    /// `None` when the Ledger was open.
    pub unopened_reason: Option<UnopenedReason>,
    pub settings_revision: u64,
    /// What was kept of the current workspace for this restore.
    pub recovery: RecoveryEvidence,
    /// The date the person must type, `YYYY-MM-DD`, and the time zone it was
    /// rendered in.
    pub confirmation_date: String,
    pub confirmation_timezone: String,
    pub prepared_at_millis: i64,
    pub expires_at_millis: i64,
}

/// How far an approved restore has got. Each phase is written before the
/// step it names is taken, so a restart knows what may have happened.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestorePhase {
    /// Approved and claimed; nothing replaced yet.
    Approved,
    /// One executor has claimed it and is staging the snapshot beside the
    /// Ledger; the live Ledger has not moved.
    Staging,
    /// About to move the live Ledger aside as the rollback copy.
    MovingLiveAside,
    /// About to move the checked snapshot into the live name.
    Installing,
    /// About to apply the archived settings.
    ApplyingSettings,
    /// Installed and settings applied; being checked.
    Verifying,
    /// A failure after replacement began; putting the previous Ledger and
    /// settings back.
    RollingBack,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RestoreOperation {
    Prepared {
        prepared: PreparedRestore,
    },
    Executing {
        prepared: PreparedRestore,
        idempotency_id: String,
        receipt_id: String,
        approved_at_millis: i64,
        phase: RestorePhase,
    },
}

/// The DG3 restore amendment's results (§3.7).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreOutcome {
    Restored,
    /// Nothing was changed.
    FailedBeforeReplacement,
    /// It failed after replacement began and the previous Ledger was put back.
    RecoveryPutBack,
    /// Even putting it back failed: System Health, naming the recovery backup.
    RecoveryFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreTerminal {
    pub idempotency_id: String,
    pub prepared_intent_id: String,
    pub outcome: RestoreOutcome,
    pub recovery: RecoveryEvidence,
    pub completed_at_millis: i64,
    /// Restored from an older schema: the authority-inventory SHA-256 of the
    /// Ledger whose projections must be marked out of sync once the upgrade
    /// has made it openable (S7 plan §7); cleared only for that Ledger.
    #[serde(default)]
    pub projections_pending_for: Option<String>,
    /// The earlier `RecoveryFailed` restore (its idempotency id) this one
    /// resolved by replacing the uncertain files. That record is kept.
    #[serde(default)]
    pub resolved_recovery_failure: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreControl {
    pub format: String,
    pub active: Option<RestoreOperation>,
    pub finished: Vec<RestoreTerminal>,
}

impl Default for RestoreControl {
    fn default() -> Self {
        Self {
            format: FORMAT.to_owned(),
            active: None,
            finished: Vec::new(),
        }
    }
}

/// The v1 document, read only to be understood as v2.
mod v1 {
    use serde::Deserialize;

    use super::{
        CurrentSourceBinding, LedgerBinding, RecoveryEvidence, RestoreOutcome, RestorePhase,
    };

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct PreparedRestore {
        prepared_intent_id: String,
        payload_sha256: String,
        archive_id: String,
        archive_container_sha256: String,
        archive_created_at: String,
        archive_schema_version: u32,
        archive_ledger_revision: u64,
        archive_record_count: u64,
        snapshot_sha256: String,
        settings_member_sha256: String,
        inventory_member_sha256: String,
        current: LedgerBinding,
        settings_revision: u64,
        recovery_archive_id: String,
        confirmation_date: String,
        confirmation_timezone: String,
        prepared_at_millis: i64,
        expires_at_millis: i64,
    }

    impl From<PreparedRestore> for super::PreparedRestore {
        fn from(v1: PreparedRestore) -> Self {
            Self {
                prepared_intent_id: v1.prepared_intent_id,
                payload_sha256: v1.payload_sha256,
                archive_id: v1.archive_id,
                archive_container_sha256: v1.archive_container_sha256,
                archive_created_at: v1.archive_created_at,
                archive_schema_version: v1.archive_schema_version,
                archive_ledger_revision: v1.archive_ledger_revision,
                archive_record_count: v1.archive_record_count,
                snapshot_sha256: v1.snapshot_sha256,
                settings_member_sha256: v1.settings_member_sha256,
                inventory_member_sha256: v1.inventory_member_sha256,
                // v1 only ever restored over an open Ledger.
                current: CurrentSourceBinding::Inspectable {
                    binding: v1.current,
                },
                unopened_reason: None,
                settings_revision: v1.settings_revision,
                recovery: RecoveryEvidence::OperationalBackup {
                    archive_id: v1.recovery_archive_id,
                },
                confirmation_date: v1.confirmation_date,
                confirmation_timezone: v1.confirmation_timezone,
                prepared_at_millis: v1.prepared_at_millis,
                expires_at_millis: v1.expires_at_millis,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
    pub(super) enum RestoreOperation {
        Prepared {
            prepared: PreparedRestore,
        },
        Executing {
            prepared: PreparedRestore,
            idempotency_id: String,
            receipt_id: String,
            approved_at_millis: i64,
            phase: RestorePhase,
        },
    }

    impl From<RestoreOperation> for super::RestoreOperation {
        fn from(v1: RestoreOperation) -> Self {
            match v1 {
                RestoreOperation::Prepared { prepared } => Self::Prepared {
                    prepared: prepared.into(),
                },
                RestoreOperation::Executing {
                    prepared,
                    idempotency_id,
                    receipt_id,
                    approved_at_millis,
                    phase,
                } => Self::Executing {
                    prepared: prepared.into(),
                    idempotency_id,
                    receipt_id,
                    approved_at_millis,
                    phase,
                },
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct RestoreTerminal {
        idempotency_id: String,
        prepared_intent_id: String,
        outcome: RestoreOutcome,
        recovery_archive_id: String,
        completed_at_millis: i64,
        #[serde(default)]
        projections_pending_for: Option<String>,
    }

    impl From<RestoreTerminal> for super::RestoreTerminal {
        fn from(v1: RestoreTerminal) -> Self {
            Self {
                idempotency_id: v1.idempotency_id,
                prepared_intent_id: v1.prepared_intent_id,
                outcome: v1.outcome,
                recovery: RecoveryEvidence::OperationalBackup {
                    archive_id: v1.recovery_archive_id,
                },
                completed_at_millis: v1.completed_at_millis,
                projections_pending_for: v1.projections_pending_for,
                resolved_recovery_failure: None,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct RestoreControl {
        #[allow(dead_code)]
        format: String,
        active: Option<RestoreOperation>,
        finished: Vec<RestoreTerminal>,
    }

    impl From<RestoreControl> for super::RestoreControl {
        fn from(v1: RestoreControl) -> Self {
            Self {
                format: super::FORMAT.to_owned(),
                // A v1 Prepared Intent was approved against the v1 preview
                // text, which this version no longer produces: it is dropped,
                // as a restart drops any Prepared Intent, so it can never be
                // approved. An executing one is past approval and is kept.
                active: v1.active.and_then(|operation| match operation {
                    RestoreOperation::Prepared { .. } => None,
                    executing @ RestoreOperation::Executing { .. } => Some(executing.into()),
                }),
                finished: v1.finished.into_iter().map(Into::into).collect(),
            }
        }
    }
}

#[derive(Debug)]
pub enum RestoreControlError {
    /// The document exists but is not one this version reads.
    Unreadable,
    /// Another writer held it for too long; nothing was written.
    Busy,
    Io(io::Error),
}

impl std::fmt::Display for RestoreControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("restore control record is unreadable"),
            Self::Busy => formatter.write_str("restore control record is held by another writer"),
            Self::Io(error) => write!(formatter, "restore control record I/O failed: {error}"),
        }
    }
}

impl std::error::Error for RestoreControlError {}

#[derive(Deserialize)]
struct FormatOnly {
    format: String,
}

/// A document's bytes as v2: read directly, converted from v1, or
/// unreadable. A v2 document whose file-set bindings do not match their own
/// digests is unreadable too.
fn parse(bytes: &[u8]) -> Result<RestoreControl, RestoreControlError> {
    let format = serde_json::from_slice::<FormatOnly>(bytes)
        .map_err(|_| RestoreControlError::Unreadable)?
        .format;
    let control = match format.as_str() {
        FORMAT => serde_json::from_slice::<RestoreControl>(bytes)
            .map_err(|_| RestoreControlError::Unreadable)?,
        FORMAT_V1 => serde_json::from_slice::<v1::RestoreControl>(bytes)
            .map_err(|_| RestoreControlError::Unreadable)?
            .into(),
        _ => return Err(RestoreControlError::Unreadable),
    };
    if !bindings_consistent(&control) {
        return Err(RestoreControlError::Unreadable);
    }
    Ok(control)
}

fn bindings_consistent(control: &RestoreControl) -> bool {
    let evidence_ok = |evidence: &RecoveryEvidence| match evidence {
        RecoveryEvidence::OperationalBackup { .. } => true,
        RecoveryEvidence::PreservationCopy { files, .. } => files.is_consistent(),
    };
    let prepared_ok = |prepared: &PreparedRestore| {
        evidence_ok(&prepared.recovery)
            && match &prepared.current {
                CurrentSourceBinding::Inspectable { .. } => true,
                CurrentSourceBinding::Opaque { files } => files.is_consistent(),
            }
    };
    let active_ok = match &control.active {
        None => true,
        Some(
            RestoreOperation::Prepared { prepared } | RestoreOperation::Executing { prepared, .. },
        ) => prepared_ok(prepared),
    };
    active_ok
        && control
            .finished
            .iter()
            .all(|terminal| evidence_ok(&terminal.recovery))
}

#[derive(Clone, Debug)]
pub struct RestoreControlStore {
    path: PathBuf,
}

impl RestoreControlStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The record, or an empty one when none was written yet.
    pub fn load(&self) -> Result<RestoreControl, RestoreControlError> {
        match fs::read(&self.path) {
            Ok(bytes) => parse(&bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RestoreControl::default()),
            Err(error) => Err(RestoreControlError::Io(error)),
        }
    }

    /// Read, change and durably replace the record under the exclusive lock.
    /// `change` decides; an `Err` from it writes nothing. The new document is
    /// synced before it replaces the old, so once this returns `Ok` the
    /// change survives a crash. It is always written as v2.
    pub fn update<R, E>(
        &self,
        change: impl FnOnce(&mut RestoreControl) -> Result<R, E>,
    ) -> Result<Result<R, E>, RestoreControlError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(RestoreControlError::Io)?;
        }
        let _guard = self.lock()?;
        let mut control = self.load()?;
        let decided = match change(&mut control) {
            Ok(decided) => decided,
            Err(refused) => return Ok(Err(refused)),
        };
        control.format = FORMAT.to_owned();
        let bytes = serde_json::to_vec_pretty(&control)
            .map_err(|error| RestoreControlError::Io(io::Error::other(error)))?;
        let mut file = AtomicWriteFile::open(&self.path).map_err(RestoreControlError::Io)?;
        file.write_all(&bytes).map_err(RestoreControlError::Io)?;
        file.commit().map_err(RestoreControlError::Io)?;
        Ok(Ok(decided))
    }

    /// An exclusive hold on `<control>.lock`, released when dropped. On
    /// Windows the file is opened with no sharing, which other processes
    /// cannot also do, and the OS releases it if this process dies. Elsewhere
    /// it serializes nothing across processes (PMC ships for Windows only).
    fn lock(&self) -> Result<File, RestoreControlError> {
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
                Err(error) => return Err(RestoreControlError::Io(error)),
            }
        }
        Err(RestoreControlError::Busy)
    }
}
