use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    evidence::{
        CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm, OperationContext,
        RelocateEvidenceReference, UpdateEvidenceVerification, VaultRelativePath,
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
            "pmc-synthetic-evidence-relocation-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

const PINNED_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn seed_pinned_evidence(writer: &mut SqliteProductLedger) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-1").unwrap();
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

fn seed_unpinned_evidence(writer: &mut SqliteProductLedger) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-unpinned").unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse("Research/unpinned.md").unwrap(),
                fingerprint: None,
                verification: EvidenceVerification::Unverified,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-evidence-create-unpinned")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-evidence-correlation-unpinned")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-evidence-audit-unpinned").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    id
}

fn relocate_command(
    id: EvidenceReferenceId,
    expected_version: u64,
    expected_current_path: &str,
    new_vault_path: &str,
    observed_digest: &str,
    observed_at: i64,
    idempotency_id: &str,
) -> RelocateEvidenceReference {
    RelocateEvidenceReference {
        id,
        expected_version: AggregateVersion::new(expected_version).unwrap(),
        expected_current_path: VaultRelativePath::parse(expected_current_path).unwrap(),
        new_vault_path: VaultRelativePath::parse(new_vault_path).unwrap(),
        observed_fingerprint: EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            IntegrityDigest::parse(observed_digest).unwrap(),
        ),
        observed_at: UtcTimestamp::from_unix_millis(observed_at),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
            correlation_id: CorrelationId::parse("synthetic-relocation-correlation-1").unwrap(),
        },
    }
}

#[test]
fn relocate_evidence_reference_moves_the_path_and_reverifies_with_the_same_fingerprint() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let outcome = writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    assert_eq!(outcome.record.id, id);
    assert_eq!(outcome.record.version.get(), 2);
    assert_eq!(
        outcome.record.vault_path.as_str(),
        "Research/Archive/notes.md"
    );
    assert_eq!(
        outcome.record.verification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(5_000),
            integrity_digest: IntegrityDigest::parse(PINNED_DIGEST).unwrap(),
        }
    );
    // Fingerprint/classification/provenance must survive unchanged.
    assert_eq!(
        outcome.record.fingerprint.unwrap().digest().as_str(),
        PINNED_DIGEST
    );
    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(writer.revision().unwrap(), 2);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 2);
}

#[test]
fn relocate_evidence_reference_rejects_a_different_observed_fingerprint() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let result = writer.relocate_evidence_reference(
        relocate_command(
            id,
            1,
            "Research/notes.md",
            "Research/Archive/notes.md",
            OTHER_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn relocate_evidence_reference_rejects_an_unpinned_reference() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);

    let result = writer.relocate_evidence_reference(
        relocate_command(
            id,
            1,
            "Research/unpinned.md",
            "Research/Archive/unpinned.md",
            OTHER_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn relocate_evidence_reference_rejects_a_stale_expected_current_path() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let result = writer.relocate_evidence_reference(
        relocate_command(
            id,
            1,
            "Research/wrong-current-path.md",
            "Research/Archive/notes.md",
            PINNED_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn relocate_evidence_reference_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let result = writer.relocate_evidence_reference(
        relocate_command(
            id,
            2,
            "Research/notes.md",
            "Research/Archive/notes.md",
            PINNED_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn relocate_evidence_reference_rejects_a_no_op_path() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let result = writer.relocate_evidence_reference(
        relocate_command(
            id,
            1,
            "Research/notes.md",
            "Research/notes.md",
            PINNED_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn relocate_evidence_reference_rejects_an_unknown_evidence_id() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let unknown_id = EvidenceReferenceId::parse("synthetic-evidence-unknown").unwrap();

    let result = writer.relocate_evidence_reference(
        relocate_command(
            unknown_id,
            1,
            "Research/notes.md",
            "Research/Archive/notes.md",
            PINNED_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert!(result.is_err());
}

#[test]
fn replaying_the_same_idempotency_id_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let first = writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    let second = writer
        .relocate_evidence_reference(
            relocate_command(
                id,
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(6_000),
        )
        .unwrap();

    assert_eq!(first.record, second.record);
    assert_eq!(first.audit_event.id(), second.audit_event.id());
    assert_eq!(writer.revision().unwrap(), 2);
}

#[test]
fn relocate_evidence_reference_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);
    writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    let result = writer.relocate_evidence_reference(
        relocate_command(
            id,
            1,
            "Research/notes.md",
            "Research/Elsewhere/notes.md",
            PINNED_DIGEST,
            5_000,
            "synthetic-relocate-1",
        ),
        AuditEventId::parse("synthetic-relocate-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(6_000),
    );
    assert!(result.is_err());
}

#[test]
fn replaying_an_older_relocation_idempotency_id_returns_its_own_original_outcome_not_current_state()
{
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    let first = writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();
    assert_eq!(first.record.version.get(), 2);
    assert_eq!(
        first.record.vault_path.as_str(),
        "Research/Archive/notes.md"
    );

    let second = writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                2,
                "Research/Archive/notes.md",
                "Research/Elsewhere/notes.md",
                PINNED_DIGEST,
                9_000,
                "synthetic-relocate-2",
            ),
            AuditEventId::parse("synthetic-relocate-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap();
    assert_eq!(second.record.version.get(), 3);
    assert_eq!(
        second.record.vault_path.as_str(),
        "Research/Elsewhere/notes.md"
    );

    let replayed_first = writer
        .relocate_evidence_reference(
            relocate_command(
                id,
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-should-be-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(99_000),
        )
        .unwrap();

    assert_eq!(replayed_first.record, first.record);
    assert_eq!(replayed_first.record.version.get(), 2);
    assert_eq!(
        replayed_first.record.vault_path.as_str(),
        "Research/Archive/notes.md"
    );
    assert_eq!(writer.revision().unwrap(), 3);
}

#[test]
fn relocate_evidence_reference_survives_a_subsequent_verification_update() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);

    writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                1,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    let updated = writer
        .update_evidence_verification(
            UpdateEvidenceVerification {
                id,
                expected_version: AggregateVersion::new(2).unwrap(),
                verification: EvidenceVerification::IntegrityMismatch,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-verify-after-relocate")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-verify-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("synthetic-verify-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap();

    assert_eq!(updated.record.version.get(), 3);
    assert_eq!(
        updated.record.vault_path.as_str(),
        "Research/Archive/notes.md"
    );
    assert_eq!(
        updated.record.verification,
        EvidenceVerification::IntegrityMismatch
    );
}

/// Since v45 the verification replay reports the path the verification was
/// recorded against, not the live row's path after a later relocation.
#[test]
fn verification_replay_after_a_relocation_reports_the_path_it_was_recorded_against() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);
    let verify = || UpdateEvidenceVerification {
        id: id.clone(),
        expected_version: AggregateVersion::initial(),
        verification: EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(2_000),
            integrity_digest: IntegrityDigest::parse(PINNED_DIGEST).unwrap(),
        },
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-verify-before-relocate").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-verify-correlation-0").unwrap(),
        },
    };
    let first = writer
        .update_evidence_verification(
            verify(),
            AuditEventId::parse("synthetic-verify-audit-0").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(first.record.vault_path.as_str(), "Research/notes.md");
    assert_eq!(first.record.version.get(), 2);

    writer
        .relocate_evidence_reference(
            relocate_command(
                id.clone(),
                2,
                "Research/notes.md",
                "Research/Archive/notes.md",
                PINNED_DIGEST,
                5_000,
                "synthetic-relocate-1",
            ),
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_000),
        )
        .unwrap();

    let replayed = writer
        .update_evidence_verification(
            verify(),
            AuditEventId::parse("synthetic-verify-audit-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap();
    assert_eq!(replayed.record.vault_path.as_str(), "Research/notes.md");
    assert_eq!(replayed.record.version.get(), 2);
    assert_eq!(replayed.audit_event.id(), first.audit_event.id());
    drop(writer);

    let stored: String = rusqlite::Connection::open(&ledger.0)
        .unwrap()
        .query_row(
            "SELECT command_vault_path FROM evidence_verification_command_results WHERE idempotency_id='synthetic-verify-before-relocate'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, "Research/notes.md");
}
