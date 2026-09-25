//! H2a governance for publishing a managed Projection change set.
//!
//! Deliberately its own narrow, Projection-owned H2a mechanism -- NOT a
//! [`crate::work_management::WorkManagementOperation`] variant (Projection
//! publish is not a Work Management aggregate operation; that enum is a
//! closed algebra the five Work Management aggregate families depend on)
//! and NOT built on [`crate::execution`]'s Relationship H2b removal
//! machinery (that machinery's contracts are removal-specific -- recovery
//! evidence, named confirmation -- and this is non-destructive H2a, not
//! H2b). Follows the precedent set by Evidence Supersession
//! (`crate::evidence`'s `EvidenceSupersession*` types): reuses only
//! genuinely generic shared primitives ([`PreparedIntentId`],
//! `ApprovalReceiptId` at the future persistence layer, identity/audit/
//! timestamp types, [`WorkManagementRationale`]); everything with real H2a
//! *behavior*
//! (preview, digest, approval) is its own type here.
//!
//! Unlike every other H2a mechanism in this codebase, this one does not
//! mutate a Ledger aggregate at all -- there is no record this operation
//! creates, updates, or transitions. Its only effect is publishing managed
//! files to the Product Vault; a repository layer is responsible for
//! turning the effects this module returns into real filesystem writes
//! (through a narrow atomic-replacement port) and for recording the
//! resulting manifest as "what's now published" for the next diff.
//!
//! `pmc-domain` cannot depend on `pmc-knowledge` (the established
//! dependency direction runs the other way -- `pmc-knowledge` depends on
//! `pmc-domain`, never the reverse), so [`ManagedProjectionRebuildRecordKind`]
//! and [`ManagedProjectionRebuildChangeKind`] are independently defined here
//! rather than reusing `pmc_knowledge::projections::ProjectionRecordType`/
//! `ProjectionArtifactChange`. The publication orchestration caller converts
//! between the two.
#![allow(clippy::result_large_err)]

use sha2::{Digest, Sha256};

use crate::audit::AuditActor;
use crate::classification::DataClassification;
use crate::identity::{CorrelationId, IdempotencyId, PreparedIntentId};
use crate::time::UtcTimestamp;
use crate::value::{DomainValueError, ValueErrorKind};
use crate::work_management::WorkManagementRationale;

/// Fixed five-minute expiry, matching every other H2/H2a prepared intent's
/// TTL in this codebase (see `work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS`
/// and `evidence::EVIDENCE_SUPERSESSION_TTL_MILLIS`).
pub const MANAGED_PROJECTION_REBUILD_TTL_MILLIS: i64 = 300_000;

/// ADR 0007's accepted H1-Auto ceiling, provisional until measured. It
/// lives here rather than beside the classifier because the Ledger must be
/// able to re-check it when admitting an H1-Auto publication, and
/// `pmc-ledger` cannot reach `pmc-knowledge`. One definition, two readers.
pub const MANAGED_PROJECTION_H1_AUTO_MAX_FILES: usize = 500;
pub const MANAGED_PROJECTION_H1_AUTO_MAX_DURATION_MILLIS: u64 = 30_000;

/// Proof that an H1-Auto publication is genuinely within ADR 0007's bounds.
///
/// This type exists so the persistence layer cannot be told "this one is
/// H1-Auto" and believe it. The only way to obtain one is
/// [`Self::authorize`], which re-checks every predicate itself: both
/// ceilings, that the work is incremental rather than an initial or full
/// rebuild, and that no integrity conflict is present. ADR 0007 also
/// forbids splitting one logical operation into smaller batches to slip
/// under the ceiling; that is upheld upstream, where the change set is only
/// ever produced whole.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedProjectionH1AutoAuthorization {
    change_count: usize,
    estimated_duration_millis: u64,
}

impl ManagedProjectionH1AutoAuthorization {
    pub fn authorize(
        change_count: usize,
        estimated_duration_millis: u64,
        is_initial_or_full_rebuild: bool,
        integrity_conflict: bool,
    ) -> Result<Self, ManagedProjectionRebuildError> {
        if change_count == 0 {
            return Err(ManagedProjectionRebuildError::EmptyChangeSet);
        }
        if integrity_conflict
            || is_initial_or_full_rebuild
            || change_count > MANAGED_PROJECTION_H1_AUTO_MAX_FILES
            || estimated_duration_millis > MANAGED_PROJECTION_H1_AUTO_MAX_DURATION_MILLIS
        {
            return Err(ManagedProjectionRebuildError::H1AutoNotPermitted);
        }
        Ok(Self {
            change_count,
            estimated_duration_millis,
        })
    }

    #[must_use]
    pub const fn change_count(self) -> usize {
        self.change_count
    }

    #[must_use]
    pub const fn estimated_duration_millis(self) -> u64 {
        self.estimated_duration_millis
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedProjectionRebuildRecordKind {
    Product,
    Project,
    Action,
    Decision,
    Risk,
    Kpi,
}

impl ManagedProjectionRebuildRecordKind {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Project => "project",
            Self::Action => "action",
            Self::Decision => "decision",
            Self::Risk => "risk",
            Self::Kpi => "kpi",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "product" => Ok(Self::Product),
            "project" => Ok(Self::Project),
            "action" => Ok(Self::Action),
            "decision" => Ok(Self::Decision),
            "risk" => Ok(Self::Risk),
            "kpi" => Ok(Self::Kpi),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedProjectionRebuildChangeKind {
    Created,
    Updated,
    Removed,
}

impl ManagedProjectionRebuildChangeKind {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Removed => "removed",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "created" => Ok(Self::Created),
            "updated" => Ok(Self::Updated),
            "removed" => Ok(Self::Removed),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

/// One artifact this rebuild will create, update, or remove -- bound
/// exactly into the preview's digest so execute-time drift (a different
/// file changing, or the same file changing again between prepare and
/// execute) is caught.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionRebuildChangeEntry {
    pub relative_path: String,
    pub record_kind: ManagedProjectionRebuildRecordKind,
    pub record_id: String,
    pub change: ManagedProjectionRebuildChangeKind,
    /// The honest classification of the file this entry acts on: the newly
    /// generated file's for `Created`/`Updated`, and the previously
    /// published file's (read from the baseline manifest) for `Removed`.
    pub classification: DataClassification,
    /// The new file's complete-file hash for `Created`/`Updated`; `None`
    /// for `Removed`, since nothing is written for a removal.
    pub expected_final_file_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareRebuildManagedProjections {
    pub ledger_schema_version: u32,
    pub ledger_revision: u64,
    /// A digest over the exact "currently published" manifest this change
    /// set was diffed against -- binds the baseline, not just the result,
    /// so a concurrent publish landing between prepare and execute is also
    /// caught as drift, not only a concurrent Ledger change.
    pub published_baseline_digest: String,
    /// Must be sorted by `relative_path` with no duplicate path -- this is
    /// validated, not assumed, since it is what makes the preview
    /// canonical and the digest reproducible from independently-computed
    /// inputs.
    pub changes: Vec<ManagedProjectionRebuildChangeEntry>,
    pub estimated_duration_millis: u64,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

/// Fail-closed errors `prepare_rebuild_managed_projections` and
/// `execute_rebuild_managed_projections` can return.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedProjectionRebuildError {
    /// `changes` is empty -- there is nothing to prepare.
    EmptyChangeSet,
    /// `changes` is not sorted by `relative_path`, or contains a duplicate
    /// path.
    ChangesNotCanonical,
    /// Execute-time only: the acknowledged digest does not match the
    /// prepared intent it claims to approve.
    DigestMismatch,
    /// Execute-time only: `now` is at or past the prepared intent's expiry.
    Expired,
    /// Execute-time only: re-deriving from freshly supplied current state
    /// (Ledger revision/schema, published baseline, or the change set
    /// itself) produced a different digest than what was prepared --
    /// something changed since prepare.
    PreviewChanged,
    /// The approving actor is not `HeadOfProducts`.
    UnauthorizedActor,
    /// The approval was constructed without explicit confirmation.
    MissingConfirmation,
    /// The approval names a different prepared intent than the one supplied.
    PreparedIntentMismatch,
    /// The work does not satisfy ADR 0007's H1-Auto bounds, so it must be
    /// escalated to H2a rather than automated.
    H1AutoNotPermitted,
    /// Another publication is still `publishing` or `cancelling`. ADR 0007
    /// writes files outside any Ledger transaction, so two publications in
    /// flight would race each other on disk with no record of which bytes
    /// won. A second one is refused until the first is terminal -- and a
    /// crashed first one stays non-terminal until crash recovery for
    /// interrupted publications resolves it, deliberately.
    PublicationInFlight,
}

/// A committed, immutable H2a preview: every fact the eventual execution is
/// allowed to act on, and nothing else. Two independently constructed
/// previews over identical inputs always produce identical
/// [`ManagedProjectionRebuildPayloadDigest`] values -- this is the exactness
/// [`execute_rebuild_managed_projections`] relies on to detect drift.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionRebuildPreview {
    pub prepared_intent_id: PreparedIntentId,
    pub ledger_schema_version: u32,
    pub ledger_revision: u64,
    pub published_baseline_digest: String,
    pub changes: Vec<ManagedProjectionRebuildChangeEntry>,
    /// The most restrictive classification across every file this rebuild
    /// touches -- the effective classification the whole operation carries,
    /// derived (never caller-supplied) by folding each entry's own
    /// classification through [`DataClassification::combine`], exactly as
    /// Evidence Supersession folds its source/replacement/link
    /// classifications. Every H2a preview carries a classification, and the
    /// shared `prepared_intents` row this is
    /// persisted into declares it NOT NULL.
    pub classification: DataClassification,
    pub estimated_duration_millis: u64,
    pub rationale: WorkManagementRationale,
    pub expires_at: UtcTimestamp,
}

/// A 64-lowercase-hex-character SHA-256 digest binding a
/// [`ManagedProjectionRebuildPreview`] exactly, mirroring
/// `evidence::EvidenceSupersessionPayloadDigest`'s own shape and the same
/// length-prefixed-field hashing technique (see `digest_field` below).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionRebuildPayloadDigest(String);

impl ManagedProjectionRebuildPayloadDigest {
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
pub struct ManagedProjectionRebuildPreparedIntent {
    preview: ManagedProjectionRebuildPreview,
    payload_digest: ManagedProjectionRebuildPayloadDigest,
    created_at: UtcTimestamp,
}

impl ManagedProjectionRebuildPreparedIntent {
    /// Reconstruct a previously prepared, previously validated intent from
    /// durable storage. Doc-hidden: only the repository that owns this
    /// data should call it -- ordinary callers get one only from
    /// [`prepare_rebuild_managed_projections`].
    #[doc(hidden)]
    #[must_use]
    pub const fn from_persisted(
        preview: ManagedProjectionRebuildPreview,
        payload_digest: ManagedProjectionRebuildPayloadDigest,
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
    pub const fn preview(&self) -> &ManagedProjectionRebuildPreview {
        &self.preview
    }

    #[must_use]
    pub const fn payload_digest(&self) -> &ManagedProjectionRebuildPayloadDigest {
        &self.payload_digest
    }

    #[must_use]
    pub const fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }
}

/// H2a step 1: preview a Projection rebuild. Pure -- takes the already
/// diffed, canonically sorted change set a caller computed (via
/// `pmc_knowledge::projections::diff_against_published`, then converted
/// into this module's own entry shape), returns a canonical, digest-bound
/// prepared intent. Nothing is mutated.
pub fn prepare_rebuild_managed_projections(
    intent: &PrepareRebuildManagedProjections,
    prepared_intent_id: PreparedIntentId,
    now: UtcTimestamp,
) -> Result<ManagedProjectionRebuildPreparedIntent, ManagedProjectionRebuildError> {
    if intent.changes.is_empty() {
        return Err(ManagedProjectionRebuildError::EmptyChangeSet);
    }
    if intent
        .changes
        .windows(2)
        .any(|pair| pair[0].relative_path >= pair[1].relative_path)
    {
        return Err(ManagedProjectionRebuildError::ChangesNotCanonical);
    }
    let expires_at = UtcTimestamp::from_unix_millis(
        now.unix_millis()
            .saturating_add(MANAGED_PROJECTION_REBUILD_TTL_MILLIS),
    );
    // Fold, never trust a caller-supplied value: the operation's effective
    // classification is the most restrictive one across every file it
    // touches. `combine` treats Unclassified as most restrictive, so an
    // unclassified file forces the whole rebuild to Unclassified rather
    // than silently publishing under a weaker label.
    let classification = intent
        .changes
        .iter()
        .fold(DataClassification::Public, |accumulated, entry| {
            accumulated.combine(entry.classification)
        });
    let preview = ManagedProjectionRebuildPreview {
        prepared_intent_id,
        ledger_schema_version: intent.ledger_schema_version,
        ledger_revision: intent.ledger_revision,
        published_baseline_digest: intent.published_baseline_digest.clone(),
        changes: intent.changes.clone(),
        classification,
        estimated_duration_millis: intent.estimated_duration_millis,
        rationale: intent.rationale.clone(),
        expires_at,
    };
    let payload_digest = compute_managed_projection_rebuild_digest(&preview);
    Ok(ManagedProjectionRebuildPreparedIntent {
        preview,
        payload_digest,
        created_at: now,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionRebuildApproval {
    prepared_id: PreparedIntentId,
    actor: AuditActor,
    acknowledged_payload_digest: ManagedProjectionRebuildPayloadDigest,
    idempotency_id: IdempotencyId,
}

impl ManagedProjectionRebuildApproval {
    /// Default-deny: only `HeadOfProducts` may approve a Projection
    /// rebuild, matching every other H2a approval authority in this
    /// codebase. Explicit non-phrase approval, matching
    /// `EvidenceSupersessionApproval::new`'s own required-confirmation
    /// shape -- callers supply `confirmed: true` only after the human
    /// actually approved.
    pub fn new(
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: ManagedProjectionRebuildPayloadDigest,
        idempotency_id: IdempotencyId,
        confirmed: bool,
    ) -> Result<Self, ManagedProjectionRebuildError> {
        if actor != AuditActor::HeadOfProducts {
            return Err(ManagedProjectionRebuildError::UnauthorizedActor);
        }
        if !confirmed {
            return Err(ManagedProjectionRebuildError::MissingConfirmation);
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
    pub const fn acknowledged_payload_digest(&self) -> &ManagedProjectionRebuildPayloadDigest {
        &self.acknowledged_payload_digest
    }

    #[must_use]
    pub const fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteRebuildManagedProjections {
    pub approval: ManagedProjectionRebuildApproval,
    pub context: OperationContext,
}

/// What execution actually does, described as data -- a repository layer
/// is responsible for turning this into real filesystem writes (through an
/// atomic-replacement port) and for recording the resulting manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionRebuildEffects {
    pub changes: Vec<ManagedProjectionRebuildChangeEntry>,
}

/// What actually happened to one file during publication. ADR 0007 requires
/// a cancelled or partial rebuild to enumerate committed, failed and
/// untouched paths rather than reporting a single aggregate outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedProjectionPublishItemState {
    Untouched,
    Committed,
    Failed,
}

impl ManagedProjectionPublishItemState {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Untouched => "untouched",
            Self::Committed => "committed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionPublishItem {
    pub relative_path: String,
    pub state: ManagedProjectionPublishItemState,
}

/// Terminal status of a publication. Orthogonal to effect scope, per ADR
/// 0006: a `Cancelled` operation can still have committed some files, and
/// the two are reported separately rather than collapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedProjectionPublishStatus {
    Succeeded,
    Failed,
    Cancelled,
}

impl ManagedProjectionPublishStatus {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// How much of the intended effect actually landed. Derived from the item
/// states rather than supplied, so it cannot be misreported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedProjectionEffectScope {
    None,
    Partial,
    Complete,
}

impl ManagedProjectionEffectScope {
    #[must_use]
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Partial => "partial",
            Self::Complete => "complete",
        }
    }

    /// `Complete` only when every item committed; `None` when none did.
    #[must_use]
    pub fn derive(items: &[ManagedProjectionPublishItem]) -> Self {
        let committed = items
            .iter()
            .filter(|item| item.state == ManagedProjectionPublishItemState::Committed)
            .count();
        if committed == 0 {
            Self::None
        } else if committed == items.len() {
            Self::Complete
        } else {
            Self::Partial
        }
    }
}

/// One row of a verified manifest generation -- what is now published.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionManifestEntry {
    pub relative_path: String,
    pub record_kind: ManagedProjectionRebuildRecordKind,
    pub record_id: String,
    pub source_revision: u64,
    pub classification: DataClassification,
    pub managed_payload_sha256: String,
    pub final_file_sha256: String,
}

/// An immutable manifest generation, supplied only when the complete
/// published set has been verified on disk. Its absence is what keeps a
/// partially-published Vault `Out-of-sync` against the previous verified
/// generation, per ADR 0007's rule that Synchronized is an integrity
/// result and not a publish-attempt result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionManifestGeneration {
    pub manifest_id: String,
    pub ledger_schema_version: u32,
    pub ledger_revision: u64,
    pub manifest_digest: String,
    pub entries: Vec<ManagedProjectionManifestEntry>,
}

/// The result of a publication attempt, handed back to the Ledger to make
/// terminal. `manifest` is `Some` only when the whole set verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedProjectionPublishReport {
    pub status: ManagedProjectionPublishStatus,
    pub items: Vec<ManagedProjectionPublishItem>,
    pub manifest: Option<ManagedProjectionManifestGeneration>,
}

/// H2a step 2: re-validate a previously prepared rebuild against freshly
/// supplied current state and, only if nothing has drifted, return the
/// effects to publish. Pure, like `prepare_rebuild_managed_projections` -- mutates
/// nothing itself. `current_changes` is expected to come from re-running
/// generate+diff against the current Ledger and the current published
/// baseline, exactly as prepare-time did.
pub fn execute_rebuild_managed_projections(
    approval: &ManagedProjectionRebuildApproval,
    prepared: &ManagedProjectionRebuildPreparedIntent,
    current_ledger_schema_version: u32,
    current_ledger_revision: u64,
    current_published_baseline_digest: &str,
    current_changes: &[ManagedProjectionRebuildChangeEntry],
    now: UtcTimestamp,
) -> Result<ManagedProjectionRebuildEffects, ManagedProjectionRebuildError> {
    if approval.prepared_id() != prepared.id() {
        return Err(ManagedProjectionRebuildError::PreparedIntentMismatch);
    }
    if approval.acknowledged_payload_digest().as_str() != prepared.payload_digest().as_str() {
        return Err(ManagedProjectionRebuildError::DigestMismatch);
    }
    if now.unix_millis() >= prepared.preview().expires_at.unix_millis() {
        return Err(ManagedProjectionRebuildError::Expired);
    }
    let preview = prepared.preview();
    if current_ledger_schema_version != preview.ledger_schema_version
        || current_ledger_revision != preview.ledger_revision
        || current_published_baseline_digest != preview.published_baseline_digest
        || current_changes != preview.changes.as_slice()
    {
        return Err(ManagedProjectionRebuildError::PreviewChanged);
    }
    Ok(ManagedProjectionRebuildEffects {
        changes: preview.changes.clone(),
    })
}

fn digest_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn digest_change_entry(digest: &mut Sha256, entry: &ManagedProjectionRebuildChangeEntry) {
    digest_field(digest, entry.relative_path.as_bytes());
    digest_field(digest, entry.record_kind.as_persisted().as_bytes());
    digest_field(digest, entry.record_id.as_bytes());
    // Same `as_persisted` spelling the SQL layer writes, so a digest can
    // never silently disagree with what was stored.
    digest_field(digest, entry.change.as_persisted().as_bytes());
    digest_field(digest, entry.classification.as_persisted().as_bytes());
    match &entry.expected_final_file_sha256 {
        Some(hash) => {
            digest_field(digest, b"1");
            digest_field(digest, hash.as_bytes());
        }
        None => digest_field(digest, b"0"),
    }
}

fn compute_managed_projection_rebuild_digest(
    preview: &ManagedProjectionRebuildPreview,
) -> ManagedProjectionRebuildPayloadDigest {
    let mut digest = Sha256::new();
    digest_field(
        &mut digest,
        b"product-mission-control.projection-rebuild.prepared-intent.v1",
    );
    digest_field(&mut digest, preview.prepared_intent_id.as_str().as_bytes());
    digest_field(&mut digest, &preview.ledger_schema_version.to_be_bytes());
    digest_field(&mut digest, &preview.ledger_revision.to_be_bytes());
    digest_field(&mut digest, preview.published_baseline_digest.as_bytes());
    digest_field(&mut digest, &(preview.changes.len() as u64).to_be_bytes());
    for entry in &preview.changes {
        digest_change_entry(&mut digest, entry);
    }
    digest_field(
        &mut digest,
        preview.classification.as_persisted().as_bytes(),
    );
    digest_field(
        &mut digest,
        &preview.estimated_duration_millis.to_be_bytes(),
    );
    digest_field(&mut digest, preview.rationale.as_str().as_bytes());
    digest_field(&mut digest, &preview.expires_at.unix_millis().to_be_bytes());
    ManagedProjectionRebuildPayloadDigest(format!("{:x}", digest.finalize()))
}
