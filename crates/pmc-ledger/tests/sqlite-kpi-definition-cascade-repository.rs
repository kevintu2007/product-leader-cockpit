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
        UpdateKpiDefinitionDetails,
    },
    provenance::Provenance,
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
            "pmc-synthetic-kpi-definition-cascade-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn next_audit_id() -> AuditEventId {
    let sequence = NEXT_AUDIT.fetch_add(1, Ordering::Relaxed);
    AuditEventId::parse(format!("synthetic-derived-audit-{sequence}")).unwrap()
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

fn observation_command(
    id: &str,
    idempotency_suffix: &str,
    classification: Option<DataClassification>,
) -> CreateKpiObservation {
    CreateKpiObservation {
        id: KpiObservationId::parse(id).unwrap(),
        kpi_id: KpiId::parse("synthetic-kpi-1").unwrap(),
        value: ShortText::parse("42").unwrap(),
        observed_at: UtcTimestamp::from_unix_millis(500),
        source: LongText::parse("Synthetic observation source.").unwrap(),
        classification,
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(format!(
                "synthetic-observation-create-{idempotency_suffix}"
            ))
            .unwrap(),
            correlation_id: CorrelationId::parse(format!(
                "synthetic-observation-correlation-{idempotency_suffix}"
            ))
            .unwrap(),
        },
    }
}

fn current_classification(ledger: &std::path::Path, id: &str) -> String {
    let connection = rusqlite::Connection::open(ledger).unwrap();
    connection
        .query_row(
            "SELECT classification FROM aggregate_registry WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

fn current_version(ledger: &std::path::Path, id: &str) -> i64 {
    let connection = rusqlite::Connection::open(ledger).unwrap();
    connection
        .query_row(
            "SELECT version FROM aggregate_registry WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn writer_updates_a_persisted_kpi_definition_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let update = UpdateKpiDefinitionDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        name: ShortText::parse("Renamed KPI").unwrap(),
        definition: LongText::parse("Renamed definition.").unwrap(),
        owner: ShortText::parse("New Owner").unwrap(),
        target: ShortText::parse("200 widgets/week").unwrap(),
        cadence: ShortText::parse("monthly").unwrap(),
        source: LongText::parse("New source system.").unwrap(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-2").unwrap(),
        },
    };
    let updated = writer
        .update_kpi_definition_details(update, next_audit_id, UtcTimestamp::from_unix_millis(2_000))
        .unwrap();
    assert_eq!(updated.record.name.as_str(), "Renamed KPI");
    assert_eq!(updated.record.owner.as_str(), "New Owner");
    assert_eq!(updated.record.classification, DataClassification::Internal);
    assert_eq!(updated.record.version.get(), 2);
    assert_eq!(writer.revision().unwrap(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 2);
}

#[test]
fn update_kpi_definition_details_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateKpiDefinitionDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        name: created.record.name.clone(),
        definition: created.record.definition.clone(),
        owner: created.record.owner.clone(),
        target: created.record.target.clone(),
        cadence: created.record.cadence.clone(),
        source: created.record.source.clone(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-2").unwrap(),
        },
    };
    writer
        .update_kpi_definition_details(
            update.clone(),
            next_audit_id,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let mut stale = update;
    stale.context.idempotency_id = IdempotencyId::parse("synthetic-kpi-update-2").unwrap();
    let result = writer.update_kpi_definition_details(
        stale,
        next_audit_id,
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_kpi_definition_details_rejects_lowering_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Confidential)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let lowering = UpdateKpiDefinitionDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        name: created.record.name.clone(),
        definition: created.record.definition.clone(),
        owner: created.record.owner.clone(),
        target: created.record.target.clone(),
        cadence: created.record.cadence.clone(),
        source: created.record.source.clone(),
        classification: Some(DataClassification::Public),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-update-lower-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-2").unwrap(),
        },
    };
    let result = writer.update_kpi_definition_details(
        lowering,
        next_audit_id,
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_kpi_definition_details_cascades_a_raised_classification_only_to_observations_it_actually_raises(
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_kpi_definition(
            definition_command(Some(DataClassification::Internal)),
            AuditEventId::parse("synthetic-kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    // Below the Definition's new classification: must be raised.
    writer
        .create_kpi_observation(
            observation_command(
                "synthetic-observation-low",
                "low",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-observation-audit-low").unwrap(),
            UtcTimestamp::from_unix_millis(1_500),
        )
        .unwrap();
    // Already above the Definition's new classification: must be left alone.
    writer
        .create_kpi_observation(
            observation_command(
                "synthetic-observation-high",
                "high",
                Some(DataClassification::Restricted),
            ),
            AuditEventId::parse("synthetic-observation-audit-high").unwrap(),
            UtcTimestamp::from_unix_millis(1_600),
        )
        .unwrap();

    let update = UpdateKpiDefinitionDetails {
        id: KpiId::parse("synthetic-kpi-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
        name: ShortText::parse("Synthetic KPI").unwrap(),
        definition: LongText::parse("Synthetic only; no organizational data.").unwrap(),
        owner: ShortText::parse("Synthetic Owner").unwrap(),
        target: ShortText::parse("100 widgets/week").unwrap(),
        cadence: ShortText::parse("weekly").unwrap(),
        source: LongText::parse("Synthetic source system.").unwrap(),
        classification: Some(DataClassification::Confidential),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-update-raise-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-raise-1").unwrap(),
        },
    };
    let updated = writer
        .update_kpi_definition_details(update, next_audit_id, UtcTimestamp::from_unix_millis(2_000))
        .unwrap();
    assert_eq!(
        updated.record.classification,
        DataClassification::Confidential
    );

    assert_eq!(
        current_classification(&ledger.0, "synthetic-observation-low"),
        "confidential"
    );
    assert_eq!(current_version(&ledger.0, "synthetic-observation-low"), 2);

    assert_eq!(
        current_classification(&ledger.0, "synthetic-observation-high"),
        "restricted"
    );
    assert_eq!(current_version(&ledger.0, "synthetic-observation-high"), 1);
}

#[test]
fn update_kpi_definition_details_replay_does_not_repeat_the_cascade_or_allocate_audit_ids() {
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
            observation_command(
                "synthetic-observation-low",
                "low",
                Some(DataClassification::Internal),
            ),
            AuditEventId::parse("synthetic-observation-audit-low").unwrap(),
            UtcTimestamp::from_unix_millis(1_500),
        )
        .unwrap();

    let update = UpdateKpiDefinitionDetails {
        id: KpiId::parse("synthetic-kpi-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
        name: ShortText::parse("Synthetic KPI").unwrap(),
        definition: LongText::parse("Synthetic only; no organizational data.").unwrap(),
        owner: ShortText::parse("Synthetic Owner").unwrap(),
        target: ShortText::parse("100 widgets/week").unwrap(),
        cadence: ShortText::parse("weekly").unwrap(),
        source: LongText::parse("Synthetic source system.").unwrap(),
        classification: Some(DataClassification::Confidential),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-kpi-update-raise-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-kpi-correlation-raise-1").unwrap(),
        },
    };
    let first = writer
        .update_kpi_definition_details(
            update.clone(),
            next_audit_id,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(current_version(&ledger.0, "synthetic-observation-low"), 2);

    let replayed = writer
        .update_kpi_definition_details(
            update,
            || panic!("replay must not allocate new audit ids"),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(replayed.record, first.record);
    assert_eq!(replayed.audit_event.id(), first.audit_event.id());
    // The cascade must not run a second time: still version 2, not 3.
    assert_eq!(current_version(&ledger.0, "synthetic-observation-low"), 2);
}
