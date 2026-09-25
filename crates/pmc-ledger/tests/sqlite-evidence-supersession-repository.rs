use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    evidence::{
        ApproveAndExecuteSupersedeEvidenceReference, CreateEvidenceReference, EvidenceFingerprint,
        EvidenceLinkTarget, EvidenceSupersessionApproval, FingerprintAlgorithm, LinkEvidence,
        OperationContext as EvidenceOperationContext, PrepareSupersedeEvidenceReference,
        VaultRelativePath,
    },
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, PreparedIntentId, ProductId,
    },
    portfolio::{
        CreateProduct, LongText, OperationContext as PortfolioOperationContext, ShortText,
    },
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::{EvidenceVerification, IntegrityDigest, WorkManagementRationale},
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
            "pmc-synthetic-evidence-supersession-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

const PINNED_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REPLACEMENT_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn seed_source_evidence(writer: &mut SqliteProductLedger) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-source").unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse("Research/notes.md").unwrap(),
                fingerprint: Some(EvidenceFingerprint::new(
                    FingerprintAlgorithm::Sha256,
                    IntegrityDigest::parse(PINNED_DIGEST).unwrap(),
                )),
                verification: EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(1_000),
                    integrity_digest: IntegrityDigest::parse(PINNED_DIGEST).unwrap(),
                },
                classification: Some(DataClassification::Internal),
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
    id
}

fn seed_product(writer: &mut SqliteProductLedger) -> ProductId {
    let product_id = ProductId::parse("synthetic-product-1").unwrap();
    writer
        .create_product(
            CreateProduct {
                id: product_id.clone(),
                name: ShortText::parse("Synthetic Product").unwrap(),
                details: LongText::parse("Synthetic only; no organizational data.").unwrap(),
                classification: Some(DataClassification::Internal),
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
    product_id
}

fn link_product(
    writer: &mut SqliteProductLedger,
    evidence_id: EvidenceReferenceId,
    product_id: ProductId,
) {
    writer
        .link_evidence(
            LinkEvidence {
                evidence_id,
                expected_evidence_version: AggregateVersion::initial(),
                target: EvidenceLinkTarget::Product(product_id),
                context: EvidenceOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-link-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-link-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
}

fn prepare_command(
    source_id: EvidenceReferenceId,
    expected_source_version: u64,
    idempotency_id: &str,
) -> PrepareSupersedeEvidenceReference {
    PrepareSupersedeEvidenceReference {
        source_id,
        expected_source_version: AggregateVersion::new(expected_source_version).unwrap(),
        replacement_id: EvidenceReferenceId::parse("synthetic-evidence-replacement").unwrap(),
        replacement_vault_path: VaultRelativePath::parse("Research/notes-v2.md").unwrap(),
        replacement_fingerprint: EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            IntegrityDigest::parse(REPLACEMENT_DIGEST).unwrap(),
        ),
        replacement_observed_at: UtcTimestamp::from_unix_millis(5_000),
        replacement_classification: DataClassification::Internal,
        replacement_provenance: Provenance::UserEntered,
        rationale: WorkManagementRationale::parse("File was re-exported after a content rewrite")
            .unwrap(),
        context: EvidenceOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
            correlation_id: CorrelationId::parse("synthetic-supersede-correlation-1").unwrap(),
        },
    }
}

#[test]
fn prepare_supersede_evidence_reference_binds_the_full_live_link_set() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);
    let product_id = seed_product(&mut writer);
    link_product(&mut writer, source_id.clone(), product_id);

    let prepared = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id.clone(), 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    assert_eq!(prepared.preview().source_id, source_id);
    assert_eq!(prepared.preview().links.len(), 1);
    assert_eq!(
        prepared.preview().replacement_id.as_str(),
        "synthetic-evidence-replacement"
    );
}

#[test]
fn approve_and_execute_creates_the_replacement_clones_links_and_supersedes_the_source() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);
    let product_id = seed_product(&mut writer);
    link_product(&mut writer, source_id.clone(), product_id);

    let prepared = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id.clone(), 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-execute-1").unwrap(),
        true,
    )
    .unwrap();

    let outcome = writer
        .approve_and_execute_supersede_evidence_reference(
            ApproveAndExecuteSupersedeEvidenceReference {
                approval,
                context: EvidenceOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-execute-correlation-1")
                        .unwrap(),
                },
            },
            ApprovalReceiptId::parse("synthetic-receipt-1").unwrap(),
            AuditEventId::parse("synthetic-execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(6_000),
        )
        .unwrap();

    assert_eq!(outcome.record.id.as_str(), "synthetic-evidence-replacement");
    assert_eq!(outcome.record.version.get(), 1);
    assert_eq!(outcome.record.vault_path.as_str(), "Research/notes-v2.md");
    assert_eq!(
        outcome
            .record
            .fingerprint
            .clone()
            .unwrap()
            .digest()
            .as_str(),
        REPLACEMENT_DIGEST
    );

    let replacement_id = EvidenceReferenceId::parse("synthetic-evidence-replacement").unwrap();
    let replacement_current = writer
        .get_evidence_reference(&replacement_id, CorrelationId::parse("read-1").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(replacement_current, outcome.record);

    let source_current = writer
        .get_evidence_reference(&source_id, CorrelationId::parse("read-2").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(source_current.version.get(), 2);
    // The source's own immutable fields must survive supersession unchanged.
    assert_eq!(source_current.vault_path.as_str(), "Research/notes.md");
    assert_eq!(
        source_current.fingerprint.unwrap().digest().as_str(),
        PINNED_DIGEST
    );
}

#[test]
fn a_superseded_source_rejects_a_new_ordinary_link() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);
    let product_id = seed_product(&mut writer);

    let prepared = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id.clone(), 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-execute-1").unwrap(),
        true,
    )
    .unwrap();
    writer
        .approve_and_execute_supersede_evidence_reference(
            ApproveAndExecuteSupersedeEvidenceReference {
                approval,
                context: EvidenceOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-execute-correlation-1")
                        .unwrap(),
                },
            },
            ApprovalReceiptId::parse("synthetic-receipt-1").unwrap(),
            AuditEventId::parse("synthetic-execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(6_000),
        )
        .unwrap();

    let result = writer.link_evidence(
        LinkEvidence {
            evidence_id: source_id,
            expected_evidence_version: AggregateVersion::new(2).unwrap(),
            target: EvidenceLinkTarget::Product(product_id),
            context: EvidenceOperationContext {
                idempotency_id: IdempotencyId::parse("synthetic-link-after-supersede").unwrap(),
                correlation_id: CorrelationId::parse("synthetic-link-correlation-2").unwrap(),
            },
        },
        AuditEventId::parse("synthetic-link-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(7_000),
    );
    assert!(result.is_err());
}

#[test]
fn execute_rejects_a_link_set_changed_since_prepare() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);
    let product_id = seed_product(&mut writer);

    let prepared = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id.clone(), 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    // A link is added after prepare but before execute.
    link_product(&mut writer, source_id, product_id);

    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-execute-1").unwrap(),
        true,
    )
    .unwrap();
    let result = writer.approve_and_execute_supersede_evidence_reference(
        ApproveAndExecuteSupersedeEvidenceReference {
            approval,
            context: EvidenceOperationContext {
                idempotency_id: IdempotencyId::parse("synthetic-execute-1").unwrap(),
                correlation_id: CorrelationId::parse("synthetic-execute-correlation-1").unwrap(),
            },
        },
        ApprovalReceiptId::parse("synthetic-receipt-1").unwrap(),
        AuditEventId::parse("synthetic-execute-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(6_000),
    );
    assert!(result.is_err());
}

#[test]
fn prepare_rejects_superseding_an_already_superseded_source() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);

    let prepared = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id.clone(), 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-execute-1").unwrap(),
        true,
    )
    .unwrap();
    writer
        .approve_and_execute_supersede_evidence_reference(
            ApproveAndExecuteSupersedeEvidenceReference {
                approval,
                context: EvidenceOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-execute-correlation-1")
                        .unwrap(),
                },
            },
            ApprovalReceiptId::parse("synthetic-receipt-1").unwrap(),
            AuditEventId::parse("synthetic-execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(6_000),
        )
        .unwrap();

    let result = writer.prepare_supersede_evidence_reference(
        prepare_command(source_id, 2, "synthetic-prepare-2"),
        PreparedIntentId::parse("synthetic-prepared-2").unwrap(),
        UtcTimestamp::from_unix_millis(7_000),
    );
    assert!(result.is_err());
}

#[test]
fn replaying_the_same_prepare_idempotency_id_returns_the_original_prepared_intent() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);

    let first = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id.clone(), 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    let second = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id, 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-should-be-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap();

    assert_eq!(first.id(), second.id());
    assert_eq!(
        first.payload_digest().as_str(),
        second.payload_digest().as_str()
    );
}

#[test]
fn replaying_the_same_execute_idempotency_id_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let source_id = seed_source_evidence(&mut writer);

    let prepared = writer
        .prepare_supersede_evidence_reference(
            prepare_command(source_id, 1, "synthetic-prepare-1"),
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-execute-1").unwrap(),
        true,
    )
    .unwrap();
    let context = EvidenceOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-execute-1").unwrap(),
        correlation_id: CorrelationId::parse("synthetic-execute-correlation-1").unwrap(),
    };

    let first = writer
        .approve_and_execute_supersede_evidence_reference(
            ApproveAndExecuteSupersedeEvidenceReference {
                approval: approval.clone(),
                context: context.clone(),
            },
            ApprovalReceiptId::parse("synthetic-receipt-1").unwrap(),
            AuditEventId::parse("synthetic-execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(6_000),
        )
        .unwrap();
    let second = writer
        .approve_and_execute_supersede_evidence_reference(
            ApproveAndExecuteSupersedeEvidenceReference { approval, context },
            ApprovalReceiptId::parse("synthetic-receipt-should-be-ignored").unwrap(),
            AuditEventId::parse("synthetic-execute-audit-should-be-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(99_000),
        )
        .unwrap();

    assert_eq!(first.record, second.record);
    assert_eq!(first.audit_event.id(), second.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 3);
}
