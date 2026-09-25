//! The desktop-facing facade for the two Evidence writes that touch the
//! filesystem: pin a fingerprint, and re-observe verification.
//!
//! `knowledge.rs` already composes each of these over
//! `pmc_knowledge::evidence::observe_evidence` and the Ledger. What this
//! module adds is the part a host needs and should not invent for itself:
//! **which Vault**, **validated when**, and **one error type** whose
//! variants a surface can map to safe errors without matching on three
//! different enums.
//!
//! ## The Vault the host may read
//!
//! [`DesktopVault`] holds a *candidate path*, never a validated handle.
//! A Vault is a directory on a live filesystem: it can appear after launch
//! (the person runs `pmc-seed`), disappear, or be swapped for something
//! else. A handle validated once at startup would make every later
//! operation trust a check from an arbitrarily long time ago, so the
//! candidate is re-validated on each use through `VaultRoot::validate`,
//! which re-applies the canonical-path and reparse-point rules at the
//! moment they matter. This does **not** close the narrower race inside a
//! single operation between resolution and read -- that Vault path TOCTOU
//! race needs handle-relative Windows APIs for a proper fix, which is
//! deliberately deferred.
//!
//! Only the Training workspace has a Vault the application may derive. A
//! Live Vault root is user-selected and explicitly configured, and
//! the surface that would configure it is S10 Settings, which is not built.
//! [`VaultUnavailable::NotConfigured`] says exactly that rather than
//! pretending a Vault is missing.
//!
//! ## What the caller supplies, and what it does not
//!
//! Neither filesystem operation takes a path. Both read the Evidence record
//! and use the path it already stores, so the boundary rule the desktop
//! write path established -- the caller supplies the identity it read and
//! its own words, and the host mints everything else -- holds without an
//! exception. Relocate, which cannot work that way because a person has to
//! choose a new path, is deliberately **not** in this module.
//!
//! The third operation, [`link_to_product`], never touches the Vault at all:
//! it is a Ledger write between two identities the person read. That is why
//! it stays available while the Vault is unavailable, and why it is scoped to
//! one target kind -- O01's Evidence tab reads `evidence_links` where
//! `target_type = 'product'`, and a generic thirteen-kind target picker is a
//! different surface with a different authorization story.
#![allow(clippy::result_large_err)]

use std::path::PathBuf;

use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::error::DomainError;
use pmc_domain::evidence::{
    EvidenceLinkRecord, EvidenceLinkTarget, EvidenceReferenceRecord, LinkEvidence, MutationOutcome,
    OperationContext,
};
use pmc_domain::identity::{
    AggregateVersion, CorrelationId, EvidenceReferenceId, IdempotencyId, ProductId,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;
use pmc_domain::DomainValueError;
use pmc_knowledge::vault::VaultRoot;
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};
use pmc_platform::settings::DeferredDirectoryPath;
use pmc_platform::workspace::WorkspaceIdentity;

use crate::knowledge::{
    pin_evidence_fingerprint, reobserve_evidence_verification, PinEvidenceFingerprintError,
    ReobserveEvidenceError, ReobserveOutcome,
};

/// Why no Vault root could be produced. Both are ordinary conditions a
/// surface should explain, not failures to hide.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VaultUnavailable {
    /// This workspace has no Vault the application may derive. True for Live
    /// until a Vault root can be configured.
    NotConfigured,
    /// A candidate exists but is not a usable Vault root right now: missing,
    /// not a directory, or a link. This is what an unseeded Training
    /// workspace looks like, and also what a swapped root looks like.
    InvalidRoot,
    /// A change of the Vault folder was interrupted and could not be
    /// resolved at startup (item ⑦). Which folder is the Vault is not known
    /// for certain, so none is used until the next start resolves it.
    ChangeUnresolved,
}

/// The candidate Product Vault for the running workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopVault {
    candidate: Option<PathBuf>,
    unresolved: bool,
}

impl DesktopVault {
    /// Derive the candidate for a resolved workspace. Touches no disk: the
    /// path comes from the already-validated workspace root, and validation
    /// happens per operation in [`Self::root`].
    ///
    /// Only Training has a derived candidate. A Live workspace's Vault is
    /// the folder the person configured, which this cannot see; the caller
    /// reads it from settings per operation and uses [`Self::at`].
    #[must_use]
    pub fn resolve(identity: &WorkspaceIdentity) -> Self {
        Self {
            candidate: identity.synthetic_vault_root(),
            unresolved: false,
        }
    }

    /// No Vault may be used: an interrupted change of the folder could not
    /// be resolved at startup, so the settings may name a folder the change
    /// never finished committing — or not name one it did.
    #[must_use]
    pub const fn change_unresolved() -> Self {
        Self {
            candidate: None,
            unresolved: true,
        }
    }

    /// The Live Vault folder as the settings hold it (item ⑦). `None` means
    /// none is configured. Touches no disk, like [`Self::resolve`]:
    /// [`Self::root`] validates it at the moment of use, so a folder on an
    /// unplugged drive reads as unavailable and not as unconfigured.
    ///
    /// The host passes the setting itself rather than a path: no path type
    /// belongs in host code (ADR 0011 and the policy check).
    #[must_use]
    pub fn configured(folder: Option<&DeferredDirectoryPath>) -> Self {
        Self {
            candidate: folder.map(|folder| folder.as_stored().to_path_buf()),
            unresolved: false,
        }
    }

    /// A freshly validated Vault root, or the reason there is none.
    pub fn root(&self) -> Result<VaultRoot, VaultUnavailable> {
        if self.unresolved {
            return Err(VaultUnavailable::ChangeUnresolved);
        }
        let candidate = self
            .candidate
            .as_ref()
            .ok_or(VaultUnavailable::NotConfigured)?;
        VaultRoot::validate(candidate).map_err(|_| VaultUnavailable::InvalidRoot)
    }
}

/// One error type for both operations. Every variant means either nothing
/// was written, or -- for [`Self::Write`] -- that the Ledger itself decided,
/// and its own safe envelope is carried through untouched.
#[derive(Debug)]
pub enum EvidenceWriteError {
    /// No Vault to read the bytes from.
    Vault(VaultUnavailable),
    /// No Evidence with this id exists in the Ledger.
    NotFound,
    /// The record has moved past the version the caller saw; refused before
    /// the filesystem was touched. Carries the version the Ledger holds now.
    VersionConflict { current: AggregateVersion },
    /// The reference already carries a pin. A pin is the identity and is
    /// never rewritten.
    AlreadyPinned,
    /// The source could not be read as a regular file inside the Vault, so
    /// there are no bytes to make an identity from. Carries what observation
    /// found instead; that observation is never persisted by the pin flow.
    SourceNotObservable(EvidenceVerification),
    /// The stored path failed Vault containment validation. Security
    /// relevant, and never a routine "file missing".
    Containment,
    /// The Ledger read failed.
    Read(DomainError),
    /// The Ledger write failed, including an expected-version conflict.
    Write(LedgerTransactionError<DomainError>),
    /// A host-minted id failed its own parser. A host bug, not a caller one.
    Id(DomainValueError),
}

impl From<VaultUnavailable> for EvidenceWriteError {
    fn from(reason: VaultUnavailable) -> Self {
        Self::Vault(reason)
    }
}

impl From<ReobserveEvidenceError> for EvidenceWriteError {
    fn from(error: ReobserveEvidenceError) -> Self {
        match error {
            ReobserveEvidenceError::EvidenceNotFound => Self::NotFound,
            ReobserveEvidenceError::VersionConflict { current } => {
                Self::VersionConflict { current }
            }
            ReobserveEvidenceError::Observation(_) => Self::Containment,
            ReobserveEvidenceError::Read(domain) => Self::Read(domain),
            ReobserveEvidenceError::Write(transaction) => Self::Write(transaction),
        }
    }
}

impl From<PinEvidenceFingerprintError> for EvidenceWriteError {
    fn from(error: PinEvidenceFingerprintError) -> Self {
        match error {
            PinEvidenceFingerprintError::EvidenceNotFound => Self::NotFound,
            PinEvidenceFingerprintError::VersionConflict { current } => {
                Self::VersionConflict { current }
            }
            PinEvidenceFingerprintError::AlreadyPinned => Self::AlreadyPinned,
            PinEvidenceFingerprintError::SourceNotObservable(observed) => {
                Self::SourceNotObservable(observed)
            }
            PinEvidenceFingerprintError::Observation(_) => Self::Containment,
            PinEvidenceFingerprintError::Read(domain) => Self::Read(domain),
            PinEvidenceFingerprintError::Write(transaction) => Self::Write(transaction),
        }
    }
}

/// What a re-observation did. `Unchanged` is a real outcome, not a failure:
/// the live state matches what was stored, so no version moved and no audit
/// event exists. A surface that reported it as success-with-a-write would be
/// claiming a Ledger effect that never happened.
#[derive(Debug)]
pub enum ReobserveResult {
    Unchanged(EvidenceReferenceRecord),
    Persisted(MutationOutcome<EvidenceReferenceRecord>),
}

/// H1-User: pin a fingerprint onto a reference created without one.
///
/// The digest is computed here, from the live file at the record's own
/// current path, and bound to that record's current version -- the Ledger
/// cannot tell a fresh digest from a stale one, so freshness is this flow's
/// guarantee. Only an `ObservedUnpinned` observation can become a pin.
#[allow(clippy::too_many_arguments)]
pub fn pin_fingerprint<I: AuditEventIdSource>(
    ledger: &mut SqliteProductLedger,
    vault: &DesktopVault,
    evidence_id: &EvidenceReferenceId,
    expected_version: AggregateVersion,
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    ids: &mut I,
    now: UtcTimestamp,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, EvidenceWriteError> {
    let root = vault.root()?;
    let audit_event_id = ids.next_audit_event_id().map_err(EvidenceWriteError::Id)?;
    Ok(pin_evidence_fingerprint(
        ledger,
        &root,
        evidence_id,
        expected_version,
        idempotency_id,
        correlation_id,
        audit_event_id,
        now,
    )?)
}

/// H1-User: re-observe one Evidence source against the live filesystem and
/// persist the outcome only if it differs from what is stored.
#[allow(clippy::too_many_arguments)]
pub fn reobserve<I: AuditEventIdSource>(
    ledger: &mut SqliteProductLedger,
    vault: &DesktopVault,
    evidence_id: &EvidenceReferenceId,
    expected_version: AggregateVersion,
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    ids: &mut I,
    now: UtcTimestamp,
) -> Result<ReobserveResult, EvidenceWriteError> {
    let root = vault.root()?;
    let audit_event_id = ids.next_audit_event_id().map_err(EvidenceWriteError::Id)?;
    Ok(
        match reobserve_evidence_verification(
            ledger,
            &root,
            evidence_id,
            expected_version,
            idempotency_id,
            correlation_id,
            audit_event_id,
            now,
        )? {
            ReobserveOutcome::Unchanged(record) => ReobserveResult::Unchanged(record),
            ReobserveOutcome::Persisted(outcome) => ReobserveResult::Persisted(outcome),
        },
    )
}

/// H1-User: link an existing Evidence reference to one Product.
///
/// No Vault, no observation, no path: the Ledger checks that both identities
/// exist, that the Evidence is at the version the person saw, that it is not
/// superseded, and that this pair is not already linked, then records the
/// link at the combined classification of the two. Every refusal is the
/// Ledger's own domain error and is carried through as [`EvidenceWriteError::Write`].
///
/// Linking does not advance the Evidence aggregate version (the supersession
/// contract says so explicitly), so a caller must re-read the Product rather
/// than assume `version + 1`.
#[allow(clippy::too_many_arguments)]
pub fn link_to_product<I: AuditEventIdSource>(
    ledger: &mut SqliteProductLedger,
    product_id: ProductId,
    evidence_id: &EvidenceReferenceId,
    expected_version: AggregateVersion,
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    ids: &mut I,
    now: UtcTimestamp,
) -> Result<MutationOutcome<EvidenceLinkRecord>, EvidenceWriteError> {
    let audit_event_id = ids.next_audit_event_id().map_err(EvidenceWriteError::Id)?;
    ledger
        .link_evidence(
            LinkEvidence {
                evidence_id: evidence_id.clone(),
                expected_evidence_version: expected_version,
                target: EvidenceLinkTarget::Product(product_id),
                context: OperationContext {
                    idempotency_id,
                    correlation_id,
                },
            },
            audit_event_id,
            now,
        )
        .map_err(EvidenceWriteError::Write)
}
