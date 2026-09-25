use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{AuditEventId, CorrelationId, IdempotencyId, KpiId},
    portfolio::{CreateKpiDefinition, LongText, OperationContext, ShortText},
    provenance::Provenance,
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-kpi-definition-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn create_command() -> CreateKpiDefinition {
    CreateKpiDefinition {
        id: KpiId::parse("synthetic-kpi-1").unwrap(),
        name: ShortText::parse("Synthetic KPI").unwrap(),
        definition: LongText::parse("Synthetic only; no organizational data.").unwrap(),
        owner: ShortText::parse("Synthetic Owner").unwrap(),
        target: ShortText::parse("100 widgets/week").unwrap(),
        cadence: ShortText::parse("weekly").unwrap(),
        source: LongText::parse("Synthetic source system.").unwrap(),
        classification: Some(DataClassification::Internal),
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_persists_a_created_kpi_definition_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_kpi_definition(
            create_command(),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(created.record.name.as_str(), "Synthetic KPI");
    assert_eq!(created.record.owner.as_str(), "Synthetic Owner");
    assert_eq!(created.record.classification, DataClassification::Internal);
    assert_eq!(created.record.version.get(), 1);
    assert_eq!(writer.revision().unwrap(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 1);
}

#[test]
fn create_kpi_definition_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = writer
        .create_kpi_definition(
            create_command(),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let replayed = writer
        .create_kpi_definition(
            create_command(),
            AuditEventId::parse("synthetic-kpi-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(replayed.record, first.record);
    assert_eq!(replayed.audit_event.id(), first.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 1);
}

#[test]
fn create_kpi_definition_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            create_command(),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let mut drifted = create_command();
    drifted.owner = ShortText::parse("A different owner").unwrap();
    let result = writer.create_kpi_definition(
        drifted,
        AuditEventId::parse("synthetic-kpi-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn create_kpi_definition_with_no_explicit_classification_defaults_to_unclassified() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut command = create_command();
    command.classification = None;
    let created = writer
        .create_kpi_definition(
            command,
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(
        created.record.classification,
        DataClassification::Unclassified
    );
}
