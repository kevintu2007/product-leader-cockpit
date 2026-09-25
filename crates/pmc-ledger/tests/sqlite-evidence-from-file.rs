//! Evidence from a file (schema v48; DG3 Vault-root and Evidence-from-file
//! amendment §4; product owner 2026-09-23): the reserved create, one
//! reference per Vault file compared the way the caller's `same_name` says,
//! and the two narrow reads that name existing references by id only.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::classification::DataClassification;
use pmc_domain::error::DomainError;
use pmc_domain::evidence::{
    EvidenceFingerprint, FingerprintAlgorithm, OperationContext, VaultRelativePath,
};
use pmc_domain::identity::{AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId};
use pmc_domain::provenance::Provenance;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};
use pmc_ledger::sqlite::{
    LedgerTransactionError, ReservationRequest, ReservedEntityKind, ReservedId, SqliteProductLedger,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

const CREATE_EVIDENCE: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::EvidenceReference,
    operation: "create_evidence_reference",
};

fn ledger() -> SqliteProductLedger {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| panic!("clock"))
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf =
        std::env::temp_dir().join(format!("pmc-evidence-from-file-{nonce}-{sequence}.sqlite3"));
    SqliteProductLedger::open(path).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Exact comparison: two spellings are two names.
fn exact(a: &str, b: &str) -> bool {
    a == b
}

/// A stand-in for Windows' comparison in these Ledger tests (the real one,
/// and its non-ASCII cases, are tested where it lives, in `pmc-platform`).
fn ascii_case_insensitive(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn context(request: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn fingerprint(fill: char) -> EvidenceFingerprint {
    EvidenceFingerprint::new(
        FingerprintAlgorithm::Sha256,
        IntegrityDigest::parse(fill.to_string().repeat(64)).unwrap_or_else(|_| panic!("digest")),
    )
}

fn path(value: &str) -> VaultRelativePath {
    VaultRelativePath::parse(value).unwrap_or_else(|_| panic!("path"))
}

fn reserve(
    ledger: &mut SqliteProductLedger,
    request: &str,
    id: &str,
) -> ReservedId<EvidenceReferenceId> {
    let id = id.to_owned();
    ledger
        .reserve_or_get_record_id(
            &IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
            CREATE_EVIDENCE,
            at(50),
            move || EvidenceReferenceId::parse(id.clone()),
        )
        .unwrap_or_else(|e| panic!("{e:?}"))
}

#[allow(clippy::too_many_arguments, clippy::result_large_err)]
fn create(
    ledger: &mut SqliteProductLedger,
    reserved: &ReservedId<EvidenceReferenceId>,
    request: &str,
    vault_path: &str,
    fill: char,
    same_name: fn(&str, &str) -> bool,
) -> Result<
    pmc_domain::evidence::MutationOutcome<pmc_domain::evidence::EvidenceReferenceRecord>,
    LedgerTransactionError<DomainError>,
> {
    ledger.create_evidence_reference_from_reservation(
        reserved,
        path(vault_path),
        fingerprint(fill),
        at(100),
        DataClassification::Internal,
        context(request),
        AuditEventId::parse(format!("audit-{request}")).unwrap_or_else(|_| panic!("id")),
        at(120),
        same_name,
    )
}

fn conflict_key(
    result: Result<impl std::fmt::Debug, LedgerTransactionError<DomainError>>,
) -> String {
    match result {
        Err(LedgerTransactionError::Operation(error)) => error.message_key().as_str().to_owned(),
        other => panic!("expected an operation error, got {other:?}"),
    }
}

#[test]
fn a_file_becomes_a_pinned_verified_user_entered_reference_with_the_reserved_id() {
    let mut ledger = ledger();
    let reserved = reserve(&mut ledger, "sheet-1", "evidence-from-file-1");
    let created = create(
        &mut ledger,
        &reserved,
        "sheet-1",
        "notes/Q3 review.md",
        'a',
        exact,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let record = created.record;
    assert_eq!(record.id.as_str(), "evidence-from-file-1");
    assert_eq!(record.vault_path.as_str(), "notes/Q3 review.md");
    // Pinned and verified from the one observation: the same digest, the
    // observation's instant.
    assert_eq!(record.fingerprint, Some(fingerprint('a')));
    assert_eq!(
        record.verification,
        EvidenceVerification::Verified {
            verified_at: at(100),
            integrity_digest: fingerprint('a').digest().clone(),
        }
    );
    assert_eq!(record.classification, DataClassification::Internal);
    assert_eq!(record.provenance, Provenance::UserEntered);

    // Read back from the Ledger, not only from the outcome.
    let stored = ledger
        .get_evidence_reference(&record.id, context("read").correlation_id)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the created reference must be stored"));
    assert_eq!(stored, record);
}

#[test]
fn a_retry_replays_the_create_instead_of_refusing_its_own_path() {
    let mut ledger = ledger();
    let reserved = reserve(&mut ledger, "sheet-2", "evidence-retry");
    let first = create(&mut ledger, &reserved, "sheet-2", "a.md", 'a', exact)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let revision = ledger.revision().unwrap_or_else(|e| panic!("{e:?}"));
    // The sheet retries with the same request id: the reservation reads back
    // the same id, and the create answers what it did the first time.
    let again_reserved = reserve(&mut ledger, "sheet-2", "never-minted");
    assert!(again_reserved.replayed());
    let again = create(&mut ledger, &again_reserved, "sheet-2", "a.md", 'a', exact)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.record, first.record);
    assert_eq!(
        ledger.revision().unwrap_or_else(|e| panic!("{e:?}")),
        revision
    );
}

#[test]
fn a_second_reference_for_the_same_file_is_refused_and_writes_nothing() {
    let mut ledger = ledger();
    let first = reserve(&mut ledger, "sheet-3a", "evidence-first");
    create(
        &mut ledger,
        &first,
        "sheet-3a",
        "Reports/Q3.md",
        'a',
        ascii_case_insensitive,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));

    for (request, spelling) in [("sheet-3b", "Reports/Q3.md"), ("sheet-3c", "reports/q3.MD")] {
        let reserved = reserve(&mut ledger, request, &format!("evidence-{request}"));
        let revision = ledger.revision().unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            conflict_key(create(
                &mut ledger,
                &reserved,
                request,
                spelling,
                'b',
                ascii_case_insensitive
            )),
            "evidence.path_already_referenced",
            "{spelling}"
        );
        assert_eq!(
            ledger.revision().unwrap_or_else(|e| panic!("{e:?}")),
            revision
        );
        assert!(ledger
            .get_evidence_reference(reserved.id(), context("read").correlation_id)
            .unwrap_or_else(|e| panic!("{e:?}"))
            .is_none());
    }

    // The comparison is the caller's: under exact comparison a case-only
    // spelling is another name.
    let reserved = reserve(&mut ledger, "sheet-3d", "evidence-exact");
    create(
        &mut ledger,
        &reserved,
        "sheet-3d",
        "reports/q3.MD",
        'c',
        exact,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn the_reads_name_existing_references_by_id_and_version_only() {
    let mut ledger = ledger();
    let reserved = reserve(&mut ledger, "sheet-4a", "evidence-read-a");
    create(
        &mut ledger,
        &reserved,
        "sheet-4a",
        "Board/Minutes.md",
        'd',
        exact,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let reserved = reserve(&mut ledger, "sheet-4b", "evidence-read-b");
    create(
        &mut ledger,
        &reserved,
        "sheet-4b",
        "Board/Copy of minutes.md",
        'd',
        exact,
    )
    .unwrap_or_else(|e| panic!("{e:?}"));

    let at_path = ledger
        .find_evidence_at_vault_path(&path("board/minutes.md"), ascii_case_insensitive)
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(at_path.len(), 1);
    assert_eq!(at_path[0].id.as_str(), "evidence-read-a");
    assert_eq!(at_path[0].version.get(), 1);
    assert!(ledger
        .find_evidence_at_vault_path(&path("board/minutes.md"), exact)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_empty());

    // Same content under two paths: both, by id.
    let same = ledger
        .find_evidence_with_fingerprint(&fingerprint('d'))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        same.iter()
            .map(|found| found.id.as_str())
            .collect::<Vec<_>>(),
        ["evidence-read-a", "evidence-read-b"]
    );
    assert!(ledger
        .find_evidence_with_fingerprint(&fingerprint('e'))
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_empty());
}

#[test]
fn a_reservation_for_another_request_or_kind_cannot_create_evidence() {
    let mut ledger = ledger();
    // Reserved for this request, used under another.
    let reserved = reserve(&mut ledger, "sheet-5a", "evidence-other-request");
    assert_eq!(
        conflict_key(create(
            &mut ledger,
            &reserved,
            "sheet-5b",
            "x.md",
            'a',
            exact
        )),
        "evidence.idempotency_conflict"
    );
    // Reserved as another kind of record.
    let risk_kind = ledger
        .reserve_or_get_record_id(
            &IdempotencyId::parse("sheet-5c").unwrap_or_else(|_| panic!("id")),
            ReservationRequest {
                kind: ReservedEntityKind::Risk,
                operation: "create_risk",
            },
            at(50),
            || EvidenceReferenceId::parse("evidence-as-risk"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        conflict_key(create(
            &mut ledger,
            &risk_kind,
            "sheet-5c",
            "y.md",
            'a',
            exact
        )),
        "evidence.idempotency_conflict"
    );
}
