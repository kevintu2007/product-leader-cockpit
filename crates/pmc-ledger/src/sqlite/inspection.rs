//! Read-only inspection of a Ledger file of any version (slice 8d; ADR 0004;
//! ADR 0010 addendum 2026-09-22).
//!
//! `SqliteProductLedger::open` accepts only the current schema and creates a
//! Ledger when the file is empty. Deciding whether an existing file can be
//! upgraded must happen before that, without constructing the Ledger and
//! without writing anything: this module reads the file through an
//! `immutable=1` read-only connection and classifies it. A supported older
//! Ledger is validated against exactly the descriptors that produced its
//! version — schema, migration registry, metadata, foreign keys and SQLite
//! integrity — so "upgradeable" never means "claims to be version N".

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use super::migrations::{
    descriptors_through, production_descriptors, validate_registry_against, Migration,
};
use super::schema::{schema_objects, validate_canonical_schema_against};
use super::{
    authority_inventory_of, classify_open_error, configure_inspection_safety, immutable_read_uri,
    pragma_i64, read_revision, snapshot_sha256, AuthorityInventory, LedgerOpenError,
    LedgerSnapshotError, LedgerSnapshotManifest, APPLICATION_ID,
};

/// The oldest schema this binary upgrades in place: v46 is the first real-data
/// baseline (product owner, 2026-09-19). Development-era Ledgers are older.
pub const FIRST_UPGRADEABLE_SCHEMA_VERSION: u32 = 46;

/// What a supported Ledger holds, read without changing it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerInspection {
    pub schema_version: u32,
    pub ledger_revision: u64,
    pub inventory: AuthorityInventory,
    /// SHA-256 over the schema and every row of every table: any write at
    /// all — including ones that touch neither the revision nor an aggregate,
    /// such as a Prepared Intent — changes it.
    pub content_sha256: String,
    /// The latest updated_at of any aggregate, read from the file itself:
    /// what "last change" means for a Ledger that is not open (the
    /// restore-unopened amendment §3.4). None when it holds no record.
    pub last_change_at_millis: Option<i64>,
}

/// How this binary can treat the Ledger file at a path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerCompatibility {
    /// No Ledger yet: the file is missing or empty (`open` would create one).
    Absent,
    /// The current schema, fully valid.
    Current(LedgerInspection),
    /// A supported older schema, fully valid at its own version.
    UpgradeableFrom(LedgerInspection),
    /// A PMC Ledger older than the first supported baseline.
    UnsupportedOld { found: u32 },
    /// A PMC Ledger newer than this binary.
    Future { found: u32 },
    /// A SQLite database that is not a PMC Ledger.
    NotALedger,
    /// A claimed PMC Ledger that fails any check at its own version.
    Corrupt,
    /// A non-empty write-ahead log sits beside the file: the last session did
    /// not close cleanly, and the main file alone is not the whole Ledger.
    UncleanShutdown,
}

/// Classify the Ledger at `path` for this binary. `Err` only for storage that
/// cannot be read at all (unavailable, busy, or a path outside policy).
pub fn inspect_ledger(path: impl AsRef<Path>) -> Result<LedgerCompatibility, LedgerOpenError> {
    inspect_ledger_against(path.as_ref(), production_descriptors())
}

/// As [`inspect_ledger`], for a given descriptor list (tests extend it with a
/// test-only next version).
pub(super) fn inspect_ledger_against(
    path: &Path,
    descriptors: &'static [Migration],
) -> Result<LedgerCompatibility, LedgerOpenError> {
    if !path.exists() {
        return Ok(LedgerCompatibility::Absent);
    }
    let length = path
        .metadata()
        .map_err(|_| LedgerOpenError::StorageUnavailable)?
        .len();
    if length == 0 {
        return Ok(LedgerCompatibility::Absent);
    }
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    if std::fs::metadata(wal).is_ok_and(|metadata| metadata.len() > 0) {
        return Ok(LedgerCompatibility::UncleanShutdown);
    }
    let connection = Connection::open_with_flags(
        immutable_read_uri(path)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(classify_open_error)?;
    match classify_connection(&connection, descriptors) {
        Ok(compatibility) => Ok(compatibility),
        Err(
            LedgerOpenError::CorruptDatabase
            | LedgerOpenError::InvalidMetadata
            | LedgerOpenError::FutureSchema { .. }
            | LedgerOpenError::UnsupportedSchema { .. },
        ) => Ok(LedgerCompatibility::Corrupt),
        Err(LedgerOpenError::UnclaimedDatabase | LedgerOpenError::WrongApplication) => {
            Ok(LedgerCompatibility::NotALedger)
        }
        Err(error) => Err(error),
    }
}

/// Classify an open read-only connection against `descriptors`, whose last
/// entry is this binary's current version.
pub(super) fn classify_connection(
    connection: &Connection,
    descriptors: &'static [Migration],
) -> Result<LedgerCompatibility, LedgerOpenError> {
    configure_inspection_safety(connection)?;
    let current = descriptors.last().map_or(0, |migration| migration.version);
    let application_id = pragma_i64(connection, "application_id")?;
    let user_version = pragma_i64(connection, "user_version")?;
    if application_id == 0 && user_version == 0 && schema_objects(connection)?.is_empty() {
        return Ok(LedgerCompatibility::Absent);
    }
    if application_id != i64::from(APPLICATION_ID) {
        return Ok(LedgerCompatibility::NotALedger);
    }
    let Ok(version) = u32::try_from(user_version) else {
        return Ok(LedgerCompatibility::Corrupt);
    };
    if version > current {
        return Ok(LedgerCompatibility::Future { found: version });
    }
    if version < FIRST_UPGRADEABLE_SCHEMA_VERSION {
        return Ok(LedgerCompatibility::UnsupportedOld { found: version });
    }
    let Some(prefix) = descriptors_through(descriptors, version) else {
        return Ok(LedgerCompatibility::Corrupt);
    };
    let Some(inspection) = validate_at(connection, prefix, version) else {
        return Ok(LedgerCompatibility::Corrupt);
    };
    Ok(if version == current {
        LedgerCompatibility::Current(inspection)
    } else {
        LedgerCompatibility::UpgradeableFrom(inspection)
    })
}

/// Every check a Ledger at `version` must pass; `None` on the first failure.
pub(super) fn validate_at(
    connection: &Connection,
    prefix: &[Migration],
    version: u32,
) -> Option<LedgerInspection> {
    validate_canonical_schema_against(connection, prefix).ok()?;
    validate_registry_against(connection, prefix).ok()?;
    let (singleton, metadata_version, revision): (i64, i64, i64) = connection
        .query_row(
            "SELECT singleton, schema_version, ledger_revision FROM ledger_metadata",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()?;
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM ledger_metadata", [], |row| row.get(0))
        .ok()?;
    if singleton != 1 || metadata_version != i64::from(version) || revision < 0 || rows != 1 {
        return None;
    }
    let foreign_key_violations = connection
        .prepare("PRAGMA foreign_key_check")
        .and_then(|mut statement| {
            statement
                .query_map([], |_| Ok(()))
                .map(|violations| violations.count())
        })
        .ok()?;
    if foreign_key_violations != 0 {
        return None;
    }
    let integrity: Vec<String> = connection
        .prepare("PRAGMA integrity_check")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get(0))
                .and_then(Iterator::collect)
        })
        .ok()?;
    if integrity != ["ok"] {
        return None;
    }
    Some(LedgerInspection {
        schema_version: version,
        ledger_revision: read_revision(connection).ok()?,
        inventory: authority_inventory_of(connection).ok()?,
        content_sha256: content_sha256(connection)?,
        last_change_at_millis: connection
            .query_row(
                "SELECT MAX(updated_at) FROM aggregate_registry",
                [],
                |row| row.get(0),
            )
            .ok()?,
    })
}

/// The Ledger's whole logical content as one digest: each schema object's
/// type, name and SQL, then each table's rows, rendered value by value and
/// sorted, table by table in name order. Independent of page layout, so a
/// backup-API snapshot has the same digest as its source.
fn content_sha256(connection: &Connection) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let objects: Vec<(String, String, Option<String>)> = connection
        .prepare("SELECT type, name, sql FROM sqlite_schema ORDER BY type, name")
        .ok()?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .ok()?
        .collect::<Result<_, _>>()
        .ok()?;
    for (kind, name, sql) in &objects {
        hasher.update(format!("object\t{kind}\t{name}\t{sql:?}\n"));
    }
    for (kind, name, _) in &objects {
        if kind != "table" || name.starts_with("sqlite_") {
            continue;
        }
        let quoted = name.replace('"', "\"\"");
        let mut statement = connection
            .prepare(&format!("SELECT * FROM \"{quoted}\""))
            .ok()?;
        let columns = statement.column_count();
        let mut rows: Vec<String> = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| {
                        row.get::<_, rusqlite::types::Value>(index)
                            .map(|value| format!("{value:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(|values| values.join("\t"))
            })
            .ok()?
            .collect::<Result<_, _>>()
            .ok()?;
        rows.sort();
        hasher.update(format!("table\t{name}\t{}\n", rows.len()));
        for row in rows {
            hasher.update(row);
            hasher.update("\n");
        }
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// A standalone Ledger snapshot — one a backup made, or one taken out of an
/// archive — checked against its manifest without opening it for writing:
/// the file's SHA-256, no write-ahead log or shared-memory file beside it,
/// and every check [`inspect_ledger`] makes at the manifest's own schema
/// version, which may be older than this binary's (a pre-upgrade backup must
/// stay verifiable after the upgrade). Returns what it read, including the
/// authority inventory.
pub fn inspect_standalone_snapshot(
    path: impl AsRef<Path>,
    manifest: &LedgerSnapshotManifest,
) -> Result<LedgerInspection, LedgerSnapshotError> {
    inspect_standalone_snapshot_against(path.as_ref(), manifest, production_descriptors())
}

pub(super) fn inspect_standalone_snapshot_against(
    path: &Path,
    manifest: &LedgerSnapshotManifest,
    descriptors: &'static [Migration],
) -> Result<LedgerInspection, LedgerSnapshotError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        if std::fs::metadata(sidecar).is_ok() {
            return Err(LedgerSnapshotError::VerificationFailed);
        }
    }
    if snapshot_sha256(path)? != manifest.sha256 {
        return Err(LedgerSnapshotError::VerificationFailed);
    }
    let connection = Connection::open_with_flags(
        immutable_read_uri(path).map_err(|_| LedgerSnapshotError::VerificationFailed)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| LedgerSnapshotError::VerificationFailed)?;
    let inspection = match classify_connection(&connection, descriptors) {
        Ok(
            LedgerCompatibility::Current(inspection)
            | LedgerCompatibility::UpgradeableFrom(inspection),
        ) => inspection,
        _ => return Err(LedgerSnapshotError::VerificationFailed),
    };
    if inspection.schema_version != manifest.schema_version
        || inspection.ledger_revision != manifest.ledger_revision
    {
        return Err(LedgerSnapshotError::VerificationFailed);
    }
    Ok(inspection)
}
/// A verified standalone snapshot of the Ledger file at `source`, taken
/// without constructing `SqliteProductLedger` — so it works for a Ledger this
/// binary will upgrade but cannot open (the pre-upgrade backup). The source
/// must be closed (no write-ahead log) and supported; `destination` must not
/// exist. Returns the manifest and what the snapshot holds.
pub fn snapshot_ledger_file(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(LedgerSnapshotManifest, LedgerInspection), LedgerSnapshotError> {
    snapshot_ledger_file_against(
        source.as_ref(),
        destination.as_ref(),
        production_descriptors(),
    )
}

pub(super) fn snapshot_ledger_file_against(
    source: &Path,
    destination: &Path,
    descriptors: &'static [Migration],
) -> Result<(LedgerSnapshotManifest, LedgerInspection), LedgerSnapshotError> {
    let inspection = match inspect_ledger_against(source, descriptors) {
        Ok(
            LedgerCompatibility::Current(inspection)
            | LedgerCompatibility::UpgradeableFrom(inspection),
        ) => inspection,
        _ => return Err(LedgerSnapshotError::VerificationFailed),
    };
    if destination
        .try_exists()
        .map_err(|_| LedgerSnapshotError::DestinationUnavailable)?
    {
        return Err(LedgerSnapshotError::DestinationAlreadyExists);
    }
    let reader = Connection::open_with_flags(
        immutable_read_uri(source).map_err(|_| LedgerSnapshotError::SnapshotFailed)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| LedgerSnapshotError::SnapshotFailed)?;
    let mut writer = Connection::open_with_flags(
        destination,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| LedgerSnapshotError::DestinationUnavailable)?;
    {
        let backup = rusqlite::backup::Backup::new(&reader, &mut writer)
            .map_err(|_| LedgerSnapshotError::SnapshotFailed)?;
        backup
            .run_to_completion(128, std::time::Duration::from_millis(1), None)
            .map_err(|_| LedgerSnapshotError::SnapshotFailed)?;
    }
    drop(writer);
    drop(reader);
    let manifest = LedgerSnapshotManifest {
        schema_version: inspection.schema_version,
        ledger_revision: inspection.ledger_revision,
        sha256: snapshot_sha256(destination)?,
    };
    let snapshot = inspect_standalone_snapshot_against(destination, &manifest, descriptors)?;
    if snapshot != inspection {
        return Err(LedgerSnapshotError::VerificationFailed);
    }
    Ok((manifest, snapshot))
}
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::super::migrations::bootstrap_at_for_test;
    use super::super::SqliteProductLedger;
    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn scratch_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pmc-inspection-{nonce}-{sequence}.sqlite3"))
    }

    fn fresh_ledger() -> PathBuf {
        let path = scratch_path();
        drop(SqliteProductLedger::open(&path).unwrap());
        path
    }

    fn tamper(path: &Path, sql: &str) {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch(sql).unwrap();
    }

    #[test]
    fn a_missing_file_is_absent() {
        assert_eq!(
            inspect_ledger(scratch_path()).unwrap(),
            LedgerCompatibility::Absent
        );
    }

    #[test]
    fn a_fresh_ledger_is_current_with_an_empty_inventory() {
        let LedgerCompatibility::Current(inspection) = inspect_ledger(fresh_ledger()).unwrap()
        else {
            panic!("expected Current");
        };
        assert_eq!(inspection.schema_version, 48);
        assert_eq!(inspection.inventory.record_count, 0);
    }

    /// A fresh Ledger as the previous release wrote it.
    fn fresh_v46_ledger() -> PathBuf {
        let path = scratch_path();
        bootstrap_at_for_test(&path, 46);
        path
    }

    #[test]
    fn a_v46_ledger_is_upgradeable_for_this_binary() {
        let compatibility = inspect_ledger(fresh_v46_ledger()).unwrap();
        assert!(
            matches!(&compatibility, LedgerCompatibility::UpgradeableFrom(inspection) if inspection.schema_version == 46),
            "{compatibility:?}"
        );
    }

    #[test]
    fn a_newer_ledger_is_future_and_an_older_one_unsupported() {
        let future = fresh_ledger();
        tamper(&future, "PRAGMA user_version=49;");
        assert_eq!(
            inspect_ledger(&future).unwrap(),
            LedgerCompatibility::Future { found: 49 }
        );
        let old = fresh_ledger();
        tamper(&old, "PRAGMA user_version=45;");
        assert_eq!(
            inspect_ledger(&old).unwrap(),
            LedgerCompatibility::UnsupportedOld { found: 45 }
        );
    }

    #[test]
    fn another_database_is_not_a_ledger() {
        let path = scratch_path();
        tamper(&path, "CREATE TABLE unrelated(x INTEGER);");
        assert_eq!(
            inspect_ledger(&path).unwrap(),
            LedgerCompatibility::NotALedger
        );
    }

    #[test]
    fn a_ledger_that_claims_its_version_but_differs_is_corrupt() {
        let extra_table = fresh_ledger();
        tamper(&extra_table, "CREATE TABLE smuggled(x INTEGER);");
        assert_eq!(
            inspect_ledger(&extra_table).unwrap(),
            LedgerCompatibility::Corrupt
        );
        let wrong_metadata = fresh_ledger();
        tamper(
            &wrong_metadata,
            "UPDATE ledger_metadata SET schema_version=45;",
        );
        assert_eq!(
            inspect_ledger(&wrong_metadata).unwrap(),
            LedgerCompatibility::Corrupt
        );
        let missing_registry_row = fresh_ledger();
        tamper(
            &missing_registry_row,
            "DELETE FROM schema_migrations WHERE version=48;",
        );
        assert_eq!(
            inspect_ledger(&missing_registry_row).unwrap(),
            LedgerCompatibility::Corrupt
        );
    }

    #[test]
    fn a_leftover_write_ahead_log_is_an_unclean_shutdown() {
        let path = fresh_ledger();
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        std::fs::write(PathBuf::from(wal), b"not empty").unwrap();
        assert_eq!(
            inspect_ledger(&path).unwrap(),
            LedgerCompatibility::UncleanShutdown
        );
    }

    #[test]
    fn inspecting_changes_nothing_and_leaves_no_sidecar() {
        let path = fresh_ledger();
        let before = std::fs::read(&path).unwrap();
        inspect_ledger(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            assert!(!PathBuf::from(sidecar).exists(), "{suffix} was created");
        }
    }

    fn snapshot_of(path: &Path) -> (PathBuf, LedgerSnapshotManifest) {
        let ledger = SqliteProductLedger::open(path).unwrap();
        let snapshot = scratch_path();
        let manifest = ledger.create_verified_snapshot(&snapshot).unwrap();
        (snapshot, manifest)
    }

    fn sidecars_of(path: &Path) -> Vec<String> {
        ["-wal", "-shm", "-journal"]
            .into_iter()
            .filter(|suffix| {
                let mut sidecar = path.as_os_str().to_owned();
                sidecar.push(suffix);
                PathBuf::from(sidecar).exists()
            })
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_snapshot_is_verified_without_writing_anything_beside_it() {
        let (snapshot, manifest) = snapshot_of(&fresh_ledger());
        assert!(
            sidecars_of(&snapshot).is_empty(),
            "{:?}",
            sidecars_of(&snapshot)
        );
        let inspection = inspect_standalone_snapshot(&snapshot, &manifest).unwrap();
        assert_eq!(inspection.schema_version, manifest.schema_version());
        assert!(
            sidecars_of(&snapshot).is_empty(),
            "{:?}",
            sidecars_of(&snapshot)
        );
    }

    #[test]
    fn a_v46_snapshot_stays_verifiable_for_this_binary() {
        let source = fresh_v46_ledger();
        let snapshot = scratch_path();
        let (manifest, _) = snapshot_ledger_file(&source, &snapshot).unwrap();
        let inspection = inspect_standalone_snapshot(&snapshot, &manifest).unwrap();
        assert_eq!(inspection.schema_version, 46);
    }

    #[test]
    fn a_snapshot_with_a_sidecar_or_other_bytes_is_refused() {
        let (snapshot, manifest) = snapshot_of(&fresh_ledger());
        let mut wal = snapshot.as_os_str().to_owned();
        wal.push("-wal");
        std::fs::write(PathBuf::from(&wal), b"").unwrap();
        assert!(inspect_standalone_snapshot(&snapshot, &manifest).is_err());
        std::fs::remove_file(PathBuf::from(wal)).unwrap();

        let mut bytes = std::fs::read(&snapshot).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        std::fs::write(&snapshot, bytes).unwrap();
        assert!(inspect_standalone_snapshot(&snapshot, &manifest).is_err());
    }

    #[test]
    fn a_v46_ledger_file_is_snapshotted_for_this_binary() {
        let source = fresh_v46_ledger();
        let destination = scratch_path();
        let (manifest, snapshot) = snapshot_ledger_file(&source, &destination).unwrap();
        assert_eq!(manifest.schema_version(), 46);
        assert_eq!(snapshot.schema_version, 46);
        assert!(sidecars_of(&destination).is_empty());
        assert!(sidecars_of(&source).is_empty());
        // And it stays a verifiable standalone snapshot.
        inspect_standalone_snapshot(&destination, &manifest).unwrap();
    }
}
