//! In-place upgrade of an older supported Ledger (slice 8d; ADR 0004; the
//! product owner's decisions of 2026-09-19 and 2026-09-22).
//!
//! One SQLite transaction takes the Ledger from its version to this binary's:
//! foreign keys off before `BEGIN IMMEDIATE`, the source re-checked inside the
//! transaction, each descriptor's SQL with its registry row, the metadata and
//! `user_version`, then — still inside the transaction — foreign keys,
//! integrity, the migration registry and the canonical schema of the target.
//! Only if all of that holds does it commit; anything else rolls back.
//!
//! It never runs on its own authority. The caller passes the source it
//! inspected and a proof that a backup of exactly that source was just made
//! and verified ([`VerifiedPreUpgradeBackup`]); a Ledger that changed since,
//! or a proof about anything else, is refused before a write connection is
//! opened. After the commit the file is inspected again, so the outcome
//! reported is what the file now is, never what was hoped.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::inspection::{
    classify_connection, inspect_ledger_against, validate_at, LedgerCompatibility, LedgerInspection,
};
use super::migrations::{descriptors_through, production_descriptors, Migration};
use super::{classify_open_error, pragma_i64, LedgerOpenError, BUSY_TIMEOUT};

/// Exactly what was inspected, and so exactly what the backup must hold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeSource {
    pub schema_version: u32,
    pub ledger_revision: u64,
    /// SHA-256 of the authority inventory: every record's type, id, version.
    pub inventory_sha256: String,
    /// SHA-256 of the whole logical content (see `LedgerInspection`): the
    /// binding that makes any later write, of any kind, a changed source.
    pub content_sha256: String,
}

impl From<&LedgerInspection> for UpgradeSource {
    fn from(inspection: &LedgerInspection) -> Self {
        Self {
            schema_version: inspection.schema_version,
            ledger_revision: inspection.ledger_revision,
            inventory_sha256: inspection.inventory.sha256.clone(),
            content_sha256: inspection.content_sha256.clone(),
        }
    }
}

/// A backup made for this upgrade and verified end to end. Implemented by
/// the backup pipeline (`pmc-application`), whose receipt can only come from
/// a completed run; the Ledger crate depends on nothing of it but this.
pub trait VerifiedPreUpgradeBackup {
    /// The source the archived snapshot held.
    fn verified_source(&self) -> &UpgradeSource;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeOutcome {
    /// Committed and read back at the target version.
    Upgraded { from: u32, to: u32 },
}

/// Why no upgrade is reported. The last three are the three failure
/// outcomes of the accepted DG3 upgrade-gate amendment §4.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerUpgradeError {
    /// The file is not an upgradeable Ledger for this binary.
    NotUpgradeable(LedgerCompatibility),
    /// The proof is about another source than the one passed.
    ProofDoesNotMatch,
    /// The Ledger is no longer the source that was inspected and backed up.
    SourceChanged,
    /// Storage could not be opened or locked; nothing was attempted.
    Storage(LedgerOpenError),
    /// Nothing changed: the transaction rolled back.
    RolledBack,
    /// It committed, but the file does not read back as the target.
    UpgradedButUnreadable,
    /// Whether it committed cannot be told from the file.
    OutcomeUnknown,
}

/// Upgrade the Ledger at `path` to this binary's schema version.
pub fn upgrade_in_place(
    path: impl AsRef<Path>,
    expected: &UpgradeSource,
    proof: &impl VerifiedPreUpgradeBackup,
) -> Result<UpgradeOutcome, LedgerUpgradeError> {
    upgrade_with(
        path.as_ref(),
        expected,
        proof.verified_source(),
        production_descriptors(),
        None,
    )
}

/// Where a test may make the upgrade fail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) enum FailAt {
    AfterForeignKeysOff,
    AfterMigrationSql(u32),
    AfterRegistryRow(u32),
    AfterMetadata,
    AfterUserVersion,
    BeforeCommit,
    /// After the commit: the read-back is made to fail.
    ReadBack,
}

fn injected(fail_at: Option<FailAt>, step: FailAt) -> bool {
    fail_at == Some(step)
}

pub(super) fn upgrade_with(
    path: &Path,
    expected: &UpgradeSource,
    proven: &UpgradeSource,
    descriptors: &'static [Migration],
    fail_at: Option<FailAt>,
) -> Result<UpgradeOutcome, LedgerUpgradeError> {
    if proven != expected {
        return Err(LedgerUpgradeError::ProofDoesNotMatch);
    }
    let target = descriptors.last().map_or(0, |migration| migration.version);
    match inspect_ledger_against(path, descriptors).map_err(LedgerUpgradeError::Storage)? {
        LedgerCompatibility::UpgradeableFrom(inspection) => {
            if UpgradeSource::from(&inspection) != *expected {
                return Err(LedgerUpgradeError::SourceChanged);
            }
        }
        other => return Err(LedgerUpgradeError::NotUpgradeable(other)),
    }
    let target_descriptors =
        descriptors_through(descriptors, target).ok_or(LedgerUpgradeError::RolledBack)?;

    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| LedgerUpgradeError::Storage(classify_open_error(error)))?;
    let transacted = run_transaction(
        &mut connection,
        expected,
        descriptors,
        target_descriptors,
        target,
        fail_at,
    );
    match transacted {
        Ok(()) => {}
        Err(LedgerUpgradeError::OutcomeUnknown) => {
            // COMMIT itself failed. Say only what the file shows.
            drop(connection);
            return match inspect_ledger_against(path, descriptors) {
                Ok(LedgerCompatibility::UpgradeableFrom(inspection))
                    if UpgradeSource::from(&inspection) == *expected =>
                {
                    Err(LedgerUpgradeError::RolledBack)
                }
                Ok(LedgerCompatibility::Current(inspection))
                    if inspection.schema_version == target =>
                {
                    Ok(UpgradeOutcome::Upgraded {
                        from: expected.schema_version,
                        to: target,
                    })
                }
                _ => Err(LedgerUpgradeError::OutcomeUnknown),
            };
        }
        Err(error) => {
            // Dropped uncommitted: SQLite rolled the transaction back.
            drop(connection);
            return Err(error);
        }
    }
    // Committed. Fold the WAL into the file and close, then read it back:
    // the outcome is what the file shows. A checkpoint that could not finish
    // leaves a WAL beside it, which the read-back reports as not readable —
    // never as unchanged.
    let _ = connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    drop(connection);
    if injected(fail_at, FailAt::ReadBack) {
        return Err(LedgerUpgradeError::UpgradedButUnreadable);
    }
    match inspect_ledger_against(path, descriptors) {
        Ok(LedgerCompatibility::Current(inspection)) if inspection.schema_version == target => {
            Ok(UpgradeOutcome::Upgraded {
                from: expected.schema_version,
                to: target,
            })
        }
        _ => Err(LedgerUpgradeError::UpgradedButUnreadable),
    }
}

/// Everything between `foreign_keys=OFF` and COMMIT. An `Err` returns before
/// the commit, so dropping the transaction rolls every change back — except
/// a failed COMMIT itself, which is reported as `OutcomeUnknown` unless the
/// file shows it plainly rolled back.
fn run_transaction(
    connection: &mut Connection,
    expected: &UpgradeSource,
    descriptors: &'static [Migration],
    target_descriptors: &[Migration],
    target: u32,
    fail_at: Option<FailAt>,
) -> Result<(), LedgerUpgradeError> {
    let storage = |error| LedgerUpgradeError::Storage(classify_open_error(error));
    connection.busy_timeout(BUSY_TIMEOUT).map_err(storage)?;
    // Before BEGIN: SQLite ignores this pragma inside a transaction.
    connection
        .execute_batch("PRAGMA trusted_schema = OFF; PRAGMA foreign_keys = OFF;")
        .map_err(storage)?;
    if pragma_i64(connection, "foreign_keys").map_err(LedgerUpgradeError::Storage)? != 0 {
        return Err(LedgerUpgradeError::Storage(
            LedgerOpenError::PolicyViolation,
        ));
    }
    if injected(fail_at, FailAt::AfterForeignKeysOff) {
        return Err(LedgerUpgradeError::RolledBack);
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;

    // The source again, now under the write lock.
    match classify_connection(&transaction, descriptors) {
        Ok(LedgerCompatibility::UpgradeableFrom(inspection))
            if UpgradeSource::from(&inspection) == *expected => {}
        _ => return Err(LedgerUpgradeError::SourceChanged),
    }

    let rolled_back = |_| LedgerUpgradeError::RolledBack;
    let first =
        usize::try_from(expected.schema_version).map_err(|_| LedgerUpgradeError::RolledBack)?;
    for migration in target_descriptors
        .get(first..)
        .ok_or(LedgerUpgradeError::RolledBack)?
    {
        super::migrations::validate_descriptor(migration).map_err(rolled_back)?;
        transaction
            .execute_batch(migration.sql)
            .map_err(|_| LedgerUpgradeError::RolledBack)?;
        if injected(fail_at, FailAt::AfterMigrationSql(migration.version)) {
            return Err(LedgerUpgradeError::RolledBack);
        }
        transaction
            .execute(
                "INSERT INTO schema_migrations(version,migration_key,checksum) VALUES(?1,?2,?3)",
                (
                    i64::from(migration.version),
                    migration.key,
                    migration.checksum,
                ),
            )
            .map_err(|_| LedgerUpgradeError::RolledBack)?;
        if injected(fail_at, FailAt::AfterRegistryRow(migration.version)) {
            return Err(LedgerUpgradeError::RolledBack);
        }
    }
    transaction
        .execute(
            "UPDATE ledger_metadata SET schema_version=?1 WHERE singleton=1",
            [i64::from(target)],
        )
        .map_err(|_| LedgerUpgradeError::RolledBack)?;
    if injected(fail_at, FailAt::AfterMetadata) {
        return Err(LedgerUpgradeError::RolledBack);
    }
    transaction
        .execute_batch(&format!("PRAGMA user_version={target};"))
        .map_err(|_| LedgerUpgradeError::RolledBack)?;
    if injected(fail_at, FailAt::AfterUserVersion) {
        return Err(LedgerUpgradeError::RolledBack);
    }
    // Foreign keys, integrity, registry, canonical schema and metadata — all
    // at the target, inside the transaction.
    if validate_at(&transaction, target_descriptors, target).is_none() {
        return Err(LedgerUpgradeError::RolledBack);
    }
    if injected(fail_at, FailAt::BeforeCommit) {
        return Err(LedgerUpgradeError::RolledBack);
    }
    transaction
        .commit()
        .map_err(|_| LedgerUpgradeError::OutcomeUnknown)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pmc_domain::classification::DataClassification;
    use pmc_domain::identity::{AuditEventId, CorrelationId, IdempotencyId, PortfolioId};
    use pmc_domain::portfolio::{CreatePortfolio, LongText, OperationContext, ShortText};
    use pmc_domain::provenance::Provenance;
    use pmc_domain::time::UtcTimestamp;
    use rusqlite::types::Value;
    use sha2::{Digest, Sha256};

    use super::super::migrations::MIGRATIONS_FOR_TESTS;
    use super::super::{immutable_read_uri, SqliteProductLedger, CURRENT_SCHEMA_VERSION};
    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Proof(UpgradeSource);

    impl VerifiedPreUpgradeBackup for Proof {
        fn verified_source(&self) -> &UpgradeSource {
            &self.0
        }
    }

    fn scratch_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pmc-upgrade-{nonce}-{sequence}.sqlite3"))
    }

    fn add_portfolio(ledger: &mut SqliteProductLedger, id: &str) {
        ledger
            .create_portfolio(
                CreatePortfolio {
                    id: PortfolioId::parse(id).unwrap(),
                    name: ShortText::parse("Synthetic").unwrap(),
                    details: LongText::parse("Synthetic only.").unwrap(),
                    classification: Some(DataClassification::Internal),
                    provenance: Provenance::UserEntered,
                    context: OperationContext {
                        idempotency_id: IdempotencyId::parse(format!("idem-{id}")).unwrap(),
                        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
                    },
                },
                AuditEventId::parse(format!("audit-{id}")).unwrap(),
                UtcTimestamp::from_unix_millis(1_000),
            )
            .unwrap();
    }

    /// A current Ledger holding two Portfolios, closed: what this binary
    /// writes, so a test that needs a *next* version starts from it.
    fn populated_current() -> PathBuf {
        let path = scratch_path();
        let mut ledger = SqliteProductLedger::open(&path).unwrap();
        add_portfolio(&mut ledger, "synthetic-portfolio-a");
        add_portfolio(&mut ledger, "synthetic-portfolio-b");
        drop(ledger);
        path
    }

    /// A populated v46 Ledger: a private copy of the checked-in fixture. This
    /// binary cannot write one (its writers are newer), which is the point.
    fn populated_v46() -> PathBuf {
        frozen_v46_copy().0
    }

    fn source_for(path: &Path, descriptors: &'static [Migration]) -> UpgradeSource {
        match inspect_ledger_against(path, descriptors).unwrap() {
            LedgerCompatibility::UpgradeableFrom(inspection) => UpgradeSource::from(&inspection),
            other => panic!("expected UpgradeableFrom, got {other:?}"),
        }
    }

    /// Every table's rows, the schema, and the header pragmas: equal digests
    /// mean the Ledger is logically unchanged.
    fn logical_digest(path: &Path) -> String {
        let connection = Connection::open_with_flags(
            immutable_read_uri(path).unwrap(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        let mut hasher = Sha256::new();
        for pragma in ["user_version", "application_id"] {
            let value: i64 = connection
                .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
                .unwrap();
            hasher.update(format!("{pragma}={value}\n"));
        }
        let objects: Vec<(String, String, Option<String>)> = connection
            .prepare("SELECT type,name,sql FROM sqlite_schema ORDER BY type,name")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for (kind, name, sql) in &objects {
            hasher.update(format!("{kind}|{name}|{sql:?}\n"));
            if kind != "table" || name.starts_with("sqlite_") {
                continue;
            }
            let mut statement = connection
                .prepare(&format!("SELECT * FROM \"{name}\""))
                .unwrap();
            let columns = statement.column_count();
            let mut rows: Vec<String> = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get::<_, Value>(index).map(|value| format!("{value:?}")))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|values| values.join("|"))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            rows.sort();
            for row in rows {
                hasher.update(format!("{name}:{row}\n"));
            }
        }
        format!("{:x}", hasher.finalize())
    }

    fn count(path: &Path, sql: &str) -> i64 {
        let connection = Connection::open_with_flags(
            immutable_read_uri(path).unwrap(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        connection.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// The v47 objects are there, empty, and the Ledger is current.
    fn assert_upgraded_to_current(path: &Path) {
        for table in [
            "record_id_reservations",
            "risk_response_replay_operations",
            "risk_response_command_updates",
            "risk_response_replay_audits",
        ] {
            assert_eq!(
                count(path, &format!("SELECT count(*) FROM {table}")),
                0,
                "{table}"
            );
        }
        assert_current_schema(path);
    }

    /// The v47 and v48 registry rows and the v48 indexes are there, every
    /// foreign key holds, and the production entry point opens it.
    fn assert_current_schema(path: &Path) {
        for index in [
            "idx_evidence_references_vault_relative_path",
            "idx_evidence_references_fingerprint",
        ] {
            assert_eq!(
                count(
                    path,
                    &format!(
                        "SELECT count(*) FROM sqlite_schema WHERE type='index' AND name='{index}'"
                    )
                ),
                1,
                "{index}"
            );
        }
        assert_eq!(
            count(
                path,
                "SELECT count(*) FROM schema_migrations WHERE version=48 AND migration_key='0048_evidence_from_file'"
            ),
            1
        );
        assert_eq!(
            count(path, "SELECT count(*) FROM pragma_foreign_key_check"),
            0
        );
        assert_eq!(
            count(
                path,
                "SELECT count(*) FROM schema_migrations WHERE version=47 AND migration_key='0047_record_entry_foundation'"
            ),
            1
        );
        let opened = SqliteProductLedger::open(path).unwrap();
        assert_eq!(opened.schema_version(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn a_populated_v46_ledger_upgrades_with_its_records_intact() {
        let path = populated_v46();
        let descriptors = production_descriptors();
        let source = source_for(&path, descriptors);
        let outcome = upgrade_with(&path, &source, &source, descriptors, None).unwrap();
        assert_eq!(outcome, UpgradeOutcome::Upgraded { from: 46, to: 48 });
        let LedgerCompatibility::Current(after) =
            inspect_ledger_against(&path, descriptors).unwrap()
        else {
            panic!("not current after the upgrade");
        };
        // The same records at the same versions, untouched by the migration.
        assert_eq!(after.inventory.sha256, source.inventory_sha256);
        assert_eq!(after.ledger_revision, source.ledger_revision);
        assert_upgraded_to_current(&path);
        // Asked again, nothing is done: it is current now.
        assert!(matches!(
            upgrade_with(&path, &source, &source, descriptors, None),
            Err(LedgerUpgradeError::NotUpgradeable(
                LedgerCompatibility::Current(_)
            ))
        ));
    }

    /// v48 rebuilds `record_id_reservations` to widen its checks: every row a
    /// v47 Ledger reserved survives it unchanged, the table stays immutable
    /// and undeletable, and Evidence can now be reserved.
    #[test]
    fn a_v47_ledger_keeps_every_reservation_through_v48() {
        let path = scratch_path();
        super::super::migrations::bootstrap_at_for_test(&path, 47);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('sheet-risk','risk','create_risk','risk','risk-reserved',300);
                 INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('sheet-link','relationship','create_relationship','relationship','relationship-reserved',301);",
            )
            .unwrap();
        // A v47 Ledger refuses an Evidence reservation: that is what v48 changes.
        assert!(connection
            .execute(
                "INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('sheet-evidence','evidence','create_evidence_reference','evidence_reference','evidence-reserved',302)",
                [],
            )
            .is_err());
        drop(connection);
        let rows = |path: &Path| -> Vec<String> {
            let connection = Connection::open(path).unwrap();
            let mut statement = connection
                .prepare("SELECT idempotency_id||'|'||namespace||'|'||operation||'|'||entity_kind||'|'||generated_id||'|'||reserved_at FROM record_id_reservations ORDER BY idempotency_id")
                .unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        };
        let before = rows(&path);
        assert_eq!(before.len(), 2);

        let descriptors = production_descriptors();
        let source = source_for(&path, descriptors);
        assert_eq!(
            upgrade_with(&path, &source, &source, descriptors, None).unwrap(),
            UpgradeOutcome::Upgraded { from: 47, to: 48 }
        );
        assert_eq!(rows(&path), before);
        assert_current_schema(&path);

        let connection = Connection::open(&path).unwrap();
        assert!(connection
            .execute(
                "UPDATE record_id_reservations SET generated_id='risk-other' WHERE idempotency_id='sheet-risk'",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                "DELETE FROM record_id_reservations WHERE idempotency_id='sheet-risk'",
                [],
            )
            .is_err());
        connection
            .execute(
                "INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('sheet-evidence','evidence','create_evidence_reference','evidence_reference','evidence-reserved',302)",
                [],
            )
            .unwrap();
        // The same id twice is still refused.
        assert!(connection
            .execute(
                "INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('sheet-evidence-2','evidence','create_evidence_reference','evidence_reference','evidence-reserved',303)",
                [],
            )
            .is_err());
    }

    #[test]
    fn a_failure_at_any_step_before_commit_changes_nothing_and_can_be_retried() {
        let descriptors = production_descriptors();
        for step in [
            FailAt::AfterForeignKeysOff,
            FailAt::AfterMigrationSql(47),
            FailAt::AfterRegistryRow(47),
            FailAt::AfterMigrationSql(48),
            FailAt::AfterRegistryRow(48),
            FailAt::AfterMetadata,
            FailAt::AfterUserVersion,
            FailAt::BeforeCommit,
        ] {
            let path = populated_v46();
            let before = logical_digest(&path);
            let source = source_for(&path, descriptors);
            assert_eq!(
                upgrade_with(&path, &source, &source, descriptors, Some(step)),
                Err(LedgerUpgradeError::RolledBack),
                "{step:?}"
            );
            assert_eq!(logical_digest(&path), before, "{step:?} left a change");
            assert_eq!(source_for(&path, descriptors), source, "{step:?}");
            assert!(
                upgrade_with(&path, &source, &source, descriptors, None).is_ok(),
                "{step:?}: the retry must succeed"
            );
        }
    }

    #[test]
    fn a_failed_read_back_after_commit_is_not_reported_as_unchanged() {
        let path = populated_v46();
        let descriptors = production_descriptors();
        let source = source_for(&path, descriptors);
        assert_eq!(
            upgrade_with(&path, &source, &source, descriptors, Some(FailAt::ReadBack)),
            Err(LedgerUpgradeError::UpgradedButUnreadable)
        );
        // It did commit: the file is current.
        assert!(matches!(
            inspect_ledger_against(&path, descriptors).unwrap(),
            LedgerCompatibility::Current(_)
        ));
    }

    #[test]
    fn a_proof_about_another_source_or_a_changed_ledger_is_refused_untouched() {
        let path = populated_v46();
        let descriptors = production_descriptors();
        let source = source_for(&path, descriptors);
        let before = logical_digest(&path);

        let other = UpgradeSource {
            inventory_sha256: "0".repeat(64),
            ..source.clone()
        };
        assert_eq!(
            upgrade_with(&path, &source, &other, descriptors, None),
            Err(LedgerUpgradeError::ProofDoesNotMatch)
        );
        assert_eq!(logical_digest(&path), before);

        // A write after the backup (as the previous release's writers would
        // leave it): the Ledger is no longer that source.
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "UPDATE ledger_metadata SET ledger_revision=ledger_revision+1 WHERE singleton=1",
                [],
            )
            .unwrap();
        drop(connection);
        let changed = logical_digest(&path);
        assert_eq!(
            upgrade_with(&path, &source, &source, descriptors, None),
            Err(LedgerUpgradeError::SourceChanged)
        );
        assert_eq!(logical_digest(&path), changed);
    }

    #[test]
    fn a_target_that_fails_its_checks_rolls_back() {
        // A next version whose data step leaves a dangling foreign key: it
        // applies with foreign keys off, and the in-transaction check must
        // refuse it. From a current Ledger, so the test needs no fixture.
        const BAD_SQL: &str = "INSERT INTO relationships(id,kind,purpose) VALUES('orphan','portfolio_product',NULL);\n";
        let mut list = MIGRATIONS_FOR_TESTS.to_vec();
        list.push(Migration {
            version: 49,
            key: "0049_test_only_dangling",
            checksum: Box::leak(
                format!("sha256:{:x}", Sha256::digest(BAD_SQL.as_bytes())).into_boxed_str(),
            ),
            sql: BAD_SQL,
        });
        let descriptors: &'static [Migration] = Box::leak(list.into_boxed_slice());
        let path = populated_current();
        let before = logical_digest(&path);
        let source = source_for(&path, descriptors);
        assert_eq!(
            upgrade_with(&path, &source, &source, descriptors, None),
            Err(LedgerUpgradeError::RolledBack)
        );
        assert_eq!(logical_digest(&path), before);
    }

    #[test]
    fn the_production_entry_point_refuses_a_current_ledger() {
        let path = populated_current();
        let LedgerCompatibility::Current(inspection) = super::super::inspect_ledger(&path).unwrap()
        else {
            panic!("expected Current");
        };
        let source = UpgradeSource::from(&inspection);
        assert!(matches!(
            upgrade_in_place(&path, &source, &Proof(source.clone())),
            Err(LedgerUpgradeError::NotUpgradeable(
                LedgerCompatibility::Current(_)
            ))
        ));
    }

    /// A private copy of the checked-in v46 fixture, after checking it is the
    /// file its provenance names.
    fn frozen_v46_copy() -> (PathBuf, String) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");
        let bytes = std::fs::read(fixtures.join("ledger-v46-seeded.sqlite3")).unwrap();
        let provenance =
            std::fs::read_to_string(fixtures.join("ledger-v46-seeded.provenance.json")).unwrap();
        assert!(
            provenance.contains(&format!(
                "\"fixture_sha256\": \"{:x}\"",
                Sha256::digest(&bytes)
            )),
            "the v46 fixture is not the file its provenance records"
        );
        let copy = scratch_path();
        std::fs::write(&copy, bytes).unwrap();
        (copy, provenance)
    }

    fn provenance_field<'a>(provenance: &'a str, field: &str) -> &'a str {
        let start = provenance.find(&format!("\"{field}\": ")).unwrap() + field.len() + 4;
        let rest = &provenance[start..];
        rest[..rest.find([',', '\n']).unwrap()].trim_matches('"')
    }

    #[test]
    fn the_frozen_v46_fixture_inspects_as_its_provenance_says() {
        let (path, provenance) = frozen_v46_copy();
        let inspection = match super::super::inspect_ledger(&path).unwrap() {
            LedgerCompatibility::UpgradeableFrom(inspection) => inspection,
            other => panic!("expected an upgradeable v46, got {other:?}"),
        };
        assert_eq!(inspection.schema_version, 46);
        assert_eq!(
            inspection.ledger_revision.to_string(),
            provenance_field(&provenance, "ledger_revision")
        );
        assert_eq!(
            inspection.inventory.sha256,
            provenance_field(&provenance, "authority_inventory_sha256")
        );
        assert_eq!(
            inspection.inventory.record_count.to_string(),
            provenance_field(&provenance, "authority_record_count")
        );
    }

    #[test]
    fn the_frozen_v46_fixture_upgrades_with_every_record() {
        let (path, provenance) = frozen_v46_copy();
        let source = source_for(&path, production_descriptors());
        assert_eq!(
            upgrade_in_place(&path, &source, &Proof(source.clone())),
            Ok(UpgradeOutcome::Upgraded { from: 46, to: 48 })
        );
        let LedgerCompatibility::Current(after) = super::super::inspect_ledger(&path).unwrap()
        else {
            panic!("not current after the upgrade");
        };
        assert_eq!(after.inventory.sha256, source.inventory_sha256);
        assert_eq!(after.ledger_revision, source.ledger_revision);
        assert_eq!(
            after.inventory.record_count.to_string(),
            provenance_field(&provenance, "authority_record_count")
        );
        assert_upgraded_to_current(&path);
    }

    #[test]
    fn a_failure_on_the_frozen_fixture_rolls_back_completely() {
        let (path, _) = frozen_v46_copy();
        let descriptors = production_descriptors();
        let before = logical_digest(&path);
        let source = source_for(&path, descriptors);
        assert_eq!(
            upgrade_with(
                &path,
                &source,
                &source,
                descriptors,
                Some(FailAt::BeforeCommit)
            ),
            Err(LedgerUpgradeError::RolledBack)
        );
        assert_eq!(logical_digest(&path), before);
    }

    #[test]
    fn a_write_that_moves_neither_revision_nor_inventory_still_changes_the_source() {
        let path = populated_v46();
        let descriptors = production_descriptors();
        let source = source_for(&path, descriptors);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "UPDATE aggregate_registry SET updated_at=updated_at+1 WHERE id=(SELECT id FROM aggregate_registry ORDER BY id LIMIT 1)",
                [],
            )
            .unwrap();
        drop(connection);
        let now = source_for(&path, descriptors);
        assert_eq!(now.ledger_revision, source.ledger_revision);
        assert_eq!(now.inventory_sha256, source.inventory_sha256);
        assert_ne!(now.content_sha256, source.content_sha256);
        assert_eq!(
            upgrade_with(&path, &source, &source, descriptors, None),
            Err(LedgerUpgradeError::SourceChanged)
        );
    }
}
