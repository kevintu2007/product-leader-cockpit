//! Knowledge workflow composition (Evidence/Vault): the actual
//! "observe the live filesystem, then persist" wiring between
//! `pmc_knowledge`'s pure filesystem observation and `pmc_ledger`'s SQLite
//! persistence. Neither of those crates depends on the other; this is the
//! one place that does, as the ownership of
//! `crates/pmc-application/src/knowledge/` prescribes.
#![allow(clippy::result_large_err)]

use pmc_domain::error::DomainError;
use pmc_domain::evidence::{
    EvidenceFingerprint, EvidenceReferenceRecord, FingerprintAlgorithm, MutationOutcome,
    OperationContext, PinEvidenceFingerprint, UpdateEvidenceVerification,
};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;
use pmc_knowledge::evidence::{observe_evidence, EvidenceObservationError};
use pmc_knowledge::vault::VaultRoot;
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};

/// Fail-closed errors for [`reobserve_evidence_verification`].
#[derive(Debug)]
pub enum ReobserveEvidenceError {
    /// No Evidence with this ID exists in the Ledger.
    EvidenceNotFound,
    /// The record has moved past the version the caller saw. Refused before
    /// the filesystem is touched: an observation made on behalf of a view
    /// the person was no longer looking at must not be written. Carries the
    /// version the Ledger holds now, so the caller can re-read and decide.
    VersionConflict { current: AggregateVersion },
    /// The resolved path failed Vault containment validation (a
    /// security-relevant condition -- see
    /// `pmc_knowledge::evidence::EvidenceObservationError`).
    Observation(EvidenceObservationError),
    /// The Ledger read that fetched the current record failed.
    Read(DomainError),
    /// The Ledger write that persisted the new verification state failed.
    Write(LedgerTransactionError<DomainError>),
}

/// What happened after re-observing one Evidence source.
#[derive(Debug)]
pub enum ReobserveOutcome {
    /// The freshly observed state matches what was already stored: no
    /// Ledger write happened, so there is no new version or audit event.
    Unchanged(EvidenceReferenceRecord),
    /// The observed state differed from what was stored and has been
    /// persisted.
    Persisted(MutationOutcome<EvidenceReferenceRecord>),
}

/// Re-observe one Evidence source against the live filesystem and persist
/// the outcome back to the Ledger -- the composition point between
/// `pmc_knowledge::evidence::observe_evidence` (pure filesystem read) and
/// `SqliteProductLedger::update_evidence_verification` (persistence).
///
/// Reads the Evidence's current record first (for its `vault_path`, pinned
/// `fingerprint`, current `verification`, and `version`), so callers only
/// need to supply the Evidence's identity and the version they saw, not its
/// whole state. Callers must still supply a fresh `idempotency_id`/
/// `correlation_id`/`audit_event_id` per invocation -- this function does
/// not decide *when* to re-observe (a cadence, an on-demand request), only
/// *how*.
///
/// `expected_version` is the version the caller acted on. The Ledger's own
/// expected-version check catches a change made *after* this function's
/// read; it cannot tell that the caller's view was already stale *before*
/// it. Comparing here, before observing, closes that half.
#[allow(clippy::too_many_arguments)]
pub fn reobserve_evidence_verification(
    ledger: &mut SqliteProductLedger,
    vault: &VaultRoot,
    evidence_id: &EvidenceReferenceId,
    expected_version: AggregateVersion,
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    audit_event_id: AuditEventId,
    now: UtcTimestamp,
) -> Result<ReobserveOutcome, ReobserveEvidenceError> {
    let current = ledger
        .get_evidence_reference(evidence_id, correlation_id.clone())
        .map_err(ReobserveEvidenceError::Read)?
        .ok_or(ReobserveEvidenceError::EvidenceNotFound)?;
    if current.version != expected_version {
        return Err(ReobserveEvidenceError::VersionConflict {
            current: current.version,
        });
    }

    let observed = observe_evidence(
        vault,
        &current.vault_path,
        current
            .fingerprint
            .as_ref()
            .map(pmc_domain::evidence::EvidenceFingerprint::digest),
        &current.verification,
        now,
    )
    .map_err(ReobserveEvidenceError::Observation)?;

    if observed == current.verification {
        return Ok(ReobserveOutcome::Unchanged(current));
    }

    let outcome = ledger
        .update_evidence_verification(
            UpdateEvidenceVerification {
                id: evidence_id.clone(),
                expected_version: current.version,
                verification: observed,
                context: OperationContext {
                    idempotency_id,
                    correlation_id,
                },
            },
            audit_event_id,
            now,
        )
        .map_err(ReobserveEvidenceError::Write)?;
    Ok(ReobserveOutcome::Persisted(outcome))
}

/// Fail-closed errors for [`pin_evidence_fingerprint`]. Every variant means
/// nothing was written.
#[derive(Debug)]
pub enum PinEvidenceFingerprintError {
    /// No Evidence with this ID exists in the Ledger.
    EvidenceNotFound,
    /// The record has moved past the version the caller saw. Refused before
    /// the filesystem is touched, for the reason given on
    /// [`ReobserveEvidenceError::VersionConflict`].
    VersionConflict { current: AggregateVersion },
    /// The reference already carries a pin. A pin is the identity and is
    /// never rewritten: same bytes are re-observed, a move is a relocation,
    /// different bytes are a supersession.
    AlreadyPinned,
    /// The source could not be read as a regular file inside the Vault, so
    /// there are no bytes to make an identity from. Carries what observation
    /// found instead, which is never persisted by this flow.
    SourceNotObservable(EvidenceVerification),
    /// The resolved path failed Vault containment validation.
    Observation(EvidenceObservationError),
    /// The Ledger read that fetched the current record failed.
    Read(DomainError),
    /// The Ledger write failed -- including an expected-version conflict,
    /// which means the record changed between the read and the pin. The
    /// caller must re-run the whole flow with a fresh idempotency key; the
    /// observation is not reused.
    Write(LedgerTransactionError<DomainError>),
}

/// Fingerprint pin -- pin a fingerprint onto a reference created without
/// one: read the record, hash the live file through the Vault-contained
/// observation path, and submit the typed result in one motion.
///
/// The Ledger cannot tell a fresh digest from a stale one, so this function
/// is where freshness is guaranteed: the digest submitted is the one just
/// computed at the record's current path, bound to that record's current
/// version and path, and never carried across a conflict. Only an
/// `ObservedUnpinned` observation can become a pin; anything else --
/// missing, unreadable, not a regular file -- is refused without a write.
#[allow(clippy::too_many_arguments)]
pub fn pin_evidence_fingerprint(
    ledger: &mut SqliteProductLedger,
    vault: &VaultRoot,
    evidence_id: &EvidenceReferenceId,
    expected_version: AggregateVersion,
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    audit_event_id: AuditEventId,
    now: UtcTimestamp,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, PinEvidenceFingerprintError> {
    let current = ledger
        .get_evidence_reference(evidence_id, correlation_id.clone())
        .map_err(PinEvidenceFingerprintError::Read)?
        .ok_or(PinEvidenceFingerprintError::EvidenceNotFound)?;
    if current.version != expected_version {
        return Err(PinEvidenceFingerprintError::VersionConflict {
            current: current.version,
        });
    }
    if current.fingerprint.is_some() {
        return Err(PinEvidenceFingerprintError::AlreadyPinned);
    }

    let observed = observe_evidence(vault, &current.vault_path, None, &current.verification, now)
        .map_err(PinEvidenceFingerprintError::Observation)?;
    let EvidenceVerification::ObservedUnpinned {
        observed_at,
        integrity_digest,
    } = observed
    else {
        return Err(PinEvidenceFingerprintError::SourceNotObservable(observed));
    };

    ledger
        .pin_evidence_fingerprint(
            PinEvidenceFingerprint {
                id: evidence_id.clone(),
                expected_version: current.version,
                expected_current_path: current.vault_path.clone(),
                observed_fingerprint: EvidenceFingerprint::new(
                    FingerprintAlgorithm::Sha256,
                    integrity_digest,
                ),
                observed_at,
                context: OperationContext {
                    idempotency_id,
                    correlation_id,
                },
            },
            audit_event_id,
            now,
        )
        .map_err(PinEvidenceFingerprintError::Write)
}
