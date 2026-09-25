use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{AuditEventId, CorrelationId, IdempotencyId, StakeholderId},
    provenance::Provenance,
    relationships::{
        CreateStakeholder, OperationContext, StakeholderKind, StakeholderName,
        UpdateStakeholderDetails,
    },
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);
static NEXT_AUDIT: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-stakeholder-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn next_audit_id() -> AuditEventId {
    let sequence = NEXT_AUDIT.fetch_add(1, Ordering::Relaxed);
    AuditEventId::parse(format!("synthetic-derived-audit-{sequence}")).unwrap()
}

fn create_command(id: &str, classification: Option<DataClassification>) -> CreateStakeholder {
    CreateStakeholder {
        id: StakeholderId::parse(id).unwrap(),
        name: StakeholderName::parse("Synthetic Stakeholder").unwrap(),
        kind: StakeholderKind::Person,
        classification,
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(format!("synthetic-{id}-create-1")).unwrap(),
            correlation_id: CorrelationId::parse(format!("synthetic-{id}-correlation-1")).unwrap(),
        },
    }
}

#[test]
fn writer_persists_a_created_stakeholder_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(created.value().name().as_str(), "Synthetic Stakeholder");
    assert_eq!(
        created.value().classification(),
        DataClassification::Internal
    );
    assert_eq!(created.value().version().get(), 1);
    assert_eq!(writer.revision().unwrap(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 1);
}

#[test]
fn create_stakeholder_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let replayed = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(replayed.value(), first.value());
    assert_eq!(
        replayed.outcome().audit_event_ids(),
        first.outcome().audit_event_ids()
    );
    assert_eq!(writer.revision().unwrap(), 1);
}

#[test]
fn create_stakeholder_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let mut drifted = create_command(
        "synthetic-stakeholder-1",
        Some(DataClassification::Internal),
    );
    drifted.name = StakeholderName::parse("A different name").unwrap();
    let result = writer.create_stakeholder(
        drifted,
        AuditEventId::parse("synthetic-stakeholder-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_updates_a_persisted_stakeholder_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let update = UpdateStakeholderDetails {
        id: created.value().id().clone(),
        expected_version: created.value().version(),
        name: StakeholderName::parse("Renamed Stakeholder").unwrap(),
        classification: Some(DataClassification::Confidential),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-stakeholder-1-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-stakeholder-1-correlation-2").unwrap(),
        },
    };
    let updated = writer
        .update_stakeholder(update, next_audit_id, UtcTimestamp::from_unix_millis(2_000))
        .unwrap();
    assert_eq!(updated.value().name().as_str(), "Renamed Stakeholder");
    assert_eq!(
        updated.value().classification(),
        DataClassification::Confidential
    );
    assert_eq!(updated.value().version().get(), 2);
    assert_eq!(writer.revision().unwrap(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 2);
}

#[test]
fn update_stakeholder_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateStakeholderDetails {
        id: created.value().id().clone(),
        expected_version: created.value().version(),
        name: StakeholderName::parse("First update").unwrap(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-stakeholder-1-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-stakeholder-1-correlation-2").unwrap(),
        },
    };
    writer
        .update_stakeholder(
            update.clone(),
            next_audit_id,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let mut stale = update;
    stale.name = StakeholderName::parse("Second update using stale version").unwrap();
    stale.context.idempotency_id =
        IdempotencyId::parse("synthetic-stakeholder-1-update-2").unwrap();
    let result =
        writer.update_stakeholder(stale, next_audit_id, UtcTimestamp::from_unix_millis(3_000));
    assert!(result.is_err());
}

#[test]
fn update_stakeholder_rejects_lowering_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Confidential),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let lowering = UpdateStakeholderDetails {
        id: created.value().id().clone(),
        expected_version: created.value().version(),
        name: created.value().name().clone(),
        classification: Some(DataClassification::Public),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-stakeholder-1-update-lower-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-stakeholder-1-correlation-3").unwrap(),
        },
    };
    let result = writer.update_stakeholder(
        lowering,
        next_audit_id,
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_stakeholder_cascades_a_raised_classification_to_every_linked_relationship() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let subject = writer
        .create_stakeholder(
            create_command(
                "synthetic-stakeholder-1",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let other = writer
        .create_stakeholder(
            create_command("synthetic-stakeholder-2", Some(DataClassification::Public)),
            AuditEventId::parse("synthetic-stakeholder-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();

    // Synthetic relationship linking both Stakeholders, staged directly at
    // the storage layer since Link commands are not yet implemented -- this
    // isolates the test to update_stakeholder's own cascade behavior.
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "INSERT INTO aggregate_registry VALUES('synthetic-relationship-1','relationship',1,'internal',900,900);
             INSERT INTO relationships VALUES('synthetic-relationship-1','stakeholder_subject','responsibility');
             INSERT INTO relationship_endpoints VALUES('synthetic-relationship-1',0,'stakeholder','synthetic-stakeholder-1',1,'internal',NULL);
             INSERT INTO relationship_endpoints VALUES('synthetic-relationship-1',1,'stakeholder','synthetic-stakeholder-2',1,'public',NULL);",
        )
        .unwrap();
    drop(connection);

    let update = UpdateStakeholderDetails {
        id: subject.value().id().clone(),
        expected_version: subject.value().version(),
        name: subject.value().name().clone(),
        classification: Some(DataClassification::Confidential),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-stakeholder-1-update-raise-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-stakeholder-1-correlation-raise-1")
                .unwrap(),
        },
    };
    let updated = writer
        .update_stakeholder(update, next_audit_id, UtcTimestamp::from_unix_millis(2_000))
        .unwrap();
    assert_eq!(
        updated.value().classification(),
        DataClassification::Confidential
    );
    assert_eq!(updated.outcome().audit_event_ids().len(), 2);
    drop(other);

    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    let (relationship_classification, relationship_version): (String, i64) = connection
        .query_row(
            "SELECT classification,version FROM aggregate_registry WHERE id='synthetic-relationship-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(relationship_classification, "confidential");
    assert_eq!(relationship_version, 2);

    let endpoint_classification: String = connection
        .query_row(
            "SELECT target_classification FROM relationship_endpoints WHERE relationship_id='synthetic-relationship-1' AND target_id='synthetic-stakeholder-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(endpoint_classification, "confidential");
}
