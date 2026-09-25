use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    evidence::{
        CreateEvidenceReference, EvidenceLinkTarget, LinkEvidence,
        OperationContext as EvidenceOperationContext,
    },
    identity::{AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId, ProductId},
    portfolio::{
        CreateProduct, LongText, OperationContext as PortfolioOperationContext, ShortText,
    },
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::EvidenceVerification,
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
            "pmc-synthetic-evidence-link-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn seed_evidence_and_product(
    writer: &mut SqliteProductLedger,
    evidence_classification: DataClassification,
    product_classification: DataClassification,
) -> (EvidenceReferenceId, ProductId) {
    let evidence_id = EvidenceReferenceId::parse("synthetic-evidence-1").unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: evidence_id.clone(),
                vault_path: pmc_domain::evidence::VaultRelativePath::parse(
                    "Research/Competitive/notes.md",
                )
                .unwrap(),
                fingerprint: None,
                verification: EvidenceVerification::Unverified,
                classification: Some(evidence_classification),
                provenance: Provenance::UserEntered,
                context: EvidenceOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-evidence-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-evidence-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let product_id = ProductId::parse("synthetic-product-1").unwrap();
    writer
        .create_product(
            CreateProduct {
                id: product_id.clone(),
                name: ShortText::parse("Synthetic Product").unwrap(),
                details: LongText::parse("Synthetic only; no organizational data.").unwrap(),
                classification: Some(product_classification),
                provenance: Provenance::UserEntered,
                context: PortfolioOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-product-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    (evidence_id, product_id)
}

fn link_command(
    evidence_id: EvidenceReferenceId,
    product_id: ProductId,
    idempotency_id: &str,
) -> LinkEvidence {
    LinkEvidence {
        evidence_id,
        expected_evidence_version: pmc_domain::identity::AggregateVersion::initial(),
        target: EvidenceLinkTarget::Product(product_id),
        context: EvidenceOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
            correlation_id: CorrelationId::parse("synthetic-link-correlation-1").unwrap(),
        },
    }
}

#[test]
fn link_evidence_persists_a_link_with_the_most_restrictive_combined_classification() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (evidence_id, product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Confidential,
    );

    let linked = writer
        .link_evidence(
            link_command(evidence_id.clone(), product_id.clone(), "synthetic-link-1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    assert_eq!(linked.record.evidence_id, evidence_id);
    assert!(matches!(
        linked.record.target,
        EvidenceLinkTarget::Product(ref id) if *id == product_id
    ));
    assert_eq!(
        linked.record.classification,
        DataClassification::Internal.combine(DataClassification::Confidential)
    );
}

#[test]
fn link_evidence_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (evidence_id, product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Internal,
    );
    writer
        .link_evidence(
            link_command(evidence_id, product_id, "synthetic-link-1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let revision_after_link = writer.revision().unwrap();
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), revision_after_link);
}

#[test]
fn replaying_the_same_idempotency_id_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (evidence_id, product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Internal,
    );
    let first = writer
        .link_evidence(
            link_command(evidence_id.clone(), product_id.clone(), "synthetic-link-1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let second = writer
        .link_evidence(
            link_command(evidence_id, product_id, "synthetic-link-1"),
            AuditEventId::parse("synthetic-link-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(first.record, second.record);
    assert_eq!(first.audit_event.id(), second.audit_event.id());
}

#[test]
fn link_evidence_rejects_a_mismatched_expected_evidence_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (evidence_id, product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Internal,
    );
    let mut command = link_command(evidence_id, product_id, "synthetic-link-1");
    command.expected_evidence_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let result = writer.link_evidence(
        command,
        AuditEventId::parse("synthetic-link-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn link_evidence_rejects_an_unknown_evidence_id() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (_evidence_id, product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Internal,
    );
    let unknown_evidence_id = EvidenceReferenceId::parse("synthetic-evidence-unknown").unwrap();
    let result = writer.link_evidence(
        link_command(unknown_evidence_id, product_id, "synthetic-link-1"),
        AuditEventId::parse("synthetic-link-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn link_evidence_rejects_an_unknown_target() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (evidence_id, _product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Internal,
    );
    let unknown_product_id = ProductId::parse("synthetic-product-unknown").unwrap();
    let result = writer.link_evidence(
        link_command(evidence_id, unknown_product_id, "synthetic-link-1"),
        AuditEventId::parse("synthetic-link-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn link_evidence_rejects_relinking_the_same_pair_under_a_different_idempotency_id() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let (evidence_id, product_id) = seed_evidence_and_product(
        &mut writer,
        DataClassification::Internal,
        DataClassification::Internal,
    );
    writer
        .link_evidence(
            link_command(evidence_id.clone(), product_id.clone(), "synthetic-link-1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let result = writer.link_evidence(
        link_command(evidence_id, product_id, "synthetic-link-2"),
        AuditEventId::parse("synthetic-link-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}
