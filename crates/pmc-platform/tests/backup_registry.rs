//! The verified-archive registry of S7-A: which backups verified end to end,
//! checked at startup by size and full container hash, never refreshed.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::backup_registry::{
    backup_due, check_record, orphan_archives, BackupRecord, BackupRegistry, RecordStatus,
    RegistryError, RegistryStore, DUE_AFTER_MILLIS,
};
use sha2::{Digest, Sha256};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn scratch(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pmc-registry-{name}-{nonce}-{sequence}"));
    fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("scratch dir failed: {error}"));
    fs::canonicalize(&dir).unwrap_or_else(|error| panic!("canonicalize failed: {error}"))
}

const HOUR: i64 = 60 * 60 * 1000;

fn record(
    destination: &std::path::Path,
    name: &str,
    bytes: &[u8],
    verified_at: i64,
) -> BackupRecord {
    fs::write(destination.join(name), bytes)
        .unwrap_or_else(|error| panic!("write failed: {error}"));
    BackupRecord {
        archive_id: format!("id-{name}"),
        file_name: name.to_owned(),
        destination: destination.to_path_buf(),
        created_at_millis: verified_at - 1000,
        verified_at_millis: verified_at,
        ledger_schema_version: 46,
        ledger_revision: 7,
        container_sha256: format!("{:x}", Sha256::digest(bytes)),
        container_bytes: bytes.len() as u64,
        snapshot_sha256: "c".repeat(64),
        authority_inventory_sha256: "d".repeat(64),
        authority_record_count: 0,
    }
}

#[test]
fn the_registry_round_trips_and_starts_empty() {
    let dir = scratch("round-trip");
    let store = RegistryStore::new(dir.join("registry-v1.json"));
    assert_eq!(
        store
            .load()
            .unwrap_or_else(|error| panic!("load failed: {error}")),
        BackupRegistry::default()
    );
    let destination = scratch("round-trip-destination");
    let mut registry = BackupRegistry::default();
    registry.records.push(record(
        &destination,
        "pmc-operational-a-v1.tar.zst.age",
        b"archive",
        10 * HOUR,
    ));
    store
        .save(&registry)
        .unwrap_or_else(|error| panic!("save failed: {error}"));
    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("load failed: {error}"));
    assert_eq!(loaded.records, registry.records);
    assert_eq!(loaded.format, "pmc-verified-backup-registry/v1");
}

#[test]
fn a_record_is_valid_only_while_its_file_is_byte_for_byte_the_same() {
    let destination = scratch("check");
    let entry = record(
        &destination,
        "pmc-operational-b-v1.tar.zst.age",
        b"archive bytes",
        HOUR,
    );
    assert_eq!(check_record(&entry), RecordStatus::Valid);

    // Same size, different content: only the full hash can tell.
    fs::write(destination.join(&entry.file_name), b"archive BYTES")
        .unwrap_or_else(|error| panic!("rewrite failed: {error}"));
    assert_eq!(check_record(&entry), RecordStatus::Altered);

    fs::remove_file(destination.join(&entry.file_name))
        .unwrap_or_else(|error| panic!("remove failed: {error}"));
    assert_eq!(check_record(&entry), RecordStatus::Missing);
}

#[test]
fn a_backup_is_due_after_twenty_four_hours_or_with_none_at_all() {
    assert_eq!(DUE_AFTER_MILLIS, 24 * HOUR);
    let now = 100 * HOUR;
    assert!(backup_due(std::iter::empty(), now));
    assert!(!backup_due([now - 23 * HOUR].into_iter(), now));
    assert!(backup_due([now - 25 * HOUR].into_iter(), now));
    assert!(!backup_due([now - 30 * HOUR, now - HOUR].into_iter(), now));
}

#[test]
fn a_verification_time_in_the_future_does_not_count_as_fresh() {
    // A clock set back after a backup must not keep it "fresh" forever.
    let now = 100 * HOUR;
    assert!(backup_due([now + 2 * HOUR].into_iter(), now));
}

#[test]
fn an_archive_the_registry_does_not_know_is_an_orphan() {
    let destination = scratch("orphans");
    let known = record(
        &destination,
        "pmc-operational-c-v1.tar.zst.age",
        b"known",
        HOUR,
    );
    fs::write(
        destination.join("pmc-operational-d-v1.tar.zst.age"),
        b"unknown",
    )
    .unwrap_or_else(|error| panic!("write failed: {error}"));
    fs::write(destination.join("holiday-photo.jpg"), b"not ours")
        .unwrap_or_else(|error| panic!("write failed: {error}"));
    let mut registry = BackupRegistry::default();
    registry.records.push(known);
    assert_eq!(
        orphan_archives(&destination, &registry),
        vec!["pmc-operational-d-v1.tar.zst.age".to_owned()]
    );
}

#[test]
fn a_save_from_a_stale_read_is_refused_so_no_verified_record_is_lost() {
    let dir = scratch("conflict");
    let destination = scratch("conflict-destination");
    let store = RegistryStore::new(dir.join("registry-v1.json"));
    let mut first = store
        .load()
        .unwrap_or_else(|error| panic!("load failed: {error}"));
    let mut second = first.clone();
    first.records.push(record(
        &destination,
        "pmc-operational-e-v1.tar.zst.age",
        b"first",
        HOUR,
    ));
    second.records.push(record(
        &destination,
        "pmc-operational-f-v1.tar.zst.age",
        b"second",
        HOUR,
    ));
    store
        .save(&first)
        .unwrap_or_else(|error| panic!("save failed: {error}"));
    assert!(matches!(store.save(&second), Err(RegistryError::Conflict)));
    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("load failed: {error}"));
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.records, first.records);
}

#[cfg(windows)]
#[test]
fn a_known_archive_is_not_an_orphan_under_another_spelling_of_its_folder() {
    let destination = scratch("spelling");
    let known = record(
        &destination,
        "pmc-operational-g-v1.tar.zst.age",
        b"known",
        HOUR,
    );
    let mut registry = BackupRegistry::default();
    registry.records.push(known);
    let upper = PathBuf::from(destination.to_string_lossy().to_uppercase());
    assert!(orphan_archives(&upper, &registry).is_empty());
}

#[test]
fn concurrent_saves_from_the_same_read_let_exactly_one_through() {
    let dir = scratch("race");
    let destination = scratch("race-destination");
    let store = RegistryStore::new(dir.join("registry-v1.json"));
    let writers = 8;
    let barrier = std::sync::Barrier::new(writers);
    let outcomes: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..writers)
            .map(|index| {
                let (store, barrier, destination) = (&store, &barrier, &destination);
                scope.spawn(move || {
                    let mut registry = store
                        .load()
                        .unwrap_or_else(|error| panic!("load failed: {error}"));
                    registry.records.push(record(
                        destination,
                        &format!("pmc-operational-r{index}-v1.tar.zst.age"),
                        format!("race {index}").as_bytes(),
                        HOUR,
                    ));
                    barrier.wait();
                    match store.save(&registry) {
                        Ok(()) => true,
                        Err(RegistryError::Conflict) => false,
                        Err(error) => panic!("save failed: {error}"),
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or_else(|_| panic!("writer panicked")))
            .collect()
    });
    assert_eq!(outcomes.iter().filter(|saved| **saved).count(), 1);
    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("load failed: {error}"));
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.records.len(), 1);
}
