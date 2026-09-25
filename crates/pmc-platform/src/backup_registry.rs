//! The verified-archive registry of S7-A (ADR 0010 §4).
//!
//! One JSON document in the protected app-data root, replaced atomically. A
//! record is written only after an archive was verified end to end (fully
//! decrypted, every member checked, the Ledger snapshot reopened) and renamed
//! into place; its `verified_at` never changes afterwards. At startup a record
//! counts only while its file still has the recorded size and container
//! SHA-256 — rechecking never refreshes the time, so an old backup cannot
//! stay "recent" by being looked at.
//!
//! The registry holds the destination folder's path because only the host
//! reads it; it is never sent over IPC (ADR 0011).

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REGISTRY_FORMAT: &str = "pmc-verified-backup-registry/v1";
/// A backup is due when none verified within this window (foundation §222).
pub const DUE_AFTER_MILLIS: i64 = 24 * 60 * 60 * 1000;
/// The published archive name's fixed parts (ADR 0010 §1).
pub const ARCHIVE_PREFIX: &str = "pmc-operational-";
pub const ARCHIVE_SUFFIX: &str = "-v1.tar.zst.age";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupRecord {
    pub archive_id: String,
    pub file_name: String,
    pub destination: PathBuf,
    pub created_at_millis: i64,
    pub verified_at_millis: i64,
    pub ledger_schema_version: u32,
    pub ledger_revision: u64,
    pub container_sha256: String,
    pub container_bytes: u64,
    pub snapshot_sha256: String,
    /// SHA-256 and record count of the archive's authority inventory (ADR
    /// 0010 addendum 2026-09-22).
    pub authority_inventory_sha256: String,
    pub authority_record_count: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupRegistry {
    pub format: String,
    pub revision: u64,
    pub records: Vec<BackupRecord>,
}

impl Default for BackupRegistry {
    fn default() -> Self {
        Self {
            format: REGISTRY_FORMAT.to_owned(),
            revision: 0,
            records: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum RegistryError {
    /// The document exists but is not a registry this version reads.
    Unreadable,
    /// The document changed since it was loaded; nothing was written.
    Conflict,
    /// Another writer held the registry for too long; nothing was written.
    Busy,
    Io(io::Error),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("backup registry is unreadable"),
            Self::Conflict => formatter.write_str("backup registry changed since it was read"),
            Self::Busy => formatter.write_str("backup registry is held by another writer"),
            Self::Io(error) => write!(formatter, "backup registry I/O failed: {error}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Where the registry document lives.
#[derive(Clone, Debug)]
pub struct RegistryStore {
    path: PathBuf,
}

impl RegistryStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The registry, or an empty one when none was written yet.
    pub fn load(&self) -> Result<BackupRegistry, RegistryError> {
        match fs::read(&self.path) {
            Ok(bytes) => {
                let registry: BackupRegistry =
                    serde_json::from_slice(&bytes).map_err(|_| RegistryError::Unreadable)?;
                if registry.format != REGISTRY_FORMAT {
                    return Err(RegistryError::Unreadable);
                }
                Ok(registry)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BackupRegistry::default()),
            Err(error) => Err(RegistryError::Io(error)),
        }
    }

    /// Replace the whole document atomically, as the next revision of the one
    /// `registry` was loaded from. If the document moved on since (another
    /// writer), nothing is written and the caller reloads: a verified record
    /// is never lost to a last-writer-wins race. The check and the write
    /// happen under an exclusive lock file, so two writers cannot both pass
    /// the check.
    pub fn save(&self, registry: &BackupRegistry) -> Result<(), RegistryError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(RegistryError::Io)?;
        }
        let _guard = self.lock()?;
        if self.load()?.revision != registry.revision {
            return Err(RegistryError::Conflict);
        }
        let next = BackupRegistry {
            revision: registry.revision + 1,
            ..registry.clone()
        };
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|error| RegistryError::Io(io::Error::other(error)))?;
        let mut file = AtomicWriteFile::open(&self.path).map_err(RegistryError::Io)?;
        file.write_all(&bytes).map_err(RegistryError::Io)?;
        file.commit().map_err(RegistryError::Io)
    }

    /// An exclusive hold on `<registry>.lock`, released when dropped. On
    /// Windows the file is opened with no sharing, which other processes
    /// cannot also do, and the OS releases it if this process dies — so a
    /// crash never leaves a stale lock. Elsewhere it serializes nothing
    /// across processes (PMC ships for Windows only).
    fn lock(&self) -> Result<File, RegistryError> {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        let lock_path = PathBuf::from(name);
        let mut options = fs::OpenOptions::new();
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
                Err(error) => return Err(RegistryError::Io(error)),
            }
        }
        Err(RegistryError::Busy)
    }
}

const LOCK_ATTEMPTS: u32 = 100;
const LOCK_WAIT_MILLIS: u64 = 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordStatus {
    /// The file has the recorded size and container SHA-256.
    Valid,
    /// The file is gone, or its folder is unavailable.
    Missing,
    /// The file is there but is not the archive that was verified.
    Altered,
}

/// Size first, then the full container hash (ADR 0010 §4).
#[must_use]
pub fn check_record(record: &BackupRecord) -> RecordStatus {
    let path = record.destination.join(&record.file_name);
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return RecordStatus::Missing;
    };
    if !metadata.is_file() || metadata.len() != record.container_bytes {
        return RecordStatus::Altered;
    }
    match container_sha256(&path) {
        Ok(digest) if digest == record.container_sha256 => RecordStatus::Valid,
        Ok(_) => RecordStatus::Altered,
        Err(_) => RecordStatus::Missing,
    }
}

/// A fresh archive id: 16 random bytes as 32 lower-case hex digits, which
/// `operational_backup::is_valid_archive_id` accepts unchanged. `None` when
/// the system random source is unavailable.
#[must_use]
pub fn new_archive_id() -> Option<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// SHA-256 of a whole file, lower-case hex: the container checksum.
pub fn container_sha256(path: &Path) -> io::Result<String> {
    reader_sha256(File::open(path)?)
}

/// SHA-256 of everything a reader yields, lower-case hex.
pub fn reader_sha256(mut file: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Whether a backup is due now, given the verification times of the records
/// that are still valid. A time in the future (a clock set back) is not fresh.
pub fn backup_due(valid_verified_at: impl Iterator<Item = i64>, now_millis: i64) -> bool {
    !valid_verified_at
        .into_iter()
        .any(|verified_at| verified_at <= now_millis && now_millis - verified_at < DUE_AFTER_MILLIS)
}

/// Published archives in `destination` that the registry does not know: a
/// crash between rename and registry write, or a file copied in. They are
/// never counted; the caller surfaces them.
#[must_use]
pub fn orphan_archives(destination: &Path, registry: &BackupRegistry) -> Vec<String> {
    let Ok(entries) = fs::read_dir(destination) else {
        return Vec::new();
    };
    // Windows spells one folder many ways (case, `\\?\`); compare the
    // canonical forms, falling back to the text when a folder is unavailable.
    let canonical = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let destination = canonical(destination);
    let known: Vec<(&str, PathBuf)> = registry
        .records
        .iter()
        .map(|record| (record.file_name.as_str(), canonical(&record.destination)))
        .collect();
    let mut orphans: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(ARCHIVE_PREFIX) && name.ends_with(ARCHIVE_SUFFIX))
        .filter(|name| {
            !known
                .iter()
                .any(|(file_name, folder)| file_name == name && *folder == destination)
        })
        .collect();
    orphans.sort();
    orphans
}
