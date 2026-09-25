use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticPath(PathBuf);

impl SyntheticPath {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("pmc-synthetic-{label}-{nonce}-{sequence}.sqlite3")))
    }
}

impl Drop for SyntheticPath {
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            PathBuf::from(format!("{}-wal", self.0.display())),
            PathBuf::from(format!("{}-shm", self.0.display())),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn verified_snapshot_is_reopenable_and_never_overwrites_a_destination() {
    let source = SyntheticPath::new("snapshot-source");
    let destination = SyntheticPath::new("snapshot-destination");
    let ledger = SqliteProductLedger::open(&source.0).unwrap();

    let manifest = ledger.create_verified_snapshot(&destination.0).unwrap();

    assert_eq!(manifest.schema_version(), ledger.schema_version());
    assert_eq!(manifest.ledger_revision(), ledger.revision().unwrap());
    assert_eq!(manifest.sha256().len(), 64);
    // Verified as a standalone file first: once it is opened as a live
    // Ledger it has WAL files beside it and is no longer a snapshot.
    assert!(ledger.verify_snapshot(&destination.0, &manifest).is_ok());
    let restored = SqliteProductLedger::open(&destination.0).unwrap();
    assert_eq!(restored.schema_version(), ledger.schema_version());
    assert_eq!(restored.revision().unwrap(), ledger.revision().unwrap());
    assert!(ledger.verify_snapshot(&destination.0, &manifest).is_err());
    assert!(ledger.create_verified_snapshot(&destination.0).is_err());
}

/// Both `create_verified_snapshot` and `verify_snapshot` read their target
/// through the inspection URI, so a canonicalized (Windows verbatim) source
/// or destination once failed post-backup verification even though the
/// backup itself had succeeded.
#[test]
fn a_canonical_source_and_destination_round_trip() {
    let canonical_temp =
        std::fs::canonicalize(std::env::temp_dir()).expect("temp dir must canonicalize");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let source =
        SyntheticPath(canonical_temp.join(format!("pmc-synthetic-canon-src-{nonce}.sqlite3")));
    let destination =
        SyntheticPath(canonical_temp.join(format!("pmc-synthetic-canon-dst-{nonce}.sqlite3")));
    #[cfg(windows)]
    assert!(
        matches!(
            source.0.components().next(),
            Some(std::path::Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    std::path::Prefix::VerbatimDisk(_) | std::path::Prefix::VerbatimUNC(..)
                )
        ),
        "expected a verbatim canonical fixture, got {:?}",
        source.0
    );

    let ledger = SqliteProductLedger::open(&source.0).expect("canonical source must open");
    let manifest = ledger
        .create_verified_snapshot(&destination.0)
        .expect("canonical destination must snapshot and verify");

    assert_eq!(manifest.schema_version(), ledger.schema_version());
    assert!(ledger.verify_snapshot(&destination.0, &manifest).is_ok());
}

#[test]
fn snapshot_verification_rejects_a_checksum_mismatch() {
    let source = SyntheticPath::new("snapshot-source");
    let destination = SyntheticPath::new("snapshot-destination");
    let ledger = SqliteProductLedger::open(&source.0).unwrap();
    let manifest = ledger.create_verified_snapshot(&destination.0).unwrap();

    let mut bytes = std::fs::read(&destination.0).unwrap();
    let final_byte = bytes.last_mut().unwrap();
    *final_byte ^= 0x01;
    std::fs::write(&destination.0, bytes).unwrap();

    assert!(ledger.verify_snapshot(&destination.0, &manifest).is_err());
}
