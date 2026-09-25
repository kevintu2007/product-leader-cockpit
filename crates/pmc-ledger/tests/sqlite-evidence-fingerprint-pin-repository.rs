use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    composition_source::LedgerSnapshotForCompositionPort,
    evidence::{
        CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm, OperationContext,
        PinEvidenceFingerprint, RelocateEvidenceReference, VaultRelativePath,
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
            "pmc-synthetic-evidence-fingerprint-pin-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

const PINNED_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OBSERVED_DIGEST: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const OTHER_DIGEST: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

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

fn seed_pinned_evidence(writer: &mut SqliteProductLedger) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-pinned").unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse("Research/pinned.md").unwrap(),
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
                    idempotency_id: IdempotencyId::parse("synthetic-evidence-create-pinned")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-evidence-correlation-pinned")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-evidence-audit-pinned").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    id
}

fn pin_command(
    id: EvidenceReferenceId,
    expected_version: u64,
    expected_current_path: &str,
    observed_digest: &str,
    observed_at: i64,
    idempotency_id: &str,
) -> PinEvidenceFingerprint {
    PinEvidenceFingerprint {
        id,
        expected_version: AggregateVersion::new(expected_version).unwrap(),
        expected_current_path: VaultRelativePath::parse(expected_current_path).unwrap(),
        observed_fingerprint: EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            IntegrityDigest::parse(observed_digest).unwrap(),
        ),
        observed_at: UtcTimestamp::from_unix_millis(observed_at),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
            correlation_id: CorrelationId::parse("synthetic-pin-correlation-1").unwrap(),
        },
    }
}

fn expected_pin() -> EvidenceFingerprint {
    EvidenceFingerprint::new(
        FingerprintAlgorithm::Sha256,
        IntegrityDigest::parse(OBSERVED_DIGEST).unwrap(),
    )
}

fn expected_verified() -> EvidenceVerification {
    EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(5_000),
        integrity_digest: IntegrityDigest::parse(OBSERVED_DIGEST).unwrap(),
    }
}

fn read_back(
    ledger: &SyntheticLedger,
    id: &EvidenceReferenceId,
) -> pmc_domain::evidence::EvidenceReferenceRecord {
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .get_evidence_reference(id, CorrelationId::parse("synthetic-read-1").unwrap())
        .unwrap()
        .expect("the reference must exist")
}

#[test]
fn pin_sets_the_fingerprint_and_verifies_with_the_same_digest_once() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);
    let revision_before = writer.revision().unwrap();

    let outcome = writer
        .pin_evidence_fingerprint(
            pin_command(
                id.clone(),
                1,
                "Research/unpinned.md",
                OBSERVED_DIGEST,
                5_000,
                "synthetic-pin-1",
            ),
            AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_100),
        )
        .expect("an unpinned reference must accept its first pin");

    assert_eq!(outcome.record.fingerprint, Some(expected_pin()));
    assert_eq!(outcome.record.verification, expected_verified());
    assert_eq!(outcome.record.version.get(), 2);
    assert_eq!(outcome.record.vault_path.as_str(), "Research/unpinned.md");
    assert_eq!(outcome.audit_event.id().as_str(), "synthetic-pin-audit-1");
    assert_eq!(writer.revision().unwrap(), revision_before + 1);
    drop(writer);

    // Durable, and visible to the read surface as pinned -- from the pin
    // columns, not from the verification state.
    let after = read_back(&ledger, &id);
    assert_eq!(after.fingerprint, Some(expected_pin()));
    assert_eq!(after.verification, expected_verified());
    assert_eq!(after.version.get(), 2);
    let reader = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reader
        .read_composition_snapshot(UtcTimestamp::from_unix_millis(9_000))
        .unwrap();
    let reference = snapshot
        .evidence_references
        .iter()
        .find(|reference| reference.id == id)
        .expect("the reference must be on the read surface");
    assert!(reference.pinned);
    assert!(
        !format!("{:?}", snapshot.evidence_references).contains("unpinned.md"),
        "the pin must not bring the Vault path onto the read surface"
    );
}

#[test]
fn pin_is_refused_on_an_already_pinned_reference_and_changes_nothing() {
    // A pin is the identity. The second attempt carries a different digest,
    // which is exactly the rewrite this refusal exists to prevent.
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_pinned_evidence(&mut writer);
    let revision_before = writer.revision().unwrap();

    let result = writer.pin_evidence_fingerprint(
        pin_command(
            id.clone(),
            1,
            "Research/pinned.md",
            OTHER_DIGEST,
            5_000,
            "synthetic-pin-repin",
        ),
        AuditEventId::parse("synthetic-pin-audit-repin").unwrap(),
        UtcTimestamp::from_unix_millis(5_100),
    );

    assert!(result.is_err(), "re-pin must be refused: {result:?}");
    assert_eq!(writer.revision().unwrap(), revision_before);
    drop(writer);
    let after = read_back(&ledger, &id);
    assert_eq!(after.version.get(), 1);
    assert_eq!(
        after.fingerprint.as_ref().map(|pin| pin.digest().as_str()),
        Some(PINNED_DIGEST)
    );
}

#[test]
fn pin_is_refused_for_a_stale_version_a_stale_path_or_an_unknown_reference() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);
    let revision_before = writer.revision().unwrap();

    let stale_version = writer.pin_evidence_fingerprint(
        pin_command(
            id.clone(),
            2,
            "Research/unpinned.md",
            OBSERVED_DIGEST,
            5_000,
            "synthetic-pin-stale-version",
        ),
        AuditEventId::parse("synthetic-pin-audit-stale-version").unwrap(),
        UtcTimestamp::from_unix_millis(5_100),
    );
    assert!(stale_version.is_err(), "{stale_version:?}");

    let stale_path = writer.pin_evidence_fingerprint(
        pin_command(
            id.clone(),
            1,
            "Research/elsewhere.md",
            OBSERVED_DIGEST,
            5_000,
            "synthetic-pin-stale-path",
        ),
        AuditEventId::parse("synthetic-pin-audit-stale-path").unwrap(),
        UtcTimestamp::from_unix_millis(5_100),
    );
    assert!(stale_path.is_err(), "{stale_path:?}");

    let unknown = writer.pin_evidence_fingerprint(
        pin_command(
            EvidenceReferenceId::parse("synthetic-evidence-missing").unwrap(),
            1,
            "Research/unpinned.md",
            OBSERVED_DIGEST,
            5_000,
            "synthetic-pin-unknown",
        ),
        AuditEventId::parse("synthetic-pin-audit-unknown").unwrap(),
        UtcTimestamp::from_unix_millis(5_100),
    );
    assert!(unknown.is_err(), "{unknown:?}");

    assert_eq!(writer.revision().unwrap(), revision_before);
    drop(writer);
    let after = read_back(&ledger, &id);
    assert_eq!(after.version.get(), 1);
    assert_eq!(after.fingerprint, None);
    assert_eq!(after.verification, EvidenceVerification::Unverified);
}

#[test]
fn pin_rejects_an_idempotency_id_already_claimed_by_another_evidence_command() {
    // The claim trigger shares one namespace across every Evidence command:
    // the create's idempotency key cannot be reused for a pin.
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);

    let result = writer.pin_evidence_fingerprint(
        pin_command(
            id.clone(),
            1,
            "Research/unpinned.md",
            OBSERVED_DIGEST,
            5_000,
            "synthetic-evidence-create-unpinned",
        ),
        AuditEventId::parse("synthetic-pin-audit-collision").unwrap(),
        UtcTimestamp::from_unix_millis(5_100),
    );

    assert!(result.is_err(), "{result:?}");
    drop(writer);
    assert_eq!(read_back(&ledger, &id).version.get(), 1);
}

#[test]
fn an_exact_replay_returns_the_original_outcome_even_after_a_later_relocation() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);
    let first = writer
        .pin_evidence_fingerprint(
            pin_command(
                id.clone(),
                1,
                "Research/unpinned.md",
                OBSERVED_DIGEST,
                5_000,
                "synthetic-pin-1",
            ),
            AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_100),
        )
        .unwrap();

    // Now pinned, the reference can be relocated -- which moves the live
    // path and bumps the version to 3.
    writer
        .relocate_evidence_reference(
            RelocateEvidenceReference {
                id: id.clone(),
                expected_version: AggregateVersion::new(2).unwrap(),
                expected_current_path: VaultRelativePath::parse("Research/unpinned.md").unwrap(),
                new_vault_path: VaultRelativePath::parse("Archive/moved.md").unwrap(),
                observed_fingerprint: expected_pin(),
                observed_at: UtcTimestamp::from_unix_millis(6_000),
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-relocate-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-relocate-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-relocate-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(6_100),
        )
        .expect("a pinned reference relocates with the same bytes");
    let revision_before = writer.revision().unwrap();

    let replayed = writer
        .pin_evidence_fingerprint(
            pin_command(
                id.clone(),
                1,
                "Research/unpinned.md",
                OBSERVED_DIGEST,
                5_000,
                "synthetic-pin-1",
            ),
            AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_100),
        )
        .expect("an exact replay must return the original outcome");

    // The original, not the current state: version 2 and the path at pin
    // time, while the live row is at version 3 under the new path.
    assert_eq!(replayed, first);
    assert_eq!(replayed.record.version.get(), 2);
    assert_eq!(replayed.record.vault_path.as_str(), "Research/unpinned.md");
    assert_eq!(replayed.record.fingerprint, Some(expected_pin()));
    assert_eq!(replayed.record.verification, expected_verified());
    assert_eq!(
        writer.revision().unwrap(),
        revision_before,
        "a replay writes nothing"
    );
    drop(writer);
    let live = read_back(&ledger, &id);
    assert_eq!(live.version.get(), 3);
    assert_eq!(live.vault_path.as_str(), "Archive/moved.md");
}

#[test]
fn a_same_id_replay_with_a_different_payload_is_a_conflict() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);
    writer
        .pin_evidence_fingerprint(
            pin_command(
                id.clone(),
                1,
                "Research/unpinned.md",
                OBSERVED_DIGEST,
                5_000,
                "synthetic-pin-1",
            ),
            AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_100),
        )
        .unwrap();
    let revision_before = writer.revision().unwrap();

    // Same key, later observation time: a re-observation under a reused key
    // is not a replay and must not be reported as one.
    let result = writer.pin_evidence_fingerprint(
        pin_command(
            id.clone(),
            1,
            "Research/unpinned.md",
            OBSERVED_DIGEST,
            5_001,
            "synthetic-pin-1",
        ),
        AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(5_100),
    );

    assert!(result.is_err(), "{result:?}");
    assert_eq!(writer.revision().unwrap(), revision_before);
}

#[test]
fn relocation_still_refuses_an_unpinned_reference_until_it_is_pinned() {
    // The invariant the relocate replay decoder relies on: a relocated
    // reference always has a pin.
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = seed_unpinned_evidence(&mut writer);
    let relocate = |writer: &mut SqliteProductLedger, expected_version: u64, key: &str| {
        writer.relocate_evidence_reference(
            RelocateEvidenceReference {
                id: id.clone(),
                expected_version: AggregateVersion::new(expected_version).unwrap(),
                expected_current_path: VaultRelativePath::parse("Research/unpinned.md").unwrap(),
                new_vault_path: VaultRelativePath::parse("Archive/moved.md").unwrap(),
                observed_fingerprint: expected_pin(),
                observed_at: UtcTimestamp::from_unix_millis(6_000),
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(key).unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-relocate-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse(format!("{key}-audit")).unwrap(),
            UtcTimestamp::from_unix_millis(6_100),
        )
    };

    assert!(relocate(&mut writer, 1, "synthetic-relocate-before-pin").is_err());
    writer
        .pin_evidence_fingerprint(
            pin_command(
                id.clone(),
                1,
                "Research/unpinned.md",
                OBSERVED_DIGEST,
                5_000,
                "synthetic-pin-1",
            ),
            AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(5_100),
        )
        .unwrap();
    relocate(&mut writer, 2, "synthetic-relocate-after-pin")
        .expect("once pinned, the same bytes may move");
}
