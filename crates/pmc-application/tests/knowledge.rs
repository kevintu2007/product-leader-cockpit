use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_application::knowledge::{
    pin_evidence_fingerprint, reobserve_evidence_verification, PinEvidenceFingerprintError,
    ReobserveOutcome,
};
use pmc_domain::{
    classification::DataClassification,
    evidence::{CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm},
    evidence::{OperationContext, VaultRelativePath},
    identity::{AggregateVersion, AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId},
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::{EvidenceVerification, IntegrityDigest},
};
use pmc_knowledge::vault::VaultRoot;
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

struct Fixture {
    ledger_path: PathBuf,
    vault_dir: PathBuf,
}

impl Fixture {
    fn new(test_name: &str) -> Self {
        let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nonce = nonce();
        let base = std::env::temp_dir().join(format!(
            "pmc-synthetic-knowledge-{test_name}-{nonce}-{sequence}"
        ));
        let vault_dir = base.join("vault");
        fs::create_dir_all(&vault_dir)
            .unwrap_or_else(|error| panic!("fixture setup failed: {error}"));
        let vault_dir = fs::canonicalize(&vault_dir)
            .unwrap_or_else(|error| panic!("fixture canonicalize failed: {error}"));
        let ledger_path = base.join("ledger.sqlite3");
        Self {
            ledger_path,
            vault_dir,
        }
    }

    fn vault(&self) -> VaultRoot {
        VaultRoot::validate(&self.vault_dir).unwrap()
    }

    fn write_note(&self, relative: &str, content: &[u8]) {
        let path = self.vault_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|error| panic!("setup failed: {error}"));
        }
        fs::write(&path, content).unwrap_or_else(|error| panic!("setup failed: {error}"));
    }

    fn remove_note(&self, relative: &str) {
        fs::remove_file(self.vault_dir.join(relative))
            .unwrap_or_else(|error| panic!("setup failed: {error}"));
    }
}

fn seed_evidence(
    writer: &mut SqliteProductLedger,
    relative_path: &str,
    fingerprint: Option<EvidenceFingerprint>,
    verification: EvidenceVerification,
) -> EvidenceReferenceId {
    let id = EvidenceReferenceId::parse("synthetic-evidence-1").unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: id.clone(),
                vault_path: VaultRelativePath::parse(relative_path).unwrap(),
                fingerprint,
                verification,
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

#[test]
fn reobserve_persists_an_unpinned_observation_for_an_existing_file_never_a_verification() {
    // Nothing was pinned, so a successful read is ObservedUnpinned --
    // recorded with its digest and time, but never called Verified.
    let fixture = Fixture::new("first-time");
    fixture.write_note("notes.md", b"synthetic content");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let id = seed_evidence(
        &mut writer,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = fixture.vault();

    let outcome = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-reobserve-1").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-1").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();

    let ReobserveOutcome::Persisted(mutation) = outcome else {
        panic!("expected Persisted, got {outcome:?}");
    };
    assert!(
        matches!(
            mutation.record.verification,
            EvidenceVerification::ObservedUnpinned { observed_at, .. }
                if observed_at == UtcTimestamp::from_unix_millis(2_000)
        ),
        "an unpinned reference must never verify; got {:?}",
        mutation.record.verification
    );
    assert_eq!(mutation.record.version.get(), 2);

    // Confirm it was actually written back, not just returned in-memory.
    let read = writer
        .get_evidence_reference(&id, CorrelationId::parse("synthetic-read-1").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(read.verification, mutation.record.verification);
    assert_eq!(read.version.get(), 2);
}

#[test]
fn reobserve_is_a_no_op_once_a_missing_file_has_already_degraded() {
    // A successful re-verification always refreshes `verified_at` to `now`
    // by design (that is what "last verified" means), so a Verified
    // Evidence re-observed against unchanged content is *never* a true
    // no-op. The genuine no-op case is a file that is *already*
    // DegradedLastVerified and is *still* missing on the next check: the
    // original verified_at is carried forward unchanged both times.
    let fixture = Fixture::new("no-op");
    fixture.write_note("notes.md", b"here for now");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let digest =
        pmc_platform::filesystem::compute_sha256_fingerprint(&fixture.vault_dir.join("notes.md"))
            .map(|digest| IntegrityDigest::parse(digest).unwrap())
            .unwrap();
    let id = seed_evidence(
        &mut writer,
        "notes.md",
        Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            digest.clone(),
        )),
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(500),
            integrity_digest: digest,
        },
    );
    let vault = fixture.vault();
    fixture.remove_note("notes.md");

    let first = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-reobserve-1").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-1").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();
    let ReobserveOutcome::Persisted(first_mutation) = first else {
        panic!("expected the first reobservation (Verified -> DegradedLastVerified) to persist");
    };
    assert!(matches!(
        first_mutation.record.verification,
        EvidenceVerification::DegradedLastVerified { .. }
    ));
    let revision_after_first = writer.revision().unwrap();

    // Second reobservation: the file is still missing, and the Evidence is
    // already DegradedLastVerified with the same original verified_at, so
    // this must be a genuine Unchanged no-op that never touches the Ledger.
    let second = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &id,
        first_mutation.record.version,
        IdempotencyId::parse("synthetic-reobserve-2").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-2").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    )
    .unwrap();

    match second {
        ReobserveOutcome::Unchanged(record) => {
            assert_eq!(record.verification, first_mutation.record.verification);
        }
        ReobserveOutcome::Persisted(_) => panic!("expected Unchanged, got Persisted"),
    }
    assert_eq!(writer.revision().unwrap(), revision_after_first);
}

#[test]
fn reobserve_detects_a_changed_file_as_an_integrity_mismatch() {
    let fixture = Fixture::new("mismatch");
    fixture.write_note("notes.md", b"original content");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let pinned = EvidenceFingerprint::new(
        FingerprintAlgorithm::Sha256,
        pmc_platform::filesystem::compute_sha256_fingerprint(&fixture.vault_dir.join("notes.md"))
            .map(|digest| IntegrityDigest::parse(digest).unwrap())
            .unwrap(),
    );
    let id = seed_evidence(
        &mut writer,
        "notes.md",
        Some(pinned),
        EvidenceVerification::Unverified,
    );
    let vault = fixture.vault();

    // Change the file's content after Evidence was pinned to the original.
    fixture.write_note("notes.md", b"tampered content");

    let outcome = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-reobserve-1").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-1").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();

    let ReobserveOutcome::Persisted(mutation) = outcome else {
        panic!("expected Persisted, got {outcome:?}");
    };
    assert!(matches!(
        mutation.record.verification,
        EvidenceVerification::IntegrityMismatch
    ));
}

#[test]
fn reobserve_degrades_when_the_file_is_removed_and_keeps_the_original_verified_at() {
    let fixture = Fixture::new("degrade");
    fixture.write_note("notes.md", b"here for now");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let digest =
        pmc_platform::filesystem::compute_sha256_fingerprint(&fixture.vault_dir.join("notes.md"))
            .map(|digest| IntegrityDigest::parse(digest).unwrap())
            .unwrap();
    let id = seed_evidence(
        &mut writer,
        "notes.md",
        Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            digest.clone(),
        )),
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(500),
            integrity_digest: digest,
        },
    );
    let vault = fixture.vault();
    fixture.remove_note("notes.md");

    let outcome = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-reobserve-1").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-1").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(9_999),
    )
    .unwrap();

    let ReobserveOutcome::Persisted(mutation) = outcome else {
        panic!("expected Persisted, got {outcome:?}");
    };
    match mutation.record.verification {
        EvidenceVerification::DegradedLastVerified {
            last_verified_at, ..
        } => {
            assert_eq!(last_verified_at, UtcTimestamp::from_unix_millis(500));
        }
        other => panic!("expected DegradedLastVerified, got {other:?}"),
    }
}

#[test]
fn reobserve_rejects_an_unknown_evidence_id() {
    let fixture = Fixture::new("unknown");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let vault = fixture.vault();
    let unknown = EvidenceReferenceId::parse("synthetic-evidence-unknown").unwrap();

    let result = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &unknown,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-reobserve-1").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-1").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );

    assert!(matches!(
        result,
        Err(pmc_application::knowledge::ReobserveEvidenceError::EvidenceNotFound)
    ));
}

#[test]
fn pin_hashes_the_live_file_and_from_then_on_drift_is_detected() {
    // Fingerprint pin, end to end: an unpinned reference gains an identity from
    // the bytes actually on disk, and the next re-observation after the
    // file changes reports a mismatch -- the detection the unpinned state
    // could never give.
    let fixture = Fixture::new("pin");
    fixture.write_note("notes.md", b"the bytes that become the identity");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let id = seed_evidence(
        &mut writer,
        "notes.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = fixture.vault();
    let expected =
        pmc_platform::filesystem::compute_sha256_fingerprint(&fixture.vault_dir.join("notes.md"))
            .map(|digest| IntegrityDigest::parse(digest).unwrap())
            .unwrap();

    let pinned = pin_evidence_fingerprint(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-pin-1").unwrap(),
        CorrelationId::parse("synthetic-pin-correlation-1").unwrap(),
        AuditEventId::parse("synthetic-pin-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap();

    assert_eq!(
        pinned.record.fingerprint,
        Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            expected.clone()
        ))
    );
    assert_eq!(
        pinned.record.verification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(2_000),
            integrity_digest: expected,
        }
    );
    assert_eq!(pinned.record.version.get(), 2);

    fixture.write_note("notes.md", b"different bytes under the same name");
    let outcome = reobserve_evidence_verification(
        &mut writer,
        &vault,
        &id,
        pinned.record.version,
        IdempotencyId::parse("synthetic-reobserve-after-pin").unwrap(),
        CorrelationId::parse("synthetic-reobserve-correlation-after-pin").unwrap(),
        AuditEventId::parse("synthetic-reobserve-audit-after-pin").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    )
    .unwrap();
    let ReobserveOutcome::Persisted(mutation) = outcome else {
        panic!("a changed pinned file must persist a new state, got {outcome:?}");
    };
    assert_eq!(
        mutation.record.verification,
        EvidenceVerification::IntegrityMismatch
    );
}

#[test]
fn pin_refuses_a_missing_file_and_writes_nothing() {
    let fixture = Fixture::new("pin-missing");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let id = seed_evidence(
        &mut writer,
        "missing.md",
        None,
        EvidenceVerification::Unverified,
    );
    let vault = fixture.vault();
    let revision_before = writer.revision().unwrap();

    let result = pin_evidence_fingerprint(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-pin-missing").unwrap(),
        CorrelationId::parse("synthetic-pin-correlation-missing").unwrap(),
        AuditEventId::parse("synthetic-pin-audit-missing").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );

    assert!(
        matches!(
            result,
            Err(PinEvidenceFingerprintError::SourceNotObservable(
                EvidenceVerification::Unverified
            ))
        ),
        "a file that cannot be read has no bytes to pin: {result:?}"
    );
    assert_eq!(writer.revision().unwrap(), revision_before);
    let read = writer
        .get_evidence_reference(&id, CorrelationId::parse("synthetic-read-1").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(read.version.get(), 1);
    assert_eq!(read.fingerprint, None);
}

#[test]
fn pin_refuses_an_already_pinned_reference_before_touching_the_file() {
    let fixture = Fixture::new("pin-already");
    fixture.write_note("notes.md", b"already pinned content");
    let mut writer = SqliteProductLedger::open(&fixture.ledger_path).unwrap();
    let digest =
        pmc_platform::filesystem::compute_sha256_fingerprint(&fixture.vault_dir.join("notes.md"))
            .map(|digest| IntegrityDigest::parse(digest).unwrap())
            .unwrap();
    let id = seed_evidence(
        &mut writer,
        "notes.md",
        Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            digest.clone(),
        )),
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(1_000),
            integrity_digest: digest,
        },
    );
    let vault = fixture.vault();
    let revision_before = writer.revision().unwrap();

    let result = pin_evidence_fingerprint(
        &mut writer,
        &vault,
        &id,
        AggregateVersion::initial(),
        IdempotencyId::parse("synthetic-pin-already").unwrap(),
        CorrelationId::parse("synthetic-pin-correlation-already").unwrap(),
        AuditEventId::parse("synthetic-pin-audit-already").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );

    assert!(
        matches!(result, Err(PinEvidenceFingerprintError::AlreadyPinned)),
        "{result:?}"
    );
    assert_eq!(writer.revision().unwrap(), revision_before);
}
