//! After an Operational Restore the projection head cannot claim
//! Synchronized (S7 plan §7): the Ledger came back, the files on disk did
//! not.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::{Connection, OptionalExtension};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-restore-marker-{nonce}-{sequence}.sqlite3"))
}

fn head_state(path: &PathBuf) -> Option<String> {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT integrity_state FROM projection_manifest_head WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

#[test]
fn a_restored_ledger_has_its_projections_marked_out_of_sync() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap();
    // A verified head, as a Ledger that had published projections would have.
    drop(ledger);
    // Foreign keys off for this fixture only, so the generation's
    // publication id need not exist.
    Connection::open(&path)
        .unwrap()
        .execute_batch(&format!(
            "PRAGMA foreign_keys=OFF;
             INSERT INTO projection_manifest_generations (manifest_id,projection_schema,ledger_schema_version,ledger_revision,manifest_digest,verified_at,publication_idempotency_id) VALUES ('synthetic-manifest-1','pmc.projection/v1',46,0,'{}',1,'synthetic-publication-1');
             INSERT INTO projection_manifest_head (singleton,manifest_id,integrity_state,updated_at) VALUES (1,'synthetic-manifest-1','verified',1);",
            "a".repeat(64)
        ))
        .unwrap();
    assert_eq!(head_state(&path).as_deref(), Some("verified"));

    ledger = SqliteProductLedger::open(&path).unwrap();
    ledger
        .mark_projections_out_of_sync_after_restore(UtcTimestamp::from_unix_millis(2_000))
        .unwrap();
    drop(ledger);
    assert_eq!(head_state(&path).as_deref(), Some("out_of_sync"));
}

#[test]
fn a_ledger_that_never_published_gets_an_out_of_sync_head() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap();
    ledger
        .mark_projections_out_of_sync_after_restore(UtcTimestamp::from_unix_millis(2_000))
        .unwrap();
    drop(ledger);
    assert_eq!(head_state(&path).as_deref(), Some("out_of_sync"));
}
