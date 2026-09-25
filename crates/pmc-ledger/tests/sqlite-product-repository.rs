use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{AuditEventId, CorrelationId, IdempotencyId, ProductId},
    portfolio::{CreateProduct, LongText, OperationContext, ShortText, UpdateProductDetails},
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
            "pmc-synthetic-product-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn create_command() -> CreateProduct {
    CreateProduct {
        id: ProductId::parse("synthetic-product-1").unwrap(),
        name: ShortText::parse("Synthetic Product").unwrap(),
        details: LongText::parse("Synthetic only; no organizational data.").unwrap(),
        classification: Some(DataClassification::Internal),
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-product-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-product-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_persists_a_created_product_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_product(
            create_command(),
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(created.record.name.as_str(), "Synthetic Product");
    assert_eq!(created.record.classification, DataClassification::Internal);
    assert_eq!(created.record.version.get(), 1);
    assert_eq!(writer.revision().unwrap(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 1);
}

#[test]
fn create_product_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = writer
        .create_product(
            create_command(),
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let replayed = writer
        .create_product(
            create_command(),
            AuditEventId::parse("synthetic-product-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(replayed.record, first.record);
    assert_eq!(replayed.audit_event.id(), first.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 1);
}

#[test]
fn create_product_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_product(
            create_command(),
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let mut drifted = create_command();
    drifted.details =
        LongText::parse("A different payload under the same idempotency id.").unwrap();
    let result = writer.create_product(
        drifted,
        AuditEventId::parse("synthetic-product-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_updates_a_persisted_product_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_product(
            create_command(),
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let update = UpdateProductDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        name: ShortText::parse("Renamed Synthetic Product").unwrap(),
        details: LongText::parse("Updated synthetic details.").unwrap(),
        classification: Some(DataClassification::Confidential),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-product-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-product-correlation-2").unwrap(),
        },
    };
    let updated = writer
        .update_product_details(
            update,
            AuditEventId::parse("synthetic-product-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(updated.record.name.as_str(), "Renamed Synthetic Product");
    assert_eq!(
        updated.record.classification,
        DataClassification::Confidential
    );
    assert_eq!(updated.record.version.get(), 2);
    assert_eq!(writer.revision().unwrap(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 2);
}

#[test]
fn update_product_details_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_product(
            create_command(),
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let update = UpdateProductDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        name: ShortText::parse("First update").unwrap(),
        details: LongText::parse("First update details.").unwrap(),
        classification: None,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-product-update-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-product-correlation-2").unwrap(),
        },
    };
    writer
        .update_product_details(
            update.clone(),
            AuditEventId::parse("synthetic-product-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();

    let mut stale = update;
    stale.name = ShortText::parse("Second update using stale version").unwrap();
    stale.context.idempotency_id = IdempotencyId::parse("synthetic-product-update-2").unwrap();
    let result = writer.update_product_details(
        stale,
        AuditEventId::parse("synthetic-product-audit-4").unwrap(),
        UtcTimestamp::from_unix_millis(4_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_product_details_rejects_lowering_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut initial = create_command();
    initial.classification = Some(DataClassification::Confidential);
    let created = writer
        .create_product(
            initial,
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let lowering = UpdateProductDetails {
        id: created.record.id.clone(),
        expected_version: created.record.version,
        name: created.record.name.clone(),
        details: created.record.details.clone(),
        classification: Some(DataClassification::Public),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-product-update-lower-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-product-correlation-3").unwrap(),
        },
    };
    let result = writer.update_product_details(
        lowering,
        AuditEventId::parse("synthetic-product-audit-5").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
    assert_eq!(writer.revision().unwrap(), 1);
}
