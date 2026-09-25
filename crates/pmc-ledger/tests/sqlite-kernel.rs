use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use pmc_ledger::sqlite::{
    LedgerOpenError, LedgerTransactionError, SqliteProductLedger, APPLICATION_ID,
    CURRENT_SCHEMA_VERSION,
};
use rusqlite::Connection;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticWorkspace(PathBuf);

impl SyntheticWorkspace {
    fn new(label: &str) -> Self {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pmc-synthetic-sqlite-kernel-{label}-{nonce}-{sequence}"
        ));
        fs::create_dir(&path).expect("synthetic workspace must be creatable");
        Self(path)
    }

    fn database(&self) -> PathBuf {
        self.0.join("product-ledger.sqlite3")
    }
}

impl Drop for SyntheticWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pragma_i64(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .expect("test pragma must be readable")
}

fn create_foreign_database(path: &Path, setup: &str) {
    let connection = Connection::open(path).expect("foreign fixture must open");
    connection
        .execute_batch(setup)
        .expect("foreign fixture setup must succeed");
}

fn database_artifacts(path: &Path) -> Vec<Option<Vec<u8>>> {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
    .iter()
    .map(|artifact| fs::read(artifact).ok())
    .collect()
}

fn assert_rejected_without_artifact_changes(path: &Path, expected: LedgerOpenError) {
    let before = database_artifacts(path);
    let result = SqliteProductLedger::open(path);
    assert!(matches!(result, Err(error) if error == expected));
    let after = database_artifacts(path);
    for (index, (actual, original)) in after.iter().zip(&before).enumerate() {
        let first_difference = actual.as_ref().zip(original.as_ref()).and_then(|(a, b)| {
            a.iter()
                .zip(b)
                .position(|(actual_byte, original_byte)| actual_byte != original_byte)
        });
        assert!(
            actual == original,
            "database artifact {index} changed: before_length={:?}, after_length={:?}, first_difference={first_difference:?}",
            original.as_ref().map(Vec::len),
            actual.as_ref().map(Vec::len),
        );
    }
}

fn create_valid_ledger(path: &Path) {
    drop(SqliteProductLedger::open(path).expect("valid fixture must initialize"));
}

#[test]
fn new_ledger_initializes_identity_schema_and_exact_connection_policy() {
    let workspace = SyntheticWorkspace::new("new");
    let path = workspace.database();

    let ledger = SqliteProductLedger::open(&path).expect("new synthetic ledger must open");

    assert_eq!(ledger.schema_version(), CURRENT_SCHEMA_VERSION);
    assert_eq!(ledger.revision().expect("revision must be readable"), 0);
    assert_eq!(
        ledger.connection_policy().expect("policy must be readable"),
        pmc_ledger::sqlite::ConnectionPolicy {
            foreign_keys: true,
            journal_mode: pmc_ledger::sqlite::JournalMode::Wal,
            synchronous: pmc_ledger::sqlite::SynchronousMode::Full,
            busy_timeout: Duration::from_millis(5_000),
            trusted_schema: false,
            extension_loading_denied: true,
        }
    );
    drop(ledger);

    let raw = Connection::open(&path).expect("initialized fixture must be inspectable");
    assert_eq!(
        pragma_i64(&raw, "application_id"),
        i64::from(APPLICATION_ID)
    );
    assert_eq!(
        pragma_i64(&raw, "user_version"),
        i64::from(CURRENT_SCHEMA_VERSION)
    );
}

/// Every real caller reaches the Ledger through a canonicalized root --
/// `ProtectedSettingsRoot`, `WorkspaceIdentity`, and `VaultRoot` all hand out
/// `fs::canonicalize` output, which on Windows carries the verbatim `\\?\`
/// prefix. The rest of this file builds fixtures from `temp_dir()`, which is
/// never canonical, so that production path shape went unexercised until the
/// first real desktop wiring failed on it.
#[test]
fn a_canonical_root_path_initializes_and_reopens() {
    let workspace = SyntheticWorkspace::new("canonical");
    let canonical_root =
        fs::canonicalize(&workspace.0).expect("synthetic workspace must canonicalize");
    // Assert the fixture still carries the prefix, so this test cannot
    // silently stop covering the condition it exists for.
    #[cfg(windows)]
    assert!(
        matches!(
            canonical_root.components().next(),
            Some(std::path::Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    std::path::Prefix::VerbatimDisk(_) | std::path::Prefix::VerbatimUNC(..)
                )
        ),
        "expected a verbatim canonical fixture, got {canonical_root:?}"
    );
    let path = canonical_root.join("product-ledger.sqlite3");

    let ledger = SqliteProductLedger::open(&path).expect("canonical path must initialize");
    assert_eq!(ledger.schema_version(), CURRENT_SCHEMA_VERSION);
    assert_eq!(ledger.revision().expect("revision must be readable"), 0);
    drop(ledger);

    let reopened = SqliteProductLedger::open(&path).expect("canonical path must reopen");
    assert_eq!(reopened.schema_version(), CURRENT_SCHEMA_VERSION);
    assert_eq!(reopened.revision().expect("revision must be readable"), 0);
    assert_eq!(
        reopened
            .connection_policy()
            .expect("policy must be readable")
            .journal_mode,
        pmc_ledger::sqlite::JournalMode::Wal
    );
}

#[test]
fn valid_ledger_reopens_without_changing_identity_or_revision() {
    let workspace = SyntheticWorkspace::new("reopen");
    let path = workspace.database();
    let first = SqliteProductLedger::open(&path).expect("new ledger must open");
    drop(first);

    let reopened = SqliteProductLedger::open(&path).expect("valid ledger must reopen");

    assert_eq!(reopened.schema_version(), CURRENT_SCHEMA_VERSION);
    assert_eq!(reopened.revision().expect("revision must be readable"), 0);
}

#[test]
fn unclaimed_nonempty_database_is_rejected_without_modification() {
    let workspace = SyntheticWorkspace::new("unclaimed");
    let path = workspace.database();
    create_foreign_database(&path, "CREATE TABLE foreign_record(value TEXT);");
    let before = fs::read(&path).expect("fixture bytes must be readable");

    let result = SqliteProductLedger::open(&path);

    assert!(matches!(result, Err(LedgerOpenError::UnclaimedDatabase)));
    assert_eq!(
        fs::read(&path).expect("fixture bytes must remain readable"),
        before
    );
}

#[test]
fn wrong_identity_is_rejected_without_modification() {
    let workspace = SyntheticWorkspace::new("wrong-id");
    let path = workspace.database();
    create_foreign_database(
        &path,
        "PRAGMA application_id = 1179402569; CREATE TABLE foreign_record(value TEXT);",
    );
    let before = fs::read(&path).expect("fixture bytes must be readable");

    let result = SqliteProductLedger::open(&path);

    assert!(matches!(result, Err(LedgerOpenError::WrongApplication)));
    assert_eq!(
        fs::read(&path).expect("fixture bytes must remain readable"),
        before
    );
}

#[test]
fn future_schema_is_rejected_without_modification() {
    // One past the current version, derived rather than written down: a
    // literal here silently stops being "future" on the next schema bump.
    let future_version = CURRENT_SCHEMA_VERSION + 1;
    let workspace = SyntheticWorkspace::new("future");
    let path = workspace.database();
    create_foreign_database(
        &path,
        &format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {future_version}; CREATE TABLE ledger_metadata(singleton INTEGER PRIMARY KEY, schema_version INTEGER NOT NULL, ledger_revision INTEGER NOT NULL); INSERT INTO ledger_metadata VALUES(1, {future_version}, 0);"
        ),
    );
    let before = fs::read(&path).expect("fixture bytes must be readable");

    let result = SqliteProductLedger::open(&path);

    assert!(matches!(
        result,
        Err(LedgerOpenError::FutureSchema { found }) if found == future_version
    ));
    assert_eq!(
        fs::read(&path).expect("fixture bytes must remain readable"),
        before
    );
}

#[test]
fn inconsistent_metadata_is_rejected_without_modification() {
    let workspace = SyntheticWorkspace::new("metadata");
    let path = workspace.database();
    create_foreign_database(
        &path,
        &format!(
            "PRAGMA application_id = {APPLICATION_ID}; PRAGMA user_version = {CURRENT_SCHEMA_VERSION}; CREATE TABLE ledger_metadata(singleton INTEGER PRIMARY KEY, schema_version INTEGER NOT NULL, ledger_revision INTEGER NOT NULL); INSERT INTO ledger_metadata VALUES(1, 1, 0);"
        ),
    );
    let before = fs::read(&path).expect("fixture bytes must be readable");

    let result = SqliteProductLedger::open(&path);

    assert!(matches!(result, Err(LedgerOpenError::InvalidMetadata)));
    assert_eq!(
        fs::read(&path).expect("fixture bytes must remain readable"),
        before
    );
}

#[test]
fn corrupt_database_is_rejected_without_modification() {
    let workspace = SyntheticWorkspace::new("corrupt");
    let path = workspace.database();
    fs::write(&path, b"synthetic non-sqlite bytes").expect("fixture must be writable");
    let before = fs::read(&path).expect("fixture bytes must be readable");

    let result = SqliteProductLedger::open(&path);

    assert!(matches!(result, Err(LedgerOpenError::CorruptDatabase)));
    assert_eq!(
        fs::read(&path).expect("fixture bytes must remain readable"),
        before
    );
}

#[test]
fn previously_used_but_currently_empty_sqlite_is_never_adopted() {
    let workspace = SyntheticWorkspace::new("used-empty");
    let path = workspace.database();
    create_foreign_database(
        &path,
        "CREATE TABLE transient_record(id INTEGER PRIMARY KEY AUTOINCREMENT); INSERT INTO transient_record DEFAULT VALUES; DROP TABLE transient_record;",
    );

    assert_rejected_without_artifact_changes(&path, LedgerOpenError::UnclaimedDatabase);
}

#[test]
fn canonical_allowlist_rejects_views_triggers_and_extra_tables_without_changes() {
    for (label, setup) in [
        (
            "extra-view",
            "CREATE VIEW synthetic_view AS SELECT ledger_revision FROM ledger_metadata",
        ),
        (
            "extra-trigger",
            "CREATE TRIGGER synthetic_trigger AFTER UPDATE ON ledger_metadata BEGIN SELECT 1; END",
        ),
        (
            "extra-table",
            "CREATE TABLE synthetic_extra(value TEXT) STRICT",
        ),
        (
            "extra-index",
            "CREATE INDEX synthetic_index ON ledger_metadata(ledger_revision)",
        ),
    ] {
        let workspace = SyntheticWorkspace::new(label);
        let path = workspace.database();
        create_valid_ledger(&path);
        create_foreign_database(&path, setup);

        assert_rejected_without_artifact_changes(&path, LedgerOpenError::InvalidMetadata);
    }
}

#[test]
fn noncanonical_metadata_table_is_rejected_without_changes() {
    let workspace = SyntheticWorkspace::new("noncanonical-metadata");
    let path = workspace.database();
    create_valid_ledger(&path);
    create_foreign_database(
        &path,
        "DROP TABLE ledger_metadata;
         CREATE TABLE ledger_metadata(
             singleton INTEGER PRIMARY KEY,
             schema_version INTEGER NOT NULL,
             ledger_revision INTEGER NOT NULL
         );
         INSERT INTO ledger_metadata VALUES(1, 1, 0);",
    );

    assert_rejected_without_artifact_changes(&path, LedgerOpenError::InvalidMetadata);
}

fn assert_transaction_revalidation_denies(
    ledger: &mut SqliteProductLedger,
    expected_revision: u64,
) {
    let called = Cell::new(false);
    let result = ledger.with_immediate_transaction(|_| {
        called.set(true);
        Ok::<(), ()>(())
    });
    assert!(matches!(
        result,
        Err(LedgerTransactionError::IncompatibleLedger)
    ));
    assert!(!called.get());
    assert_eq!(
        ledger.revision().expect("revision must remain readable"),
        expected_revision
    );
}

#[test]
fn transaction_revalidates_application_identity_under_the_write_lock() {
    let workspace = SyntheticWorkspace::new("transaction-app-id");
    let path = workspace.database();
    let mut ledger = SqliteProductLedger::open(&path).expect("ledger must open");
    create_foreign_database(&path, "PRAGMA application_id = 1179402569;");

    assert_transaction_revalidation_denies(&mut ledger, 0);
}

#[test]
fn transaction_revalidates_schema_version_under_the_write_lock() {
    let workspace = SyntheticWorkspace::new("transaction-version");
    let path = workspace.database();
    let mut ledger = SqliteProductLedger::open(&path).expect("ledger must open");
    create_foreign_database(&path, "PRAGMA user_version = 3;");

    assert_transaction_revalidation_denies(&mut ledger, 0);
}

#[test]
fn transaction_revalidates_metadata_under_the_write_lock() {
    let workspace = SyntheticWorkspace::new("transaction-metadata");
    let path = workspace.database();
    let mut ledger = SqliteProductLedger::open(&path).expect("ledger must open");
    create_foreign_database(
        &path,
        "UPDATE ledger_metadata SET schema_version = 0 WHERE singleton = 1;",
    );

    assert_transaction_revalidation_denies(&mut ledger, 0);
}

#[test]
fn transaction_revalidates_schema_inventory_under_the_write_lock() {
    let workspace = SyntheticWorkspace::new("transaction-schema");
    let path = workspace.database();
    let mut ledger = SqliteProductLedger::open(&path).expect("ledger must open");
    create_foreign_database(
        &path,
        "CREATE VIEW synthetic_view AS SELECT ledger_revision FROM ledger_metadata;",
    );

    assert_transaction_revalidation_denies(&mut ledger, 0);
}

#[test]
fn operation_error_rolls_back_the_entire_immediate_transaction() {
    let workspace = SyntheticWorkspace::new("rollback");
    let mut ledger = SqliteProductLedger::open(workspace.database()).expect("ledger must open");

    let result = ledger.with_immediate_transaction(|transaction| {
        transaction
            .advance_revision(0)
            .map_err(|_| "synthetic revision conflict")?;
        Err::<(), _>("synthetic rejection")
    });

    assert!(matches!(
        result,
        Err(LedgerTransactionError::Operation("synthetic rejection"))
    ));
    assert_eq!(ledger.revision().expect("revision must be readable"), 0);
}

#[test]
fn unwinding_operation_drops_and_rolls_back_the_immediate_transaction() {
    let workspace = SyntheticWorkspace::new("drop-rollback");
    let mut ledger = SqliteProductLedger::open(workspace.database()).expect("ledger must open");

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), LedgerTransactionError<()>> =
            ledger.with_immediate_transaction(|transaction| {
                transaction
                    .advance_revision(0)
                    .expect("synthetic revision must advance before unwind");
                panic!("synthetic operation unwind");
            });
    }));

    assert!(unwind.is_err());
    assert_eq!(ledger.revision().expect("revision must be readable"), 0);
}

#[test]
fn successful_immediate_transaction_is_durable_before_success() {
    let workspace = SyntheticWorkspace::new("commit");
    let path = workspace.database();
    let mut ledger = SqliteProductLedger::open(&path).expect("ledger must open");

    let revision = ledger
        .with_immediate_transaction(|transaction| {
            transaction.advance_revision(0).map_err(|_| "conflict")
        })
        .expect("transaction must commit");
    drop(ledger);

    assert_eq!(revision, 1);
    let reopened = SqliteProductLedger::open(path).expect("committed ledger must reopen");
    assert_eq!(reopened.revision().expect("revision must be readable"), 1);
}

#[test]
fn competing_writer_fails_safely_after_the_bounded_busy_timeout() {
    let workspace = SyntheticWorkspace::new("busy");
    let path = workspace.database();
    let first = SqliteProductLedger::open(&path).expect("first writer must open");
    let mut second = SqliteProductLedger::open(&path).expect("second writer must open");
    let raw_lock = Connection::open(&path).expect("synthetic lock connection must open");
    raw_lock
        .execute_batch("BEGIN IMMEDIATE")
        .expect("synthetic writer lock must start");
    let started = Instant::now();

    let result = second.with_immediate_transaction(|_| Ok::<(), ()>(()));

    let elapsed = started.elapsed();
    assert!(matches!(result, Err(LedgerTransactionError::Busy)));
    assert!(elapsed >= Duration::from_secs(4));
    assert!(elapsed < Duration::from_secs(8));
    drop(first);
}
