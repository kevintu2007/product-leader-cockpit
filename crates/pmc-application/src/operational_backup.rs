//! One Operational Backup, end to end (S7-A; ADR 0010; S7 plan §6-§7).
//!
//! 1. [`snapshot_ledger`] — the S2 kernel's verified SQLite snapshot, into a
//!    private work area under the protected app-data root. The caller holds
//!    the Ledger lock only for this step.
//! 2. [`publish_backup`] — without the Ledger lock: the encrypted archive is
//!    written as a `.partial` file **beside the destination's final name**
//!    (so publication is a same-folder rename), synced, re-hashed, then
//!    decrypted again into the work area, every member checked, the Ledger
//!    snapshot reopened through the caller's verifier, and only then renamed
//!    into place. The record it returns is what the registry stores.
//!
//! On any failure nothing is published: the partial file is removed and the
//! work area emptied. [`clear_work_area`] is also run before every backup and
//! at startup, so a crash never leaves plaintext behind for long.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    inspect_standalone_snapshot, AuthorityInventory, LedgerInspection, LedgerSnapshotManifest,
    OperationalBackupPort, SqliteProductLedger,
};
use pmc_platform::backup_archive::{
    read_archive, write_archive, ArchiveMemberSource, ManifestInput, Passphrase,
};
use pmc_platform::backup_registry::{
    container_sha256, reader_sha256, BackupRecord, ARCHIVE_PREFIX, ARCHIVE_SUFFIX,
};
use pmc_platform::settings::CanonicalDirectoryPath;

/// The archive's members (ADR 0010 §3).
pub const LEDGER_MEMBER: &str = "ledger.sqlite3";
pub const SETTINGS_MEMBER: &str = "settings.json";
pub const INVENTORY_MEMBER: &str = "authority-inventory.txt";

/// A verified Ledger snapshot and the authority inventory read under the
/// same Ledger lock (ADR 0010 addendum 2026-09-22).
#[derive(Debug)]
pub struct LedgerSnapshot {
    pub path: PathBuf,
    pub manifest: LedgerSnapshotManifest,
    pub inventory: AuthorityInventory,
}

#[derive(Debug)]
pub enum BackupError {
    /// The archive id is not 8-64 lower-case ASCII letters or digits.
    InvalidArchiveId,
    /// The private work area could not be prepared or cleared.
    WorkArea(io::Error),
    /// The S2 kernel could not make or verify the Ledger snapshot.
    Snapshot,
    /// The Ledger is no longer the source the caller inspected (a
    /// pre-upgrade backup must hold exactly that source).
    SourceChanged,
    /// Writing the archive or its partial file in the destination failed.
    Write(io::Error),
    /// The archive, once written, did not verify end to end.
    VerificationFailed,
    /// The verified archive could not be renamed into place.
    Publication(io::Error),
    /// Verified and in place, but its audit event or registry entry could not
    /// be written: it is an orphan, never counted and never deleted.
    NotRecorded,
    /// Cleaning up after a backup failed: plaintext may remain in the work
    /// area, or a partial file in the destination. Reported ahead of any other
    /// error, because the caller must retry the cleanup.
    Residue(io::Error),
}

/// Empty the work area (creating it if absent). Run before each backup and at
/// startup: whatever a crash left there is plaintext and must not stay.
pub fn clear_work_area(work: &Path) -> io::Result<()> {
    match fs::remove_dir_all(work) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(work)
}

/// Step 1, under the caller's Ledger lock: a verified snapshot in the work
/// area, and the authority inventory of the same state.
pub fn snapshot_ledger(
    ledger: &SqliteProductLedger,
    work: &Path,
) -> Result<LedgerSnapshot, BackupError> {
    clear_work_area(work).map_err(BackupError::WorkArea)?;
    let snapshot_dir = work.join("snapshot");
    fs::create_dir(&snapshot_dir).map_err(BackupError::WorkArea)?;
    let snapshot = snapshot_dir.join(LEDGER_MEMBER);
    let taken = OperationalBackupPort::create_verified_snapshot(ledger, &snapshot)
        .ok()
        .and_then(|manifest| {
            ledger
                .authority_inventory()
                .ok()
                .map(|inventory| (manifest, inventory))
        });
    match taken {
        Some((manifest, inventory)) => Ok(LedgerSnapshot {
            path: snapshot,
            manifest,
            inventory,
        }),
        // The kernel may have written plaintext before it failed.
        None => match clear_work_area(work) {
            Ok(()) => Err(BackupError::Snapshot),
            Err(error) => Err(BackupError::Residue(error)),
        },
    }
}

/// Step 1 for a Ledger this binary may not be able to open (the pre-upgrade
/// backup): a verified snapshot of the closed file at `ledger`, read without
/// constructing `SqliteProductLedger`, and what it holds.
pub fn snapshot_ledger_file(
    ledger: &Path,
    work: &Path,
) -> Result<(LedgerSnapshot, LedgerInspection), BackupError> {
    clear_work_area(work).map_err(BackupError::WorkArea)?;
    let snapshot_dir = work.join("snapshot");
    fs::create_dir(&snapshot_dir).map_err(BackupError::WorkArea)?;
    let snapshot = snapshot_dir.join(LEDGER_MEMBER);
    match pmc_ledger::sqlite::snapshot_ledger_file(ledger, &snapshot) {
        Ok((manifest, inspection)) => Ok((
            LedgerSnapshot {
                path: snapshot,
                manifest,
                inventory: inspection.inventory.clone(),
            },
            inspection,
        )),
        Err(_) => match clear_work_area(work) {
            Ok(()) => Err(BackupError::Snapshot),
            Err(error) => Err(BackupError::Residue(error)),
        },
    }
}

/// Archive ids are random lower-case hex from the host; anything else is
/// refused rather than rewritten, so two ids can never share a file name.
#[must_use]
pub fn is_valid_archive_id(archive_id: &str) -> bool {
    (8..=64).contains(&archive_id.len())
        && archive_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
}

/// `pmc-operational-<UTC time to the millisecond>-<id>-v1.tar.zst.age`: the
/// ADR's name with the archive id added so two backups in one millisecond do
/// not collide. Only characters safe in a file name are kept; the pipeline
/// accepts only ids that pass [`is_valid_archive_id`], which this leaves
/// unchanged.
#[must_use]
pub fn archive_file_name(now: UtcTimestamp, archive_id: &str) -> String {
    let time: String = now
        .to_rfc3339_utc()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let id: String = archive_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(|character| character.to_lowercase())
        .collect();
    format!("{ARCHIVE_PREFIX}{time}-{id}{ARCHIVE_SUFFIX}")
}

/// `pmc-preservation-<UTC time to the millisecond>-<id>-v1.tar.zst.age`: a
/// preservation copy's name (restore-unopened amendment §1), built like
/// [`archive_file_name`] but outside the `pmc-operational-` namespace, so the
/// backup registry never takes it for an Operational Backup.
#[must_use]
pub fn preservation_file_name(now: UtcTimestamp, id: &str) -> String {
    let time: String = now
        .to_rfc3339_utc()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let id: String = id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(|character| character.to_lowercase())
        .collect();
    format!(
        "{}{time}-{id}{}",
        pmc_platform::preservation::PRESERVATION_PREFIX,
        pmc_platform::preservation::PRESERVATION_SUFFIX
    )
}
/// Everything [`publish_backup`] needs.
pub struct PublishInput<'a> {
    pub snapshot: &'a LedgerSnapshot,
    /// The non-secret settings export (`SettingsDocument::backup_export`).
    pub settings_json: &'a [u8],
    pub destination: &'a CanonicalDirectoryPath,
    pub work: &'a Path,
    pub passphrase: &'a Passphrase,
    pub archive_id: &'a str,
    pub clock: &'a dyn Fn() -> UtcTimestamp,
    /// Reopens an extracted Ledger snapshot and checks it against the
    /// manifest (the S2 kernel's `verify_snapshot`).
    pub verify_ledger: &'a dyn Fn(&Path, &LedgerSnapshotManifest) -> bool,
}

/// Step 2, without the Ledger lock. Returns the registry record of a backup
/// that verified end to end and is now in place.
pub fn publish_backup(input: PublishInput<'_>) -> Result<BackupRecord, BackupError> {
    if !is_valid_archive_id(input.archive_id) {
        return match clear_work_area(input.work) {
            Ok(()) => Err(BackupError::InvalidArchiveId),
            Err(error) => Err(BackupError::Residue(error)),
        };
    }
    let created_at = (input.clock)();
    let file_name = archive_file_name(created_at, input.archive_id);
    let destination = input.destination.as_path();
    let final_path = destination.join(&file_name);
    let partial_path = destination.join(format!(".{file_name}.partial"));

    let outcome = write_and_verify(&input, &file_name, &final_path, &partial_path, created_at);
    // Nothing half-made stays in the destination; nothing plaintext stays in
    // the work area. A failed cleanup outranks the original error.
    if outcome.is_err() {
        match fs::remove_file(&partial_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                let _ = clear_work_area(input.work);
                return Err(BackupError::Residue(error));
            }
        }
    }
    clear_work_area(input.work).map_err(BackupError::Residue)?;
    outcome
}

/// The partial file, opened once for writing, hashing and verification. On
/// Windows other processes may read it or move it, but not write to it while
/// it is open, so the bytes verified are the bytes that were written.
fn create_partial(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_DELETE: u32 = 0x4;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
    }
    options.open(path)
}

fn write_and_verify(
    input: &PublishInput<'_>,
    file_name: &str,
    final_path: &Path,
    partial_path: &Path,
    created_at: UtcTimestamp,
) -> Result<BackupRecord, BackupError> {
    if final_path.exists() {
        return Err(BackupError::Write(io::Error::from(
            io::ErrorKind::AlreadyExists,
        )));
    }
    let settings_path = input.work.join(SETTINGS_MEMBER);
    fs::write(&settings_path, input.settings_json).map_err(BackupError::WorkArea)?;
    let inventory_path = input.work.join(INVENTORY_MEMBER);
    fs::write(&inventory_path, &input.snapshot.inventory.bytes).map_err(BackupError::WorkArea)?;
    let snapshot_manifest = &input.snapshot.manifest;

    // Write beside the destination, create_new so nothing is overwritten.
    let mut file = create_partial(partial_path).map_err(BackupError::Write)?;
    let members = [
        ArchiveMemberSource {
            name: INVENTORY_MEMBER,
            path: &inventory_path,
        },
        ArchiveMemberSource {
            name: LEDGER_MEMBER,
            path: &input.snapshot.path,
        },
        ArchiveMemberSource {
            name: SETTINGS_MEMBER,
            path: &settings_path,
        },
    ];
    let manifest_input = ManifestInput {
        archive_id: input.archive_id.to_owned(),
        created_at: created_at.to_rfc3339_utc(),
        ledger_schema_version: snapshot_manifest.schema_version(),
        ledger_revision: snapshot_manifest.ledger_revision(),
        snapshot_checksum: snapshot_manifest.sha256().to_owned(),
    };
    let mut writer = io::BufWriter::new(&mut file);
    let written = write_archive(&mut writer, input.passphrase, &manifest_input, &members)
        .map_err(|error| BackupError::Write(io::Error::other(error.to_string())))?;
    writer
        .into_inner()
        .map_err(|error| BackupError::Write(error.into_error()))?;
    file.sync_all().map_err(BackupError::Write)?;

    // The bytes on disk are the bytes that were written — read back through
    // the same handle.
    file.seek(SeekFrom::Start(0)).map_err(BackupError::Write)?;
    let on_disk = reader_sha256(io::BufReader::new(&mut file)).map_err(BackupError::Write)?;
    if on_disk != written.container_sha256 {
        return Err(BackupError::VerificationFailed);
    }

    // Decrypt it again, completely, into a fresh verification folder.
    let verify_dir = input.work.join("verify");
    fs::create_dir(&verify_dir).map_err(BackupError::WorkArea)?;
    file.seek(SeekFrom::Start(0)).map_err(BackupError::Write)?;
    let verified = read_archive(
        io::BufReader::new(&mut file),
        input.passphrase,
        Some(&verify_dir),
    )
    .map_err(|_| BackupError::VerificationFailed)?;
    if verified.manifest != written.manifest {
        return Err(BackupError::VerificationFailed);
    }
    // The Ledger inside must reopen as the snapshot it was.
    let extracted = verify_dir.join(LEDGER_MEMBER);
    if !(input.verify_ledger)(&extracted, snapshot_manifest) {
        return Err(BackupError::VerificationFailed);
    }
    // And its authority inventory, derived afresh, must be the one carried.
    let carried =
        fs::read(verify_dir.join(INVENTORY_MEMBER)).map_err(|_| BackupError::VerificationFailed)?;
    // Read-only and version-aware, so a pre-upgrade archive stays verifiable.
    let derived = inspect_standalone_snapshot(&extracted, snapshot_manifest)
        .map_err(|_| BackupError::VerificationFailed)?
        .inventory;
    if carried != derived.bytes || derived != input.snapshot.inventory {
        return Err(BackupError::VerificationFailed);
    }

    // Only now is it published. The handle is closed first so the rename
    // needs no sharing from it; the published file is then hashed again, and
    // a later change to it shows as Altered through the recorded hash.
    drop(file);
    fs::rename(partial_path, final_path).map_err(BackupError::Publication)?;
    match container_sha256(final_path) {
        Ok(digest) if digest == written.container_sha256 => {}
        _ => {
            return match fs::remove_file(final_path) {
                Ok(()) => Err(BackupError::VerificationFailed),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Err(BackupError::VerificationFailed)
                }
                Err(error) => Err(BackupError::Residue(error)),
            };
        }
    }
    Ok(BackupRecord {
        archive_id: input.archive_id.to_owned(),
        file_name: file_name.to_owned(),
        destination: input.destination.as_path().to_path_buf(),
        created_at_millis: created_at.unix_millis(),
        verified_at_millis: (input.clock)().unix_millis(),
        ledger_schema_version: snapshot_manifest.schema_version(),
        ledger_revision: snapshot_manifest.ledger_revision(),
        container_sha256: written.container_sha256,
        container_bytes: written.container_bytes,
        snapshot_sha256: snapshot_manifest.sha256().to_owned(),
        authority_inventory_sha256: derived.sha256,
        authority_record_count: derived.record_count,
    })
}
