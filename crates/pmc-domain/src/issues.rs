//! Governed Issue lifecycle and the sole in-memory Issue record authority.
#![allow(clippy::result_large_err)]

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
    AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
    AuditPolicyOutcome, AuditTarget,
};
use crate::classification::DataClassification;
use crate::error::{
    DomainError, ErrorCode, MessageKey, MessageParam, SafeErrorExtension, SafeParamValue,
};
use crate::identity::{
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
    IdempotencyId, IssueId, PreparedIntentId, RiskId,
};
use crate::time::Clock;
use crate::value::BoundedText;
use crate::work_management::{
    validate_and_mint_work_management_h2a_receipt, ApprovalAuthorizationPort, EvidenceOrJudgment,
    EvidenceReferenceMetadata, EvidenceRole, HumanJudgment, IssueState, SupportWitness,
    WorkManagementApproval, WorkManagementAuthoritativeSnapshot, WorkManagementCurrentPolicy,
    WorkManagementOperation, WorkManagementPayloadDigest, WorkManagementPreparedIntent,
    WorkManagementRationale,
};

pub use crate::work_management::IssueResolutionType;

pub type IssueTitle = BoundedText<240>;
pub type IssueDetails = BoundedText<2_000>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueRecord {
    id: IssueId,
    source_risk_id: Option<RiskId>,
    recurrence_of: Option<IssueId>,
    title: IssueTitle,
    details: IssueDetails,
    classification: DataClassification,
    state: IssueState,
    resolution_type: Option<IssueResolutionType>,
    resolution_rationale: Option<WorkManagementRationale>,
    resolution_evidence: Vec<EvidenceReferenceId>,
    closure_verification_evidence: Vec<EvidenceReferenceId>,
    failed_verification_evidence: Vec<EvidenceReferenceId>,
    reopen_rationales: Vec<WorkManagementRationale>,
    support_history: Vec<SupportWitness>,
    version: AggregateVersion,
}

impl IssueRecord {
    /// Rehydrate the one legal independently-created Issue shape.
    ///
    /// Recurrence requires the referenced closed Issue to be validated as a
    /// collection and therefore remains outside this single-record constructor.
    pub fn from_persisted_created_open(
        id: IssueId,
        title: IssueTitle,
        details: IssueDetails,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Result<Self, crate::DomainValueError> {
        if classification == DataClassification::Unclassified
            || version != AggregateVersion::initial()
        {
            return Err(crate::DomainValueError::new(
                crate::value::ValueErrorKind::UnknownPersistedValue,
            ));
        }
        Ok(Self::new(id, title, details, classification, None, None))
    }

    /// Rehydrate the one legal Issue shape created by an occurred Risk.
    /// Other Issue lifecycle shapes remain owned by the Issue persistence path.
    pub fn from_persisted_created_from_risk(
        id: IssueId,
        risk_id: RiskId,
        title: IssueTitle,
        details: IssueDetails,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Result<Self, crate::DomainValueError> {
        if classification == DataClassification::Unclassified
            || version != AggregateVersion::initial()
        {
            return Err(crate::DomainValueError::new(
                crate::value::ValueErrorKind::UnknownPersistedValue,
            ));
        }
        Ok(Self::from_risk(id, risk_id, title, details, classification))
    }

    /// Rehydrate an Issue in any lifecycle state reachable through the H2a
    /// Resolve/Close/Reopen transition family, from durably persisted parts.
    ///
    /// Unlike the two `from_persisted_created_*` constructors, this accepts
    /// arbitrary version and history, since Issue -- unlike Risk -- cycles
    /// between Open and Resolved an unbounded number of times before a
    /// terminal Close. Only classification is validated here; the shape of
    /// the accumulated history is the caller's (the persistence decoder's)
    /// responsibility, since it is reconstructed from the Issue's own durable
    /// audit-backed tables rather than accepted from an untrusted boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: IssueId,
        source_risk_id: Option<RiskId>,
        recurrence_of: Option<IssueId>,
        title: IssueTitle,
        details: IssueDetails,
        classification: DataClassification,
        state: IssueState,
        resolution_type: Option<IssueResolutionType>,
        resolution_rationale: Option<WorkManagementRationale>,
        resolution_evidence: Vec<EvidenceReferenceId>,
        closure_verification_evidence: Vec<EvidenceReferenceId>,
        failed_verification_evidence: Vec<EvidenceReferenceId>,
        reopen_rationales: Vec<WorkManagementRationale>,
        support_history: Vec<SupportWitness>,
        version: AggregateVersion,
    ) -> Result<Self, crate::DomainValueError> {
        if classification == DataClassification::Unclassified {
            return Err(crate::DomainValueError::new(
                crate::value::ValueErrorKind::UnknownPersistedValue,
            ));
        }
        Ok(Self {
            id,
            source_risk_id,
            recurrence_of,
            title,
            details,
            classification,
            state,
            resolution_type,
            resolution_rationale,
            resolution_evidence,
            closure_verification_evidence,
            failed_verification_evidence,
            reopen_rationales,
            support_history,
            version,
        })
    }

    pub(crate) fn from_risk(
        id: IssueId,
        risk_id: RiskId,
        title: IssueTitle,
        details: IssueDetails,
        classification: DataClassification,
    ) -> Self {
        Self::new(id, title, details, classification, Some(risk_id), None)
    }
    fn new(
        id: IssueId,
        title: IssueTitle,
        details: IssueDetails,
        classification: DataClassification,
        source_risk_id: Option<RiskId>,
        recurrence_of: Option<IssueId>,
    ) -> Self {
        Self {
            id,
            source_risk_id,
            recurrence_of,
            title,
            details,
            classification,
            state: IssueState::Open,
            resolution_type: None,
            resolution_rationale: None,
            resolution_evidence: vec![],
            closure_verification_evidence: vec![],
            failed_verification_evidence: vec![],
            reopen_rationales: vec![],
            support_history: vec![],
            version: AggregateVersion::initial(),
        }
    }
    pub fn id(&self) -> &IssueId {
        &self.id
    }
    pub fn source_risk_id(&self) -> Option<&RiskId> {
        self.source_risk_id.as_ref()
    }
    pub fn recurrence_of(&self) -> Option<&IssueId> {
        self.recurrence_of.as_ref()
    }
    pub fn title(&self) -> &IssueTitle {
        &self.title
    }
    pub fn details(&self) -> &IssueDetails {
        &self.details
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn state(&self) -> IssueState {
        self.state
    }
    pub const fn resolution_type(&self) -> Option<IssueResolutionType> {
        self.resolution_type
    }
    pub fn resolution_rationale(&self) -> Option<&WorkManagementRationale> {
        self.resolution_rationale.as_ref()
    }
    pub fn resolution_evidence(&self) -> &[EvidenceReferenceId] {
        &self.resolution_evidence
    }
    pub fn closure_verification_evidence(&self) -> &[EvidenceReferenceId] {
        &self.closure_verification_evidence
    }
    pub fn failed_verification_evidence(&self) -> &[EvidenceReferenceId] {
        &self.failed_verification_evidence
    }
    pub fn reopen_rationales(&self) -> &[WorkManagementRationale] {
        &self.reopen_rationales
    }
    pub fn support_history(&self) -> &[SupportWitness] {
        &self.support_history
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueH1RuntimeSnapshotError {
    InvalidRecord,
    DuplicateIssue,
}

/// Opaque, domain-validated restart input for the independent Issue H1 path.
///
/// This boundary intentionally accepts only the canonical initial Open Issue
/// shape. Risk-derived Issues, recurrences, and later lifecycle states belong
/// to their own persistence verticals and cannot be smuggled into this
/// runtime authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueH1RuntimeSnapshot {
    records: Vec<IssueRecord>,
}

impl IssueH1RuntimeSnapshot {
    pub fn try_new(records: Vec<IssueRecord>) -> Result<Self, IssueH1RuntimeSnapshotError> {
        let mut identities = std::collections::BTreeSet::new();
        for record in &records {
            if !identities.insert(record.id.clone()) {
                return Err(IssueH1RuntimeSnapshotError::DuplicateIssue);
            }
            if record.source_risk_id.is_some()
                || record.recurrence_of.is_some()
                || record.classification == DataClassification::Unclassified
                || record.state != IssueState::Open
                || record.version != AggregateVersion::initial()
                || record.resolution_type.is_some()
                || record.resolution_rationale.is_some()
                || !record.resolution_evidence.is_empty()
                || !record.closure_verification_evidence.is_empty()
                || !record.failed_verification_evidence.is_empty()
                || !record.reopen_rationales.is_empty()
                || !record.support_history.is_empty()
            {
                return Err(IssueH1RuntimeSnapshotError::InvalidRecord);
            }
        }
        Ok(Self { records })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records().is_empty()
    }

    /// Internal composition seam; callers cannot inject an arbitrary authority.
    pub(crate) fn records(&self) -> &[IssueRecord] {
        &self.records
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueH2aRuntimeSnapshotError {
    /// A preview that is not an Issue operation, or whose target Issue is
    /// absent from the records the snapshot carries.
    InvalidPreparedIntent,
    DuplicateIssue,
    DuplicatePreparedIntent,
    /// v46: a rejection whose topology does not match what
    /// [`InMemoryIssueService::reject_issue_prepared_intent`] produces, or
    /// whose preview is also handed in as outstanding.
    InvalidRejection,
}

/// Typed durable evidence for one Issue H2a rejection (v46): the preview
/// that was refused and the zero-effect outcome that refused it. Validated
/// by [`IssueH2aRuntimeSnapshot::try_new_with_rejections`], so SQLite is
/// never the sole judge of a valid rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueH2aRejectionReplay {
    prepared: WorkManagementPreparedIntent,
    context: IssueOperationContext,
    outcome: crate::work_management::RejectedPreparedIntentOutcome,
}

impl IssueH2aRejectionReplay {
    pub fn new(
        prepared: WorkManagementPreparedIntent,
        context: IssueOperationContext,
        outcome: crate::work_management::RejectedPreparedIntentOutcome,
    ) -> Self {
        Self {
            prepared,
            context,
            outcome,
        }
    }
}

/// The Issue every Issue-owned H2a preview targets -- the three lifecycle
/// operations and classification lowering. `None` means the preview does not
/// belong to the Issue namespace at all.
fn prepared_issue_target(prepared: &WorkManagementPreparedIntent) -> Option<IssueId> {
    match prepared.operation() {
        WorkManagementOperation::ResolveIssue { issue_id, .. }
        | WorkManagementOperation::CloseIssue { issue_id, .. }
        | WorkManagementOperation::ReopenIssue { issue_id, .. }
        | WorkManagementOperation::LowerIssueClassification { issue_id, .. } => {
            Some(issue_id.clone())
        }
        _ => None,
    }
}

fn rejected_issue_id(prepared: &WorkManagementPreparedIntent) -> Option<IssueId> {
    match prepared.operation() {
        WorkManagementOperation::ResolveIssue { issue_id, .. }
        | WorkManagementOperation::CloseIssue { issue_id, .. }
        | WorkManagementOperation::ReopenIssue { issue_id, .. } => Some(issue_id.clone()),
        _ => None,
    }
}

/// A rejection is valid only when its outcome and audit say exactly what
/// the service would have said.
fn valid_issue_rejection(rejection: &IssueH2aRejectionReplay, issue_id: &IssueId) -> bool {
    let outcome = &rejection.outcome;
    let audit = outcome.audit_event();
    outcome.prepared_intent_id() == rejection.prepared.id()
        && outcome.expired_at_rejection()
            == (outcome.rejected_at() >= rejection.prepared.preview().expires_at())
        && audit.occurred_at() == outcome.rejected_at()
        && audit.actor() == AuditActor::HeadOfProducts
        && audit.module() == AuditModule::WorkManagement
        && audit.code().as_str() == crate::work_management::ISSUE_PREPARED_REJECTED_AUDIT_CODE
        && audit.target() == &AuditTarget::Issue(issue_id.clone())
        && audit.correlation_id() == &rejection.context.correlation_id
        && audit.policy_outcome() == AuditPolicyOutcome::Allowed
        && audit.approval_outcome() == AuditApprovalOutcome::Rejected
}

/// Opaque, domain-validated restart input for the Issue H2a execute path.
///
/// Unlike [`IssueH1RuntimeSnapshot`], this accepts Issues in any lifecycle
/// state together with their outstanding, not-yet-approved H2a previews, so
/// an `approve_and_execute_resolve/close/reopen` call can rehydrate the
/// specific Issue and prepared intent it targets from durable storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueH2aRuntimeSnapshot {
    records: Vec<IssueRecord>,
    prepared: Vec<WorkManagementPreparedIntent>,
    rejections: Vec<IssueH2aRejectionReplay>,
}

impl IssueH2aRuntimeSnapshot {
    pub fn try_new(
        records: Vec<IssueRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
    ) -> Result<Self, IssueH2aRuntimeSnapshotError> {
        Self::try_new_with_rejections(records, prepared, vec![])
    }

    /// v46: records, outstanding previews, and durable rejections. A
    /// rejection must name a preview that is *not* outstanding, must target
    /// an Issue in `records`, and must carry the exact zero-effect audit the
    /// service produces; anything else is refused as corrupt.
    pub fn try_new_with_rejections(
        records: Vec<IssueRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
        rejections: Vec<IssueH2aRejectionReplay>,
    ) -> Result<Self, IssueH2aRuntimeSnapshotError> {
        let mut identities = std::collections::BTreeSet::new();
        for record in &records {
            if !identities.insert(record.id.clone()) {
                return Err(IssueH2aRuntimeSnapshotError::DuplicateIssue);
            }
        }
        let mut prepared_identities = std::collections::BTreeSet::new();
        for intent in &prepared {
            if !prepared_identities.insert(intent.id().clone()) {
                return Err(IssueH2aRuntimeSnapshotError::DuplicatePreparedIntent);
            }
            // A preview with no record behind it would let the runtime answer
            // about an Issue it cannot see. Harmless while every caller passed
            // exactly one record and its own preview; load-bearing once a
            // whole Ledger is decoded at once.
            match prepared_issue_target(intent) {
                Some(target) if identities.contains(&target) => {}
                _ => return Err(IssueH2aRuntimeSnapshotError::InvalidPreparedIntent),
            }
        }
        let mut rejected_identities = std::collections::BTreeSet::new();
        let mut rejection_idempotency = std::collections::BTreeSet::new();
        let mut rejection_audits = std::collections::BTreeSet::new();
        for rejection in &rejections {
            let Some(issue_id) = rejected_issue_id(&rejection.prepared) else {
                return Err(IssueH2aRuntimeSnapshotError::InvalidRejection);
            };
            if prepared_identities.contains(rejection.prepared.id())
                || !rejected_identities.insert(rejection.prepared.id().clone())
                || !rejection_idempotency.insert(rejection.context.idempotency_id.clone())
                || !rejection_audits.insert(rejection.outcome.audit_event().id().clone())
                || !identities.contains(&issue_id)
                || !valid_issue_rejection(rejection, &issue_id)
            {
                return Err(IssueH2aRuntimeSnapshotError::InvalidRejection);
            }
        }
        Ok(Self {
            records,
            prepared,
            rejections,
        })
    }

    /// Durable rejections that survived a restart (v46).
    pub fn rejections(&self) -> &[IssueH2aRejectionReplay] {
        &self.rejections
    }

    /// Outstanding, not-yet-approved Issue H2a previews that survived a
    /// restart.
    pub fn prepared(&self) -> &[WorkManagementPreparedIntent] {
        &self.prepared
    }

    /// Every Issue the snapshot carries, at the state it had reached. The
    /// runtime still takes the snapshot whole -- reading the records does not
    /// let a caller inject an authority.
    pub fn records(&self) -> &[IssueRecord] {
        &self.records
    }
}

#[derive(Clone, Default)]
pub(crate) struct IssueStore {
    records: HashMap<IssueId, IssueRecord>,
}
impl IssueStore {
    pub(crate) fn get(&self, id: &IssueId) -> Option<&IssueRecord> {
        self.records.get(id)
    }
    pub(crate) fn contains(&self, id: &IssueId) -> bool {
        self.records.contains_key(id)
    }
    pub(crate) fn insert_from_risk(&mut self, issue: IssueRecord) {
        self.records.insert(issue.id.clone(), issue);
    }
    pub(crate) fn from_records(records: Vec<IssueRecord>) -> Option<Self> {
        let mut store = Self::default();
        for record in records {
            if store.records.insert(record.id.clone(), record).is_some() {
                return None;
            }
        }
        Some(store)
    }
}

#[derive(Clone, Default)]
pub(crate) struct SharedIssueAuthority {
    inner: Rc<RefCell<IssueStore>>,
}
impl SharedIssueAuthority {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub fn issue(&self, id: &IssueId) -> Option<IssueRecord> {
        self.inner.borrow().get(id).cloned()
    }
    /// Deep, isolated copy used by the in-memory Product Ledger transaction stage.
    pub(crate) fn isolated_copy(&self) -> Self {
        Self {
            inner: Rc::new(RefCell::new(self.inner.borrow().clone())),
        }
    }
    pub(crate) fn snapshot(&self) -> IssueStore {
        self.inner.borrow().clone()
    }
    pub(crate) fn apply_unchecked(&self, store: IssueStore) {
        *self.inner.borrow_mut() = store;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueOperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateIssue {
    pub id: IssueId,
    pub title: IssueTitle,
    pub details: IssueDetails,
    pub classification: DataClassification,
    pub recurrence_of: Option<IssueId>,
    pub context: IssueOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareResolveIssue {
    pub issue_id: IssueId,
    pub expected_version: AggregateVersion,
    pub resolution_type: IssueResolutionType,
    pub rationale: WorkManagementRationale,
    pub evidence_ids: Vec<EvidenceReferenceId>,
    /// The person's written Judgment for proceeding on partly verified
    /// Evidence (`ObservedUnpinned` / `DegradedLastVerified`). It never
    /// substitutes for Evidence and cannot override `Unverified` or
    /// `IntegrityMismatch`. It lives on the support witness, not on the
    /// operation.
    pub judgment: Option<HumanJudgment>,
    pub context: IssueOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareCloseIssue {
    pub issue_id: IssueId,
    pub expected_version: AggregateVersion,
    pub evidence_ids: Vec<EvidenceReferenceId>,
    /// See [`PrepareResolveIssue::judgment`].
    pub judgment: Option<HumanJudgment>,
    pub context: IssueOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareReopenIssue {
    pub issue_id: IssueId,
    pub expected_version: AggregateVersion,
    pub rationale: WorkManagementRationale,
    pub evidence_ids: Vec<EvidenceReferenceId>,
    /// See [`PrepareResolveIssue::judgment`].
    pub judgment: Option<HumanJudgment>,
    pub context: IssueOperationContext,
}
/// v46: the Head of Products refuses a pending resolve/close/reopen
/// preview. See [`InMemoryIssueService::reject_issue_prepared_intent`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectIssuePreparedIntent {
    pub prepared_id: PreparedIntentId,
    pub actor: AuditActor,
    pub context: IssueOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteIssueTransition {
    pub approval: WorkManagementApproval,
    pub context: IssueOperationContext,
}
/// H2a "Lower Data Classification" for Issue. Built as its
/// own independent method pair rather than routing through the shared
/// `prepare`/`execute` cause functions Resolve/Close/Reopen funnel through
/// -- those hardcode a required Evidence role via `required_role(&op)`
/// (which itself ends in `unreachable!()` for any other operation) and a
/// state-transition-specific `next_state`. Neither applies to a
/// classification-only change, and reusing them would have made a failed
/// lowering panic instead of returning an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerIssueClassification {
    pub issue_id: IssueId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: IssueOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerIssueClassification {
    pub approval: WorkManagementApproval,
    pub context: IssueOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueMutationOutcome {
    pub record: IssueRecord,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: Option<ApprovalReceiptId>,
}

pub trait IssueServiceIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, crate::DomainValueError>;
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, crate::DomainValueError>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueExecutionPolicy {
    Allowed,
    Denied,
}
pub trait IssueExecutionPolicyPort {
    fn current_policy(&self, operation: &WorkManagementOperation) -> IssueExecutionPolicy;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssueEvidenceAuthorityError {
    Unavailable,
    NotFound,
}
pub trait IssueEvidenceAuthorityPort {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyIssueEvidenceAuthority;
impl IssueEvidenceAuthorityPort for DenyIssueEvidenceAuthority {
    fn resolve(
        &self,
        _: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        Err(IssueEvidenceAuthorityError::Unavailable)
    }
}
pub trait IssueClassificationAuthorityPort {
    fn current_classification(
        &self,
        id: &IssueId,
        recorded: DataClassification,
    ) -> Result<DataClassification, IssueEvidenceAuthorityError>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordedIssueClassification;
impl IssueClassificationAuthorityPort for RecordedIssueClassification {
    fn current_classification(
        &self,
        _: &IssueId,
        recorded: DataClassification,
    ) -> Result<DataClassification, IssueEvidenceAuthorityError> {
        Ok(recorded)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Identity {
    Create(
        IssueId,
        IssueTitle,
        IssueDetails,
        DataClassification,
        Option<IssueId>,
    ),
    Prepare(
        WorkManagementOperation,
        Vec<EvidenceReferenceId>,
        Option<HumanJudgment>,
    ),
    Execute(PreparedIntentId, AuditActor, WorkManagementPayloadDigest),
    PrepareLowerClassification(
        IssueId,
        AggregateVersion,
        DataClassification,
        WorkManagementRationale,
    ),
    RejectPrepared(PreparedIntentId, AuditActor),
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum Stored {
    Mutation(IssueMutationOutcome),
    Prepared(WorkManagementPreparedIntent),
    Rejected(crate::work_management::RejectedPreparedIntentOutcome),
}
#[derive(Clone, Default)]
struct IssueStateStore {
    prepared: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    idem: HashMap<IdempotencyId, (Identity, Stored)>,
    audits: Vec<AuditEvent>,
}
pub struct InMemoryIssueService<C, I, Z, P, E, A> {
    clock: C,
    ids: I,
    authorization: Z,
    policy: P,
    evidence: E,
    classification: A,
    authority: SharedIssueAuthority,
    state: IssueStateStore,
    fail_next_commit: bool,
}
impl<
        C: Clock,
        I: IssueServiceIdSource,
        Z: ApprovalAuthorizationPort,
        P: IssueExecutionPolicyPort,
        E: IssueEvidenceAuthorityPort,
        A: IssueClassificationAuthorityPort,
    > InMemoryIssueService<C, I, Z, P, E, A>
{
    pub fn new(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence: E,
        classification: A,
    ) -> Self {
        Self::new_with_issue_authority(
            clock,
            ids,
            authorization,
            policy,
            evidence,
            classification,
            SharedIssueAuthority::new(),
        )
    }
    pub(crate) fn new_with_issue_authority(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence: E,
        classification: A,
        authority: SharedIssueAuthority,
    ) -> Self {
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence,
            classification,
            authority,
            state: IssueStateStore::default(),
            fail_next_commit: false,
        }
    }
    /// Reconstructs an in-memory Issue runtime whose authority already
    /// carries every Issue in its current (possibly non-Open) lifecycle
    /// state, additionally seeding the specific outstanding H2a previews an
    /// `approve_and_execute_resolve/close/reopen` call needs to find its
    /// target. This is intentionally not a generic state import surface.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rehydrate_with_prepared_and_issue_authority(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence: E,
        classification: A,
        prepared: Vec<WorkManagementPreparedIntent>,
        rejections: Vec<IssueH2aRejectionReplay>,
        authority: SharedIssueAuthority,
    ) -> Self {
        let prepared = prepared
            .into_iter()
            .map(|intent| (intent.id().clone(), intent))
            .collect();
        // v46: a durable rejection replays as itself and keeps its audit.
        let mut idem = HashMap::new();
        let mut audits = Vec::with_capacity(rejections.len());
        for rejection in rejections {
            audits.push(rejection.outcome.audit_event().clone());
            idem.insert(
                rejection.context.idempotency_id,
                (
                    Identity::RejectPrepared(
                        rejection.prepared.id().clone(),
                        AuditActor::HeadOfProducts,
                    ),
                    Stored::Rejected(rejection.outcome),
                ),
            );
        }
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence,
            classification,
            authority,
            state: IssueStateStore {
                prepared,
                idem,
                audits,
            },
            fail_next_commit: false,
        }
    }
    pub fn issue(&self, id: &IssueId) -> Option<IssueRecord> {
        self.authority.issue(id)
    }
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.state.audits
    }
    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true
    }
    /// Produces an isolated transactional copy sharing only the explicitly
    /// supplied isolated Issue authority.
    pub(crate) fn isolated_copy_with_issue_authority(&self, authority: SharedIssueAuthority) -> Self
    where
        C: Clone,
        I: Clone,
        Z: Clone,
        P: Clone,
        E: Clone,
        A: Clone,
    {
        Self {
            clock: self.clock.clone(),
            ids: self.ids.clone(),
            authorization: self.authorization.clone(),
            policy: self.policy.clone(),
            evidence: self.evidence.clone(),
            classification: self.classification.clone(),
            authority,
            state: self.state.clone(),
            fail_next_commit: false,
        }
    }
    pub fn discard_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let existed = self.state.prepared.remove(id).is_some();
        if existed {
            self.state
                .idem
                .retain(|_, (_, v)| !matches!(v,Stored::Prepared(p) if p.id()==id));
        }
        existed
    }
    /// v46 H2a rejection: the Head of Products refuses a pending resolve,
    /// close or reopen preview. A successful command with no effect: the
    /// intent is consumed and can never be executed, one zero-effect audit
    /// is recorded, no receipt is minted and no Issue changes. Rejecting an
    /// already consumed intent (executed or already rejected) is a conflict;
    /// an unknown intent is not found; an expired-but-unconsumed preview is
    /// rejected normally and the outcome says it had expired. Unlike a
    /// silent [`Self::discard_prepared_intent`], this is the recorded retreat
    /// route DG3 requires of reject/cancel.
    pub fn reject_issue_prepared_intent(
        &mut self,
        c: RejectIssuePreparedIntent,
    ) -> Result<crate::work_management::RejectedPreparedIntentOutcome, DomainError> {
        let sig = Identity::RejectPrepared(c.prepared_id.clone(), c.actor);
        if let Some(stored) = self.replay(&c.context, &sig)? {
            return match stored {
                Stored::Rejected(outcome) => Ok(outcome),
                _ => Err(self.error(
                    ErrorCode::DomainIdempotencyConflict,
                    "issue.idempotency_conflict",
                    &c.context,
                    false,
                )),
            };
        }
        if c.actor != AuditActor::HeadOfProducts || !self.authorization.authorize(c.actor) {
            return Err(self.error(
                ErrorCode::SecurityPolicyDenied,
                "issue.security_denied",
                &c.context,
                false,
            ));
        }
        let Some(intent) = self.state.prepared.get(&c.prepared_id).cloned() else {
            let known = self.state.idem.values().any(|(_, v)| match v {
                Stored::Prepared(p) => p.id() == &c.prepared_id,
                Stored::Rejected(outcome) => outcome.prepared_intent_id() == &c.prepared_id,
                Stored::Mutation(_) => false,
            });
            return Err(if known {
                self.error(
                    ErrorCode::DomainConflict,
                    "issue.stale_or_illegal",
                    &c.context,
                    false,
                )
            } else {
                self.error(
                    ErrorCode::DomainNotFound,
                    "issue.not_found",
                    &c.context,
                    false,
                )
            });
        };
        let Some(issue_id) = rejected_issue_id(&intent) else {
            return Err(self.error(
                ErrorCode::DomainConflict,
                "issue.invalid_intent",
                &c.context,
                false,
            ));
        };
        let now = self.clock.now();
        let audit_id = self.ids.next_audit_event_id().map_err(|_| {
            self.error(
                ErrorCode::PlatformInternal,
                "issue.infrastructure",
                &c.context,
                true,
            )
        })?;
        let audit = crate::work_management::prepared_intent_rejection_audit(
            audit_id,
            now,
            crate::work_management::ISSUE_PREPARED_REJECTED_AUDIT_CODE,
            AuditTarget::Issue(issue_id),
            c.context.correlation_id.clone(),
        )
        .ok_or_else(|| {
            self.error(
                ErrorCode::PlatformInternal,
                "issue.infrastructure",
                &c.context,
                true,
            )
        })?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(self.error(
                ErrorCode::PlatformInternal,
                "issue.infrastructure",
                &c.context,
                true,
            ));
        }
        let outcome = crate::work_management::RejectedPreparedIntentOutcome::new(
            intent.id().clone(),
            now,
            now >= intent.preview().expires_at(),
            audit.clone(),
        );
        let mut staged = self.state.clone();
        staged.prepared.remove(&c.prepared_id);
        staged.audits.push(audit);
        staged.idem.insert(
            c.context.idempotency_id,
            (sig, Stored::Rejected(outcome.clone())),
        );
        self.state = staged;
        Ok(outcome)
    }
    pub fn create_issue(&mut self, c: CreateIssue) -> Result<IssueMutationOutcome, DomainError> {
        let sig = Identity::Create(
            c.id.clone(),
            c.title.clone(),
            c.details.clone(),
            c.classification,
            c.recurrence_of.clone(),
        );
        if let Some(Stored::Mutation(v)) = self.replay(&c.context, &sig)? {
            return Ok(v);
        }
        if c.classification == DataClassification::Unclassified {
            return Err(self.error(
                ErrorCode::ValidationInvalidField,
                "issue.classification_required",
                &c.context,
                false,
            ));
        }
        let mut issues = self.authority.snapshot();
        if issues.contains(&c.id) {
            return Err(self.error(ErrorCode::DomainConflict, "issue.exists", &c.context, false));
        }
        let mut effective_classification = c.classification;
        if let Some(old) = &c.recurrence_of {
            match issues.get(old) {
                Some(v) if v.state == IssueState::Closed => {
                    let source = self
                        .classification
                        .current_classification(old, v.classification)
                        .map_err(|_| {
                            self.error(
                                ErrorCode::PlatformInternal,
                                "issue.infrastructure",
                                &c.context,
                                true,
                            )
                        })?;
                    effective_classification =
                        DataClassification::combine(effective_classification, source);
                }
                Some(_) => return Err(self.lifecycle_error(old, &c.context)),
                None => {
                    return Err(self.error(
                        ErrorCode::DomainNotFound,
                        "issue.not_found",
                        &c.context,
                        false,
                    ))
                }
            }
        }
        if effective_classification == DataClassification::Unclassified {
            return Err(self.error(
                ErrorCode::SecurityPolicyDenied,
                "issue.classification_required",
                &c.context,
                false,
            ));
        }
        let record = IssueRecord::new(
            c.id.clone(),
            c.title,
            c.details,
            effective_classification,
            None,
            c.recurrence_of,
        );
        let audit = self.make_audit(&c.id, "issue.created", &c.context, false, true)?;
        let out = IssueMutationOutcome {
            record: record.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(self.error(
                ErrorCode::PlatformInternal,
                "issue.infrastructure",
                &c.context,
                true,
            ));
        }
        let mut staged = self.state.clone();
        issues.insert_from_risk(record);
        staged.audits.push(audit);
        staged.idem.insert(
            c.context.idempotency_id,
            (sig, Stored::Mutation(out.clone())),
        );
        self.authority.apply_unchecked(issues);
        self.state = staged;
        Ok(out)
    }
    pub fn prepare_resolve_issue(
        &mut self,
        c: PrepareResolveIssue,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let op = WorkManagementOperation::ResolveIssue {
            issue_id: c.issue_id.clone(),
            issue_version: c.expected_version,
            resolution_type: c.resolution_type,
            rationale: c.rationale.clone(),
        };
        self.prepare(op, c.evidence_ids, c.judgment, c.context)
    }
    pub fn prepare_close_issue(
        &mut self,
        c: PrepareCloseIssue,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let op = WorkManagementOperation::CloseIssue {
            issue_id: c.issue_id.clone(),
            issue_version: c.expected_version,
        };
        self.prepare(op, c.evidence_ids, c.judgment, c.context)
    }
    pub fn prepare_reopen_issue(
        &mut self,
        c: PrepareReopenIssue,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let op = WorkManagementOperation::ReopenIssue {
            issue_id: c.issue_id.clone(),
            issue_version: c.expected_version,
            rationale: c.rationale.clone(),
        };
        self.prepare(op, c.evidence_ids, c.judgment, c.context)
    }
    pub fn prepare_lower_issue_classification(
        &mut self,
        c: PrepareLowerIssueClassification,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let sig = Identity::PrepareLowerClassification(
            c.issue_id.clone(),
            c.expected_version,
            c.proposed_classification,
            c.rationale.clone(),
        );
        if let Some(Stored::Prepared(v)) = self.replay(&c.context, &sig)? {
            return Ok(v);
        }
        let issue = self.authority.issue(&c.issue_id).ok_or_else(|| {
            self.error(
                ErrorCode::DomainNotFound,
                "issue.not_found",
                &c.context,
                false,
            )
        })?;
        if issue.version != c.expected_version {
            return Err(self.lifecycle_error(&c.issue_id, &c.context));
        }
        if !is_genuine_lowering(issue.classification, c.proposed_classification) {
            return Err(self.error(
                ErrorCode::DomainConflict,
                "issue.classification_lowering_not_a_lowering",
                &c.context,
                false,
            ));
        }
        let current_classification = issue.classification;
        let op = WorkManagementOperation::LowerIssueClassification {
            issue_id: c.issue_id.clone(),
            issue_version: c.expected_version,
            current_classification,
            proposed_classification: c.proposed_classification,
            rationale: c.rationale,
        };
        if self.policy.current_policy(&op) == IssueExecutionPolicy::Denied {
            return Err(self.error(
                ErrorCode::SecurityPolicyDenied,
                "issue.security_denied",
                &c.context,
                false,
            ));
        }
        let pid = self.ids.next_prepared_intent_id().map_err(|_| {
            self.error(
                ErrorCode::PlatformInternal,
                "issue.infrastructure",
                &c.context,
                true,
            )
        })?;
        let p = WorkManagementPreparedIntent::prepare(
            pid,
            op,
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            self.error(
                ErrorCode::DomainConflict,
                "issue.invalid_intent",
                &c.context,
                false,
            )
        })?;
        self.state.prepared.insert(p.id().clone(), p.clone());
        self.state
            .idem
            .insert(c.context.idempotency_id, (sig, Stored::Prepared(p.clone())));
        Ok(p)
    }
    pub fn approve_and_execute(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        let target = self.prepared_issue(c.approval.prepared_id());
        match self.execute(c.clone()) {
            Ok(v) => Ok(v),
            Err(f) => {
                self.failure_audit(target, "issue.execute_denied", &c.context, f)?;
                Err(self.map_failure(f, &c.context))
            }
        }
    }
    pub fn approve_and_execute_resolve(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        self.ensure_prepared_operation(
            c.approval.prepared_id(),
            |operation| matches!(operation, WorkManagementOperation::ResolveIssue { .. }),
            &c.context,
        )?;
        self.approve_and_execute(c)
    }
    pub fn approve_and_execute_close(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        self.ensure_prepared_operation(
            c.approval.prepared_id(),
            |operation| matches!(operation, WorkManagementOperation::CloseIssue { .. }),
            &c.context,
        )?;
        self.approve_and_execute(c)
    }
    pub fn approve_and_execute_reopen(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        self.ensure_prepared_operation(
            c.approval.prepared_id(),
            |operation| matches!(operation, WorkManagementOperation::ReopenIssue { .. }),
            &c.context,
        )?;
        self.approve_and_execute(c)
    }
    pub fn approve_and_execute_lower_issue_classification(
        &mut self,
        c: ApproveAndExecuteLowerIssueClassification,
    ) -> Result<IssueMutationOutcome, DomainError> {
        let target = self.prepared_issue(c.approval.prepared_id());
        match self.execute_lower_issue_classification(c.clone()) {
            Ok(v) => Ok(v),
            Err(f) => {
                self.failure_audit(target, "issue.execute_denied", &c.context, f)?;
                Err(self.map_failure(f, &c.context))
            }
        }
    }

    fn ensure_prepared_operation(
        &self,
        id: &PreparedIntentId,
        expected: impl FnOnce(&WorkManagementOperation) -> bool,
        context: &IssueOperationContext,
    ) -> Result<(), DomainError> {
        let matches = self
            .state
            .prepared
            .get(id)
            .is_some_and(|prepared| expected(prepared.operation()));
        if matches {
            Ok(())
        } else {
            Err(self.error(
                ErrorCode::DomainConflict,
                "issue.prepared_operation_mismatch",
                context,
                false,
            ))
        }
    }
    fn prepare(
        &mut self,
        op: WorkManagementOperation,
        evidence_ids: Vec<EvidenceReferenceId>,
        judgment: Option<HumanJudgment>,
        context: IssueOperationContext,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let sig = Identity::Prepare(op.clone(), evidence_ids.clone(), judgment.clone());
        if let Some(Stored::Prepared(v)) = self.replay(&context, &sig)? {
            return Ok(v);
        }
        let (id, version, required) = match &op {
            WorkManagementOperation::ResolveIssue {
                issue_id,
                issue_version,
                ..
            } => (issue_id, *issue_version, IssueState::Open),
            WorkManagementOperation::CloseIssue {
                issue_id,
                issue_version,
            } => (issue_id, *issue_version, IssueState::Resolved),
            WorkManagementOperation::ReopenIssue {
                issue_id,
                issue_version,
                ..
            } => (issue_id, *issue_version, IssueState::Resolved),
            _ => unreachable!(),
        };
        let issue = self.authority.issue(id).ok_or_else(|| {
            self.error(
                ErrorCode::DomainNotFound,
                "issue.not_found",
                &context,
                false,
            )
        })?;
        if issue.version != version || issue.state != required {
            return Err(self.lifecycle_error(id, &context));
        }
        let judgments: Vec<HumanJudgment> = judgment.into_iter().collect();
        let support = match self.support(&evidence_ids, &judgments, required_role(&op)) {
            Ok(value) => value,
            Err(failure) => {
                let failure = if failure == Failure::EvidenceUnavailable {
                    Failure::PrepareEvidenceUnavailable
                } else {
                    failure
                };
                self.failure_audit(Some(id.clone()), "issue.prepare_denied", &context, failure)?;
                return Err(self.map_failure(failure, &context));
            }
        };
        let authoritative_class = match self
            .classification
            .current_classification(id, issue.classification)
        {
            Ok(value) => value,
            Err(_) => {
                self.failure_audit(
                    Some(id.clone()),
                    "issue.prepare_denied",
                    &context,
                    Failure::Infrastructure,
                )?;
                return Err(self.map_failure(Failure::Infrastructure, &context));
            }
        };
        let class = DataClassification::combine(issue.classification, authoritative_class);
        if class == DataClassification::Unclassified {
            self.failure_audit(
                Some(id.clone()),
                "issue.prepare_denied",
                &context,
                Failure::Evidence,
            )?;
            return Err(self.map_failure(Failure::Evidence, &context));
        }
        if self.policy.current_policy(&op) == IssueExecutionPolicy::Denied {
            self.failure_audit(
                Some(id.clone()),
                "issue.prepare_denied",
                &context,
                Failure::PolicyDenied,
            )?;
            return Err(self.map_failure(Failure::PolicyDenied, &context));
        }
        let pid = self.ids.next_prepared_intent_id().map_err(|_| {
            self.error(
                ErrorCode::PlatformInternal,
                "issue.infrastructure",
                &context,
                true,
            )
        })?;
        let p =
            WorkManagementPreparedIntent::prepare(pid, op, class, Some(support), self.clock.now())
                .map_err(|_| {
                    self.error(
                        ErrorCode::DomainConflict,
                        "issue.invalid_intent",
                        &context,
                        false,
                    )
                })?;
        self.state.prepared.insert(p.id().clone(), p.clone());
        self.state
            .idem
            .insert(context.idempotency_id, (sig, Stored::Prepared(p.clone())));
        Ok(p)
    }
    fn execute(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, Failure> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(Failure::Changed);
        }
        let sig = Identity::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(Stored::Mutation(v)) = self
            .replay(&c.context, &sig)
            .map_err(|_| Failure::Idempotency)?
        {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(Failure::Changed)?;
        let (id, v, required, next_state) = match p.operation() {
            WorkManagementOperation::ResolveIssue {
                issue_id,
                issue_version,
                ..
            } => (
                issue_id.clone(),
                *issue_version,
                IssueState::Open,
                IssueState::Resolved,
            ),
            WorkManagementOperation::CloseIssue {
                issue_id,
                issue_version,
            } => (
                issue_id.clone(),
                *issue_version,
                IssueState::Resolved,
                IssueState::Closed,
            ),
            WorkManagementOperation::ReopenIssue {
                issue_id,
                issue_version,
                ..
            } => (
                issue_id.clone(),
                *issue_version,
                IssueState::Resolved,
                IssueState::Open,
            ),
            _ => return Err(Failure::Changed),
        };
        let current = self.authority.issue(&id).ok_or(Failure::Changed)?;
        if current.version != v || current.state != required {
            return Err(Failure::Changed);
        }
        let prepared_support = p.preview().support().ok_or(Failure::Evidence)?;
        let evidence_ids = prepared_support
            .evidence()
            .iter()
            .map(|x| x.id().clone())
            .collect::<Vec<_>>();
        // The Judgment the person wrote at prepare time is part of the exact
        // preview they approved; re-derive the support with it so the fresh
        // digest can match.
        let judgments = prepared_support.judgments().to_vec();
        let support = self.support(&evidence_ids, &judgments, required_role(p.operation()))?;
        let authoritative_class = self
            .classification
            .current_classification(&id, current.classification)
            .map_err(|_| Failure::Infrastructure)?;
        let class = DataClassification::combine(current.classification, authoritative_class);
        if class == DataClassification::Unclassified {
            return Err(Failure::Changed);
        }
        let op = p.operation().clone();
        if self.policy.current_policy(&op) == IssueExecutionPolicy::Denied {
            return Err(Failure::PolicyDenied);
        }
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            op.clone(),
            class,
            Some(support.clone()),
            self.clock.now(),
        )
        .map_err(|_| Failure::Changed)?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: op.clone(),
            classification: fresh.preview().classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: Some(support.clone()),
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            &p,
            &c.approval,
            &snapshot,
            self.clock.now(),
            self.ids
                .next_approval_receipt_id()
                .map_err(|_| Failure::Infrastructure)?,
            &self.authorization,
        )
        .map_err(|e| {
            use crate::work_management::WorkManagementApprovalValidationError as V;
            match e {
                V::Unauthorized => Failure::Unauthorized,
                V::PreparedIntentMismatch | V::DigestMismatch | V::Expired | V::PreviewChanged => {
                    Failure::Changed
                }
            }
        })?;
        let mut record = current;
        record.state = next_state;
        record.classification = fresh.preview().classification();
        record.version = record.version.next().ok_or(Failure::Infrastructure)?;
        record.support_history.push(support);
        let (code, ids) = match &op {
            WorkManagementOperation::ResolveIssue {
                resolution_type,
                rationale,
                ..
            } => {
                record.resolution_type = Some(*resolution_type);
                record.resolution_rationale = Some(rationale.clone());
                record.resolution_evidence = evidence_ids.clone();
                ("issue.resolved", evidence_ids)
            }
            WorkManagementOperation::CloseIssue { .. } => {
                if evidence_ids
                    .iter()
                    .any(|x| record.resolution_evidence.contains(x))
                {
                    return Err(Failure::Evidence);
                }
                record.closure_verification_evidence = evidence_ids.clone();
                ("issue.closed", evidence_ids)
            }
            WorkManagementOperation::ReopenIssue { rationale, .. } => {
                record
                    .failed_verification_evidence
                    .extend(evidence_ids.clone());
                record.reopen_rationales.push(rationale.clone());
                ("issue.reopened", evidence_ids)
            }
            _ => return Err(Failure::Changed),
        };
        let _ = ids;
        let audit = self
            .make_audit(&id, code, &c.context, true, true)
            .map_err(|_| Failure::Infrastructure)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(Failure::Infrastructure);
        }
        let out = IssueMutationOutcome {
            record: record.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: Some(receipt.id),
        };
        let mut staged = self.state.clone();
        let mut issues = self.authority.snapshot();
        issues.insert_from_risk(record);
        staged.prepared.remove(p.id());
        staged.audits.push(audit);
        staged.idem.insert(
            c.context.idempotency_id,
            (sig, Stored::Mutation(out.clone())),
        );
        self.authority.apply_unchecked(issues);
        self.state = staged;
        Ok(out)
    }
    fn execute_lower_issue_classification(
        &mut self,
        c: ApproveAndExecuteLowerIssueClassification,
    ) -> Result<IssueMutationOutcome, Failure> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(Failure::Changed);
        }
        let sig = Identity::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(Stored::Mutation(v)) = self
            .replay(&c.context, &sig)
            .map_err(|_| Failure::Idempotency)?
        {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(Failure::Changed)?;
        let (id, v, proposed_classification, rationale) = match p.operation() {
            WorkManagementOperation::LowerIssueClassification {
                issue_id,
                issue_version,
                proposed_classification,
                rationale,
                ..
            } => (
                issue_id.clone(),
                *issue_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => return Err(Failure::Changed),
        };
        let current = self.authority.issue(&id).ok_or(Failure::Changed)?.clone();
        if current.version != v {
            return Err(Failure::Changed);
        }
        let authoritative_class = self
            .classification
            .current_classification(&id, current.classification)
            .map_err(|_| Failure::Infrastructure)?;
        let current_classification =
            DataClassification::combine(current.classification, authoritative_class);
        if current_classification == DataClassification::Unclassified {
            return Err(Failure::Changed);
        }
        let op = WorkManagementOperation::LowerIssueClassification {
            issue_id: id.clone(),
            issue_version: current.version,
            current_classification,
            proposed_classification,
            rationale,
        };
        if self.policy.current_policy(&op) == IssueExecutionPolicy::Denied {
            return Err(Failure::PolicyDenied);
        }
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            op.clone(),
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| Failure::Changed)?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: op,
            classification: fresh.preview().classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            &p,
            &c.approval,
            &snapshot,
            self.clock.now(),
            self.ids
                .next_approval_receipt_id()
                .map_err(|_| Failure::Infrastructure)?,
            &self.authorization,
        )
        .map_err(|e| {
            use crate::work_management::WorkManagementApprovalValidationError as V;
            match e {
                V::Unauthorized => Failure::Unauthorized,
                V::PreparedIntentMismatch | V::DigestMismatch | V::Expired | V::PreviewChanged => {
                    Failure::Changed
                }
            }
        })?;
        let mut record = current;
        record.classification = proposed_classification;
        record.version = record.version.next().ok_or(Failure::Infrastructure)?;
        let audit = self
            .make_audit(&id, "issue.classification_lowered", &c.context, true, true)
            .map_err(|_| Failure::Infrastructure)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(Failure::Infrastructure);
        }
        let out = IssueMutationOutcome {
            record: record.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: Some(receipt.id),
        };
        let mut staged = self.state.clone();
        let mut issues = self.authority.snapshot();
        issues.insert_from_risk(record);
        staged.prepared.remove(p.id());
        staged.audits.push(audit);
        staged.idem.insert(
            c.context.idempotency_id,
            (sig, Stored::Mutation(out.clone())),
        );
        self.authority.apply_unchecked(issues);
        self.state = staged;
        Ok(out)
    }
    fn support(
        &self,
        ids: &[EvidenceReferenceId],
        judgments: &[HumanJudgment],
        role: EvidenceRole,
    ) -> Result<SupportWitness, Failure> {
        let mut metadata = Vec::with_capacity(ids.len());
        for id in ids {
            let m = self.evidence.resolve(id).map_err(|error| match error {
                IssueEvidenceAuthorityError::Unavailable => Failure::EvidenceUnavailable,
                IssueEvidenceAuthorityError::NotFound => Failure::Evidence,
            })?;
            if m.id() != id || m.role() != role {
                return Err(Failure::Evidence);
            }
            metadata.push(m)
        }
        // Evidence stays required: an empty Evidence list is refused even
        // when a Judgment is present.
        if metadata.is_empty() {
            return Err(Failure::Evidence);
        }
        EvidenceOrJudgment::new(metadata, judgments.to_vec())
            .map_err(|_| Failure::Evidence)?
            .evaluate_evidence_required()
            .map_err(|_| Failure::Evidence)
    }
    fn replay(
        &self,
        c: &IssueOperationContext,
        sig: &Identity,
    ) -> Result<Option<Stored>, DomainError> {
        match self.state.idem.get(&c.idempotency_id) {
            Some((old, v)) if old == sig => Ok(Some(v.clone())),
            Some(_) => Err(self.error(
                ErrorCode::DomainIdempotencyConflict,
                "issue.idempotency_conflict",
                c,
                false,
            )),
            None => Ok(None),
        }
    }
    fn prepared_issue(&self, id: &PreparedIntentId) -> Option<IssueId> {
        self.state
            .prepared
            .get(id)
            .and_then(|p| match p.operation() {
                WorkManagementOperation::ResolveIssue { issue_id, .. }
                | WorkManagementOperation::CloseIssue { issue_id, .. }
                | WorkManagementOperation::ReopenIssue { issue_id, .. }
                | WorkManagementOperation::LowerIssueClassification { issue_id, .. } => {
                    Some(issue_id.clone())
                }
                _ => None,
            })
    }
    fn make_audit(
        &mut self,
        id: &IssueId,
        code: &str,
        c: &IssueOperationContext,
        h2: bool,
        effect: bool,
    ) -> Result<AuditEvent, DomainError> {
        let aid = self.ids.next_audit_event_id().map_err(|_| {
            self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true)
        })?;
        let effects = if effect {
            vec![AuditEffectCode::parse(code).map_err(|_| {
                self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true)
            })?]
        } else {
            vec![]
        };
        let disposition = AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            if h2 {
                AuditApprovalOutcome::Approved
            } else {
                AuditApprovalOutcome::NotRequired
            },
            AuditExecutionOutcome::Succeeded,
            if effect {
                AuditEffectScope::Complete
            } else {
                AuditEffectScope::None
            },
            effects,
        )
        .map_err(|_| self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true))?;
        Ok(AuditEvent::new(
            aid,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse(code).map_err(|_| {
                    self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true)
                })?,
                AuditTarget::Issue(id.clone()),
            ),
            c.correlation_id.clone(),
            disposition,
        ))
    }
    fn failure_audit(
        &mut self,
        id: Option<IssueId>,
        code: &str,
        c: &IssueOperationContext,
        f: Failure,
    ) -> Result<(), DomainError> {
        let aid = self.ids.next_audit_event_id().map_err(|_| {
            self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true)
        })?;
        let (po, ao, eo) = match f {
            Failure::PolicyDenied => (
                AuditPolicyOutcome::Denied,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::NotAttempted,
            ),
            Failure::Infrastructure | Failure::EvidenceUnavailable => (
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Failed,
            ),
            _ => (
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Rejected,
                AuditExecutionOutcome::NotAttempted,
            ),
        };
        let d =
            AuditDisposition::new(po, ao, eo, AuditEffectScope::None, vec![]).map_err(|_| {
                self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true)
            })?;
        let target = id
            .unwrap_or_else(|| IssueId::parse("issue-unknown").unwrap_or_else(|_| unreachable!()));
        self.state.audits.push(AuditEvent::new(
            aid,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse(code).unwrap_or_else(|_| unreachable!()),
                AuditTarget::Issue(target),
            ),
            c.correlation_id.clone(),
            d,
        ));
        Ok(())
    }
    fn lifecycle_error(&self, id: &IssueId, c: &IssueOperationContext) -> DomainError {
        let mut e = self.error(
            ErrorCode::DomainConflict,
            "issue.stale_or_illegal",
            c,
            false,
        );
        if let Some(x) = self.authority.issue(id) {
            e = e
                .with_extension(SafeErrorExtension::CurrentVersion(x.version))
                .with_param(
                    MessageParam::new(
                        "current_state",
                        SafeParamValue::Identifier(x.state.as_persisted().to_owned()),
                    )
                    .unwrap_or_else(|_| unreachable!()),
                )
                .with_param(
                    MessageParam::new(
                        "allowed_next_intents",
                        SafeParamValue::FieldKey(
                            match x.state {
                                IssueState::Open => "issue.prepare_resolve",
                                IssueState::Resolved => "issue.prepare_close_or_reopen",
                                IssueState::Closed => "issue.no_transition",
                            }
                            .to_owned(),
                        ),
                    )
                    .unwrap_or_else(|_| unreachable!()),
                )
                .with_param(
                    MessageParam::new(
                        "remediation",
                        SafeParamValue::FieldKey("issue.refresh_and_reprepare".to_owned()),
                    )
                    .unwrap_or_else(|_| unreachable!()),
                );
        }
        e
    }
    fn error(
        &self,
        code: ErrorCode,
        key: &str,
        c: &IssueOperationContext,
        retryable: bool,
    ) -> DomainError {
        DomainError::new(
            code,
            MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
            c.correlation_id.clone(),
            retryable,
        )
    }
    fn map_failure(&self, f: Failure, c: &IssueOperationContext) -> DomainError {
        match f {
            Failure::PolicyDenied | Failure::Unauthorized | Failure::Evidence => self.error(
                ErrorCode::SecurityPolicyDenied,
                "issue.security_denied",
                c,
                false,
            ),
            Failure::Changed => self.error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "issue.preview_changed",
                c,
                false,
            ),
            Failure::Idempotency => self.error(
                ErrorCode::DomainIdempotencyConflict,
                "issue.idempotency_conflict",
                c,
                false,
            ),
            Failure::Infrastructure
            | Failure::EvidenceUnavailable
            | Failure::PrepareEvidenceUnavailable => {
                self.error(ErrorCode::PlatformInternal, "issue.infrastructure", c, true)
            }
        }
    }
}
/// True only for a genuine lowering (`proposed` strictly less restrictive
/// than `current`). See `portfolio::is_genuine_lowering` -- same check,
/// duplicated per module rather than shared.
fn is_genuine_lowering(current: DataClassification, proposed: DataClassification) -> bool {
    proposed.combine(current) != proposed
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    PolicyDenied,
    Unauthorized,
    Evidence,
    Changed,
    Idempotency,
    Infrastructure,
    EvidenceUnavailable,
    PrepareEvidenceUnavailable,
}
fn required_role(op: &WorkManagementOperation) -> EvidenceRole {
    match op {
        WorkManagementOperation::ResolveIssue { .. } => EvidenceRole::IssueResolution,
        WorkManagementOperation::CloseIssue { .. } => EvidenceRole::IssueClosureVerification,
        WorkManagementOperation::ReopenIssue { .. } => EvidenceRole::IssueFailedVerification,
        _ => unreachable!(),
    }
}
