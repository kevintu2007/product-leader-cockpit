use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    evidence::{
        CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm, OperationContext,
        VaultRelativePath,
    },
    identity::{AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId},
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::{EvidenceVerification, IntegrityDigest},
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
            "pmc-synthetic-evidence-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn create_command() -> CreateEvidenceReference {
    CreateEvidenceReference {
        id: EvidenceReferenceId::parse("synthetic-evidence-1").unwrap(),
        vault_path: VaultRelativePath::parse("Research/Competitive/notes.md").unwrap(),
        fingerprint: Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            IntegrityDigest::parse("a".repeat(64)).unwrap(),
        )),
        verification: EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(500),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
        classification: Some(DataClassification::Internal),
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-evidence-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-evidence-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_persists_a_created_evidence_reference_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(writer.revision().unwrap(), 0);
    let created = writer
        .create_evidence_reference(
            create_command(),
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(
        created.record.vault_path.as_str(),
        "Research/Competitive/notes.md"
    );
    assert_eq!(created.record.classification, DataClassification::Internal);
    assert_eq!(created.record.version.get(), 1);
    assert!(matches!(
        created.record.verification,
        EvidenceVerification::Verified { .. }
    ));
    assert_eq!(writer.revision().unwrap(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 1);
}

#[test]
fn create_evidence_reference_without_a_classification_defaults_to_unclassified() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut command = create_command();
    command.classification = None;
    let created = writer
        .create_evidence_reference(
            command,
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert_eq!(
        created.record.classification,
        DataClassification::Unclassified
    );
}

#[test]
fn create_evidence_reference_without_a_fingerprint_persists_and_replays_correctly() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut command = create_command();
    command.fingerprint = None;
    command.verification = EvidenceVerification::Unverified;
    let created = writer
        .create_evidence_reference(
            command.clone(),
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert!(created.record.fingerprint.is_none());
    assert!(matches!(
        created.record.verification,
        EvidenceVerification::Unverified
    ));

    let replayed = writer
        .create_evidence_reference(
            command,
            AuditEventId::parse("synthetic-evidence-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert!(replayed.record.fingerprint.is_none());
}

#[test]
fn replaying_the_same_idempotency_id_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = writer
        .create_evidence_reference(
            create_command(),
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    let second = writer
        .create_evidence_reference(
            create_command(),
            AuditEventId::parse("synthetic-evidence-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(first.record, second.record);
    assert_eq!(first.audit_event.id(), second.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 1);
}

#[test]
fn create_evidence_reference_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_evidence_reference(
            create_command(),
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let mut drifted = create_command();
    drifted.vault_path = VaultRelativePath::parse("Research/Competitive/other.md").unwrap();
    let result = writer.create_evidence_reference(
        drifted,
        AuditEventId::parse("synthetic-evidence-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn create_evidence_reference_rejects_a_duplicate_id_under_a_different_idempotency_id() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_evidence_reference(
            create_command(),
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let mut duplicate = create_command();
    duplicate.context.idempotency_id = IdempotencyId::parse("synthetic-evidence-create-2").unwrap();
    let result = writer.create_evidence_reference(
        duplicate,
        AuditEventId::parse("synthetic-evidence-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn get_evidence_reference_returns_none_for_an_unknown_id() {
    let ledger = SyntheticLedger::new();
    let writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let unknown = EvidenceReferenceId::parse("synthetic-evidence-unknown").unwrap();
    let result = writer
        .get_evidence_reference(&unknown, CorrelationId::parse("synthetic-read-1").unwrap())
        .unwrap();
    assert!(result.is_none());
}

#[test]
fn get_evidence_reference_returns_the_current_record_after_create() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let created = writer
        .create_evidence_reference(
            create_command(),
            AuditEventId::parse("synthetic-evidence-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let read = writer
        .get_evidence_reference(
            &created.record.id,
            CorrelationId::parse("synthetic-read-1").unwrap(),
        )
        .unwrap();
    assert_eq!(read, Some(created.record));
}
