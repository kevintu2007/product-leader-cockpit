//! General-purpose Product Vault Evidence linking.
//!
//! This is a standalone, Ledger-authoritative `EvidenceReference` aggregate: a
//! record of a user-owned Product Vault source (stable Vault-relative path,
//! optional content fingerprint, availability/freshness) that can be linked to
//! any supporting target (a KPI observation, status, conclusion, Action, or
//! Decision, per `docs/domain-glossary.md`'s Evidence definition).
//!
//! It is deliberately distinct from and does not modify
//! [`crate::work_management::EvidenceReferenceMetadata`]/[`crate::work_management::EvidenceRole`],
//! which remain the narrow completion/resolution-evidence witness gate for
//! Action/Decision/Issue lifecycle transitions. The two concepts share the
//! frozen [`EvidenceReferenceId`] identity space and the frozen
//! [`crate::work_management::EvidenceVerification`]/[`crate::work_management::IntegrityDigest`]
//! value types, but this module owns the general link.
#![allow(clippy::result_large_err)]

use sha2::{Digest, Sha256};

use crate::audit::{AuditActor, AuditEvent};
use crate::classification::DataClassification;
use crate::identity::{
    ActionId, ActionRequestId, AggregateVersion, CorrelationId, DecisionId, DecisionRequestId,
    EvidenceReferenceId, IdempotencyId, InitiativeId, IssueId, KpiId, KpiObservationId,
    MilestoneId, PreparedIntentId, ProductId, ProjectId, RiskId, RoadmapId,
};
use crate::provenance::Provenance;
use crate::time::UtcTimestamp;
use crate::value::{DomainValueError, ValueErrorKind};
use crate::work_management::{EvidenceVerification, IntegrityDigest, WorkManagementRationale};

const MAX_VAULT_RELATIVE_PATH_LENGTH: usize = 400;

/// A stable path identifying an Evidence source relative to the configured
/// Product Vault root. Never an absolute path; never carries the Vault root
/// itself, so it stays safe to log, audit, and persist without exposing a
/// private machine path.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct VaultRelativePath(String);

impl VaultRelativePath {
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainValueError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainValueError::new(ValueErrorKind::Empty));
        }
        if value.len() > MAX_VAULT_RELATIVE_PATH_LENGTH {
            return Err(DomainValueError::new(ValueErrorKind::TooLong));
        }
        if value.chars().any(char::is_control) {
            return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
        }
        // Vault-relative identity is a forward-slash path relative to the
        // configured root: reject absolute paths, drive letters, backslashes,
        // and any `.`/`..`/empty segment so traversal cannot hide inside a
        // value that later reaches the filesystem adapter.
        if value.starts_with('/') || value.contains(':') || value.contains('\\') {
            return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
        }
        if value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The reviewed cryptographic content-hash algorithm used for an Evidence
/// fingerprint (the Vault design's safe default). Versioned so a future
/// algorithm can be added without breaking persisted values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FingerprintAlgorithm {
    Sha256,
}

impl FingerprintAlgorithm {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "sha256" => Ok(Self::Sha256),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

/// A content fingerprint over raw Evidence file bytes, computed with a named
/// algorithm. `digest` reuses the frozen 64 lowercase-hex-character
/// [`IntegrityDigest`] value already used by [`EvidenceVerification`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceFingerprint {
    algorithm: FingerprintAlgorithm,
    digest: IntegrityDigest,
}

impl EvidenceFingerprint {
    #[must_use]
    pub const fn new(algorithm: FingerprintAlgorithm, digest: IntegrityDigest) -> Self {
        Self { algorithm, digest }
    }

    #[must_use]
    pub const fn algorithm(&self) -> FingerprintAlgorithm {
        self.algorithm
    }

    #[must_use]
    pub const fn digest(&self) -> &IntegrityDigest {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationOutcome<T> {
    pub record: T,
    pub audit_event: AuditEvent,
}

/// The Ledger-authoritative Evidence record. Note bodies stay in the
/// user-owned Product Vault; only stable identity, source path, fingerprint,
/// verification state, and classification are ever authoritative here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceReferenceRecord {
    pub id: EvidenceReferenceId,
    pub vault_path: VaultRelativePath,
    pub fingerprint: Option<EvidenceFingerprint>,
    pub verification: EvidenceVerification,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateEvidenceReference {
    pub id: EvidenceReferenceId,
    pub vault_path: VaultRelativePath,
    pub fingerprint: Option<EvidenceFingerprint>,
    pub verification: EvidenceVerification,
    /// `None` means initial/new Evidence is Unclassified unless a trusted
    /// explicit classification exists, matching the
    /// fail-closed default used by every other `Create*` command.
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
    pub context: OperationContext,
}

/// The general-purpose target an Evidence reference can support, mirroring
/// [`crate::relationships::StakeholderSubject`]'s typed-endpoint shape.
/// Excludes Portfolio and Stakeholder: `docs/domain-glossary.md` scopes Evidence to
/// supporting "a KPI observation, status, conclusion, Action, or Decision",
/// and Action Request/Decision Request/Risk/Issue are the concrete request
/// and exception-queue objects that carry that support in practice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceLinkTarget {
    Product(ProductId),
    Initiative(InitiativeId),
    Project(ProjectId),
    Roadmap(RoadmapId),
    Milestone(MilestoneId),
    Kpi(KpiId),
    KpiObservation(KpiObservationId),
    ActionRequest(ActionRequestId),
    Action(ActionId),
    DecisionRequest(DecisionRequestId),
    Decision(DecisionId),
    Risk(RiskId),
    Issue(IssueId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceLinkRecord {
    pub evidence_id: EvidenceReferenceId,
    pub target: EvidenceLinkTarget,
    /// The most restrictive trusted classification at link time (CR2):
    /// `combine` of the Evidence's own classification and the target's.
    pub classification: DataClassification,
    pub linked_at: UtcTimestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkEvidence {
    pub evidence_id: EvidenceReferenceId,
    pub expected_evidence_version: AggregateVersion,
    pub target: EvidenceLinkTarget,
    pub context: OperationContext,
}

/// Persist a re-observed availability/freshness outcome: the
/// verification field only. `vault_path`, `fingerprint`, classification,
/// and provenance are immutable once created -- re-observing never changes
/// what Evidence is being described, only what is currently known about
/// its availability. Ordinary H1-User single-Ledger-transaction command,
/// matching every other `Update<Aggregate>Details` shape: requires
/// `expected_version` and produces `version + 1`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateEvidenceVerification {
    pub id: EvidenceReferenceId,
    pub expected_version: AggregateVersion,
    pub verification: EvidenceVerification,
    pub context: OperationContext,
}

/// Move an already-pinned Evidence reference to a new Vault-relative path
/// without changing what content it identifies (H1-User). Unlike
/// [`UpdateEvidenceVerification`], `vault_path` is the one immutable field
/// this command is allowed to change -- but only because the caller has
/// already re-hashed the file at `new_vault_path` and is asserting that
/// digest still equals the fingerprint pinned at `CreateEvidenceReference`
/// time (`observed_fingerprint`). The persistence layer re-checks that
/// assertion against the stored fingerprint before committing; it never
/// trusts the caller's digest on its own. `fingerprint`/`classification`/
/// `provenance` never change here, and the resulting `verification` always
/// becomes `Verified` (this command answers "the same content moved and I
/// just confirmed it", not "something happened to its availability" --
/// that remains [`UpdateEvidenceVerification`]'s job).
///
/// A reference with no pinned fingerprint at all cannot use this command --
/// there is nothing to prove continuity against -- and a genuinely
/// different fingerprint under the same [`EvidenceReferenceId`] is out of
/// scope entirely: relocation only ever proves "this is still the same
/// content", never "replace this identity's content". A replacement case
/// needs its own separate, non-destructive command that creates a new
/// identity rather than mutating this one -- see [`SupersedeEvidenceReference`]
/// below.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelocateEvidenceReference {
    pub id: EvidenceReferenceId,
    pub expected_version: AggregateVersion,
    pub expected_current_path: VaultRelativePath,
    pub new_vault_path: VaultRelativePath,
    pub observed_fingerprint: EvidenceFingerprint,
    pub observed_at: UtcTimestamp,
    pub context: OperationContext,
}

/// Pin a fingerprint onto a reference created without one (H1-User).
///
/// A reference created without a fingerprint has no identity to verify
/// against: every successful observation is `ObservedUnpinned`. This
/// command gives it one, from bytes the application has just read at
/// `expected_current_path` through the Vault-contained observation path.
/// The persistence layer requires the reference to be *currently unpinned*
/// and re-checks version and path before committing. It never re-pins: a
/// pin is the identity (same bytes: re-observe; moved: relocate; changed:
/// supersede).
///
/// The resulting `verification` is `Verified` with this same digest and
/// time -- the bytes that became the identity were observed at that
/// instant, exactly as relocation writes the matched fingerprint as
/// `Verified`. Freshness is the application's to guarantee: the Ledger
/// cannot tell a digest computed a second ago from one computed a week
/// ago, so the application flow observes and submits in one motion and
/// never reuses an observation across a version conflict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinEvidenceFingerprint {
    pub id: EvidenceReferenceId,
    pub expected_version: AggregateVersion,
    pub expected_current_path: VaultRelativePath,
    pub observed_fingerprint: EvidenceFingerprint,
    pub observed_at: UtcTimestamp,
    pub context: OperationContext,
}

// ============================================================================
// SupersedeEvidenceReference (H2a) -- Evidence relink, replacement case: a genuinely different fingerprint under an existing, already-linked
// EvidenceReferenceId. Deliberately its own narrow, Evidence-owned H2a
// mechanism -- NOT a `WorkManagementOperation` variant (Evidence isn't a
// Work Management aggregate, and that enum is a closed algebra the other
// five aggregate families all depend on) and NOT built on `execution.rs`'s
// Relationship H2b removal machinery (that machinery's contracts are
// removal-specific -- recovery evidence, named confirmation -- and this is
// non-destructive H2a, not H2b). Reuses only genuinely generic shared
// primitives (`PreparedIntentId`, `ApprovalReceiptId`, identity/audit/
// classification/timestamp types, the generic `prepared_intents`/
// `approval_receipts` SQL envelope); everything with real H2a *behavior*
// (preview, digest, approval, authorization) is its own type here rather
// than reusing `WorkManagementApproval`/`ApprovalAuthorizationPort`, whose
// module ownership belongs to Work Management specifically.
//
// The existing identity is never mutated in place: mutating an existing
// trusted EvidenceReferenceId's fingerprint would make every historical
// link into that identity look like it always pointed at the replacement's
// bytes. Execution instead creates a brand-new EvidenceReferenceId, clones
// every existing link onto it, and marks the source `Superseded` -- history
// stays intact on both sides.
// ============================================================================

/// Fixed five-minute expiry, matching every other H2/H2a prepared intent's
/// TTL in this codebase (see `work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS`).
pub const EVIDENCE_SUPERSESSION_TTL_MILLIS: i64 = 300_000;

/// One Evidence link as it existed when a supersession was prepared (or is
/// being re-checked at execute time): not just the link's own recorded
/// classification, but the linked target's current version/classification
/// too, since either changing between prepare and execute is drift the
/// canonical preview must catch. `LinkEvidence` does not advance the
/// Evidence aggregate's own version (see `evidence_repository.rs`), so the
/// source's `expected_source_version` alone cannot detect a changed link
/// set -- the full link snapshot is what the payload digest actually binds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionLinkSnapshot {
    pub target: EvidenceLinkTarget,
    pub target_version: AggregateVersion,
    pub target_classification: DataClassification,
    pub link_classification: DataClassification,
    pub linked_at: UtcTimestamp,
}

/// The live state a caller (the SQLite repository, which owns all Evidence
/// reads) must supply before preparing or re-validating a supersession.
/// Deliberately just data, not a rehydrated service: Evidence has no
/// `InMemoryEvidenceService`, and this operation does not need one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionSourceSnapshot {
    pub record: EvidenceReferenceRecord,
    pub links: Vec<EvidenceSupersessionLinkSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareSupersedeEvidenceReference {
    pub source_id: EvidenceReferenceId,
    pub expected_source_version: AggregateVersion,
    pub replacement_id: EvidenceReferenceId,
    pub replacement_vault_path: VaultRelativePath,
    pub replacement_fingerprint: EvidenceFingerprint,
    pub replacement_observed_at: UtcTimestamp,
    pub replacement_classification: DataClassification,
    pub replacement_provenance: Provenance,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

/// Fail-closed errors `prepare_supersede_evidence_reference` and
/// `execute_supersede_evidence_reference` can return, distinct from the
/// plain `DomainError` the SQLite repository layer uses for its own H1
/// checks -- these carry the specific H2a preview-integrity semantics
/// (changed-preview vs genuinely invalid input) the repository needs to
/// translate into the right safe error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceSupersessionError {
    /// `source_id`/`expected_source_version` do not match the supplied
    /// snapshot -- the caller passed a stale or mismatched snapshot.
    SourceMismatch,
    /// `replacement_id` names the same identity as `source_id`.
    ReplacementIsSource,
    /// The observed replacement fingerprint equals the source's own pinned
    /// fingerprint -- this is a relocation, not a replacement; use
    /// [`RelocateEvidenceReference`] instead.
    NotAGenuineReplacement,
    /// `replacement_classification` is `Unclassified`.
    UnclassifiedReplacement,
    /// `replacement_classification` is less restrictive than the source's
    /// current classification.
    ReplacementLowersClassification,
    /// The source has already been superseded by an earlier execution.
    SourceAlreadySuperseded,
    /// Execute-time only: the acknowledged digest does not match the
    /// prepared intent it claims to approve.
    DigestMismatch,
    /// Execute-time only: `now` is at or past the prepared intent's expiry.
    Expired,
    /// Execute-time only: re-deriving the preview from the freshly supplied
    /// current snapshot produced a different digest than what was prepared
    /// -- something about the source, a link, or a linked target's own
    /// version/classification changed since prepare.
    PreviewChanged,
    /// The approving actor is not `HeadOfProducts`.
    UnauthorizedActor,
    /// The approval was constructed without explicit confirmation.
    MissingConfirmation,
    /// The approval names a different prepared intent than the one supplied.
    PreparedIntentMismatch,
}

/// A committed, immutable H2a preview: every fact the eventual execution is
/// allowed to act on, and nothing else. Two independently constructed
/// previews over identical inputs always produce identical
/// [`EvidenceSupersessionPayloadDigest`] values -- this is the exactness
/// [`execute_supersede_evidence_reference`] relies on to detect drift.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionPreview {
    pub prepared_intent_id: PreparedIntentId,
    pub source_id: EvidenceReferenceId,
    pub source_version: AggregateVersion,
    pub source_vault_path: VaultRelativePath,
    pub source_fingerprint: Option<EvidenceFingerprint>,
    pub source_classification: DataClassification,
    pub replacement_id: EvidenceReferenceId,
    pub replacement_vault_path: VaultRelativePath,
    pub replacement_fingerprint: EvidenceFingerprint,
    pub replacement_observed_at: UtcTimestamp,
    pub replacement_classification: DataClassification,
    pub replacement_provenance: Provenance,
    /// Deterministically sorted (by target kind, then target ID) so the
    /// digest never depends on read order.
    pub links: Vec<EvidenceSupersessionLinkSnapshot>,
    /// The most restrictive classification across the source, the
    /// replacement, and every linked target/link -- the classification the
    /// resulting replacement and its links must carry.
    pub classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub expires_at: UtcTimestamp,
}

/// A 64-lowercase-hex-character SHA-256 digest binding an
/// [`EvidenceSupersessionPreview`] exactly, mirroring
/// `work_management::WorkManagementPayloadDigest`'s own shape and the same
/// length-prefixed-field hashing technique (see `digest_field` below).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionPayloadDigest(String);

impl EvidenceSupersessionPayloadDigest {
    #[doc(hidden)]
    pub fn from_persisted(value: String) -> Result<Self, DomainValueError> {
        if value.len() != 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_digit() || (byte.is_ascii_lowercase() && byte.is_ascii_hexdigit())
            })
        {
            return Err(DomainValueError::new(ValueErrorKind::InvalidCharacter));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionPreparedIntent {
    preview: EvidenceSupersessionPreview,
    payload_digest: EvidenceSupersessionPayloadDigest,
    created_at: UtcTimestamp,
}

impl EvidenceSupersessionPreparedIntent {
    /// Reconstruct a previously prepared, previously validated intent from
    /// durable storage. Doc-hidden: only the SQLite repository that owns
    /// this data should call it -- ordinary callers get one only from
    /// [`prepare_supersede_evidence_reference`].
    #[doc(hidden)]
    #[must_use]
    pub const fn from_persisted(
        preview: EvidenceSupersessionPreview,
        payload_digest: EvidenceSupersessionPayloadDigest,
        created_at: UtcTimestamp,
    ) -> Self {
        Self {
            preview,
            payload_digest,
            created_at,
        }
    }

    #[must_use]
    pub fn id(&self) -> &PreparedIntentId {
        &self.preview.prepared_intent_id
    }

    #[must_use]
    pub const fn preview(&self) -> &EvidenceSupersessionPreview {
        &self.preview
    }

    #[must_use]
    pub const fn payload_digest(&self) -> &EvidenceSupersessionPayloadDigest {
        &self.payload_digest
    }

    #[must_use]
    pub const fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }
}

/// H2a step 1: preview an Evidence supersession. Pure -- takes the live
/// snapshot the SQLite repository already loaded, returns a canonical,
/// digest-bound prepared intent. Nothing is mutated; the repository is
/// responsible for persisting the returned value and its own idempotency
/// bookkeeping, matching how `action_repository.rs::prepare_lower_action_classification`
/// takes an already-canonical preview and only validates/persists it.
pub fn prepare_supersede_evidence_reference(
    intent: &PrepareSupersedeEvidenceReference,
    prepared_intent_id: PreparedIntentId,
    source: &EvidenceSupersessionSourceSnapshot,
    now: UtcTimestamp,
) -> Result<EvidenceSupersessionPreparedIntent, EvidenceSupersessionError> {
    if source.record.id != intent.source_id
        || source.record.version != intent.expected_source_version
    {
        return Err(EvidenceSupersessionError::SourceMismatch);
    }
    if intent.replacement_id == intent.source_id {
        return Err(EvidenceSupersessionError::ReplacementIsSource);
    }
    if let Some(existing) = &source.record.fingerprint {
        if existing.algorithm() == intent.replacement_fingerprint.algorithm()
            && existing.digest().as_str() == intent.replacement_fingerprint.digest().as_str()
        {
            return Err(EvidenceSupersessionError::NotAGenuineReplacement);
        }
    }
    if intent.replacement_classification == DataClassification::Unclassified {
        return Err(EvidenceSupersessionError::UnclassifiedReplacement);
    }
    if intent
        .replacement_classification
        .combine(source.record.classification)
        != intent.replacement_classification
    {
        return Err(EvidenceSupersessionError::ReplacementLowersClassification);
    }
    let mut links = source.links.clone();
    links.sort_by_key(link_sort_key);
    let classification = links.iter().fold(
        source
            .record
            .classification
            .combine(intent.replacement_classification),
        |accumulated, link| {
            accumulated
                .combine(link.target_classification)
                .combine(link.link_classification)
        },
    );
    let expires_at = UtcTimestamp::from_unix_millis(
        now.unix_millis()
            .saturating_add(EVIDENCE_SUPERSESSION_TTL_MILLIS),
    );
    let preview = EvidenceSupersessionPreview {
        prepared_intent_id,
        source_id: source.record.id.clone(),
        source_version: source.record.version,
        source_vault_path: source.record.vault_path.clone(),
        source_fingerprint: source.record.fingerprint.clone(),
        source_classification: source.record.classification,
        replacement_id: intent.replacement_id.clone(),
        replacement_vault_path: intent.replacement_vault_path.clone(),
        replacement_fingerprint: intent.replacement_fingerprint.clone(),
        replacement_observed_at: intent.replacement_observed_at,
        replacement_classification: intent.replacement_classification,
        replacement_provenance: intent.replacement_provenance.clone(),
        links,
        classification,
        rationale: intent.rationale.clone(),
        expires_at,
    };
    let payload_digest = compute_evidence_supersession_digest(&preview);
    Ok(EvidenceSupersessionPreparedIntent {
        preview,
        payload_digest,
        created_at: now,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionApproval {
    prepared_id: PreparedIntentId,
    actor: AuditActor,
    acknowledged_payload_digest: EvidenceSupersessionPayloadDigest,
    idempotency_id: IdempotencyId,
}

impl EvidenceSupersessionApproval {
    /// Default-deny: only `HeadOfProducts` may approve an Evidence
    /// supersession, matching every other H2a approval authority in this
    /// codebase. Explicit non-phrase approval, matching
    /// `WorkManagementApproval::new`'s own required-confirmation shape --
    /// callers supply `confirmed: true` only after the human actually
    /// approved.
    pub fn new(
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: EvidenceSupersessionPayloadDigest,
        idempotency_id: IdempotencyId,
        confirmed: bool,
    ) -> Result<Self, EvidenceSupersessionError> {
        if actor != AuditActor::HeadOfProducts {
            return Err(EvidenceSupersessionError::UnauthorizedActor);
        }
        if !confirmed {
            return Err(EvidenceSupersessionError::MissingConfirmation);
        }
        Ok(Self {
            prepared_id,
            actor,
            acknowledged_payload_digest,
            idempotency_id,
        })
    }

    #[must_use]
    pub const fn prepared_id(&self) -> &PreparedIntentId {
        &self.prepared_id
    }

    #[must_use]
    pub const fn actor(&self) -> AuditActor {
        self.actor
    }

    #[must_use]
    pub const fn acknowledged_payload_digest(&self) -> &EvidenceSupersessionPayloadDigest {
        &self.acknowledged_payload_digest
    }

    #[must_use]
    pub const fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteSupersedeEvidenceReference {
    pub approval: EvidenceSupersessionApproval,
    pub context: OperationContext,
}

/// What execution actually does, described as data -- the SQLite repository
/// is responsible for turning this into real writes inside one transaction
/// (create the replacement, clone the links, mark the source superseded,
/// audit, advance the Ledger revision).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSupersessionEffects {
    pub replacement: EvidenceReferenceRecord,
    pub replacement_links: Vec<EvidenceLinkRecord>,
    pub source_id: EvidenceReferenceId,
    pub source_new_version: AggregateVersion,
}

/// H2a step 2: re-validate a previously prepared supersession against a
/// freshly supplied current snapshot and, only if nothing has drifted,
/// return the effects to persist. Pure, like `prepare_supersede_evidence_reference` --
/// mutates nothing itself.
pub fn execute_supersede_evidence_reference(
    approval: &EvidenceSupersessionApproval,
    prepared: &EvidenceSupersessionPreparedIntent,
    current: &EvidenceSupersessionSourceSnapshot,
    already_superseded: bool,
    now: UtcTimestamp,
) -> Result<EvidenceSupersessionEffects, EvidenceSupersessionError> {
    if approval.prepared_id() != prepared.id() {
        return Err(EvidenceSupersessionError::PreparedIntentMismatch);
    }
    if approval.acknowledged_payload_digest().as_str() != prepared.payload_digest().as_str() {
        return Err(EvidenceSupersessionError::DigestMismatch);
    }
    if now.unix_millis() >= prepared.preview().expires_at.unix_millis() {
        return Err(EvidenceSupersessionError::Expired);
    }
    if already_superseded {
        return Err(EvidenceSupersessionError::SourceAlreadySuperseded);
    }
    let preview = prepared.preview();
    if current.record.id != preview.source_id || current.record.version != preview.source_version {
        return Err(EvidenceSupersessionError::PreviewChanged);
    }
    let mut current_links = current.links.clone();
    current_links.sort_by_key(link_sort_key);
    if current_links != preview.links {
        return Err(EvidenceSupersessionError::PreviewChanged);
    }
    let replacement = EvidenceReferenceRecord {
        id: preview.replacement_id.clone(),
        vault_path: preview.replacement_vault_path.clone(),
        fingerprint: Some(preview.replacement_fingerprint.clone()),
        verification: EvidenceVerification::Verified {
            verified_at: preview.replacement_observed_at,
            integrity_digest: preview.replacement_fingerprint.digest().clone(),
        },
        classification: preview.classification,
        provenance: preview.replacement_provenance.clone(),
        version: AggregateVersion::initial(),
        created_at: now,
        updated_at: now,
    };
    let replacement_links = preview
        .links
        .iter()
        .map(|link| EvidenceLinkRecord {
            evidence_id: preview.replacement_id.clone(),
            target: link.target.clone(),
            classification: preview.classification,
            linked_at: now,
        })
        .collect();
    let source_new_version = preview
        .source_version
        .next()
        .ok_or(EvidenceSupersessionError::SourceMismatch)?;
    Ok(EvidenceSupersessionEffects {
        replacement,
        replacement_links,
        source_id: preview.source_id.clone(),
        source_new_version,
    })
}

fn link_sort_key(link: &EvidenceSupersessionLinkSnapshot) -> (u8, String) {
    (
        link_target_discriminant(&link.target),
        link_target_id(&link.target),
    )
}

const fn link_target_discriminant(target: &EvidenceLinkTarget) -> u8 {
    match target {
        EvidenceLinkTarget::Product(_) => 0,
        EvidenceLinkTarget::Initiative(_) => 1,
        EvidenceLinkTarget::Project(_) => 2,
        EvidenceLinkTarget::Roadmap(_) => 3,
        EvidenceLinkTarget::Milestone(_) => 4,
        EvidenceLinkTarget::Kpi(_) => 5,
        EvidenceLinkTarget::KpiObservation(_) => 6,
        EvidenceLinkTarget::ActionRequest(_) => 7,
        EvidenceLinkTarget::Action(_) => 8,
        EvidenceLinkTarget::DecisionRequest(_) => 9,
        EvidenceLinkTarget::Decision(_) => 10,
        EvidenceLinkTarget::Risk(_) => 11,
        EvidenceLinkTarget::Issue(_) => 12,
    }
}

fn link_target_id(target: &EvidenceLinkTarget) -> String {
    match target {
        EvidenceLinkTarget::Product(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Initiative(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Project(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Roadmap(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Milestone(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Kpi(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::KpiObservation(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::ActionRequest(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Action(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::DecisionRequest(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Decision(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Risk(id) => id.as_str().to_owned(),
        EvidenceLinkTarget::Issue(id) => id.as_str().to_owned(),
    }
}

fn digest_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn digest_provenance(digest: &mut Sha256, provenance: &Provenance) {
    digest_field(digest, provenance.kind_persisted().as_bytes());
    match provenance.reference() {
        Some(reference) => {
            digest_field(digest, b"1");
            digest_field(digest, reference.as_str().as_bytes());
        }
        None => digest_field(digest, b"0"),
    }
}

fn digest_link(digest: &mut Sha256, link: &EvidenceSupersessionLinkSnapshot) {
    digest_field(digest, &[link_target_discriminant(&link.target)]);
    digest_field(digest, link_target_id(&link.target).as_bytes());
    digest_field(digest, &link.target_version.get().to_be_bytes());
    digest_field(digest, link.target_classification.as_persisted().as_bytes());
    digest_field(digest, link.link_classification.as_persisted().as_bytes());
    digest_field(digest, &link.linked_at.unix_millis().to_be_bytes());
}

fn compute_evidence_supersession_digest(
    preview: &EvidenceSupersessionPreview,
) -> EvidenceSupersessionPayloadDigest {
    let mut digest = Sha256::new();
    digest_field(
        &mut digest,
        b"product-mission-control.evidence-supersession.prepared-intent.v1",
    );
    digest_field(&mut digest, preview.prepared_intent_id.as_str().as_bytes());
    digest_field(&mut digest, preview.source_id.as_str().as_bytes());
    digest_field(&mut digest, &preview.source_version.get().to_be_bytes());
    digest_field(&mut digest, preview.source_vault_path.as_str().as_bytes());
    match &preview.source_fingerprint {
        Some(fingerprint) => {
            digest_field(&mut digest, b"1");
            digest_field(
                &mut digest,
                fingerprint.algorithm().as_persisted().as_bytes(),
            );
            digest_field(&mut digest, fingerprint.digest().as_str().as_bytes());
        }
        None => digest_field(&mut digest, b"0"),
    }
    digest_field(
        &mut digest,
        preview.source_classification.as_persisted().as_bytes(),
    );
    digest_field(&mut digest, preview.replacement_id.as_str().as_bytes());
    digest_field(
        &mut digest,
        preview.replacement_vault_path.as_str().as_bytes(),
    );
    digest_field(
        &mut digest,
        preview
            .replacement_fingerprint
            .algorithm()
            .as_persisted()
            .as_bytes(),
    );
    digest_field(
        &mut digest,
        preview.replacement_fingerprint.digest().as_str().as_bytes(),
    );
    digest_field(
        &mut digest,
        &preview.replacement_observed_at.unix_millis().to_be_bytes(),
    );
    digest_field(
        &mut digest,
        preview.replacement_classification.as_persisted().as_bytes(),
    );
    digest_provenance(&mut digest, &preview.replacement_provenance);
    digest_field(&mut digest, &(preview.links.len() as u64).to_be_bytes());
    for link in &preview.links {
        digest_link(&mut digest, link);
    }
    digest_field(
        &mut digest,
        preview.classification.as_persisted().as_bytes(),
    );
    digest_field(&mut digest, preview.rationale.as_str().as_bytes());
    digest_field(&mut digest, &preview.expires_at.unix_millis().to_be_bytes());
    EvidenceSupersessionPayloadDigest(format!("{:x}", digest.finalize()))
}
