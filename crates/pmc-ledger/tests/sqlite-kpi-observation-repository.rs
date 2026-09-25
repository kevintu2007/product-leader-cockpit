use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{AuditEventId, CorrelationId, IdempotencyId, KpiId, KpiObservationId},
    portfolio::{
        CreateKpiDefinition, CreateKpiObservation, LongText, OperationContext, ShortText,
        UpdateKpiObservationDetails,
    },
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
            "pmc-synthetic-kpi-observation-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn definition_command(classification: Option<DataClassification>) -> CreateKpiDefinition {
    CreateKpiDefinition {
        id: KpiId::parse("synthetic-kpi-1").unwrap(),
        name: ShortText::parse("Synthetic KPI").unwrap(),
        definition: LongText::parse("Synthetic only; no organizational data.").unwrap(),
        owner: ShortText::parse("Synthetic Owner").unwrap(),
        target: ShortText::parse("100 widgets/week").unwrap(),
        cadence: ShortText::parse("weekly").unwrap(),
        source: LongText::parse("Synthetic source system.").unwrap(),
        classification,
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-1").unwrap(),
        },
    }
}

fn observation_command(classification: Option<DataClassification>) -> CreateKpiObservation {
    CreateKpiObservation {
        id: KpiObservationId::parse("synthetic-observation-1").unwrap(),
        kpi_id: KpiId::parse("synthetic-kpi-1").unwrap(),
        value: ShortText::parse("42").unwrap(),
        observed_at: UtcTimestamp::from_unix_millis(500),
        source: LongText::parse("Synthetic observation source.").unwrap(),
        classification,
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-observation-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-observation-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_persists_a_created_observation_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(created.record.value.as_str(), "42");
    assert_eq!(created.record.classification, DataClassification::Internal);
    assert_eq!(created.record.version.get(), 1);
    assert_eq!(writer.revision().unwrap(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 2);
}

#[test]
fn create_kpi_observation_with_no_explicit_classification_inherits_the_definitions_current_classification(
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Confidential)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_kpi_observation(
            observation_command(None),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        created.record.classification,
        DataClassification::Confidential
    );
}

#[test]
fn create_kpi_observation_rejects_creating_under_a_missing_definition() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let result = writer.create_kpi_observation(
        observation_command(None),
        AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn create_kpi_observation_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let first = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let replayed = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(replayed.record, first.record);
    assert_eq!(replayed.audit_event.id(), first.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 2);
}

#[test]
fn create_kpi_observation_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let mut drifted = observation_command(Some(DataClassification::Internal));
    drifted.value = ShortText::parse("A different value").unwrap();
    let result = writer.create_kpi_observation(
        drifted,
        AuditEventId::parse("synthetic-observation-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_updates_a_persisted_observation_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let update = UpdateKpiObservationDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        value: ShortText::parse("43").unwrap(),
        observed_at: UtcTimestamp::from_unix_millis(600),
        source: LongText::parse("Updated synthetic source.").unwrap(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-observation-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-observation-correlation-2").unwrap(),
        },
    };
    let updated = writer
        .update_kpi_observation_details(
            update,
            AuditEventId::parse("synthetic-observation-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(updated.record.value.as_str(), "43");
    assert_eq!(updated.record.classification, DataClassification::Internal);
    assert_eq!(updated.record.version.get(), 2);
    assert_eq!(writer.revision().unwrap(), 3);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 3);
}

#[test]
fn update_kpi_observation_details_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let update = UpdateKpiObservationDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        value: ShortText::parse("43").unwrap(),
        observed_at: UtcTimestamp::from_unix_millis(600),
        source: LongText::parse("Updated synthetic source.").unwrap(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-observation-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-observation-correlation-2").unwrap(),
        },
    };
    writer
        .update_kpi_observation_details(
            update.clone(),
            AuditEventId::parse("synthetic-observation-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();

    let mut stale = update;
    stale.value = ShortText::parse("44").unwrap();
    stale.context.idempotency_id = IdempotencyId::parse("synthetic-observation-update-2").unwrap();
    let result = writer.update_kpi_observation_details(
        stale,
        AuditEventId::parse("synthetic-observation-audit-4").unwrap(),
        UtcTimestamp::from_unix_millis(4_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_kpi_observation_details_rejects_lowering_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Confidential)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Confidential)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let lowering = UpdateKpiObservationDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        value: created.record.value.clone(),
        observed_at: created.record.observed_at,
        source: created.record.source.clone(),
        classification: Some(DataClassification::Public),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-observation-update-lower-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-observation-correlation-3").unwrap(),
        },
    };
    let result = writer.update_kpi_observation_details(
        lowering,
        AuditEventId::parse("synthetic-observation-audit-5").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_kpi_observation_details_re_inherits_a_definition_classification_raised_after_creation() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let created = writer
        .create_kpi_observation(
            observation_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-observation-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(created.record.classification, DataClassification::Internal);

    // Raise the Definition's classification directly at the storage layer
    // (bypassing update_kpi_definition_details, which is not yet implemented)
    // to isolate this test to update_kpi_observation_details's own
    // re-combine-on-every-update behavior.
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE aggregate_registry SET classification='confidential' WHERE id='synthetic-kpi-1'",
            [],
        )
        .unwrap();
    drop(connection);

    let update = UpdateKpiObservationDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        value: created.record.value.clone(),
        observed_at: created.record.observed_at,
        source: created.record.source.clone(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-observation-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-observation-correlation-2").unwrap(),
        },
    };
    let updated = writer
        .update_kpi_observation_details(
            update,
            AuditEventId::parse("synthetic-observation-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(
        updated.record.classification,
        DataClassification::Confidential
    );
}
