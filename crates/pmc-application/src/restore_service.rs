//! Operational Restore (S7-B1, H2b; DG3 restore amendment, accepted
//! 2026-09-22; the 8e design; product-owner decisions of 2026-09-22).
//!
//! 1. [`BackupService::check_archive`] decrypts the whole archive into a
//!    private work area and checks it: the exact members, the Ledger snapshot
//!    against the manifest (read-only, version-aware), the authority
//!    inventory re-derived byte for byte, the settings export. Nothing is
//!    replaced; a newer or unsupported Ledger stops here. Refused while a
//!    restore is executing: its work area holds that restore's rollback
//!    material.
//! 2. [`BackupService::prepare_restore`] makes a recovery backup of the
//!    current Ledger and settings, binds the preview to the current Ledger's
//!    exact content and to the checked members' hashes, keeps the current
//!    settings as a synced preimage, and records the H2b Prepared Intent.
//! 3. [`BackupService::approve_restore`] checks the acknowledged digest and
//!    the typed date and, under the control record's lock, turns `Prepared`
//!    into `Executing` — durably, before any effect — so an intent is used
//!    once and a restart never runs an approval again.
//! 4. [`BackupService::execute_restore`], with every Ledger handle closed by
//!    the host: claim the execution (one executor only), confirm the Ledger
//!    and the checked files are still what was bound, then — each step
//!    journaled first — move the Ledger aside, move the checked snapshot into
//!    its name, apply the archived settings, and check the result. Success is
//!    recorded before the rollback copy is removed; a failure after the first
//!    move puts the previous Ledger and settings back.
//! 5. [`BackupService::reconcile_restore`] finishes whatever a crash
//!    interrupted, by putting things back, never by going forward.
//!
//! The file replacement is two std renames with a journal (product owner,
//! 2026-09-22): no platform API beyond `std`, no `unsafe`.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    inspect_ledger, inspect_standalone_snapshot, LedgerCompatibility, LedgerInspection,
    LedgerSnapshotManifest, SqliteProductLedger, UpgradeSource, CURRENT_SCHEMA_VERSION,
};
use pmc_platform::backup_archive::{read_archive, ArchiveError, Passphrase};
use pmc_platform::backup_registry::{
    check_record, container_sha256, reader_sha256, BackupRecord, RecordStatus,
};
use pmc_platform::host_audit::AuditOutcome;
use pmc_platform::local_time::local_calendar_date;
use pmc_platform::preservation::{
    bind_ledger_files_exclusive, bind_member_paths, preserve_ledger_files, PreservationError,
};
use pmc_platform::restore_control::{
    CurrentSourceBinding, FileSetBinding, LedgerBinding, LedgerFileMember, PreparedRestore,
    RecoveryEvidence, RestoreControl, RestoreControlError, RestoreOperation, RestoreOutcome,
    RestorePhase, RestoreTerminal, UnopenedReason,
};
use pmc_platform::settings::{
    BackupSettings, CanonicalDirectoryPath, PatchOutcome, SettingsDocument, SettingsStore,
};

use crate::backup_service::BackupService;
use crate::operational_backup::{
    clear_work_area, preservation_file_name, BackupError, INVENTORY_MEMBER, LEDGER_MEMBER,
    SETTINGS_MEMBER,
};

/// Host audit codes of a restore (ADR 0012 §3).
pub const RESTORE_PREPARED: &str = "restore.prepared";
pub const RESTORE_REJECTED: &str = "restore.rejected";
pub const RESTORE_STARTED: &str = "restore.started";
pub const RESTORE_FINISHED: &str = "restore.finished";

const SETTINGS_PREIMAGE: &str = "settings-preimage.json";

/// Why a restore step stopped. None of these has changed anything unless
/// the step says so.
#[derive(Debug)]
pub enum RestoreError {
    /// The file could not be read.
    Unreadable,
    /// The passphrase does not open this backup.
    WrongPassphrase,
    /// A check of the archive failed.
    Damaged,
    /// Made by a newer PMC.
    NewerVersion,
    /// A Ledger older than any this binary can open or upgrade.
    UnsupportedOld,
    /// The recovery backup of the current Ledger failed.
    RecoveryBackupFailed(BackupError),
    /// The current Ledger is not open and cannot be inspected, so no
    /// Operational Backup of it can be made (the preservation copy of the
    /// restore-unopened amendment's second cut is not built yet).
    CurrentNotInspectable,
    /// The current Ledger files could not be preserved and verified (§3.4,
    /// §5); nothing was changed.
    PreservationFailed(PreservationError),
    /// Another restore is executing.
    AnotherRestoreActive,
    /// No prepared or approved restore with that id, or it expired, or
    /// another executor already claimed it.
    NotPrepared,
    /// The acknowledged digest is not the preview's.
    DigestMismatch,
    /// The typed date is not the one shown.
    ConfirmationMismatch,
    /// The restore control record could not be read or written.
    Control(RestoreControlError),
    /// The workspace or its work area is unavailable.
    WorkArea(io::Error),
}

/// A checked archive, ready to preview.
#[derive(Clone, Debug)]
pub struct CheckedArchive {
    pub archive_id: String,
    pub container_sha256: String,
    pub created_at: String,
    pub schema_version: u32,
    pub ledger_revision: u64,
    pub record_count: u64,
    /// Older than this binary's schema: after the restore, the upgrade gate.
    pub needs_upgrade: bool,
    pub settings: BackupSettings,
    snapshot_sha256: String,
    settings_member_sha256: String,
    inventory_member_sha256: String,
}

/// The exact preview of DG3 restore amendment §3.4, and its digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestorePreview {
    pub prepared_intent_id: String,
    pub payload_sha256: String,
    pub archive_created_at: String,
    pub archive_schema_version: u32,
    pub archive_record_count: u64,
    /// `None` when the current Ledger cannot be inspected (§3.4): its count
    /// is unavailable, never zero.
    pub current_record_count: Option<u64>,
    /// When the current Ledger last changed, read from it (open or closed);
    /// None only when it holds no record.
    pub current_last_change_at_millis: Option<i64>,
    /// The recovery evidence's stable identity.
    pub recovery_archive_id: String,
    /// Which kind of recovery evidence it is (§1).
    pub recovery_kind: RecoveryKind,
    /// Its file name, which the sheet names it by.
    pub recovery_name: String,
    pub recovery_verified_at_millis: i64,
    pub confirmation_date: String,
    pub needs_upgrade: bool,
    pub expires_at_millis: i64,
}

/// How an executed restore ended, and whether "nothing was replaced" was
/// because the current Ledger's files changed after the preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutedRestore {
    pub outcome: RestoreOutcome,
    pub source_changed: bool,
}

impl From<RestoreOutcome> for ExecutedRestore {
    fn from(outcome: RestoreOutcome) -> Self {
        Self {
            outcome,
            source_changed: false,
        }
    }
}

/// The two kinds of recovery evidence (restore-unopened amendment §1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryKind {
    OperationalBackup,
    PreservationCopy,
}

/// What step 2 made to keep the current workspace, for the common end.
struct Kept {
    current: CurrentSourceBinding,
    unopened_reason: Option<UnopenedReason>,
    last_change_at_millis: Option<i64>,
    current_record_count: Option<u64>,
    recovery: RecoveryEvidence,
    recovery_name: String,
    recovery_verified_at_millis: i64,
}

impl Kept {
    fn backed_up(
        binding: LedgerBinding,
        unopened_reason: Option<UnopenedReason>,
        last_change_at_millis: Option<i64>,
        record: &BackupRecord,
    ) -> Self {
        Self {
            current: CurrentSourceBinding::Inspectable { binding },
            unopened_reason,
            last_change_at_millis,
            current_record_count: Some(record.authority_record_count),
            recovery: RecoveryEvidence::OperationalBackup {
                archive_id: record.archive_id.clone(),
            },
            recovery_name: record.file_name.clone(),
            recovery_verified_at_millis: record.verified_at_millis,
        }
    }
}

/// Where the recovery backup of the current Ledger goes. Its settings are
/// always the bound preimage's own export, never supplied separately.
pub struct RecoveryBackupInput<'a> {
    pub destination: &'a CanonicalDirectoryPath,
    pub passphrase: &'a Passphrase,
    pub archive_id: &'a str,
}

/// A backup file the person picked in the host's file dialog (ADR 0011). The
/// host holds it behind an opaque token and tells the webview its file name
/// only; its location never leaves this type.
#[derive(Clone, Debug)]
pub struct ChosenArchive {
    file: PathBuf,
}

impl ChosenArchive {
    /// A picked file, if it is a regular file (not a link or a folder).
    #[must_use]
    pub fn from_picked(file: PathBuf) -> Option<Self> {
        let metadata = fs::symlink_metadata(&file).ok()?;
        metadata.is_file().then_some(Self { file })
    }

    /// The file's own name, which the sheet shows.
    #[must_use]
    pub fn file_name(&self) -> String {
        self.file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// The Ledger reopened after a restore, or at startup.
pub enum ReopenedLedger {
    Ready(SqliteProductLedger),
    /// An older supported schema: the upgrade gate (8f) comes first.
    UpgradeRequired,
    Failed,
}

fn staged_path(ledger: &Path, prepared_intent_id: &str) -> PathBuf {
    sibling(ledger, &format!(".restore-{prepared_intent_id}.staged"))
}

fn previous_path(ledger: &Path, prepared_intent_id: &str) -> PathBuf {
    sibling(ledger, &format!(".restore-{prepared_intent_id}.previous"))
}

/// Where a member of the previous file set waits: the Ledger's rollback name,
/// and each sidecar beside it with its own suffix.
fn previous_member_path(
    ledger: &Path,
    prepared_intent_id: &str,
    member: LedgerFileMember,
) -> PathBuf {
    let mut name = previous_path(ledger, prepared_intent_id).into_os_string();
    name.push(member.suffix());
    PathBuf::from(name)
}

/// The file set a restore binds by content, if it does.
const fn opaque_files(current: &CurrentSourceBinding) -> Option<&FileSetBinding> {
    match current {
        CurrentSourceBinding::Opaque { files } => Some(files),
        CurrentSourceBinding::Inspectable { .. } => None,
    }
}

/// After a restore recorded `Restored`: remove the rollback copy of the
/// Ledger and — for an opaque restore — of exactly the sidecars it bound and
/// moved. Nothing else at a rollback name is touched.
fn remove_rollback_set(ledger: &Path, prepared_intent_id: &str, moved: Option<&FileSetBinding>) {
    let _ = fs::remove_file(previous_path(ledger, prepared_intent_id));
    for bound in moved.map(FileSetBinding::members).unwrap_or_default() {
        if bound.member != LedgerFileMember::Ledger {
            let _ = fs::remove_file(previous_member_path(
                ledger,
                prepared_intent_id,
                bound.member,
            ));
        }
    }
}

fn member_path_of(ledger: &Path, member: LedgerFileMember) -> PathBuf {
    pmc_platform::preservation::member_path(ledger, member)
}

fn sibling(ledger: &Path, name: &str) -> PathBuf {
    ledger.with_file_name(format!(
        "{}{name}",
        ledger
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    ))
}

/// A closed Ledger's SQLite sidecars: a non-empty write-ahead log means it
/// was not closed cleanly and must not be moved (`false`); an empty log and
/// a shared-memory index are stale leftovers of a clean close and are
/// removed, so nothing but the file itself takes part in the replacement.
fn clear_stale_sidecars(ledger: &Path) -> bool {
    let wal = sibling(ledger, "-wal");
    match fs::metadata(&wal) {
        Ok(metadata) if metadata.len() > 0 => return false,
        Ok(_) => {
            if fs::remove_file(&wal).is_err() {
                return false;
            }
        }
        Err(_) => {}
    }
    let shm = sibling(ledger, "-shm");
    !shm.exists() || fs::remove_file(&shm).is_ok()
}

fn binding_of(inspection: &LedgerInspection) -> LedgerBinding {
    LedgerBinding {
        schema_version: inspection.schema_version,
        ledger_revision: inspection.ledger_revision,
        inventory_sha256: inspection.inventory.sha256.clone(),
        content_sha256: inspection.content_sha256.clone(),
    }
}

/// The Ledger file's binding now, if it is a supported Ledger.
fn current_binding(ledger: &Path) -> Option<LedgerBinding> {
    match inspect_ledger(ledger).ok()? {
        LedgerCompatibility::Current(inspection)
        | LedgerCompatibility::UpgradeableFrom(inspection) => Some(binding_of(&inspection)),
        _ => None,
    }
}

/// Whether the Ledger's files are still what the preview bound.
fn still_bound(ledger: &Path, current: &CurrentSourceBinding) -> bool {
    match current {
        CurrentSourceBinding::Inspectable { binding } => {
            current_binding(ledger).as_ref() == Some(binding)
        }
        // Every present member opened with no sharing and hashed while all
        // are held; a member another program holds is "not bound".
        CurrentSourceBinding::Opaque { files } => {
            bind_ledger_files_exclusive(ledger).is_ok_and(|now| now == *files)
        }
    }
}

fn current_line(current: &CurrentSourceBinding) -> String {
    match current {
        CurrentSourceBinding::Inspectable { binding } => format!(
            "inspectable\t{}\t{}\t{}\t{}",
            binding.schema_version,
            binding.ledger_revision,
            binding.inventory_sha256,
            binding.content_sha256
        ),
        CurrentSourceBinding::Opaque { files } => format!("opaque\t{}", files.aggregate_sha256()),
    }
}

fn recovery_line(recovery: &RecoveryEvidence) -> String {
    match recovery {
        RecoveryEvidence::OperationalBackup { archive_id } => {
            format!("operational_backup\t{archive_id}")
        }
        RecoveryEvidence::PreservationCopy {
            preservation_id,
            files,
            ..
        } => format!(
            "preservation_copy\t{preservation_id}\t{}",
            files.aggregate_sha256()
        ),
    }
}

const fn unopened_name(reason: Option<UnopenedReason>) -> &'static str {
    match reason {
        None => "open",
        Some(UnopenedReason::UpgradeRequired) => "upgrade_required",
        Some(UnopenedReason::UnsupportedOld) => "unsupported_old",
        Some(UnopenedReason::OpenFailed) => "open_failed",
        Some(UnopenedReason::RestoreRecoveryRequired) => "restore_recovery_required",
    }
}

/// The canonical text the person approves; its SHA-256 is the digest. v2
/// (restore-unopened amendment §6): the kind of what is replaced and of the
/// recovery evidence are part of what is approved.
fn preview_payload(prepared: &PreparedRestore) -> String {
    format!(
        "pmc-restore-preview/v2\nintent\t{}\narchive\t{}\t{}\t{}\t{}\t{}\t{}\nsnapshot\t{}\nmembers\t{}\t{}\ncurrent\t{}\nunopened\t{}\nsettings_revision\t{}\nrecovery\t{}\nconfirm\t{}\t{}\nexpires\t{}\n",
        prepared.prepared_intent_id,
        prepared.archive_id,
        prepared.archive_container_sha256,
        prepared.archive_created_at,
        prepared.archive_schema_version,
        prepared.archive_ledger_revision,
        prepared.archive_record_count,
        prepared.snapshot_sha256,
        prepared.settings_member_sha256,
        prepared.inventory_member_sha256,
        current_line(&prepared.current),
        unopened_name(prepared.unopened_reason),
        prepared.settings_revision,
        recovery_line(&prepared.recovery),
        prepared.confirmation_date,
        prepared.confirmation_timezone,
        prepared.expires_at_millis,
    )
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    reader_sha256(bytes).unwrap_or_default()
}

fn file_sha256(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| sha256_bytes(&bytes))
}

/// Write and flush a file (FlushFileBuffers needs write access on Windows).
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

impl BackupService {
    fn restore_ledger(&self) -> Result<&PathBuf, RestoreError> {
        self.ledger
            .as_ref()
            .ok_or_else(|| RestoreError::WorkArea(io::Error::from(io::ErrorKind::NotFound)))
    }

    fn archive_dir(&self) -> PathBuf {
        self.restore_work.join("archive")
    }

    /// [`BackupService::check_archive`] for a file picked in the host.
    pub fn check_chosen_archive(
        &self,
        chosen: &ChosenArchive,
        passphrase: &Passphrase,
    ) -> Result<CheckedArchive, RestoreError> {
        self.check_archive(&chosen.file, passphrase)
    }

    /// Open this workspace's Ledger, saying why not when it cannot be.
    #[must_use]
    pub fn reopen_ledger(&self) -> ReopenedLedger {
        let Some(ledger) = self.ledger.as_ref() else {
            return ReopenedLedger::Failed;
        };
        match inspect_ledger(ledger) {
            Ok(LedgerCompatibility::UpgradeableFrom(_)) => ReopenedLedger::UpgradeRequired,
            Ok(LedgerCompatibility::Current(_)) => SqliteProductLedger::open(ledger)
                .map_or(ReopenedLedger::Failed, ReopenedLedger::Ready),
            _ => ReopenedLedger::Failed,
        }
    }

    /// The recovery backup a failed restore left, as a chosen archive, so
    /// System Health can offer it first (restore-unopened amendment §2) — no
    /// picker: the backup registry says where it is, and only an Operational
    /// Backup still listed and unaltered is offered. `None` otherwise.
    #[must_use]
    pub fn recovery_archive(&self) -> Option<ChosenArchive> {
        let control = self.restore_control.load().ok()?;
        let RecoveryEvidence::OperationalBackup { archive_id } =
            &unresolved_recovery_failure(&control)?.recovery
        else {
            return None;
        };
        let record = self
            .registry
            .load()
            .ok()?
            .records
            .into_iter()
            .find(|record| record.archive_id == *archive_id)?;
        if check_record(&record) != RecordStatus::Valid {
            return None;
        }
        ChosenArchive::from_picked(record.destination.join(&record.file_name))
    }
    /// The file name of the recovery backup System Health names when a
    /// restore could not be put back, or is still marked executing (the
    /// archive id when the registry no longer lists it).
    #[must_use]
    pub fn recovery_archive_needed(&self) -> Option<String> {
        let control = self.restore_control.load().ok()?;
        let archive_id = match control.active {
            Some(RestoreOperation::Executing { prepared, .. }) => {
                prepared.recovery.identity().to_owned()
            }
            _ => unresolved_recovery_failure(&control)
                .map(|finished| finished.recovery.identity().to_owned())?,
        };
        let listed = self.registry.load().ok().and_then(|registry| {
            registry
                .records
                .into_iter()
                .find(|record| record.archive_id == archive_id)
                .map(|record| record.file_name)
        });
        Some(listed.unwrap_or(archive_id))
    }

    /// Step 1: decrypt and check the whole archive; nothing is replaced. A
    /// restore still only prepared is dropped (its work area is reused); one
    /// executing is never touched.
    pub fn check_archive(
        &self,
        archive: &Path,
        passphrase: &Passphrase,
    ) -> Result<CheckedArchive, RestoreError> {
        self.restore_control
            .update(|control| match control.active {
                Some(RestoreOperation::Executing { .. }) => Err(RestoreError::AnotherRestoreActive),
                _ => {
                    control.active = None;
                    Ok(())
                }
            })
            .map_err(RestoreError::Control)??;
        let before = container_sha256(archive).map_err(|_| RestoreError::Unreadable)?;
        clear_work_area(&self.restore_work).map_err(RestoreError::WorkArea)?;
        let extracted = self.archive_dir();
        fs::create_dir(&extracted).map_err(RestoreError::WorkArea)?;
        let file = fs::File::open(archive).map_err(|_| RestoreError::Unreadable)?;
        let verified = read_archive(io::BufReader::new(file), passphrase, Some(&extracted))
            .map_err(|error| match error {
                ArchiveError::CannotDecrypt => RestoreError::WrongPassphrase,
                ArchiveError::Io(_) => RestoreError::Unreadable,
                _ => RestoreError::Damaged,
            })?;
        // The file read is the file hashed.
        if container_sha256(archive).map_err(|_| RestoreError::Unreadable)? != before {
            return Err(RestoreError::Damaged);
        }
        let manifest = verified.manifest;
        let mut names: Vec<&str> = manifest
            .members
            .iter()
            .map(|member| member.name.as_str())
            .collect();
        names.sort_unstable();
        if names != [INVENTORY_MEMBER, LEDGER_MEMBER, SETTINGS_MEMBER] {
            return Err(RestoreError::Damaged);
        }
        let snapshot = extracted.join(LEDGER_MEMBER);
        let needs_upgrade = match inspect_ledger(&snapshot).map_err(|_| RestoreError::Damaged)? {
            LedgerCompatibility::Current(_) => false,
            LedgerCompatibility::UpgradeableFrom(_) => true,
            LedgerCompatibility::Future { .. } => return Err(RestoreError::NewerVersion),
            LedgerCompatibility::UnsupportedOld { .. } => return Err(RestoreError::UnsupportedOld),
            _ => return Err(RestoreError::Damaged),
        };
        let claimed = LedgerSnapshotManifest::claimed(
            manifest.ledger_schema_version,
            manifest.ledger_revision,
            manifest.snapshot_checksum.clone(),
        );
        let inspection =
            inspect_standalone_snapshot(&snapshot, &claimed).map_err(|_| RestoreError::Damaged)?;
        let carried =
            fs::read(extracted.join(INVENTORY_MEMBER)).map_err(|_| RestoreError::Damaged)?;
        if carried != inspection.inventory.bytes {
            return Err(RestoreError::Damaged);
        }
        let settings_bytes =
            fs::read(extracted.join(SETTINGS_MEMBER)).map_err(|_| RestoreError::Damaged)?;
        let settings = BackupSettings::parse(&settings_bytes).map_err(|_| RestoreError::Damaged)?;
        Ok(CheckedArchive {
            archive_id: manifest.archive_id,
            container_sha256: before,
            created_at: manifest.created_at,
            schema_version: inspection.schema_version,
            ledger_revision: inspection.ledger_revision,
            record_count: inspection.inventory.record_count,
            needs_upgrade,
            settings,
            snapshot_sha256: manifest.snapshot_checksum,
            settings_member_sha256: sha256_bytes(&settings_bytes),
            inventory_member_sha256: sha256_bytes(&carried),
        })
    }

    /// Step 2: the recovery backup of the current Ledger and settings, the
    /// binding to the Ledger's exact content, the synced settings preimage,
    /// and the Prepared Intent. Called with the current Ledger held so no
    /// write can slip in before the binding is taken.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_restore(
        &self,
        checked: &CheckedArchive,
        current: &SqliteProductLedger,
        settings: &SettingsDocument,
        recovery: &RecoveryBackupInput<'_>,
        prepared_intent_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
        valid_for_millis: i64,
    ) -> Result<RestorePreview, RestoreError> {
        if matches!(
            self.restore_control
                .load()
                .map_err(RestoreError::Control)?
                .active,
            Some(RestoreOperation::Executing { .. })
        ) {
            return Err(RestoreError::AnotherRestoreActive);
        }
        // The checked files must still be the ones checked.
        if file_sha256(&self.archive_dir().join(SETTINGS_MEMBER)).as_deref()
            != Some(checked.settings_member_sha256.as_str())
            || file_sha256(&self.archive_dir().join(INVENTORY_MEMBER)).as_deref()
                != Some(checked.inventory_member_sha256.as_str())
        {
            return Err(RestoreError::Damaged);
        }
        // The recovery backup: the ordinary pipeline, of the current Ledger
        // and exactly the settings that will be replaced.
        let settings_json = settings
            .backup_export()
            .map_err(|_| RestoreError::RecoveryBackupFailed(BackupError::Snapshot))?;
        let snapshot = self
            .snapshot(current)
            .map_err(RestoreError::RecoveryBackupFailed)?;
        let binding = inspect_standalone_snapshot(&snapshot.path, &snapshot.manifest)
            .map(|inspection| binding_of(&inspection))
            .map_err(|_| RestoreError::RecoveryBackupFailed(BackupError::Snapshot))?;
        let record = self
            .publish(
                &snapshot,
                &settings_json,
                recovery.destination,
                recovery.passphrase,
                recovery.archive_id,
                clock,
                &|path, manifest| inspect_standalone_snapshot(path, manifest).is_ok(),
            )
            .map_err(RestoreError::RecoveryBackupFailed)?;
        let audited = self.audit(
            pmc_platform::host_audit::BACKUP_COMPLETED,
            AuditOutcome::Succeeded,
            vec![
                ("archive_id".to_owned(), record.archive_id.clone()),
                (
                    "record_count".to_owned(),
                    record.authority_record_count.to_string(),
                ),
                ("purpose".to_owned(), "pre_restore".to_owned()),
            ],
            clock(),
        );
        if !audited || self.register(&record).is_err() {
            return Err(RestoreError::RecoveryBackupFailed(BackupError::NotRecorded));
        }
        let last_change = current.last_change_at_millis().ok().flatten();
        self.finish_prepare(
            checked,
            settings,
            Kept::backed_up(binding, None, last_change, &record),
            prepared_intent_id,
            clock,
            valid_for_millis,
        )
    }

    /// Step 2 for a Ledger that is not open because it is an older supported
    /// format — the upgrade gate (DG3 restore-unopened amendment §1, §2;
    /// first cut, accepted 2026-09-23). The recovery evidence is a verified
    /// Operational Backup of the closed file, made the way the upgrade makes
    /// its own (`purpose: pre_restore`), and the preview binds the file's
    /// exact content, which the execution checks again before anything
    /// moves. Any other unopened state is refused: a file that cannot be
    /// inspected needs the preservation copy of the second cut.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_restore_closed(
        &self,
        checked: &CheckedArchive,
        settings: &SettingsDocument,
        recovery: &RecoveryBackupInput<'_>,
        prepared_intent_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
        valid_for_millis: i64,
    ) -> Result<RestorePreview, RestoreError> {
        if matches!(
            self.restore_control
                .load()
                .map_err(RestoreError::Control)?
                .active,
            Some(RestoreOperation::Executing { .. })
        ) {
            return Err(RestoreError::AnotherRestoreActive);
        }
        if file_sha256(&self.archive_dir().join(SETTINGS_MEMBER)).as_deref()
            != Some(checked.settings_member_sha256.as_str())
            || file_sha256(&self.archive_dir().join(INVENTORY_MEMBER)).as_deref()
                != Some(checked.inventory_member_sha256.as_str())
        {
            return Err(RestoreError::Damaged);
        }
        let ledger = self.restore_ledger()?.clone();
        let inspection = match inspect_ledger(&ledger) {
            Ok(LedgerCompatibility::UpgradeableFrom(inspection)) => inspection,
            _ => return Err(RestoreError::CurrentNotInspectable),
        };
        let settings_json = settings
            .backup_export()
            .map_err(|_| RestoreError::RecoveryBackupFailed(BackupError::Snapshot))?;
        let record = self
            .back_up_closed_ledger(
                &UpgradeSource::from(&inspection),
                &settings_json,
                recovery.destination,
                recovery.passphrase,
                recovery.archive_id,
                clock,
                "pre_restore",
            )
            .map_err(RestoreError::RecoveryBackupFailed)?;
        self.finish_prepare(
            checked,
            settings,
            Kept::backed_up(
                binding_of(&inspection),
                Some(UnopenedReason::UpgradeRequired),
                inspection.last_change_at_millis,
                &record,
            ),
            prepared_intent_id,
            clock,
            valid_for_millis,
        )
    }

    /// Step 2 for a Ledger PMC cannot inspect — it does not open, a failed
    /// recovery left it uncertain, or it is an older format this version
    /// cannot read (DG3 restore-unopened amendment §1, §2; cut B). The
    /// recovery evidence is a preservation copy of its exact files, made in
    /// the backup folder and matched byte for byte before it counts; the
    /// preview binds those files by content, and says the Ledger's count and
    /// last change are unavailable rather than inventing them.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_restore_preserved(
        &self,
        checked: &CheckedArchive,
        settings: &SettingsDocument,
        reason: UnopenedReason,
        recovery: &RecoveryBackupInput<'_>,
        prepared_intent_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
        valid_for_millis: i64,
    ) -> Result<RestorePreview, RestoreError> {
        // A Ledger waiting for its upgrade inspects: it takes the backup path.
        if reason == UnopenedReason::UpgradeRequired {
            return Err(RestoreError::CurrentNotInspectable);
        }
        if matches!(
            self.restore_control
                .load()
                .map_err(RestoreError::Control)?
                .active,
            Some(RestoreOperation::Executing { .. })
        ) {
            return Err(RestoreError::AnotherRestoreActive);
        }
        if file_sha256(&self.archive_dir().join(SETTINGS_MEMBER)).as_deref()
            != Some(checked.settings_member_sha256.as_str())
            || file_sha256(&self.archive_dir().join(INVENTORY_MEMBER)).as_deref()
                != Some(checked.inventory_member_sha256.as_str())
        {
            return Err(RestoreError::Damaged);
        }
        let ledger = self.restore_ledger()?.clone();
        let created = clock();
        let file_name = preservation_file_name(created, recovery.archive_id);
        let copy = preserve_ledger_files(
            &ledger,
            recovery.destination.as_path(),
            recovery.passphrase,
            recovery.archive_id,
            &created.to_rfc3339_utc(),
            &file_name,
        )
        .map_err(RestoreError::PreservationFailed)?;
        let verified_at = clock().unix_millis();
        self.finish_prepare(
            checked,
            settings,
            Kept {
                current: CurrentSourceBinding::Opaque {
                    files: copy.files.clone(),
                },
                unopened_reason: Some(reason),
                last_change_at_millis: None,
                current_record_count: None,
                recovery: RecoveryEvidence::PreservationCopy {
                    preservation_id: copy.preservation_id,
                    file_name: copy.file_name.clone(),
                    files: copy.files,
                },
                recovery_name: copy.file_name,
                recovery_verified_at_millis: verified_at,
            },
            prepared_intent_id,
            clock,
            valid_for_millis,
        )
    }

    /// Step 2's common end: the synced settings preimage, the Prepared
    /// Intent bound to what is replaced and what was kept, and the preview.
    fn finish_prepare(
        &self,
        checked: &CheckedArchive,
        settings: &SettingsDocument,
        kept: Kept,
        prepared_intent_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
        valid_for_millis: i64,
    ) -> Result<RestorePreview, RestoreError> {
        // The settings as they are, flushed before any approval can exist.
        let preimage = serde_json_bytes(settings)?;
        write_synced(&self.restore_work.join(SETTINGS_PREIMAGE), &preimage)
            .map_err(RestoreError::WorkArea)?;

        let now = clock().unix_millis();
        let timezone = settings.display.timezone.clone();
        let confirmation_date =
            local_calendar_date(&checked.created_at, &timezone).ok_or(RestoreError::Damaged)?;
        let mut prepared = PreparedRestore {
            prepared_intent_id: prepared_intent_id.to_owned(),
            payload_sha256: String::new(),
            archive_id: checked.archive_id.clone(),
            archive_container_sha256: checked.container_sha256.clone(),
            archive_created_at: checked.created_at.clone(),
            archive_schema_version: checked.schema_version,
            archive_ledger_revision: checked.ledger_revision,
            archive_record_count: checked.record_count,
            snapshot_sha256: checked.snapshot_sha256.clone(),
            settings_member_sha256: checked.settings_member_sha256.clone(),
            inventory_member_sha256: checked.inventory_member_sha256.clone(),
            current: kept.current,
            unopened_reason: kept.unopened_reason,
            settings_revision: settings.revision,
            recovery: kept.recovery.clone(),
            confirmation_date: confirmation_date.clone(),
            confirmation_timezone: timezone,
            prepared_at_millis: now,
            expires_at_millis: now.saturating_add(valid_for_millis),
        };
        prepared.payload_sha256 = sha256_bytes(preview_payload(&prepared).as_bytes());
        let stored = prepared.clone();
        self.restore_control
            .update(|control| match control.active {
                Some(RestoreOperation::Executing { .. }) => Err(RestoreError::AnotherRestoreActive),
                _ => {
                    control.active = Some(RestoreOperation::Prepared { prepared: stored });
                    Ok(())
                }
            })
            .map_err(RestoreError::Control)??;
        let _ = self.audit(
            RESTORE_PREPARED,
            AuditOutcome::Succeeded,
            vec![
                (
                    "prepared_intent_id".to_owned(),
                    prepared.prepared_intent_id.clone(),
                ),
                ("archive_id".to_owned(), prepared.archive_id.clone()),
                (
                    "recovery_archive_id".to_owned(),
                    kept.recovery.identity().to_owned(),
                ),
                (
                    "recovery_kind".to_owned(),
                    match kept.recovery {
                        RecoveryEvidence::OperationalBackup { .. } => "operational_backup",
                        RecoveryEvidence::PreservationCopy { .. } => "preservation_copy",
                    }
                    .to_owned(),
                ),
            ],
            clock(),
        );
        Ok(RestorePreview {
            prepared_intent_id: prepared.prepared_intent_id,
            payload_sha256: prepared.payload_sha256,
            archive_created_at: prepared.archive_created_at,
            archive_schema_version: prepared.archive_schema_version,
            archive_record_count: prepared.archive_record_count,
            current_record_count: kept.current_record_count,
            current_last_change_at_millis: kept.last_change_at_millis,
            recovery_archive_id: kept.recovery.identity().to_owned(),
            recovery_kind: match kept.recovery {
                RecoveryEvidence::OperationalBackup { .. } => RecoveryKind::OperationalBackup,
                RecoveryEvidence::PreservationCopy { .. } => RecoveryKind::PreservationCopy,
            },
            recovery_name: kept.recovery_name,
            recovery_verified_at_millis: kept.recovery_verified_at_millis,
            confirmation_date,
            needs_upgrade: checked.needs_upgrade,
            expires_at_millis: prepared.expires_at_millis,
        })
    }

    /// Reject: the Prepared Intent is discarded and the choice recorded.
    pub fn reject_restore(
        &self,
        prepared_intent_id: &str,
        now: UtcTimestamp,
    ) -> Result<(), RestoreError> {
        self.restore_control
            .update(|control| match &control.active {
                Some(RestoreOperation::Prepared { prepared })
                    if prepared.prepared_intent_id == prepared_intent_id =>
                {
                    control.active = None;
                    Ok(())
                }
                _ => Err(RestoreError::NotPrepared),
            })
            .map_err(RestoreError::Control)??;
        let _ = clear_work_area(&self.restore_work);
        let _ = self.audit(
            RESTORE_REJECTED,
            AuditOutcome::Refused,
            vec![(
                "prepared_intent_id".to_owned(),
                prepared_intent_id.to_owned(),
            )],
            now,
        );
        Ok(())
    }

    /// Step 3: approve. Under the control record's lock the Prepared Intent
    /// is checked and consumed — `Executing` is durable before this returns.
    /// A repeated idempotency id returns the outcome it already had.
    pub fn approve_restore(
        &self,
        prepared_intent_id: &str,
        acknowledged_payload_sha256: &str,
        typed_date: &str,
        idempotency_id: &str,
        receipt_id: &str,
        now: UtcTimestamp,
    ) -> Result<Option<RestoreOutcome>, RestoreError> {
        let now_millis = now.unix_millis();
        self.restore_control
            .update(|control| {
                if let Some(finished) = control
                    .finished
                    .iter()
                    .find(|finished| finished.idempotency_id == idempotency_id)
                {
                    return Ok(Some(finished.outcome));
                }
                let Some(RestoreOperation::Prepared { prepared }) = &control.active else {
                    return Err(RestoreError::NotPrepared);
                };
                if prepared.prepared_intent_id != prepared_intent_id
                    || now_millis > prepared.expires_at_millis
                {
                    return Err(RestoreError::NotPrepared);
                }
                if prepared.payload_sha256 != acknowledged_payload_sha256
                    || sha256_bytes(preview_payload(prepared).as_bytes()) != prepared.payload_sha256
                {
                    return Err(RestoreError::DigestMismatch);
                }
                if typed_date != prepared.confirmation_date {
                    return Err(RestoreError::ConfirmationMismatch);
                }
                control.active = Some(RestoreOperation::Executing {
                    prepared: prepared.clone(),
                    idempotency_id: idempotency_id.to_owned(),
                    receipt_id: receipt_id.to_owned(),
                    approved_at_millis: now_millis,
                    phase: RestorePhase::Approved,
                });
                Ok(None)
            })
            .map_err(RestoreError::Control)?
    }

    /// Move the executing restore from `from` to `to`, only if it is at
    /// `from`: the compare-and-set that lets exactly one executor proceed.
    fn advance(&self, from: RestorePhase, to: RestorePhase) -> Result<(), RestoreError> {
        self.restore_control
            .update(|control| match &mut control.active {
                Some(RestoreOperation::Executing { phase, .. }) if *phase == from => {
                    *phase = to;
                    Ok(())
                }
                _ => Err(RestoreError::NotPrepared),
            })
            .map_err(RestoreError::Control)?
    }

    fn finish(
        &self,
        outcome: RestoreOutcome,
        projections_pending_for: Option<String>,
        now: UtcTimestamp,
    ) -> Result<RestoreOutcome, RestoreError> {
        let mut facts = Vec::new();
        self.restore_control
            .update(|control| {
                let Some(RestoreOperation::Executing {
                    prepared,
                    idempotency_id,
                    ..
                }) = control.active.take()
                else {
                    return Err(RestoreError::NotPrepared);
                };
                facts = vec![
                    (
                        "prepared_intent_id".to_owned(),
                        prepared.prepared_intent_id.clone(),
                    ),
                    ("archive_id".to_owned(), prepared.archive_id.clone()),
                    (
                        "recovery_archive_id".to_owned(),
                        prepared.recovery.identity().to_owned(),
                    ),
                    ("outcome".to_owned(), format!("{outcome:?}")),
                ];
                // A restore over the uncertain files of a failed recovery
                // that succeeded resolves that failure; the record stays.
                let resolved_recovery_failure = (outcome == RestoreOutcome::Restored
                    && prepared.unopened_reason == Some(UnopenedReason::RestoreRecoveryRequired))
                .then(|| unresolved_recovery_failure(control))
                .flatten()
                .map(|failed| failed.idempotency_id.clone());
                control.finished.push(RestoreTerminal {
                    idempotency_id,
                    prepared_intent_id: prepared.prepared_intent_id,
                    outcome,
                    recovery: prepared.recovery,
                    completed_at_millis: now.unix_millis(),
                    projections_pending_for,
                    resolved_recovery_failure,
                });
                Ok(())
            })
            .map_err(RestoreError::Control)??;
        let _ = self.audit(
            RESTORE_FINISHED,
            match outcome {
                RestoreOutcome::Restored => AuditOutcome::Succeeded,
                _ => AuditOutcome::Failed,
            },
            facts,
            now,
        );
        // A failed recovery keeps its material for System Health.
        if outcome != RestoreOutcome::RecoveryFailed {
            let _ = clear_work_area(&self.restore_work);
        }
        Ok(outcome)
    }

    /// Step 4, with every Ledger handle closed by the caller.
    pub fn execute_restore(
        &self,
        settings_store: &SettingsStore,
        now: UtcTimestamp,
    ) -> Result<RestoreOutcome, RestoreError> {
        self.execute_restore_explained(settings_store, now)
            .map(|executed| executed.outcome)
    }

    /// [`Self::execute_restore`], saying also whether nothing was replaced
    /// because the current Ledger's files were no longer what the preview
    /// bound (restore-unopened amendment §5: its own message).
    pub fn execute_restore_explained(
        &self,
        settings_store: &SettingsStore,
        now: UtcTimestamp,
    ) -> Result<ExecutedRestore, RestoreError> {
        let ledger = self.restore_ledger()?.clone();
        // Exactly one executor: Approved -> Staging under the lock.
        self.advance(RestorePhase::Approved, RestorePhase::Staging)?;
        let prepared = match self
            .restore_control
            .load()
            .map_err(RestoreError::Control)?
            .active
        {
            Some(RestoreOperation::Executing { prepared, .. }) => prepared,
            _ => return Err(RestoreError::NotPrepared),
        };
        let _ = self.audit(
            RESTORE_STARTED,
            AuditOutcome::Succeeded,
            vec![(
                "prepared_intent_id".to_owned(),
                prepared.prepared_intent_id.clone(),
            )],
            now,
        );
        // Before anything moves: the Ledger still exactly what was bound and
        // closed cleanly; the checked snapshot, settings and inventory still
        // the files that were checked.
        let snapshot = self.archive_dir().join(LEDGER_MEMBER);
        let claimed = LedgerSnapshotManifest::claimed(
            prepared.archive_schema_version,
            prepared.archive_ledger_revision,
            prepared.snapshot_sha256.clone(),
        );
        let settings_bytes = fs::read(self.archive_dir().join(SETTINGS_MEMBER)).ok();
        let settings = settings_bytes
            .as_deref()
            .filter(|bytes| sha256_bytes(bytes) == prepared.settings_member_sha256)
            .and_then(|bytes| BackupSettings::parse(bytes).ok());
        let inventory_ok = file_sha256(&self.archive_dir().join(INVENTORY_MEMBER)).as_deref()
            == Some(prepared.inventory_member_sha256.as_str());
        // Settings changed since the preview: its recovery backup and preimage
        // hold the older ones, so going on would lose the change.
        let settings_unchanged = settings_store
            .read()
            .is_ok_and(|document| document.revision == prepared.settings_revision);
        // An opaque file set's sidecars are part of what is bound and what is
        // moved: never cleared as leftovers.
        let opaque = opaque_files(&prepared.current).cloned();
        let source_bound = still_bound(&ledger, &prepared.current);
        let ready = settings_unchanged
            && source_bound
            && (opaque.is_some() || clear_stale_sidecars(&ledger))
            && inspect_standalone_snapshot(&snapshot, &claimed).is_ok()
            && inventory_ok;
        let Some(settings) = settings.filter(|_| ready) else {
            return self
                .finish(RestoreOutcome::FailedBeforeReplacement, None, now)
                .map(|outcome| ExecutedRestore {
                    outcome,
                    source_changed: !source_bound,
                });
        };
        // Stage it beside the Ledger, same volume, so installing is a rename.
        let staged = staged_path(&ledger, &prepared.prepared_intent_id);
        let _ = fs::remove_file(&staged);
        let staged_ok = fs::copy(&snapshot, &staged).is_ok()
            && fs::OpenOptions::new()
                .write(true)
                .open(&staged)
                .and_then(|file| file.sync_all())
                .is_ok()
            && inspect_standalone_snapshot(&staged, &claimed).is_ok();
        if !staged_ok {
            let _ = fs::remove_file(&staged);
            return self
                .finish(RestoreOutcome::FailedBeforeReplacement, None, now)
                .map(ExecutedRestore::from);
        }

        let previous = previous_path(&ledger, &prepared.prepared_intent_id);
        self.advance(RestorePhase::Staging, RestorePhase::MovingLiveAside)?;
        if fs::rename(&ledger, &previous).is_err() {
            // The live name was not moved: nothing changed.
            let _ = fs::remove_file(&staged);
            return self
                .finish(RestoreOutcome::FailedBeforeReplacement, None, now)
                .map(ExecutedRestore::from);
        }
        // From here a failure puts the previous Ledger back. An opaque file
        // set moves whole, and what now sits at the rollback names must be
        // exactly the set the preview bound — which also catches a change in
        // the moment between checking it and moving it (std cannot hold a
        // Windows lock through a rename) — with nothing left at the live
        // names, before anything is installed.
        let moved_whole = opaque.as_ref().is_none_or(|files| {
            self.move_sidecars_aside(&ledger, &prepared.prepared_intent_id, files)
        });
        let installed = moved_whole
            && self
                .advance(RestorePhase::MovingLiveAside, RestorePhase::Installing)
                .is_ok()
            && fs::rename(&staged, &ledger).is_ok()
            && self
                .advance(RestorePhase::Installing, RestorePhase::ApplyingSettings)
                .is_ok()
            && settings_store
                .read()
                .ok()
                .and_then(|document| {
                    settings_store
                        .apply_restore_export(document.revision, &settings)
                        .ok()
                })
                .is_some_and(|outcome| matches!(outcome, PatchOutcome::Committed { .. }))
            && self
                .advance(RestorePhase::ApplyingSettings, RestorePhase::Verifying)
                .is_ok()
            && current_binding(&ledger).is_some_and(|binding| {
                // The installed file is the archive's Ledger: its inventory,
                // derived afresh, is the member that was checked.
                binding.ledger_revision == prepared.archive_ledger_revision
                    && binding.schema_version == prepared.archive_schema_version
                    && binding.inventory_sha256 == prepared.inventory_member_sha256
            });
        // The projection files were not restored with the Ledger (S7 plan
        // §7). A current-version Ledger is marked now; an older one once the
        // upgrade gate has made it openable — bound to its inventory.
        let needs_upgrade = prepared.archive_schema_version < CURRENT_SCHEMA_VERSION;
        let marked = installed
            && (needs_upgrade
                || SqliteProductLedger::open(&ledger)
                    .ok()
                    .and_then(|mut restored| {
                        restored
                            .mark_projections_out_of_sync_after_restore(now)
                            .ok()
                    })
                    .is_some());
        if marked {
            let pending_for = if needs_upgrade {
                current_binding(&ledger).map(|binding| binding.inventory_sha256)
            } else {
                None
            };
            // Recorded first; only then is the rollback copy removed, so a
            // crash in between leaves a truthful Restored with a spare copy.
            let outcome = self.finish(RestoreOutcome::Restored, pending_for, now)?;
            remove_rollback_set(&ledger, &prepared.prepared_intent_id, opaque.as_ref());
            return Ok(outcome.into());
        }
        let outcome = self.roll_back(&ledger, &prepared, settings_store);
        self.finish(outcome, None, now).map(ExecutedRestore::from)
    }

    /// Move an opaque file set's sidecars to their rollback names (the Ledger
    /// itself already moved), then check the moved set, held with no sharing,
    /// is exactly `files` and nothing is left at the live names.
    fn move_sidecars_aside(
        &self,
        ledger: &Path,
        prepared_intent_id: &str,
        files: &FileSetBinding,
    ) -> bool {
        for bound in files.members() {
            if bound.member == LedgerFileMember::Ledger {
                continue;
            }
            let live = member_path_of(ledger, bound.member);
            let aside = previous_member_path(ledger, prepared_intent_id, bound.member);
            if fs::rename(&live, &aside).is_err() {
                return false;
            }
        }
        let moved: Vec<(LedgerFileMember, PathBuf)> = LedgerFileMember::ALL
            .iter()
            .map(|member| {
                (
                    *member,
                    previous_member_path(ledger, prepared_intent_id, *member),
                )
            })
            .collect();
        bind_member_paths(&moved, true).is_ok_and(|now| now == *files)
            && LedgerFileMember::ALL
                .iter()
                .all(|member| !member_path_of(ledger, *member).exists())
    }

    /// Put the previous Ledger and settings back; say what the file shows.
    fn roll_back(
        &self,
        ledger: &Path,
        prepared: &PreparedRestore,
        settings_store: &SettingsStore,
    ) -> RestoreOutcome {
        // Only from a phase at or after the first move (or a rollback a crash
        // interrupted): a compare-and-set like every other phase change.
        let _ = self
            .restore_control
            .update(|control| match &mut control.active {
                Some(RestoreOperation::Executing { phase, .. })
                    if matches!(
                        phase,
                        RestorePhase::MovingLiveAside
                            | RestorePhase::Installing
                            | RestorePhase::ApplyingSettings
                            | RestorePhase::Verifying
                            | RestorePhase::RollingBack
                    ) =>
                {
                    *phase = RestorePhase::RollingBack;
                    Ok(())
                }
                _ => Err(()),
            });
        let previous = previous_path(ledger, &prepared.prepared_intent_id);
        let staged = staged_path(ledger, &prepared.prepared_intent_id);
        if !previous.exists() {
            // The live Ledger never left its name, so nothing was installed
            // and no setting applied.
            let _ = fs::remove_file(&staged);
            return if still_bound(ledger, &prepared.current) {
                RestoreOutcome::FailedBeforeReplacement
            } else {
                RestoreOutcome::RecoveryFailed
            };
        }
        // Whatever is in the live name now is the restored copy.
        if ledger.exists() && fs::remove_file(ledger).is_err() {
            return RestoreOutcome::RecoveryFailed;
        }
        match opaque_files(&prepared.current) {
            // Inspectable: its sidecars were cleared before the move, so any
            // at the live names now are the installed copy's.
            None => {
                for suffix in ["-wal", "-shm"] {
                    let _ = fs::remove_file(sibling(ledger, suffix));
                }
            }
            // Opaque, member by member: a bound sidecar waiting at its
            // rollback name goes back over whatever the installed copy left;
            // one that never moved is the original and stays; an unbound one
            // at the live name is the installed copy's and goes.
            Some(files) => {
                for member in [LedgerFileMember::Wal, LedgerFileMember::Shm] {
                    let live = member_path_of(ledger, member);
                    let aside = previous_member_path(ledger, &prepared.prepared_intent_id, member);
                    let bound = files.members().iter().any(|bound| bound.member == member);
                    if aside.exists() {
                        if (live.exists() && fs::remove_file(&live).is_err())
                            || fs::rename(&aside, &live).is_err()
                        {
                            return RestoreOutcome::RecoveryFailed;
                        }
                    } else if !bound && live.exists() && fs::remove_file(&live).is_err() {
                        return RestoreOutcome::RecoveryFailed;
                    }
                }
            }
        }
        if fs::rename(&previous, ledger).is_err() {
            return RestoreOutcome::RecoveryFailed;
        }
        let _ = fs::remove_file(&staged);
        let settings_back = fs::read(self.restore_work.join(SETTINGS_PREIMAGE))
            .ok()
            .and_then(|bytes| serde_json_document(&bytes))
            .and_then(|preimage| {
                let now = settings_store.read().ok()?;
                if now.display == preimage.display && now.operational == preimage.operational {
                    return Some(());
                }
                settings_store
                    .put_back(now.revision, &preimage)
                    .ok()
                    .filter(|outcome| matches!(outcome, PatchOutcome::Committed { .. }))
                    .map(|_| ())
            })
            .is_some();
        if settings_back && still_bound(ledger, &prepared.current) {
            RestoreOutcome::RecoveryPutBack
        } else {
            RestoreOutcome::RecoveryFailed
        }
    }

    /// Whether the Ledger must not be opened: a restore is still executing
    /// (its reconciliation could not run or failed), or a restore ended in
    /// `RecoveryFailed` that no later restore resolved, so the file in the
    /// Ledger's name may be neither the previous Ledger nor the archive's.
    /// Unreadable control state counts as needing recovery too.
    #[must_use]
    pub fn restore_needs_recovery(&self) -> bool {
        match self.restore_control.load() {
            Ok(control) => {
                matches!(control.active, Some(RestoreOperation::Executing { .. }))
                    || unresolved_recovery_failure(&control).is_some()
            }
            Err(_) => true,
        }
    }

    /// Step 5, at startup, before the Ledger is opened: a Prepared Intent is
    /// discarded; an interrupted execution is put back, never finished.
    pub fn reconcile_restore(
        &self,
        settings_store: &SettingsStore,
        now: UtcTimestamp,
    ) -> Result<Option<RestoreOutcome>, RestoreError> {
        let control = self.restore_control.load().map_err(RestoreError::Control)?;
        match control.active {
            None => {
                // A crash between recording Restored and removing the
                // rollback copy leaves the copy behind; remove it now.
                let ledger = self.restore_ledger()?.clone();
                for finished in &control.finished {
                    if finished.outcome == RestoreOutcome::Restored {
                        // An opaque restore's bound set is the preservation
                        // copy's; an inspectable one moved the Ledger only.
                        let moved = match &finished.recovery {
                            RecoveryEvidence::PreservationCopy { files, .. } => Some(files),
                            RecoveryEvidence::OperationalBackup { .. } => None,
                        };
                        remove_rollback_set(&ledger, &finished.prepared_intent_id, moved);
                    }
                }
                Ok(None)
            }
            Some(RestoreOperation::Prepared { .. }) => {
                self.restore_control
                    .update(|control| -> Result<(), RestoreError> {
                        control.active = None;
                        Ok(())
                    })
                    .map_err(RestoreError::Control)??;
                let _ = clear_work_area(&self.restore_work);
                Ok(None)
            }
            Some(RestoreOperation::Executing {
                prepared, phase, ..
            }) => {
                let ledger = self.restore_ledger()?.clone();
                let outcome = if matches!(phase, RestorePhase::Approved | RestorePhase::Staging) {
                    let _ = fs::remove_file(staged_path(&ledger, &prepared.prepared_intent_id));
                    RestoreOutcome::FailedBeforeReplacement
                } else {
                    self.roll_back(&ledger, &prepared, settings_store)
                };
                self.finish(outcome, None, now).map(Some)
            }
        }
    }
}

/// The newest `RecoveryFailed` restore no later restore resolved (restore-
/// unopened amendment §4: resolved explicitly, and the record kept).
fn unresolved_recovery_failure(control: &RestoreControl) -> Option<&RestoreTerminal> {
    let (index, failed) = control
        .finished
        .iter()
        .enumerate()
        .rev()
        .find(|(_, finished)| finished.outcome == RestoreOutcome::RecoveryFailed)?;
    let resolved = control.finished[index + 1..].iter().any(|later| {
        later.resolved_recovery_failure.as_deref() == Some(failed.idempotency_id.as_str())
    });
    (!resolved).then_some(failed)
}

fn serde_json_bytes(document: &SettingsDocument) -> Result<Vec<u8>, RestoreError> {
    pmc_platform::settings::document_json(document)
        .map_err(|_| RestoreError::WorkArea(io::Error::from(io::ErrorKind::InvalidData)))
}

fn serde_json_document(bytes: &[u8]) -> Option<SettingsDocument> {
    pmc_platform::settings::document_from_json(bytes).ok()
}
