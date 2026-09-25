//! Governed Risk lifecycle.  This module intentionally owns only the Risk and
//! minimal occurrence Issue seam; full Issue resolution belongs to the Issue
//! lifecycle (`issues.rs`).
#![allow(clippy::result_large_err)]

use std::collections::HashMap;

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
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, IssueId,
    PreparedIntentId, RiskId, StakeholderId,
};
use crate::issues::{IssueRecord, IssueStore, SharedIssueAuthority};
use crate::time::{Clock, UtcTimestamp};
use crate::value::BoundedText;
use crate::work_management::{
    validate_and_mint_work_management_h2a_receipt, ApprovalAuthorizationPort, IssueState,
    RiskResponseType, RiskState, WorkManagementApproval, WorkManagementAuthoritativeSnapshot,
    WorkManagementCurrentPolicy, WorkManagementOperation, WorkManagementPreparedIntent,
    WorkManagementRationale,
};

pub type RiskTitle = BoundedText<240>;
pub type RiskDetails = BoundedText<2_000>;
pub type RiskRationale = BoundedText<2_000>;
pub type ResidualExposure = BoundedText<240>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskOperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskRecord {
    id: RiskId,
    title: RiskTitle,
    details: RiskDetails,
    classification: DataClassification,
    state: RiskState,
    response: Option<RiskResponseType>,
    owner: Option<StakeholderId>,
    rationale: Option<RiskRationale>,
    residual_exposure: Option<ResidualExposure>,
    next_review_at: Option<UtcTimestamp>,
    in_exception_queue: bool,
    version: AggregateVersion,
}
impl RiskRecord {
    /// Rehydrate the minimal H2a close shape: a fresh open Risk closed exactly
    /// once, with no response attributes carried from another lifecycle path.
    pub fn from_persisted_closed_from_created_open(
        id: RiskId,
        title: RiskTitle,
        details: RiskDetails,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Result<Self, RiskRehydrationError> {
        if classification == DataClassification::Unclassified
            || version
                != AggregateVersion::initial()
                    .next()
                    .unwrap_or(AggregateVersion::initial())
        {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        Ok(Self {
            id,
            title,
            details,
            classification,
            state: RiskState::Closed,
            response: None,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            in_exception_queue: false,
            version,
        })
    }
    /// Rehydrate the minimal H2a occurrence shape: a fresh open Risk that has
    /// transitioned exactly once and has not accumulated response attributes.
    pub fn from_persisted_occurred_from_created_open(
        id: RiskId,
        title: RiskTitle,
        details: RiskDetails,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Result<Self, RiskRehydrationError> {
        if classification == DataClassification::Unclassified
            || version
                != AggregateVersion::initial()
                    .next()
                    .unwrap_or(AggregateVersion::initial())
        {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        Ok(Self {
            id,
            title,
            details,
            classification,
            state: RiskState::Occurred,
            response: None,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            in_exception_queue: true,
            version,
        })
    }
    pub fn from_persisted_created_open(
        id: RiskId,
        title: RiskTitle,
        details: RiskDetails,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Result<Self, RiskRehydrationError> {
        if classification == DataClassification::Unclassified
            || version != AggregateVersion::initial()
        {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        Ok(Self {
            id,
            title,
            details,
            classification,
            state: RiskState::Open,
            response: None,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            in_exception_queue: true,
            version,
        })
    }
    /// Rehydrate an Open Risk that has never transitioned lifecycle state but
    /// may have had its classification governed-lowered one or more times
    /// (H2a `LowerRiskClassification`), which advances `version`
    /// without leaving `RiskState::Open`. Unlike
    /// [`Self::from_persisted_created_open`], `version` is not pinned to
    /// [`AggregateVersion::initial`] -- classification lowering is the only
    /// operation that can advance an Open Risk's version without a lifecycle
    /// transition, so every other narrow shape (no accumulated response
    /// attributes, `in_exception_queue` true) still applies unchanged.
    pub fn from_persisted_open_with_lowered_classification(
        id: RiskId,
        title: RiskTitle,
        details: RiskDetails,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Result<Self, RiskRehydrationError> {
        if classification == DataClassification::Unclassified {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        Ok(Self {
            id,
            title,
            details,
            classification,
            state: RiskState::Open,
            response: None,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            in_exception_queue: true,
            version,
        })
    }
    /// Rehydrate a Risk whose response has been recorded at least once
    /// (`UpdateRiskResponse`, persisted from schema v47). The response
    /// attributes are exactly what the last update stored; the exception-queue
    /// flag is what the domain derives from them — a Closed Risk leaves the
    /// queue, an Open or Occurred one is in it unless the response is Accept
    /// or Transfer — never read from a mutable column. Accept and Transfer
    /// require every attribute, as the command does.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted_responded(
        id: RiskId,
        title: RiskTitle,
        details: RiskDetails,
        classification: DataClassification,
        version: AggregateVersion,
        state: RiskState,
        response: RiskResponseType,
        owner: Option<StakeholderId>,
        rationale: Option<RiskRationale>,
        residual_exposure: Option<ResidualExposure>,
        next_review_at: Option<UtcTimestamp>,
    ) -> Result<Self, RiskRehydrationError> {
        if classification == DataClassification::Unclassified
            || version == AggregateVersion::initial()
        {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        let settled = matches!(
            response,
            RiskResponseType::Accept | RiskResponseType::Transfer
        );
        if settled
            && (owner.is_none()
                || rationale.is_none()
                || residual_exposure.is_none()
                || next_review_at.is_none())
        {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        Ok(Self {
            id,
            title,
            details,
            classification,
            state,
            response: Some(response),
            owner,
            rationale,
            residual_exposure,
            next_review_at,
            in_exception_queue: state != RiskState::Closed && !settled,
            version,
        })
    }
    pub fn id(&self) -> &RiskId {
        &self.id
    }
    pub fn title(&self) -> &RiskTitle {
        &self.title
    }
    pub fn details(&self) -> &RiskDetails {
        &self.details
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn state(&self) -> RiskState {
        self.state
    }
    pub const fn response(&self) -> Option<RiskResponseType> {
        self.response
    }
    pub fn owner(&self) -> Option<&StakeholderId> {
        self.owner.as_ref()
    }
    pub fn rationale(&self) -> Option<&RiskRationale> {
        self.rationale.as_ref()
    }
    pub fn residual_exposure(&self) -> Option<&ResidualExposure> {
        self.residual_exposure.as_ref()
    }
    pub const fn next_review_at(&self) -> Option<UtcTimestamp> {
        self.next_review_at
    }
    pub const fn in_exception_queue(&self) -> bool {
        self.in_exception_queue
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskRehydrationError {
    InvalidCreatedRisk,
    DuplicateRisk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskPersistenceSnapshot {
    risks: Vec<RiskRecord>,
    occurrence_issues: Vec<IssueRecord>,
    links: Vec<RiskIssueLink>,
    prepared: Vec<WorkManagementPreparedIntent>,
    replay: Vec<RiskReplayEntry>,
    audits: Vec<AuditEvent>,
}

/// Opaque, domain-validated restart authority for the Risk H2a runtime.
///
/// Unlike [`RiskPersistenceSnapshot`], this value cannot be built from
/// aggregate projections.  It is only emitted by a live runtime or the
/// narrow typed H2a decode input below, and is deliberately consumed by
/// rehydration.
#[doc(hidden)]
pub struct RiskH2aRuntimeSnapshot(RiskPersistenceSnapshot);

impl RiskH2aRuntimeSnapshot {
    /// Outstanding, not-yet-approved Risk H2a previews (V13) that survived a
    /// restart. Consumers that only need to know they exist, without taking
    /// rehydration authority, may read this directly instead of calling
    /// [`InMemoryRiskService::rehydrate_with_h2a`].
    pub fn prepared(&self) -> &[WorkManagementPreparedIntent] {
        self.0.prepared()
    }

    pub(crate) fn merge_occurrence_issues_into(
        &self,
        store: &mut IssueStore,
    ) -> Result<(), RiskRehydrationError> {
        for issue in &self.0.occurrence_issues {
            if store.contains(issue.id()) {
                return Err(RiskRehydrationError::DuplicateRisk);
            }
            store.insert_from_risk(issue.clone());
        }
        Ok(())
    }
}

/// The only Risk H2a terminal outcomes that can become durable replay
/// authority.  A policy denial happens after an execute attempt begins, but
/// before any state effect or approval receipt is minted.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskH2aTerminalOperation {
    RecordOccurrence,
    Close,
}

/// Typed durable evidence for one post-start Risk H2a policy denial.
///
/// This is deliberately not a generic state import. A persistence adapter
/// must decode every component into domain values and the decoder below
/// validates the operation-specific audit/error topology before it can create
/// a restart authority.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskH2aTerminalReplay {
    operation: RiskH2aTerminalOperation,
    risk_id: RiskId,
    prepared: WorkManagementPreparedIntent,
    approval: WorkManagementApproval,
    context: RiskOperationContext,
    error: DomainError,
    audit: AuditEvent,
}

impl RiskH2aTerminalReplay {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation: RiskH2aTerminalOperation,
        risk_id: RiskId,
        prepared: WorkManagementPreparedIntent,
        approval: WorkManagementApproval,
        context: RiskOperationContext,
        error: DomainError,
        audit: AuditEvent,
    ) -> Self {
        Self {
            operation,
            risk_id,
            prepared,
            approval,
            context,
            error,
            audit,
        }
    }
}

/// Typed durable evidence for one Risk H2a rejection (v46): the preview
/// that was refused and the zero-effect outcome that refused it. The decode
/// boundary validates the whole topology -- audit code, target, disposition,
/// timing, and that the intent is not also still outstanding -- so SQLite is
/// never the sole judge of what a valid rejection looks like.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskH2aRejectionReplay {
    prepared: WorkManagementPreparedIntent,
    context: RiskOperationContext,
    outcome: crate::work_management::RejectedPreparedIntentOutcome,
}

impl RiskH2aRejectionReplay {
    pub fn new(
        prepared: WorkManagementPreparedIntent,
        context: RiskOperationContext,
        outcome: crate::work_management::RejectedPreparedIntentOutcome,
    ) -> Self {
        Self {
            prepared,
            context,
            outcome,
        }
    }
}

/// Adapter-only decode boundary for durable Risk H2a terminal replay.
///
/// It accepts only typed Risk records and a policy-denial evidence bundle;
/// it cannot import arbitrary maps, SQL, or raw serialized state. Successful
/// H2a execution has a stricter, separate persistence vertical and must not
/// be inferred through this boundary.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskH2aPersistenceDecodeInput {
    base: RiskPersistenceSnapshot,
    terminal_replays: Vec<RiskH2aTerminalReplay>,
    prepared: Vec<WorkManagementPreparedIntent>,
    rejections: Vec<RiskH2aRejectionReplay>,
}

impl RiskH2aPersistenceDecodeInput {
    pub fn new(
        base: RiskPersistenceSnapshot,
        terminal_replays: Vec<RiskH2aTerminalReplay>,
    ) -> Self {
        Self {
            base,
            terminal_replays,
            prepared: vec![],
            rejections: vec![],
        }
    }

    /// Additively restores durable rejections (v46). Each is validated in
    /// [`Self::decode`]; a rejected preview that is also handed in as
    /// outstanding is refused as corrupt rather than resurrected.
    #[must_use]
    pub fn with_rejections(mut self, rejections: Vec<RiskH2aRejectionReplay>) -> Self {
        self.rejections = rejections;
        self
    }

    /// Additively restores outstanding, not-yet-approved Risk H2a previews
    /// (V13) so they survive a restart. Omitting this call keeps the prior
    /// behavior of decoding with no outstanding preview.
    #[must_use]
    pub fn with_prepared(mut self, prepared: Vec<WorkManagementPreparedIntent>) -> Self {
        self.prepared = prepared;
        self
    }

    pub fn decode(self) -> Result<RiskH2aRuntimeSnapshot, RiskRehydrationError> {
        let mut replay = Vec::with_capacity(self.terminal_replays.len());
        let mut audits = Vec::with_capacity(self.terminal_replays.len());
        let mut replay_ids = std::collections::BTreeSet::new();
        let mut audit_ids = std::collections::BTreeSet::new();
        let mut terminal_prepared_ids = std::collections::BTreeSet::new();
        for terminal in self.terminal_replays {
            if !replay_ids.insert(terminal.context.idempotency_id.clone())
                || !audit_ids.insert(terminal.audit.id().clone())
                || !terminal_prepared_ids.insert(terminal.prepared.id().clone())
                || terminal.approval.idempotency_id() != &terminal.context.idempotency_id
                || terminal.prepared.id() != terminal.approval.prepared_id()
                || terminal.prepared.payload_digest()
                    != terminal.approval.acknowledged_payload_digest()
                || terminal.error.code() != ErrorCode::SecurityPolicyDenied
                || terminal.error.retryable()
                || terminal.error.correlation_id() != &terminal.context.correlation_id
                || !self.base.risks.iter().any(|risk| {
                    risk.id() == &terminal.risk_id
                        && risk.state() == RiskState::Open
                        && prepared_matches_terminal_risk(&terminal, risk)
                })
                || !matches!(terminal.approval.actor(), AuditActor::HeadOfProducts)
                || !valid_terminal_audit(&terminal)
            {
                return Err(RiskRehydrationError::InvalidCreatedRisk);
            }
            audits.push(terminal.audit.clone());
            replay.push(RiskReplayEntry {
                idempotency_id: terminal.context.idempotency_id,
                signature: execution_signature(&terminal.approval),
                stored: Stored::Terminal(RiskTerminalFailure {
                    prepared: terminal.prepared,
                    error: terminal.error,
                    audit: terminal.audit,
                }),
            });
        }
        for rejection in self.rejections {
            let risk_id = match rejection.prepared.operation() {
                WorkManagementOperation::RecordRiskOccurrence { risk_id, .. }
                | WorkManagementOperation::CloseRisk { risk_id, .. } => risk_id.clone(),
                _ => return Err(RiskRehydrationError::InvalidCreatedRisk),
            };
            if !replay_ids.insert(rejection.context.idempotency_id.clone())
                || !audit_ids.insert(rejection.outcome.audit_event().id().clone())
                || !terminal_prepared_ids.insert(rejection.prepared.id().clone())
                || !valid_rejection(&rejection, &risk_id)
                || !self.base.risks.iter().any(|risk| risk.id() == &risk_id)
            {
                return Err(RiskRehydrationError::InvalidCreatedRisk);
            }
            audits.push(rejection.outcome.audit_event().clone());
            replay.push(RiskReplayEntry {
                idempotency_id: rejection.context.idempotency_id,
                signature: Signature::RejectPrepared(
                    rejection.prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                ),
                stored: Stored::Rejected(rejection.outcome),
            });
        }
        RiskPersistenceSnapshot::try_new_with_h2a(
            self.base.risks,
            self.base.occurrence_issues,
            self.base.links,
            self.prepared,
            replay,
            audits,
        )
        .map(RiskH2aRuntimeSnapshot)
    }

    pub fn try_into_runtime_snapshot(self) -> Result<RiskH2aRuntimeSnapshot, RiskRehydrationError> {
        self.decode()
    }
}

/// A rejection is valid only when its outcome and audit say exactly what
/// [`InMemoryRiskService::reject_risk_prepared_intent`] would have said.
fn valid_rejection(rejection: &RiskH2aRejectionReplay, risk_id: &RiskId) -> bool {
    let outcome = &rejection.outcome;
    let audit = outcome.audit_event();
    outcome.prepared_intent_id() == rejection.prepared.id()
        && outcome.expired_at_rejection()
            == (outcome.rejected_at() >= rejection.prepared.preview().expires_at())
        && audit.occurred_at() == outcome.rejected_at()
        && audit.actor() == AuditActor::HeadOfProducts
        && audit.module() == AuditModule::WorkManagement
        && audit.code().as_str() == crate::work_management::RISK_PREPARED_REJECTED_AUDIT_CODE
        && audit.target() == &AuditTarget::Risk(risk_id.clone())
        && audit.correlation_id() == &rejection.context.correlation_id
        && audit.policy_outcome() == AuditPolicyOutcome::Allowed
        && audit.approval_outcome() == AuditApprovalOutcome::Rejected
}

fn valid_terminal_audit(terminal: &RiskH2aTerminalReplay) -> bool {
    let expected_code = match terminal.operation {
        RiskH2aTerminalOperation::RecordOccurrence => "risk.occurrence_denied",
        RiskH2aTerminalOperation::Close => "risk.close_denied",
    };
    terminal.prepared.classification() == terminal.prepared.preview().classification()
        && terminal.audit.actor() == AuditActor::HeadOfProducts
        && terminal.audit.module() == AuditModule::WorkManagement
        && terminal.audit.code().as_str() == expected_code
        && terminal.audit.target() == &AuditTarget::Risk(terminal.risk_id.clone())
        && terminal.audit.correlation_id() == &terminal.context.correlation_id
        && terminal.audit.policy_outcome() == AuditPolicyOutcome::Denied
        && terminal.audit.approval_outcome() == AuditApprovalOutcome::NotRequired
        && terminal.audit.execution_outcome() == AuditExecutionOutcome::NotAttempted
        && terminal.audit.effect_scope() == AuditEffectScope::None
        && terminal.audit.actual_effects().is_empty()
}

fn prepared_matches_terminal_risk(terminal: &RiskH2aTerminalReplay, risk: &RiskRecord) -> bool {
    match (terminal.operation, terminal.prepared.operation()) {
        (
            RiskH2aTerminalOperation::RecordOccurrence,
            WorkManagementOperation::RecordRiskOccurrence {
                risk_id,
                risk_version,
                ..
            },
        )
        | (
            RiskH2aTerminalOperation::Close,
            WorkManagementOperation::CloseRisk {
                risk_id,
                risk_version,
                ..
            },
        ) => risk_id == risk.id() && *risk_version == risk.version(),
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RiskReplayEntry {
    idempotency_id: IdempotencyId,
    signature: Signature,
    stored: Stored,
}

impl RiskPersistenceSnapshot {
    pub fn try_new(
        risks: Vec<RiskRecord>,
        occurrence_issues: Vec<IssueRecord>,
        links: Vec<RiskIssueLink>,
    ) -> Result<Self, RiskRehydrationError> {
        Self::try_new_with_h2a(risks, occurrence_issues, links, vec![], vec![], vec![])
    }

    fn try_new_with_h2a(
        risks: Vec<RiskRecord>,
        occurrence_issues: Vec<IssueRecord>,
        links: Vec<RiskIssueLink>,
        prepared: Vec<WorkManagementPreparedIntent>,
        replay: Vec<RiskReplayEntry>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, RiskRehydrationError> {
        let mut ids = std::collections::BTreeSet::new();
        if risks.iter().any(|risk| !ids.insert(risk.id().clone())) {
            return Err(RiskRehydrationError::DuplicateRisk);
        }
        let mut issue_ids = std::collections::BTreeSet::new();
        if occurrence_issues
            .iter()
            .any(|issue| !issue_ids.insert(issue.id().clone()))
        {
            return Err(RiskRehydrationError::DuplicateRisk);
        }
        let risk_map = risks
            .iter()
            .map(|risk| (risk.id(), risk))
            .collect::<std::collections::BTreeMap<_, _>>();
        let issue_map = occurrence_issues
            .iter()
            .map(|issue| (issue.id(), issue))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut pairs = std::collections::BTreeSet::new();
        let mut links_per_risk = std::collections::BTreeMap::<&RiskId, usize>::new();
        for link in &links {
            let Some(risk) = risk_map.get(link.risk_id()) else {
                return Err(RiskRehydrationError::InvalidCreatedRisk);
            };
            let Some(issue) = issue_map.get(link.issue_id()) else {
                return Err(RiskRehydrationError::InvalidCreatedRisk);
            };
            if !pairs.insert((link.risk_id(), link.issue_id()))
                || risk.state() != RiskState::Occurred
                || risk.version() != link.risk_version()
                || risk.classification() != link.classification()
                || issue.source_risk_id() != Some(link.risk_id())
                || issue.recurrence_of().is_some()
                || issue.state() != IssueState::Open
                || issue.resolution_type().is_some()
                || issue.resolution_rationale().is_some()
                || !issue.resolution_evidence().is_empty()
                || !issue.closure_verification_evidence().is_empty()
                || !issue.failed_verification_evidence().is_empty()
                || !issue.reopen_rationales().is_empty()
                || !issue.support_history().is_empty()
                || issue.version() != link.issue_version()
                || issue.classification() != link.classification()
            {
                return Err(RiskRehydrationError::InvalidCreatedRisk);
            }
            *links_per_risk.entry(link.risk_id()).or_default() += 1;
        }
        for risk in &risks {
            let link_count = links_per_risk.get(risk.id()).copied().unwrap_or_default();
            if (risk.state() == RiskState::Occurred && link_count != 1)
                || (risk.state() != RiskState::Occurred && link_count != 0)
            {
                return Err(RiskRehydrationError::InvalidCreatedRisk);
            }
        }
        if occurrence_issues.len() != links.len() {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        let mut prepared_ids = std::collections::BTreeSet::new();
        if prepared
            .iter()
            .any(|intent| !prepared_ids.insert(intent.id().clone()))
        {
            return Err(RiskRehydrationError::DuplicateRisk);
        }
        let mut replay_ids = std::collections::BTreeSet::new();
        if replay
            .iter()
            .any(|entry| !replay_ids.insert(entry.idempotency_id.clone()))
        {
            return Err(RiskRehydrationError::DuplicateRisk);
        }
        let mut audit_ids = std::collections::BTreeSet::new();
        if audits
            .iter()
            .any(|audit| !audit_ids.insert(audit.id().clone()))
        {
            return Err(RiskRehydrationError::DuplicateRisk);
        }
        let mut terminal_prepared_ids = std::collections::BTreeSet::new();
        for entry in &replay {
            if let Stored::Terminal(terminal) = &entry.stored {
                let signature_matches = matches!(
                    &entry.signature,
                    Signature::Execute(prepared_id, actor, digest)
                        if prepared_id == terminal.prepared.id()
                            && actor == &AuditActor::HeadOfProducts
                            && digest == terminal.prepared.payload_digest()
                );
                if !audit_ids.contains(terminal.audit.id())
                    || terminal.audit.correlation_id() != terminal.error.correlation_id()
                    || prepared_ids.contains(terminal.prepared.id())
                    || !terminal_prepared_ids.insert(terminal.prepared.id().clone())
                    || !signature_matches
                {
                    return Err(RiskRehydrationError::InvalidCreatedRisk);
                }
            }
        }
        for entry in &replay {
            if let Stored::Rejected(outcome) = &entry.stored {
                let signature_matches = matches!(
                    &entry.signature,
                    Signature::RejectPrepared(prepared_id, actor)
                        if prepared_id == outcome.prepared_intent_id()
                            && actor == &AuditActor::HeadOfProducts
                );
                // A rejected preview is consumed. If the same id is also
                // handed in as outstanding, the two stories contradict and
                // neither is trusted.
                if !signature_matches
                    || prepared_ids.contains(outcome.prepared_intent_id())
                    || !audit_ids.contains(outcome.audit_event().id())
                {
                    return Err(RiskRehydrationError::InvalidCreatedRisk);
                }
            }
        }
        Ok(Self {
            risks,
            occurrence_issues,
            links,
            prepared,
            replay,
            audits,
        })
    }

    pub fn risks(&self) -> &[RiskRecord] {
        &self.risks
    }

    pub fn occurrence_issues(&self) -> &[IssueRecord] {
        &self.occurrence_issues
    }

    pub fn prepared(&self) -> &[WorkManagementPreparedIntent] {
        &self.prepared
    }

    pub fn links(&self) -> &[RiskIssueLink] {
        &self.links
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskIssueLink {
    risk_id: RiskId,
    issue_id: IssueId,
    risk_version: AggregateVersion,
    issue_version: AggregateVersion,
    classification: DataClassification,
}
impl RiskIssueLink {
    pub fn from_persisted(
        risk_id: RiskId,
        issue_id: IssueId,
        risk_version: AggregateVersion,
        issue_version: AggregateVersion,
        classification: DataClassification,
    ) -> Result<Self, RiskRehydrationError> {
        if classification == DataClassification::Unclassified {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        Ok(Self {
            risk_id,
            issue_id,
            risk_version,
            issue_version,
            classification,
        })
    }

    pub fn risk_id(&self) -> &RiskId {
        &self.risk_id
    }
    pub fn issue_id(&self) -> &IssueId {
        &self.issue_id
    }
    pub const fn risk_version(&self) -> AggregateVersion {
        self.risk_version
    }
    pub const fn issue_version(&self) -> AggregateVersion {
        self.issue_version
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskMutationOutcome<T> {
    pub record: T,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: Option<ApprovalReceiptId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OccurredRiskOutcome {
    pub risk: RiskRecord,
    pub issue: IssueRecord,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: ApprovalReceiptId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateRisk {
    pub id: RiskId,
    pub title: RiskTitle,
    pub details: RiskDetails,
    pub classification: DataClassification,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateRiskResponse {
    pub risk_id: RiskId,
    pub expected_version: AggregateVersion,
    pub response: RiskResponseType,
    pub owner: Option<StakeholderId>,
    pub rationale: Option<RiskRationale>,
    pub residual_exposure: Option<ResidualExposure>,
    pub next_review_at: Option<UtcTimestamp>,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareRecordRiskOccurrence {
    pub risk_id: RiskId,
    pub expected_version: AggregateVersion,
    pub issue_id: IssueId,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareCloseRisk {
    pub risk_id: RiskId,
    pub expected_version: AggregateVersion,
    pub rationale: WorkManagementRationale,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteRecordRiskOccurrence {
    pub approval: WorkManagementApproval,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteCloseRisk {
    pub approval: WorkManagementApproval,
    pub context: RiskOperationContext,
}
/// v46: the Head of Products refuses a pending occurrence/close preview.
/// See [`InMemoryRiskService::reject_risk_prepared_intent`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectRiskPreparedIntent {
    pub prepared_id: PreparedIntentId,
    pub actor: crate::audit::AuditActor,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerRiskClassification {
    pub risk_id: RiskId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: RiskOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerRiskClassification {
    pub approval: WorkManagementApproval,
    pub context: RiskOperationContext,
}

pub trait RiskServiceIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, crate::DomainValueError>;
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, crate::DomainValueError>;
}
pub trait RiskExecutionPolicyPort {
    fn current_policy(&self, operation: &WorkManagementOperation) -> RiskExecutionPolicy;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskExecutionPolicy {
    Allowed,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskEvidenceAuthorityError {
    Unavailable,
    NotFound,
}
pub trait RiskEvidenceAuthorityPort {
    fn linked_evidence_is_current(
        &self,
        _risk_id: &RiskId,
    ) -> Result<bool, RiskEvidenceAuthorityError> {
        Ok(true)
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowRiskEvidence;
impl RiskEvidenceAuthorityPort for AllowRiskEvidence {}

pub trait RiskClassificationAuthorityPort {
    fn current_classification(
        &self,
        _risk_id: &RiskId,
        recorded: DataClassification,
    ) -> Result<DataClassification, RiskEvidenceAuthorityError> {
        Ok(recorded)
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordedRiskClassification;
impl RiskClassificationAuthorityPort for RecordedRiskClassification {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Signature {
    Create(RiskId, RiskTitle, RiskDetails, DataClassification),
    Update(
        RiskId,
        AggregateVersion,
        RiskResponseType,
        Option<StakeholderId>,
        Option<RiskRationale>,
        Option<ResidualExposure>,
        Option<UtcTimestamp>,
    ),
    PrepareOccurrence(RiskId, AggregateVersion, IssueId),
    PrepareClose(RiskId, AggregateVersion, WorkManagementRationale),
    RejectPrepared(PreparedIntentId, crate::audit::AuditActor),
    PrepareLowerClassification(
        RiskId,
        AggregateVersion,
        DataClassification,
        WorkManagementRationale,
    ),
    Execute(
        PreparedIntentId,
        crate::audit::AuditActor,
        crate::work_management::WorkManagementPayloadDigest,
    ),
}

fn execution_signature(approval: &WorkManagementApproval) -> Signature {
    Signature::Execute(
        approval.prepared_id().clone(),
        approval.actor(),
        approval.acknowledged_payload_digest().clone(),
    )
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum Stored {
    Risk(RiskMutationOutcome<RiskRecord>),
    Prepared(WorkManagementPreparedIntent),
    Occurred(OccurredRiskOutcome),
    Closed(RiskMutationOutcome<RiskRecord>),
    Terminal(RiskTerminalFailure),
    Rejected(crate::work_management::RejectedPreparedIntentOutcome),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RiskTerminalFailure {
    prepared: WorkManagementPreparedIntent,
    error: DomainError,
    audit: AuditEvent,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RiskFailure {
    NotFound,
    Conflict,
    LifecycleConflict,
    IdempotencyConflict,
    PolicyDenied,
    Unauthorized,
    DigestMismatch,
    Expired,
    PreviewChanged,
    Infrastructure,
}
#[derive(Clone, Default)]
struct Store {
    risks: HashMap<RiskId, RiskRecord>,
    prepared: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    idem: HashMap<IdempotencyId, (Signature, Stored)>,
    audits: Vec<AuditEvent>,
    links: Vec<RiskIssueLink>,
}

/// Stages the Issue side of a Risk occurrence without touching authoritative state.
/// The Risk service preflights this value before its single Store replacement.
#[derive(Clone)]
pub(crate) struct RiskIssueStage {
    issues: IssueStore,
    issue: IssueRecord,
}

impl RiskIssueStage {
    fn prepare(current: &IssueStore, issue: IssueRecord) -> Result<Self, RiskFailure> {
        if current.contains(issue.id()) {
            return Err(RiskFailure::Conflict);
        }
        let mut issues = current.clone();
        issues.insert_from_risk(issue.clone());
        Ok(Self { issues, issue })
    }

    fn preflight(&self) -> Result<(), RiskFailure> {
        if self.issues.get(self.issue.id()) == Some(&self.issue) {
            Ok(())
        } else {
            Err(RiskFailure::Infrastructure)
        }
    }

    fn apply_unchecked(self, authority: &SharedIssueAuthority) {
        authority.apply_unchecked(self.issues);
    }
}

pub struct InMemoryRiskService<C, I, Z, P, E, A> {
    clock: C,
    ids: I,
    authorization: Z,
    policy: P,
    evidence: E,
    classification: A,
    issue_authority: SharedIssueAuthority,
    state: Store,
    fail_next_commit: bool,
}
impl<
        C: Clock,
        I: RiskServiceIdSource,
        Z: ApprovalAuthorizationPort,
        P: RiskExecutionPolicyPort,
        E: RiskEvidenceAuthorityPort,
        A: RiskClassificationAuthorityPort,
    > InMemoryRiskService<C, I, Z, P, E, A>
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
        issue_authority: SharedIssueAuthority,
    ) -> Self {
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence,
            classification,
            issue_authority,
            state: Store::default(),
            fail_next_commit: false,
        }
    }
    pub fn risk(&self, id: &RiskId) -> Option<&RiskRecord> {
        self.state.risks.get(id)
    }
    pub fn issue(&self, id: &IssueId) -> Option<IssueRecord> {
        self.issue_authority.issue(id)
    }
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.state.audits
    }
    pub fn risk_issue_links(&self) -> &[RiskIssueLink] {
        &self.state.links
    }
    pub fn risk_issue_link(&self, risk_id: &RiskId, issue_id: &IssueId) -> Option<&RiskIssueLink> {
        self.state
            .links
            .iter()
            .find(|link| link.risk_id() == risk_id && link.issue_id() == issue_id)
    }
    /// Produces the domain-typed Risk restart boundary.  Persistence adapters
    /// must decode and validate their own durable representation before this
    /// value is used to reconstruct authority.
    #[doc(hidden)]
    pub fn persistence_snapshot_with_h2a(
        &self,
    ) -> Result<RiskH2aRuntimeSnapshot, RiskRehydrationError> {
        let mut risks = self.state.risks.values().cloned().collect::<Vec<_>>();
        risks.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        let mut links = self.state.links.clone();
        links.sort_by(|left, right| {
            (left.risk_id().as_str(), left.issue_id().as_str())
                .cmp(&(right.risk_id().as_str(), right.issue_id().as_str()))
        });
        let shared_issues = self.issue_authority.snapshot();
        let mut occurrence_issues = links
            .iter()
            .map(|link| {
                shared_issues
                    .get(link.issue_id())
                    .cloned()
                    .ok_or(RiskRehydrationError::InvalidCreatedRisk)
            })
            .collect::<Result<Vec<_>, _>>()?;
        occurrence_issues.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        let mut replay = self
            .state
            .idem
            .iter()
            .map(|(idempotency_id, (signature, stored))| RiskReplayEntry {
                idempotency_id: idempotency_id.clone(),
                signature: signature.clone(),
                stored: stored.clone(),
            })
            .collect::<Vec<_>>();
        replay.sort_by(|left, right| {
            left.idempotency_id
                .as_str()
                .cmp(right.idempotency_id.as_str())
        });
        let mut prepared = self.state.prepared.values().cloned().collect::<Vec<_>>();
        prepared.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        RiskPersistenceSnapshot::try_new_with_h2a(
            risks,
            occurrence_issues,
            links,
            prepared,
            replay,
            self.state.audits.clone(),
        )
        .map(RiskH2aRuntimeSnapshot)
    }
    /// Reconstructs an in-memory Risk runtime from an already validated,
    /// domain-typed persistence snapshot.  This is intentionally not a
    /// generic state import surface.
    #[doc(hidden)]
    pub fn rehydrate_with_h2a(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence: E,
        classification: A,
        snapshot: RiskH2aRuntimeSnapshot,
    ) -> Result<Self, RiskRehydrationError> {
        let issue_store = IssueStore::from_records(snapshot.0.occurrence_issues.clone())
            .ok_or(RiskRehydrationError::DuplicateRisk)?;
        let issue_authority = SharedIssueAuthority::new();
        issue_authority.apply_unchecked(issue_store);
        Self::rehydrate_with_h2a_and_issue_authority(
            clock,
            ids,
            authorization,
            policy,
            evidence,
            classification,
            snapshot,
            issue_authority,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rehydrate_with_h2a_and_issue_authority(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence: E,
        classification: A,
        snapshot: RiskH2aRuntimeSnapshot,
        issue_authority: SharedIssueAuthority,
    ) -> Result<Self, RiskRehydrationError> {
        let snapshot = snapshot.0;
        if snapshot
            .occurrence_issues
            .iter()
            .any(|issue| issue_authority.issue(issue.id()).as_ref() != Some(issue))
        {
            return Err(RiskRehydrationError::InvalidCreatedRisk);
        }
        let risks = snapshot
            .risks
            .into_iter()
            .map(|risk| (risk.id().clone(), risk))
            .collect();
        let prepared = snapshot
            .prepared
            .into_iter()
            .map(|intent| (intent.id().clone(), intent))
            .collect();
        let idem = snapshot
            .replay
            .into_iter()
            .map(|entry| (entry.idempotency_id, (entry.signature, entry.stored)))
            .collect();
        Ok(Self {
            clock,
            ids,
            authorization,
            policy,
            evidence,
            classification,
            issue_authority,
            state: Store {
                risks,
                prepared,
                idem,
                audits: snapshot.audits,
                links: snapshot.links,
            },
            fail_next_commit: false,
        })
    }
    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true
    }
    /// Produces an isolated transactional copy sharing only the explicitly
    /// supplied isolated Issue authority.
    pub(crate) fn isolated_copy_with_issue_authority(
        &self,
        issue_authority: SharedIssueAuthority,
    ) -> Self
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
            issue_authority,
            state: self.state.clone(),
            fail_next_commit: false,
        }
    }
    pub fn discard_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let existed = self.state.prepared.remove(id).is_some();
        if existed {
            self.state.idem.retain(|_, (_, stored)| {
                !matches!(stored, Stored::Prepared(prepared) if prepared.id() == id)
            });
        }
        existed
    }
    /// v46 H2a rejection: the Head of Products refuses a pending occurrence
    /// or close preview. A successful command with no effect: the intent is
    /// consumed and can never be executed, one zero-effect audit is recorded,
    /// no receipt is minted and no Risk changes. Rejecting an already
    /// consumed intent (executed, terminally denied or already rejected) is a
    /// conflict; an unknown intent is not found; an expired-but-unconsumed
    /// preview is rejected normally and the outcome says it had expired.
    /// Unlike a silent [`Self::discard_prepared_intent`], this is the
    /// recorded retreat route DG3 requires of reject/cancel.
    pub fn reject_risk_prepared_intent(
        &mut self,
        c: RejectRiskPreparedIntent,
    ) -> Result<crate::work_management::RejectedPreparedIntentOutcome, DomainError> {
        let signature = Signature::RejectPrepared(c.prepared_id.clone(), c.actor);
        if let Some(stored) = self.replay(&c.context, &signature)? {
            return match stored {
                Stored::Rejected(outcome) => Ok(outcome),
                _ => Err(self.map_failure(RiskFailure::IdempotencyConflict, &c.context)),
            };
        }
        if c.actor != AuditActor::HeadOfProducts || !self.authorization.authorize(c.actor) {
            return Err(self.map_failure(RiskFailure::Unauthorized, &c.context));
        }
        let Some(intent) = self.state.prepared.get(&c.prepared_id).cloned() else {
            let known = self.state.idem.values().any(|(_, stored)| match stored {
                Stored::Prepared(prepared) => prepared.id() == &c.prepared_id,
                Stored::Rejected(outcome) => outcome.prepared_intent_id() == &c.prepared_id,
                Stored::Terminal(terminal) => terminal.prepared.id() == &c.prepared_id,
                Stored::Risk(_) | Stored::Occurred(_) | Stored::Closed(_) => false,
            });
            return Err(self.map_failure(
                if known {
                    RiskFailure::LifecycleConflict
                } else {
                    RiskFailure::NotFound
                },
                &c.context,
            ));
        };
        let risk_id = match intent.operation() {
            WorkManagementOperation::RecordRiskOccurrence { risk_id, .. }
            | WorkManagementOperation::CloseRisk { risk_id, .. } => risk_id.clone(),
            _ => return Err(self.map_failure(RiskFailure::LifecycleConflict, &c.context)),
        };
        let now = self.clock.now();
        let audit_id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| self.map_failure(RiskFailure::Infrastructure, &c.context))?;
        let audit = crate::work_management::prepared_intent_rejection_audit(
            audit_id,
            now,
            crate::work_management::RISK_PREPARED_REJECTED_AUDIT_CODE,
            AuditTarget::Risk(risk_id),
            c.context.correlation_id.clone(),
        )
        .ok_or_else(|| self.map_failure(RiskFailure::Infrastructure, &c.context))?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(self.map_failure(RiskFailure::Infrastructure, &c.context));
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
            (signature, Stored::Rejected(outcome.clone())),
        );
        self.state = staged;
        Ok(outcome)
    }
    pub fn create_risk(
        &mut self,
        c: CreateRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let sig = Signature::Create(
            c.id.clone(),
            c.title.clone(),
            c.details.clone(),
            c.classification,
        );
        if let Some(Stored::Risk(v)) = self.replay(&c.context, &sig)? {
            return Ok(v);
        }
        if c.classification == DataClassification::Unclassified {
            return Err(self.err(
                ErrorCode::ValidationInvalidField,
                "risk.classification",
                &c.context,
            ));
        }
        if self.state.risks.contains_key(&c.id) {
            return Err(self.err(ErrorCode::DomainConflict, "risk.already_exists", &c.context));
        }
        let r = RiskRecord {
            id: c.id,
            title: c.title,
            details: c.details,
            classification: c.classification,
            state: RiskState::Open,
            response: None,
            owner: None,
            rationale: None,
            residual_exposure: None,
            next_review_at: None,
            in_exception_queue: true,
            version: AggregateVersion::initial(),
        };
        let audit = self
            .audit(
                AuditTarget::Risk(r.id.clone()),
                "risk.created",
                &c.context,
                true,
            )
            .map_err(|_| self.err(ErrorCode::PlatformInternal, "audit.failed", &c.context))?;
        let out = RiskMutationOutcome {
            record: r.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        self.state.risks.insert(r.id.clone(), r);
        self.state.audits.push(audit);
        self.state
            .idem
            .insert(c.context.idempotency_id, (sig, Stored::Risk(out.clone())));
        Ok(out)
    }
    pub fn update_risk_response(
        &mut self,
        c: UpdateRiskResponse,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let sig = Signature::Update(
            c.risk_id.clone(),
            c.expected_version,
            c.response,
            c.owner.clone(),
            c.rationale.clone(),
            c.residual_exposure.clone(),
            c.next_review_at,
        );
        if let Some(Stored::Risk(v)) = self.replay(&c.context, &sig)? {
            return Ok(v);
        }
        let mut r = self
            .state
            .risks
            .get(&c.risk_id)
            .cloned()
            .ok_or_else(|| self.err(ErrorCode::DomainNotFound, "risk.not_found", &c.context))?;
        if r.version != c.expected_version || r.state != RiskState::Open {
            return Err(self.risk_conflict(&r, &c.context));
        }
        if matches!(
            c.response,
            RiskResponseType::Accept | RiskResponseType::Transfer
        ) && (c.owner.is_none()
            || c.rationale.is_none()
            || c.residual_exposure.is_none()
            || c.next_review_at.is_none())
        {
            return Err(self.err(
                ErrorCode::ValidationInvalidField,
                "risk.accepted_fields_required",
                &c.context,
            ));
        }
        r.response = Some(c.response);
        r.owner = c.owner;
        r.rationale = c.rationale;
        r.residual_exposure = c.residual_exposure;
        r.next_review_at = c.next_review_at;
        r.in_exception_queue = !matches!(
            c.response,
            RiskResponseType::Accept | RiskResponseType::Transfer
        );
        r.version = next(r.version)
            .ok_or_else(|| self.err(ErrorCode::PlatformInternal, "version.overflow", &c.context))?;
        self.replace_risk(r.clone(), sig, c.context, "risk.response_updated")
    }
    pub fn risk_reenters_queue(
        &self,
        id: &RiskId,
        now: UtcTimestamp,
        exposure_increased: bool,
        control_invalid: bool,
    ) -> Result<bool, DomainError> {
        let r = self.state.risks.get(id).ok_or_else(|| {
            DomainError::new(
                ErrorCode::DomainNotFound,
                message_key("risk.not_found"),
                correlation("risk-query"),
                false,
            )
        })?;
        if r.state != RiskState::Open {
            return Ok(false);
        }
        if !matches!(
            r.response,
            Some(RiskResponseType::Accept | RiskResponseType::Transfer)
        ) {
            return Ok(r.in_exception_queue);
        }
        let due = r.next_review_at.is_some_and(|x| now >= x);
        let stale = !self.evidence.linked_evidence_is_current(id).map_err(|_| {
            DomainError::new(
                ErrorCode::PlatformInternal,
                message_key("risk.evidence_unavailable"),
                correlation("risk-query"),
                true,
            )
        })?;
        Ok(r.in_exception_queue || due || exposure_increased || stale || control_invalid)
    }
    pub fn prepare_record_risk_occurrence(
        &mut self,
        c: PrepareRecordRiskOccurrence,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = c.context.clone();
        let target = c.risk_id.clone();
        match self.prepare_occurrence(c) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.record_prepare_failure_audit(&target, "risk.occurrence_denied", &context, e)?;
                Err(self.prepare_failure_error(&target, e, &context))
            }
        }
    }
    pub fn prepare_close_risk(
        &mut self,
        c: PrepareCloseRisk,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = c.context.clone();
        let target = c.risk_id.clone();
        match self.prepare_close(c) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.record_prepare_failure_audit(&target, "risk.close_denied", &context, e)?;
                Err(self.prepare_failure_error(&target, e, &context))
            }
        }
    }
    pub fn prepare_lower_risk_classification(
        &mut self,
        c: PrepareLowerRiskClassification,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = c.context.clone();
        let target = c.risk_id.clone();
        match self.prepare_lower_risk_classification_cause(c) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.record_prepare_failure_audit(
                    &target,
                    "risk.classification_lowering_denied",
                    &context,
                    e,
                )?;
                Err(self.prepare_failure_error(&target, e, &context))
            }
        }
    }
    pub fn approve_and_execute_record_risk_occurrence(
        &mut self,
        c: ApproveAndExecuteRecordRiskOccurrence,
    ) -> Result<OccurredRiskOutcome, DomainError> {
        self.approve_and_execute_record_risk_occurrence_traced(c).0
    }
    /// Same execution as [`Self::approve_and_execute_record_risk_occurrence`],
    /// additionally reporting whether a failure is the one accepted durable
    /// post-start terminal (`RiskFailure::PolicyDenied`) that a persistence
    /// adapter must reconstruct as an exact replay. This does not change the
    /// executed behavior; it only exposes an existing internal distinction
    /// that collapses to an identical public `DomainError` for every denied
    /// post-start failure kind.
    #[doc(hidden)]
    pub fn approve_and_execute_record_risk_occurrence_with_durability(
        &mut self,
        c: ApproveAndExecuteRecordRiskOccurrence,
    ) -> (Result<OccurredRiskOutcome, DomainError>, bool) {
        self.approve_and_execute_record_risk_occurrence_traced(c)
    }
    fn approve_and_execute_record_risk_occurrence_traced(
        &mut self,
        c: ApproveAndExecuteRecordRiskOccurrence,
    ) -> (Result<OccurredRiskOutcome, DomainError>, bool) {
        match self.replayed_terminal_error(&c.approval, &c.context) {
            Ok(Some(error)) => return (Err(error), false),
            Ok(None) => {}
            Err(error) => return (Err(error), false),
        }
        let context = c.context.clone();
        let target = self.prepared_risk_target(c.approval.prepared_id());
        let signature = execution_signature(&c.approval);
        match self.execute_occurrence(c) {
            Ok(value) => (Ok(value), false),
            Err(failure) => {
                let error = self.map_failure(failure, &context);
                let audit = match self.record_execution_failure_audit(
                    target,
                    "risk.occurrence_denied",
                    &context,
                    failure,
                ) {
                    Ok(audit) => audit,
                    Err(error) => return (Err(error), false),
                };
                let durable = is_durable_post_start_terminal(failure);
                if durable {
                    if let Some(prepared) = self.terminalize_prepared_execution(&signature) {
                        self.state.idem.insert(
                            context.idempotency_id,
                            (
                                signature,
                                Stored::Terminal(RiskTerminalFailure {
                                    prepared,
                                    error: error.clone(),
                                    audit,
                                }),
                            ),
                        );
                    }
                }
                (Err(error), durable)
            }
        }
    }
    pub fn approve_and_execute_close_risk(
        &mut self,
        c: ApproveAndExecuteCloseRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        self.approve_and_execute_close_risk_traced(c).0
    }
    /// Same execution as [`Self::approve_and_execute_close_risk`], additionally
    /// reporting whether a failure is the one accepted durable post-start
    /// terminal (`RiskFailure::PolicyDenied`). See
    /// [`Self::approve_and_execute_record_risk_occurrence_with_durability`].
    #[doc(hidden)]
    pub fn approve_and_execute_close_risk_with_durability(
        &mut self,
        c: ApproveAndExecuteCloseRisk,
    ) -> (Result<RiskMutationOutcome<RiskRecord>, DomainError>, bool) {
        self.approve_and_execute_close_risk_traced(c)
    }
    fn approve_and_execute_close_risk_traced(
        &mut self,
        c: ApproveAndExecuteCloseRisk,
    ) -> (Result<RiskMutationOutcome<RiskRecord>, DomainError>, bool) {
        match self.replayed_terminal_error(&c.approval, &c.context) {
            Ok(Some(error)) => return (Err(error), false),
            Ok(None) => {}
            Err(error) => return (Err(error), false),
        }
        let context = c.context.clone();
        let target = self.prepared_risk_target(c.approval.prepared_id());
        let signature = execution_signature(&c.approval);
        match self.execute_close(c) {
            Ok(value) => (Ok(value), false),
            Err(failure) => {
                let error = self.map_failure(failure, &context);
                let audit = match self.record_execution_failure_audit(
                    target,
                    "risk.close_denied",
                    &context,
                    failure,
                ) {
                    Ok(audit) => audit,
                    Err(error) => return (Err(error), false),
                };
                let durable = is_durable_post_start_terminal(failure);
                if durable {
                    if let Some(prepared) = self.terminalize_prepared_execution(&signature) {
                        self.state.idem.insert(
                            context.idempotency_id,
                            (
                                signature,
                                Stored::Terminal(RiskTerminalFailure {
                                    prepared,
                                    error: error.clone(),
                                    audit,
                                }),
                            ),
                        );
                    }
                }
                (Err(error), durable)
            }
        }
    }
    pub fn approve_and_execute_lower_risk_classification(
        &mut self,
        c: ApproveAndExecuteLowerRiskClassification,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        self.approve_and_execute_lower_risk_classification_traced(c)
            .0
    }
    /// Same execution as [`Self::approve_and_execute_lower_risk_classification`],
    /// additionally reporting whether a failure is the one accepted durable
    /// post-start terminal (`RiskFailure::PolicyDenied`). See
    /// [`Self::approve_and_execute_record_risk_occurrence_with_durability`].
    #[doc(hidden)]
    pub fn approve_and_execute_lower_risk_classification_with_durability(
        &mut self,
        c: ApproveAndExecuteLowerRiskClassification,
    ) -> (Result<RiskMutationOutcome<RiskRecord>, DomainError>, bool) {
        self.approve_and_execute_lower_risk_classification_traced(c)
    }
    fn approve_and_execute_lower_risk_classification_traced(
        &mut self,
        c: ApproveAndExecuteLowerRiskClassification,
    ) -> (Result<RiskMutationOutcome<RiskRecord>, DomainError>, bool) {
        match self.replayed_terminal_error(&c.approval, &c.context) {
            Ok(Some(error)) => return (Err(error), false),
            Ok(None) => {}
            Err(error) => return (Err(error), false),
        }
        let context = c.context.clone();
        let target = self.prepared_risk_target(c.approval.prepared_id());
        let signature = execution_signature(&c.approval);
        match self.execute_lower_risk_classification(c) {
            Ok(value) => (Ok(value), false),
            Err(failure) => {
                let error = self.map_failure(failure, &context);
                let audit = match self.record_execution_failure_audit(
                    target,
                    "risk.classification_lowering_denied",
                    &context,
                    failure,
                ) {
                    Ok(audit) => audit,
                    Err(error) => return (Err(error), false),
                };
                let durable = is_durable_post_start_terminal(failure);
                if durable {
                    if let Some(prepared) = self.terminalize_prepared_execution(&signature) {
                        self.state.idem.insert(
                            context.idempotency_id,
                            (
                                signature,
                                Stored::Terminal(RiskTerminalFailure {
                                    prepared,
                                    error: error.clone(),
                                    audit,
                                }),
                            ),
                        );
                    }
                }
                (Err(error), durable)
            }
        }
    }
    fn prepare_occurrence(
        &mut self,
        c: PrepareRecordRiskOccurrence,
    ) -> Result<WorkManagementPreparedIntent, RiskFailure> {
        let sig =
            Signature::PrepareOccurrence(c.risk_id.clone(), c.expected_version, c.issue_id.clone());
        if let Some(Stored::Prepared(p)) = self
            .replay(&c.context, &sig)
            .map_err(|_| RiskFailure::IdempotencyConflict)?
        {
            return Ok(p);
        }
        let r = self
            .state
            .risks
            .get(&c.risk_id)
            .ok_or(RiskFailure::NotFound)?;
        if r.version != c.expected_version || r.state != RiskState::Open {
            return Err(RiskFailure::LifecycleConflict);
        }
        let op = WorkManagementOperation::RecordRiskOccurrence {
            risk_id: r.id.clone(),
            risk_version: r.version,
            issue_id: c.issue_id,
            issue_classification: r.classification,
        };
        if matches!(self.policy.current_policy(&op), RiskExecutionPolicy::Denied) {
            return Err(RiskFailure::PolicyDenied);
        }
        let id = self
            .ids
            .next_prepared_intent_id()
            .map_err(|_| RiskFailure::Infrastructure)?;
        let p =
            WorkManagementPreparedIntent::prepare(id, op, r.classification, None, self.clock.now())
                .map_err(|_| RiskFailure::Conflict)?;
        self.state.prepared.insert(p.id().clone(), p.clone());
        self.state
            .idem
            .insert(c.context.idempotency_id, (sig, Stored::Prepared(p.clone())));
        Ok(p)
    }
    fn prepare_close(
        &mut self,
        c: PrepareCloseRisk,
    ) -> Result<WorkManagementPreparedIntent, RiskFailure> {
        let sig =
            Signature::PrepareClose(c.risk_id.clone(), c.expected_version, c.rationale.clone());
        if let Some(Stored::Prepared(p)) = self
            .replay(&c.context, &sig)
            .map_err(|_| RiskFailure::IdempotencyConflict)?
        {
            return Ok(p);
        }
        let r = self
            .state
            .risks
            .get(&c.risk_id)
            .ok_or(RiskFailure::NotFound)?;
        if r.version != c.expected_version || r.state != RiskState::Open {
            return Err(RiskFailure::LifecycleConflict);
        }
        let op = WorkManagementOperation::CloseRisk {
            risk_id: r.id.clone(),
            risk_version: r.version,
            rationale: c.rationale,
        };
        if matches!(self.policy.current_policy(&op), RiskExecutionPolicy::Denied) {
            return Err(RiskFailure::PolicyDenied);
        }
        let id = self
            .ids
            .next_prepared_intent_id()
            .map_err(|_| RiskFailure::Infrastructure)?;
        let p =
            WorkManagementPreparedIntent::prepare(id, op, r.classification, None, self.clock.now())
                .map_err(|_| RiskFailure::Conflict)?;
        let _rationale = match p.operation() {
            WorkManagementOperation::CloseRisk { rationale, .. } => rationale.clone(),
            _ => return Err(RiskFailure::Conflict),
        };
        self.state
            .idem
            .insert(c.context.idempotency_id, (sig, Stored::Prepared(p.clone())));
        self.state.prepared.insert(p.id().clone(), p.clone());
        Ok(p)
    }
    fn prepare_lower_risk_classification_cause(
        &mut self,
        c: PrepareLowerRiskClassification,
    ) -> Result<WorkManagementPreparedIntent, RiskFailure> {
        let sig = Signature::PrepareLowerClassification(
            c.risk_id.clone(),
            c.expected_version,
            c.proposed_classification,
            c.rationale.clone(),
        );
        if let Some(Stored::Prepared(p)) = self
            .replay(&c.context, &sig)
            .map_err(|_| RiskFailure::IdempotencyConflict)?
        {
            return Ok(p);
        }
        let r = self
            .state
            .risks
            .get(&c.risk_id)
            .ok_or(RiskFailure::NotFound)?;
        if r.version != c.expected_version {
            return Err(RiskFailure::LifecycleConflict);
        }
        if !is_genuine_lowering(r.classification, c.proposed_classification) {
            return Err(RiskFailure::Conflict);
        }
        let current_classification = r.classification;
        let op = WorkManagementOperation::LowerRiskClassification {
            risk_id: r.id.clone(),
            risk_version: r.version,
            current_classification,
            proposed_classification: c.proposed_classification,
            rationale: c.rationale,
        };
        if matches!(self.policy.current_policy(&op), RiskExecutionPolicy::Denied) {
            return Err(RiskFailure::PolicyDenied);
        }
        let id = self
            .ids
            .next_prepared_intent_id()
            .map_err(|_| RiskFailure::Infrastructure)?;
        let p = WorkManagementPreparedIntent::prepare(
            id,
            op,
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| RiskFailure::Conflict)?;
        self.state
            .idem
            .insert(c.context.idempotency_id, (sig, Stored::Prepared(p.clone())));
        self.state.prepared.insert(p.id().clone(), p.clone());
        Ok(p)
    }
    fn execute_occurrence(
        &mut self,
        c: ApproveAndExecuteRecordRiskOccurrence,
    ) -> Result<OccurredRiskOutcome, RiskFailure> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(RiskFailure::Conflict);
        }
        let exec_sig = Signature::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(Stored::Occurred(v)) = self
            .replay(&c.context, &exec_sig)
            .map_err(|_| RiskFailure::IdempotencyConflict)?
        {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(RiskFailure::PreviewChanged)?;
        let (id, v, iid) = match p.operation() {
            WorkManagementOperation::RecordRiskOccurrence {
                risk_id,
                risk_version,
                issue_id,
                ..
            } => (risk_id.clone(), *risk_version, issue_id.clone()),
            _ => return Err(RiskFailure::PreviewChanged),
        };
        let r = self
            .state
            .risks
            .get(&id)
            .cloned()
            .ok_or(RiskFailure::PreviewChanged)?;
        if r.version != v || r.state != RiskState::Open {
            return Err(RiskFailure::PreviewChanged);
        }
        let current_classification = self
            .classification
            .current_classification(&id, r.classification)
            .map_err(|_| RiskFailure::Infrastructure)?;
        let current_op = WorkManagementOperation::RecordRiskOccurrence {
            risk_id: id.clone(),
            risk_version: r.version,
            issue_id: iid.clone(),
            issue_classification: current_classification,
        };
        if matches!(
            self.policy.current_policy(&current_op),
            RiskExecutionPolicy::Denied
        ) {
            return Err(RiskFailure::PolicyDenied);
        }
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            current_op.clone(),
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| RiskFailure::PreviewChanged)?;
        let current = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: current_classification,
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            &p,
            &c.approval,
            &current,
            self.clock.now(),
            self.ids
                .next_approval_receipt_id()
                .map_err(|_| RiskFailure::Infrastructure)?,
            &self.authorization,
        )
        .map_err(|e| match e {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                RiskFailure::Unauthorized
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch => {
                RiskFailure::DigestMismatch
            }
            crate::work_management::WorkManagementApprovalValidationError::Expired => {
                RiskFailure::Expired
            }
            _ => RiskFailure::PreviewChanged,
        })?;
        let mut risk = r.clone();
        risk.state = RiskState::Occurred;
        risk.version = next(risk.version).ok_or(RiskFailure::Infrastructure)?;
        let issue = IssueRecord::from_risk(
            iid.clone(),
            id.clone(),
            r.title.clone(),
            r.details.clone(),
            r.classification,
        );
        let issue_stage = RiskIssueStage::prepare(&self.issue_authority.snapshot(), issue.clone())?;
        let a1 = self
            .audit_h2(
                AuditTarget::Risk(id.clone()),
                "risk.occurred",
                &c.context,
                true,
            )
            .map_err(|_| RiskFailure::Infrastructure)?;
        let a2 = self
            .audit_h2(
                AuditTarget::Issue(iid.clone()),
                "issue.created_from_risk",
                &c.context,
                true,
            )
            .map_err(|_| RiskFailure::Infrastructure)?;
        let a3 = self
            .audit_h2(
                AuditTarget::Risk(id.clone()),
                "risk.issue_linked",
                &c.context,
                true,
            )
            .map_err(|_| RiskFailure::Infrastructure)?;
        issue_stage.preflight()?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(RiskFailure::Infrastructure);
        }
        let mut staged = self.state.clone();
        staged.risks.insert(id.clone(), risk.clone());
        staged.links.push(RiskIssueLink {
            risk_id: id.clone(),
            issue_id: iid.clone(),
            risk_version: risk.version,
            issue_version: issue.version(),
            classification: r.classification,
        });
        staged.prepared.remove(p.id());
        staged.audits.extend([a1.clone(), a2.clone(), a3.clone()]);
        let out = OccurredRiskOutcome {
            risk,
            issue,
            audit_events: vec![a1, a2, a3],
            approval_receipt_id: receipt.id,
        };
        staged.idem.insert(
            c.context.idempotency_id,
            (exec_sig, Stored::Occurred(out.clone())),
        );
        issue_stage.apply_unchecked(&self.issue_authority);
        self.state = staged;
        Ok(out)
    }
    fn execute_close(
        &mut self,
        c: ApproveAndExecuteCloseRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, RiskFailure> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(RiskFailure::Conflict);
        }
        let exec_sig = Signature::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(Stored::Closed(v)) = self
            .replay(&c.context, &exec_sig)
            .map_err(|_| RiskFailure::IdempotencyConflict)?
        {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(RiskFailure::PreviewChanged)?;
        let (id, v) = match p.operation() {
            WorkManagementOperation::CloseRisk {
                risk_id,
                risk_version,
                ..
            } => (risk_id.clone(), *risk_version),
            _ => return Err(RiskFailure::PreviewChanged),
        };
        let r = self
            .state
            .risks
            .get(&id)
            .cloned()
            .ok_or(RiskFailure::PreviewChanged)?;
        if r.version != v || r.state != RiskState::Open {
            return Err(RiskFailure::PreviewChanged);
        }
        let current_classification = self
            .classification
            .current_classification(&id, r.classification)
            .map_err(|_| RiskFailure::Infrastructure)?;
        let rationale = match p.operation() {
            WorkManagementOperation::CloseRisk { rationale, .. } => rationale.clone(),
            _ => return Err(RiskFailure::PreviewChanged),
        };
        let current_op = WorkManagementOperation::CloseRisk {
            risk_id: id.clone(),
            risk_version: r.version,
            rationale,
        };
        if matches!(
            self.policy.current_policy(&current_op),
            RiskExecutionPolicy::Denied
        ) {
            return Err(RiskFailure::PolicyDenied);
        }
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            current_op.clone(),
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| RiskFailure::PreviewChanged)?;
        let current = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: current_classification,
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            &p,
            &c.approval,
            &current,
            self.clock.now(),
            self.ids
                .next_approval_receipt_id()
                .map_err(|_| RiskFailure::Infrastructure)?,
            &self.authorization,
        )
        .map_err(|e| match e {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                RiskFailure::Unauthorized
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch => {
                RiskFailure::DigestMismatch
            }
            crate::work_management::WorkManagementApprovalValidationError::Expired => {
                RiskFailure::Expired
            }
            _ => RiskFailure::PreviewChanged,
        })?;
        let mut x = r;
        x.state = RiskState::Closed;
        x.in_exception_queue = false;
        x.version = next(x.version).ok_or(RiskFailure::Infrastructure)?;
        let audit = self
            .audit_h2(
                AuditTarget::Risk(id.clone()),
                "risk.closed",
                &c.context,
                true,
            )
            .map_err(|_| RiskFailure::Infrastructure)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(RiskFailure::Infrastructure);
        }
        let mut staged = self.state.clone();
        staged.risks.insert(id, x.clone());
        staged.prepared.remove(p.id());
        staged.audits.push(audit.clone());
        let out = RiskMutationOutcome {
            record: x,
            audit_events: vec![audit],
            approval_receipt_id: Some(receipt.id),
        };
        staged.idem.insert(
            c.context.idempotency_id,
            (exec_sig, Stored::Closed(out.clone())),
        );
        self.state = staged;
        Ok(out)
    }
    fn execute_lower_risk_classification(
        &mut self,
        c: ApproveAndExecuteLowerRiskClassification,
    ) -> Result<RiskMutationOutcome<RiskRecord>, RiskFailure> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(RiskFailure::Conflict);
        }
        let exec_sig = Signature::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(Stored::Risk(v)) = self
            .replay(&c.context, &exec_sig)
            .map_err(|_| RiskFailure::IdempotencyConflict)?
        {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(RiskFailure::PreviewChanged)?;
        let (id, v, proposed_classification, rationale) = match p.operation() {
            WorkManagementOperation::LowerRiskClassification {
                risk_id,
                risk_version,
                proposed_classification,
                rationale,
                ..
            } => (
                risk_id.clone(),
                *risk_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => return Err(RiskFailure::PreviewChanged),
        };
        let r = self
            .state
            .risks
            .get(&id)
            .cloned()
            .ok_or(RiskFailure::PreviewChanged)?;
        if r.version != v {
            return Err(RiskFailure::PreviewChanged);
        }
        let current_classification = self
            .classification
            .current_classification(&id, r.classification)
            .map_err(|_| RiskFailure::Infrastructure)?;
        let current_op = WorkManagementOperation::LowerRiskClassification {
            risk_id: id.clone(),
            risk_version: r.version,
            current_classification,
            proposed_classification,
            rationale,
        };
        if matches!(
            self.policy.current_policy(&current_op),
            RiskExecutionPolicy::Denied
        ) {
            return Err(RiskFailure::PolicyDenied);
        }
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            current_op.clone(),
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| RiskFailure::PreviewChanged)?;
        let current = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: current_classification,
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            &p,
            &c.approval,
            &current,
            self.clock.now(),
            self.ids
                .next_approval_receipt_id()
                .map_err(|_| RiskFailure::Infrastructure)?,
            &self.authorization,
        )
        .map_err(|e| match e {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                RiskFailure::Unauthorized
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch => {
                RiskFailure::DigestMismatch
            }
            crate::work_management::WorkManagementApprovalValidationError::Expired => {
                RiskFailure::Expired
            }
            _ => RiskFailure::PreviewChanged,
        })?;
        let mut x = r;
        x.classification = proposed_classification;
        x.version = next(x.version).ok_or(RiskFailure::Infrastructure)?;
        let audit = self
            .audit_h2(
                AuditTarget::Risk(id.clone()),
                "risk.classification_lowered",
                &c.context,
                true,
            )
            .map_err(|_| RiskFailure::Infrastructure)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(RiskFailure::Infrastructure);
        }
        let mut staged = self.state.clone();
        staged.risks.insert(id, x.clone());
        staged.prepared.remove(p.id());
        staged.audits.push(audit.clone());
        let out = RiskMutationOutcome {
            record: x,
            audit_events: vec![audit],
            approval_receipt_id: Some(receipt.id),
        };
        staged.idem.insert(
            c.context.idempotency_id,
            (exec_sig, Stored::Risk(out.clone())),
        );
        self.state = staged;
        Ok(out)
    }
    fn replace_risk(
        &mut self,
        r: RiskRecord,
        sig: Signature,
        c: RiskOperationContext,
        code: &str,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let audit = self
            .audit(AuditTarget::Risk(r.id.clone()), code, &c, true)
            .map_err(|_| self.err(ErrorCode::PlatformInternal, "audit.failed", &c))?;
        let out = RiskMutationOutcome {
            record: r.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        self.state.risks.insert(r.id.clone(), r);
        self.state.audits.push(audit);
        self.state
            .idem
            .insert(c.idempotency_id, (sig, Stored::Risk(out.clone())));
        Ok(out)
    }
    fn replay(
        &self,
        context: &RiskOperationContext,
        sig: &Signature,
    ) -> Result<Option<Stored>, DomainError> {
        if let Some((old, v)) = self.state.idem.get(&context.idempotency_id) {
            if old == sig {
                return Ok(Some(v.clone()));
            }
            return Err(DomainError::new(
                ErrorCode::DomainIdempotencyConflict,
                message_key("risk.idempotency_conflict"),
                context.correlation_id.clone(),
                false,
            ));
        }
        Ok(None)
    }
    fn replayed_terminal_error(
        &self,
        approval: &WorkManagementApproval,
        context: &RiskOperationContext,
    ) -> Result<Option<DomainError>, DomainError> {
        let signature = execution_signature(approval);
        let Some((stored_signature, stored)) = self.state.idem.get(&context.idempotency_id) else {
            return Ok(None);
        };
        if stored_signature != &signature {
            return Err(self.err(
                ErrorCode::DomainIdempotencyConflict,
                "risk.idempotency_conflict",
                context,
            ));
        }
        match stored {
            Stored::Terminal(terminal) => Ok(Some(terminal.error.clone())),
            _ => Ok(None),
        }
    }
    fn terminalize_prepared_execution(
        &mut self,
        signature: &Signature,
    ) -> Option<WorkManagementPreparedIntent> {
        let Signature::Execute(prepared_id, _, _) = signature else {
            return None;
        };
        let prepared = self.state.prepared.remove(prepared_id)?;
        self.state.idem.retain(|_, (_, stored)| {
            !matches!(stored, Stored::Prepared(prepared) if prepared.id() == prepared_id)
        });
        Some(prepared)
    }
    fn audit(
        &mut self,
        target: AuditTarget,
        code: &str,
        c: &RiskOperationContext,
        effect: bool,
    ) -> Result<AuditEvent, crate::DomainValueError> {
        let id = self.ids.next_audit_event_id()?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(code)?,
            target,
        );
        let d = AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            if effect {
                AuditEffectScope::Complete
            } else {
                AuditEffectScope::None
            },
            if effect {
                vec![AuditEffectCode::parse(code)?]
            } else {
                vec![]
            },
        )
        .map_err(|_| crate::DomainValueError::new(crate::ValueErrorKind::InvalidCharacter))?;
        Ok(AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            action,
            c.correlation_id.clone(),
            d,
        ))
    }
    fn audit_h2(
        &mut self,
        target: AuditTarget,
        code: &str,
        c: &RiskOperationContext,
        effect: bool,
    ) -> Result<AuditEvent, crate::DomainValueError> {
        let id = self.ids.next_audit_event_id()?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(code)?,
            target,
        );
        let disposition = AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Approved,
            AuditExecutionOutcome::Succeeded,
            if effect {
                AuditEffectScope::Complete
            } else {
                AuditEffectScope::None
            },
            if effect {
                vec![AuditEffectCode::parse(code)?]
            } else {
                vec![]
            },
        )
        .map_err(|_| crate::DomainValueError::new(crate::ValueErrorKind::InvalidCharacter))?;
        Ok(AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            action,
            c.correlation_id.clone(),
            disposition,
        ))
    }
    fn map_failure(&self, failure: RiskFailure, context: &RiskOperationContext) -> DomainError {
        let (code, key, retryable) = match failure {
            RiskFailure::PolicyDenied | RiskFailure::Unauthorized => (
                ErrorCode::SecurityPolicyDenied,
                "risk.security_denied",
                false,
            ),
            RiskFailure::DigestMismatch | RiskFailure::Expired | RiskFailure::PreviewChanged => (
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "risk.preview_changed",
                false,
            ),
            RiskFailure::NotFound => (ErrorCode::DomainNotFound, "risk.not_found", false),
            RiskFailure::Conflict => (ErrorCode::DomainConflict, "risk.conflict", false),
            RiskFailure::LifecycleConflict => {
                (ErrorCode::DomainConflict, "risk.stale_or_illegal", false)
            }
            RiskFailure::IdempotencyConflict => (
                ErrorCode::DomainIdempotencyConflict,
                "risk.idempotency_conflict",
                false,
            ),
            RiskFailure::Infrastructure => {
                (ErrorCode::PlatformInternal, "risk.infrastructure", true)
            }
        };
        DomainError::new(
            code,
            message_key(key),
            context.correlation_id.clone(),
            retryable,
        )
    }
    fn record_prepare_failure_audit(
        &mut self,
        target: &RiskId,
        code: &str,
        context: &RiskOperationContext,
        failure: RiskFailure,
    ) -> Result<(), DomainError> {
        let policy = if matches!(failure, RiskFailure::PolicyDenied) {
            AuditPolicyOutcome::Denied
        } else {
            AuditPolicyOutcome::Allowed
        };
        let id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| self.err(ErrorCode::PlatformInternal, "audit.failed", context))?;
        let disposition = AuditDisposition::new(
            policy,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::NotAttempted,
            AuditEffectScope::None,
            vec![],
        )
        .unwrap_or_else(|_| unreachable!());
        self.state.audits.push(AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse(code).unwrap_or_else(|_| unreachable!()),
                AuditTarget::Risk(target.clone()),
            ),
            context.correlation_id.clone(),
            disposition,
        ));
        Ok(())
    }
    fn prepared_risk_target(&self, prepared_id: &PreparedIntentId) -> Option<RiskId> {
        self.state
            .prepared
            .get(prepared_id)
            .and_then(|prepared| match prepared.operation() {
                WorkManagementOperation::RecordRiskOccurrence { risk_id, .. }
                | WorkManagementOperation::CloseRisk { risk_id, .. }
                | WorkManagementOperation::LowerRiskClassification { risk_id, .. } => {
                    Some(risk_id.clone())
                }
                _ => None,
            })
    }
    fn record_execution_failure_audit(
        &mut self,
        target: Option<RiskId>,
        code: &str,
        context: &RiskOperationContext,
        failure: RiskFailure,
    ) -> Result<AuditEvent, DomainError> {
        let (policy, approval, execution) = match failure {
            RiskFailure::PolicyDenied => (
                AuditPolicyOutcome::Denied,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::NotAttempted,
            ),
            RiskFailure::Infrastructure => (
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Failed,
            ),
            RiskFailure::NotFound
            | RiskFailure::Conflict
            | RiskFailure::LifecycleConflict
            | RiskFailure::IdempotencyConflict
            | RiskFailure::Unauthorized
            | RiskFailure::DigestMismatch
            | RiskFailure::Expired
            | RiskFailure::PreviewChanged => (
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Rejected,
                AuditExecutionOutcome::NotAttempted,
            ),
        };
        let id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| self.err(ErrorCode::PlatformInternal, "audit.failed", context))?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(code).unwrap_or_else(|_| unreachable!()),
            AuditTarget::Risk(target.unwrap_or_else(|| {
                RiskId::parse("risk-unknown").unwrap_or_else(|_| unreachable!())
            })),
        );
        let disposition =
            AuditDisposition::new(policy, approval, execution, AuditEffectScope::None, vec![])
                .unwrap_or_else(|_| unreachable!());
        let audit = AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            action,
            context.correlation_id.clone(),
            disposition,
        );
        self.state.audits.push(audit.clone());
        Ok(audit)
    }
    fn err(&self, code: ErrorCode, key: &str, c: &RiskOperationContext) -> DomainError {
        DomainError::new(code, message_key(key), c.correlation_id.clone(), false)
    }
    fn prepare_failure_error(
        &self,
        target: &RiskId,
        failure: RiskFailure,
        context: &RiskOperationContext,
    ) -> DomainError {
        if failure == RiskFailure::LifecycleConflict {
            if let Some(risk) = self.state.risks.get(target) {
                return self.risk_conflict(risk, context);
            }
        }
        self.map_failure(failure, context)
    }
    fn err_conflict(
        &self,
        key: &str,
        c: &RiskOperationContext,
        version: Option<AggregateVersion>,
    ) -> DomainError {
        let mut error = self.err(ErrorCode::DomainConflict, key, c);
        if let Some(version) = version {
            error = error.with_extension(SafeErrorExtension::CurrentVersion(version));
        }
        error
    }
    fn risk_conflict(&self, risk: &RiskRecord, context: &RiskOperationContext) -> DomainError {
        self.err_conflict("risk.stale_or_illegal", context, Some(risk.version))
            .with_param(
                MessageParam::new(
                    "current_state",
                    SafeParamValue::Identifier(risk.state.as_persisted().to_owned()),
                )
                .unwrap_or_else(|_| unreachable!()),
            )
            .with_param(
                MessageParam::new(
                    "allowed_next_intents",
                    SafeParamValue::FieldKey(if risk.state == RiskState::Open {
                        "risk.update_response_or_prepare_transition".to_owned()
                    } else {
                        "risk.no_transition".to_owned()
                    }),
                )
                .unwrap_or_else(|_| unreachable!()),
            )
            .with_param(
                MessageParam::new(
                    "remediation",
                    SafeParamValue::FieldKey("risk.refresh_and_reprepare".to_owned()),
                )
                .unwrap_or_else(|_| unreachable!()),
            )
    }
}
fn next(v: AggregateVersion) -> Option<AggregateVersion> {
    v.next()
}

fn is_durable_post_start_terminal(failure: RiskFailure) -> bool {
    matches!(failure, RiskFailure::PolicyDenied)
}

/// True only for a genuine lowering (`proposed` strictly less restrictive
/// than `current`). See `portfolio::is_genuine_lowering` -- same check,
/// duplicated per module rather than shared.
fn is_genuine_lowering(current: DataClassification, proposed: DataClassification) -> bool {
    proposed.combine(current) != proposed
}
fn message_key(value: &str) -> MessageKey {
    MessageKey::parse(value).unwrap_or_else(|_| unreachable!())
}
fn correlation(value: &str) -> CorrelationId {
    CorrelationId::parse(value).unwrap_or_else(|_| unreachable!())
}
