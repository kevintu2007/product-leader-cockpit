use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    composition_source::LedgerSnapshotForCompositionPort,
    evidence::{
        CreateEvidenceReference, OperationContext, UpdateEvidenceVerification, VaultRelativePath,
    },
    identity::{AggregateVersion, AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId},
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
            "pmc-synthetic-evidence-verification-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn seed_evidence(writer: &mut SqliteProductLedger) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-1").unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse("Research/notes.md").unwrap(),
                fingerprint: None,
                verification: EvidenceVerification::Unverified,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: OperationContext {
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

fn update_command(
    id: EvidenceReferenceId,
    expected_version: u64,
    verification: EvidenceVerification,
    idempotency_id: &str,
) -> UpdateEvidenceVerification {
    UpdateEvidenceVerification {
        id,
        expected_version: AggregateVersion::new(expected_version).unwrap(),
        verification,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
            correlation_id: CorrelationId::parse("synthetic-verification-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_persists_an_updated_verification_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);

    let verified = EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
    };
    let updated = writer
        .update_evidence_verification(
            update_command(id.clone(), 1, verified.clone(), "synthetic-verify-1"),
            AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    assert_eq!(updated.record.id, id);
    assert_eq!(updated.record.version.get(), 2);
    assert_eq!(updated.record.verification, verified);
    // Immutable fields must survive unchanged.
    assert_eq!(updated.record.vault_path.as_str(), "Research/notes.md");
    assert_eq!(updated.record.classification, DataClassification::Internal);
    assert_eq!(writer.revision().unwrap(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 2);
}

#[test]
fn replaying_the_same_idempotency_id_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);
    let verified = EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
    };

    let first = writer
        .update_evidence_verification(
            update_command(id.clone(), 1, verified.clone(), "synthetic-verify-1"),
            AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    let second = writer
        .update_evidence_verification(
            update_command(id, 1, verified, "synthetic-verify-1"),
            AuditEventId::parse("synthetic-verify-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(6_000),
        )
        .unwrap();

    assert_eq!(first.record, second.record);
    assert_eq!(first.audit_event.id(), second.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 2);
}

#[test]
fn update_evidence_verification_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);
    writer
        .update_evidence_verification(
            update_command(
                id.clone(),
                1,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(5_000),
                    integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                },
                "synthetic-verify-1",
            ),
            AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    let result = writer.update_evidence_verification(
        update_command(
            id,
            1,
            EvidenceVerification::IntegrityMismatch,
            "synthetic-verify-1",
        ),
        AuditEventId::parse("synthetic-verify-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(6_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_evidence_verification_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);

    let result = writer.update_evidence_verification(
        update_command(
            id,
            2,
            EvidenceVerification::IntegrityMismatch,
            "synthetic-verify-1",
        ),
        AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn update_evidence_verification_rejects_an_unknown_evidence_id() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let unknown_id = EvidenceReferenceId::parse("synthetic-evidence-unknown").unwrap();

    let result = writer.update_evidence_verification(
        update_command(
            unknown_id,
            1,
            EvidenceVerification::IntegrityMismatch,
            "synthetic-verify-1",
        ),
        AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn replaying_an_older_idempotency_id_returns_its_own_original_outcome_not_current_state() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);

    let verified = EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
    };
    let first = writer
        .update_evidence_verification(
            update_command(id.clone(), 1, verified.clone(), "synthetic-verify-1"),
            AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    assert_eq!(first.record.version.get(), 2);
    assert_eq!(first.record.verification, verified);

    // A second, distinct command moves current state further -- version 3,
    // IntegrityMismatch. Replaying the first idempotency key afterward must
    // still return exactly what the first command itself produced, not this
    // newer current state.
    let mismatch = EvidenceVerification::IntegrityMismatch;
    let second = writer
        .update_evidence_verification(
            update_command(id.clone(), 2, mismatch, "synthetic-verify-2"),
            AuditEventId::parse("synthetic-verify-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap();
    assert_eq!(second.record.version.get(), 3);

    let replayed_first = writer
        .update_evidence_verification(
            update_command(id, 1, verified.clone(), "synthetic-verify-1"),
            AuditEventId::parse("synthetic-verify-audit-should-be-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(99_000),
        )
        .unwrap();

    assert_eq!(replayed_first.record, first.record);
    assert_eq!(replayed_first.record.version.get(), 2);
    assert_eq!(replayed_first.record.verification, verified);
    assert_eq!(replayed_first.audit_event.id(), first.audit_event.id());
    // Replay must not touch current state or advance the Ledger revision
    // beyond what the second (real) command already produced.
    assert_eq!(writer.revision().unwrap(), 3);
}

#[test]
fn update_evidence_verification_can_chain_through_multiple_states_bumping_version_each_time() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);

    let verified = EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
    };
    let after_verify = writer
        .update_evidence_verification(
            update_command(id.clone(), 1, verified.clone(), "synthetic-verify-1"),
            AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    assert_eq!(after_verify.record.version.get(), 2);

    let degraded = EvidenceVerification::DegradedLastVerified {
        last_verified_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
    };
    let after_degrade = writer
        .update_evidence_verification(
            update_command(id.clone(), 2, degraded.clone(), "synthetic-verify-2"),
            AuditEventId::parse("synthetic-verify-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap();
    assert_eq!(after_degrade.record.version.get(), 3);
    assert_eq!(after_degrade.record.verification, degraded);

    let mismatch = EvidenceVerification::IntegrityMismatch;
    let after_mismatch = writer
        .update_evidence_verification(
            update_command(id, 3, mismatch.clone(), "synthetic-verify-3"),
            AuditEventId::parse("synthetic-verify-audit-3").unwrap(),
            UtcTimestamp::from_unix_millis(10_000),
        )
        .unwrap();
    assert_eq!(after_mismatch.record.version.get(), 4);
    assert_eq!(after_mismatch.record.verification, mismatch);
}

#[test]
fn an_observed_unpinned_verification_persists_and_reads_back_unpinned_after_restart() {
    // The fifth verification value goes through the writer, the command-result
    // table, and the composition read surface, which also says the reference
    // is unpinned -- a fact taken from the pin columns, not from the state.
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_evidence(&mut writer);

    let observed = EvidenceVerification::ObservedUnpinned {
        observed_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse("c".repeat(64)).unwrap(),
    };
    let updated = writer
        .update_evidence_verification(
            update_command(
                id.clone(),
                1,
                observed.clone(),
                "synthetic-verify-unpinned-1",
            ),
            AuditEventId::parse("synthetic-verify-unpinned-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    assert_eq!(updated.record.verification, observed);
    assert_eq!(updated.record.version.get(), 2);

    // An exact replay returns the same fifth-valued outcome from the
    // command-result table, so the decoder there handles it too.
    let replayed = writer
        .update_evidence_verification(
            update_command(
                id.clone(),
                1,
                observed.clone(),
                "synthetic-verify-unpinned-1",
            ),
            AuditEventId::parse("synthetic-verify-unpinned-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    assert_eq!(replayed.record.verification, observed);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .read_composition_snapshot(UtcTimestamp::from_unix_millis(9_000))
        .unwrap();
    let reference = snapshot
        .evidence_references
        .iter()
        .find(|reference| reference.id == id)
        .expect("the reference must be on the read surface");
    assert_eq!(reference.verification, observed);
    assert!(!reference.pinned, "seed_evidence pins nothing");
}
