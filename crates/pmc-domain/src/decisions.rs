//! Decision Request and immutable Decision lifecycle domain service.
#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};

use crate::actions::{
    ActionEvidenceAuthorityPort, ActionExecutionPolicyPort, ActionOperationContext,
    ActionServiceError, ActionServiceIdSource, InMemoryActionService,
};
use crate::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
    AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
    AuditPolicyOutcome, AuditTarget,
};
use crate::classification::DataClassification;
use crate::error::{DomainError, ErrorCode, MessageKey};
use crate::identity::{
    ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, DecisionId,
    DecisionRequestId, EvidenceReferenceId, IdempotencyId, PreparedIntentId, StakeholderId,
};
use crate::time::{Clock, UtcTimestamp};
use crate::value::BoundedText;
use crate::work_management::{
    validate_and_mint_work_management_h2a_receipt, ApprovalAuthorizationPort, DecisionRequestState,
    DecisionResultingActionRequest, DecisionState, EvidenceOrJudgment, EvidenceReferenceMetadata,
    HumanJudgment, IncompleteDownstreamWork, PreparedIntentError, RejectedPreparedIntentOutcome,
    SupportWitness, WorkManagementApproval, WorkManagementApprovalValidationError,
    WorkManagementAuthoritativeSnapshot, WorkManagementCurrentPolicy, WorkManagementOperation,
    WorkManagementPreparedIntent, DECISION_PREPARED_REJECTED_AUDIT_CODE,
};

pub type DecisionSubject = BoundedText<240>;
pub type DecisionText = BoundedText<4_000>;
pub type DecisionWithdrawalRationale = BoundedText<2_000>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionOperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}
/// H2a rejection (v45): the Head of Products refuses a pending Resolve
/// preview. See [`InMemoryDecisionService::reject_decision_prepared_intent`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectDecisionPreparedIntent {
    pub prepared_id: PreparedIntentId,
    pub actor: AuditActor,
    pub context: DecisionOperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRequestRecord {
    id: DecisionRequestId,
    subject: DecisionSubject,
    details: DecisionText,
    intended_owner: Option<StakeholderId>,
    classification: DataClassification,
    state: DecisionRequestState,
    withdrawal_rationale: Option<DecisionWithdrawalRationale>,
    linked_decision_id: Option<DecisionId>,
    version: AggregateVersion,
}
impl DecisionRequestRecord {
    /// Reconstruct the canonical record emitted by `CreateDecisionRequestDraft`.
    ///
    /// This is intentionally narrower than a general persistence setter: a
    /// decoder cannot provide lifecycle state, terminal data, relationships,
    /// or an arbitrary aggregate version at this seam.
    #[doc(hidden)]
    pub fn from_persisted_created_draft(
        id: DecisionRequestId,
        subject: DecisionSubject,
        details: DecisionText,
        intended_owner: Option<StakeholderId>,
        classification: DataClassification,
    ) -> Self {
        Self {
            id,
            subject,
            details,
            intended_owner,
            classification,
            state: DecisionRequestState::Draft,
            withdrawal_rationale: None,
            linked_decision_id: None,
            version: AggregateVersion::initial(),
        }
    }

    /// Reconstruct the canonical record emitted after `SubmitDecisionRequest`.
    #[doc(hidden)]
    pub fn from_persisted_submitted_open(mut draft: Self) -> Option<Self> {
        if draft.state != DecisionRequestState::Draft
            || draft.version != AggregateVersion::initial()
            || draft.withdrawal_rationale.is_some()
            || draft.linked_decision_id.is_some()
        {
            return None;
        }
        draft.state = DecisionRequestState::Open;
        draft.version = AggregateVersion::initial()
            .next()
            .unwrap_or(AggregateVersion::initial());
        Some(draft)
    }

    /// Reconstruct the canonical record emitted after `WithdrawDecisionRequest`.
    #[doc(hidden)]
    pub fn from_persisted_open_to_withdrawn(
        mut open: Self,
        rationale: DecisionWithdrawalRationale,
    ) -> Option<Self> {
        if open.state != DecisionRequestState::Open
            || open.version
                != AggregateVersion::initial()
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || open.withdrawal_rationale.is_some()
            || open.linked_decision_id.is_some()
        {
            return None;
        }
        open.state = DecisionRequestState::Withdrawn;
        open.withdrawal_rationale = Some(rationale);
        open.version = AggregateVersion::initial()
            .next()
            .and_then(|version| version.next())
            .unwrap_or(AggregateVersion::initial());
        Some(open)
    }

    pub fn id(&self) -> &DecisionRequestId {
        &self.id
    }
    pub fn subject(&self) -> &DecisionSubject {
        &self.subject
    }
    pub fn details(&self) -> &DecisionText {
        &self.details
    }
    pub fn intended_owner(&self) -> Option<&StakeholderId> {
        self.intended_owner.as_ref()
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn state(&self) -> DecisionRequestState {
        self.state
    }
    pub fn withdrawal_rationale(&self) -> Option<&DecisionWithdrawalRationale> {
        self.withdrawal_rationale.as_ref()
    }
    pub fn linked_decision_id(&self) -> Option<&DecisionId> {
        self.linked_decision_id.as_ref()
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
}

// The following types are the typed domain boundary used by a persistence
// adapter. They deliberately model only accepted Decision lifecycle commands;
// raw rows, JSON, and generic CRUD are not accepted here.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionPersistenceCommand {
    CreateRequest {
        id: DecisionRequestId,
        subject: DecisionSubject,
        details: DecisionText,
        intended_owner: Option<StakeholderId>,
        classification: DataClassification,
    },
    SubmitRequest {
        request_id: DecisionRequestId,
        expected_version: AggregateVersion,
    },
    WithdrawRequest {
        request_id: DecisionRequestId,
        expected_version: AggregateVersion,
        rationale: DecisionWithdrawalRationale,
    },
    PrepareResolve {
        request_id: DecisionRequestId,
        expected_version: AggregateVersion,
        statement: DecisionText,
        rationale: DecisionText,
        impact: DecisionText,
        evidence_ids: Vec<EvidenceReferenceId>,
        judgments: Vec<HumanJudgment>,
        resulting_action_requests: Vec<DecisionResultingActionRequest>,
    },
    ExecuteResolve {
        approval: WorkManagementApproval,
    },
    /// H2a "Lower Data Classification" for Decision.
    PrepareLowerClassification {
        decision_id: DecisionId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: crate::work_management::WorkManagementRationale,
    },
    ExecuteLowerClassification {
        approval: WorkManagementApproval,
    },
    /// Decision Supersede PREPARE: mirrors `PrepareSupersedeDecision`'s own
    /// caller-controlled fields exactly (minus `context`), matching how
    /// `PrepareResolve` mirrors `PrepareResolveDecisionRequest`. The freshly
    /// minted replacement id/version/classification and the computed
    /// `incomplete_downstream` are not caller input -- they live in the
    /// `Prepared` result's `WorkManagementOperation::SupersedeDecision`
    /// instead.
    PrepareSupersede {
        decision_id: DecisionId,
        expected_version: AggregateVersion,
        replacement_statement: DecisionText,
        replacement_rationale: DecisionText,
        replacement_impact: DecisionText,
        replacement_owner: StakeholderId,
        evidence_ids: Vec<EvidenceReferenceId>,
        judgments: Vec<HumanJudgment>,
        resulting_action_requests: Vec<DecisionResultingActionRequest>,
    },
    ExecuteSupersede {
        approval: WorkManagementApproval,
    },
    /// v45: the Head of Products' explicit refusal of a pending Resolve
    /// preview. Consumes the intent; no effects.
    RejectPrepared {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
    },
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionPersistenceResult {
    Request(DecisionMutationOutcome<DecisionRequestRecord>),
    Prepared(WorkManagementPreparedIntent),
    Resolved(ResolvedDecisionOutcome),
    /// H2a "Lower Data Classification" execute outcome for Decision.
    Lowered(DecisionMutationOutcome<DecisionRecord>),
    /// Decision Supersede execute outcome.
    Superseded(SupersededDecisionOutcome),
    /// v45 rejection outcome.
    Rejected(RejectedPreparedIntentOutcome),
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionReplayCapsule {
    idempotency_id: IdempotencyId,
    original_correlation_id: CorrelationId,
    operation_ordinal: u64,
    command: DecisionPersistenceCommand,
    result: DecisionPersistenceResult,
    audit_event_ids: Vec<AuditEventId>,
}

impl DecisionReplayCapsule {
    /// Construct a capsule from already validated domain-typed values.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        idempotency_id: IdempotencyId,
        original_correlation_id: CorrelationId,
        operation_ordinal: u64,
        command: DecisionPersistenceCommand,
        result: DecisionPersistenceResult,
        audit_event_ids: Vec<AuditEventId>,
    ) -> Self {
        Self {
            idempotency_id,
            original_correlation_id,
            operation_ordinal,
            command,
            result,
            audit_event_ids,
        }
    }

    pub fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }

    pub fn original_correlation_id(&self) -> &CorrelationId {
        &self.original_correlation_id
    }

    pub const fn operation_ordinal(&self) -> u64 {
        self.operation_ordinal
    }

    pub const fn command(&self) -> &DecisionPersistenceCommand {
        &self.command
    }

    pub const fn result(&self) -> &DecisionPersistenceResult {
        &self.result
    }

    pub fn audit_event_ids(&self) -> &[AuditEventId] {
        &self.audit_event_ids
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionPersistenceDecodeInput {
    requests: Vec<DecisionRequestRecord>,
    decisions: Vec<DecisionRecord>,
    prepared: Vec<WorkManagementPreparedIntent>,
    replay: Vec<DecisionReplayCapsule>,
    audits: Vec<AuditEvent>,
}

impl DecisionPersistenceDecodeInput {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        requests: Vec<DecisionRequestRecord>,
        decisions: Vec<DecisionRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
        replay: Vec<DecisionReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Self {
        Self {
            requests,
            decisions,
            prepared,
            replay,
            audits,
        }
    }

    pub fn decode(self) -> Result<DecisionPersistenceSnapshot, DecisionRehydrationError> {
        DecisionPersistenceSnapshot::try_new(
            self.requests,
            self.decisions,
            self.prepared,
            self.replay,
            self.audits,
        )
    }

    pub fn try_into_snapshot(
        self,
    ) -> Result<DecisionPersistenceSnapshot, DecisionRehydrationError> {
        self.decode()
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionRehydrationError {
    UnsupportedState,
    DuplicateRecord,
    DuplicateIdempotency,
    DuplicateAuditEvent,
    OperationOrderMismatch,
    CommandResultMismatch,
    AuditMismatch,
    OrphanRecord,
    OrphanAuditEvent,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionPersistenceSnapshot {
    requests: Vec<DecisionRequestRecord>,
    decisions: Vec<DecisionRecord>,
    prepared: Vec<WorkManagementPreparedIntent>,
    replay: Vec<DecisionReplayCapsule>,
    audits: Vec<AuditEvent>,
}

impl DecisionPersistenceSnapshot {
    pub fn try_new(
        requests: Vec<DecisionRequestRecord>,
        decisions: Vec<DecisionRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
        replay: Vec<DecisionReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, DecisionRehydrationError> {
        let mut request_map = HashMap::new();
        for request in &requests {
            let legal_shape = match request.state {
                DecisionRequestState::Draft => {
                    request.version == AggregateVersion::initial()
                        && request.withdrawal_rationale.is_none()
                        && request.linked_decision_id.is_none()
                }
                DecisionRequestState::Open => {
                    request.version
                        == AggregateVersion::initial()
                            .next()
                            .unwrap_or(AggregateVersion::initial())
                        && request.withdrawal_rationale.is_none()
                        && request.linked_decision_id.is_none()
                }
                DecisionRequestState::Withdrawn => {
                    request.version
                        == AggregateVersion::initial()
                            .next()
                            .and_then(|version| version.next())
                            .unwrap_or(AggregateVersion::initial())
                        && request.withdrawal_rationale.is_some()
                        && request.linked_decision_id.is_none()
                }
                DecisionRequestState::Resolved => {
                    request.version
                        == AggregateVersion::initial()
                            .next()
                            .and_then(|version| version.next())
                            .unwrap_or(AggregateVersion::initial())
                        && request.withdrawal_rationale.is_none()
                        && request.linked_decision_id.is_some()
                }
            };
            if !legal_shape || request_map.insert(request.id(), request).is_some() {
                return Err(DecisionRehydrationError::DuplicateRecord);
            }
        }

        let mut decision_map = HashMap::new();
        for decision in &decisions {
            // `version` is not pinned to `initial()` here: H2a
            // `LowerDecisionClassification` is the first operation that
            // advances an Effective Decision's version without changing any
            // other field this shape checks. The real guarantee against a
            // fabricated version is the capsule-driven `decision_timeline`
            // comparison below, not this per-record prefilter.
            //
            // Decision Supersede widens this shape to a two-generation
            // chain: a root Decision still traces to a source request and
            // never `supersedes` anything; a replacement Decision traces to
            // no request and always `supersedes` exactly one prior Decision.
            // `superseded_by_decision_id` must agree with `state` in both
            // directions -- an Effective Decision has none, a Superseded one
            // always has one (an intermediate link in a longer chain can
            // carry both `supersedes_decision_id` and
            // `superseded_by_decision_id` at once). As with version above,
            // this is only a per-record shape prefilter; the real guarantee
            // that the two directions actually agree with each other is the
            // capsule-driven `decision_timeline` comparison below.
            let legal_shape = (decision.source_request_id.is_some()
                != decision.supersedes_decision_id.is_some())
                && match decision.state {
                    DecisionState::Effective => decision.superseded_by_decision_id.is_none(),
                    DecisionState::Superseded => decision.superseded_by_decision_id.is_some(),
                }
                && decision
                    .resulting_action_request_ids
                    .windows(2)
                    .all(|pair| pair[0] < pair[1]);
            if !legal_shape || decision_map.insert(decision.id(), decision).is_some() {
                return Err(DecisionRehydrationError::DuplicateRecord);
            }
        }

        let mut audit_map = HashMap::new();
        for audit in &audits {
            if audit_map.insert(audit.id(), audit).is_some() {
                return Err(DecisionRehydrationError::DuplicateAuditEvent);
            }
        }

        let mut idempotency_ids = HashSet::new();
        let mut ordered = replay.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|capsule| capsule.operation_ordinal);
        let mut timeline = HashMap::new();
        let mut decision_timeline = HashMap::new();
        let mut prepared_history = HashMap::new();
        let mut consumed_prepared = HashSet::new();
        let mut claimed_audits = Vec::new();
        for (index, capsule) in ordered.iter().enumerate() {
            if capsule.operation_ordinal != u64::try_from(index).unwrap_or(u64::MAX)
                || !idempotency_ids.insert(capsule.idempotency_id.clone())
            {
                return Err(
                    if capsule.operation_ordinal != u64::try_from(index).unwrap_or(u64::MAX) {
                        DecisionRehydrationError::OperationOrderMismatch
                    } else {
                        DecisionRehydrationError::DuplicateIdempotency
                    },
                );
            }
            validate_decision_capsule(
                capsule,
                &mut timeline,
                &mut decision_timeline,
                &mut prepared_history,
                &mut consumed_prepared,
                &audit_map,
                &mut claimed_audits,
            )?;
        }

        if request_map.len() != timeline.len() || decision_map.len() != decision_timeline.len() {
            return Err(DecisionRehydrationError::OrphanRecord);
        }
        if request_map
            .iter()
            .any(|(id, request)| timeline.get(id) != Some(request))
        {
            return Err(DecisionRehydrationError::CommandResultMismatch);
        }
        if decision_map
            .iter()
            .any(|(id, decision)| decision_timeline.get(id) != Some(decision))
        {
            return Err(DecisionRehydrationError::CommandResultMismatch);
        }
        let pending_ids = prepared
            .iter()
            .map(WorkManagementPreparedIntent::id)
            .collect::<HashSet<_>>();
        if pending_ids.len() != prepared.len()
            || pending_ids
                .iter()
                .any(|id| !prepared_history.contains_key(*id))
            || pending_ids.iter().any(|id| consumed_prepared.contains(*id))
            || prepared_history
                .keys()
                .any(|id| !consumed_prepared.contains(id) && !pending_ids.contains(id))
        {
            return Err(DecisionRehydrationError::OrphanRecord);
        }
        if claimed_audits.len() != audits.len()
            || claimed_audits.iter().collect::<HashSet<_>>().len() != claimed_audits.len()
            || claimed_audits.iter().any(|id| !audit_map.contains_key(id))
        {
            return Err(DecisionRehydrationError::AuditMismatch);
        }

        Ok(Self {
            requests,
            decisions,
            prepared,
            replay: ordered.into_iter().cloned().collect(),
            audits,
        })
    }

    pub fn requests(&self) -> &[DecisionRequestRecord] {
        &self.requests
    }

    pub fn decisions(&self) -> &[DecisionRecord] {
        &self.decisions
    }

    pub fn prepared(&self) -> &[WorkManagementPreparedIntent] {
        &self.prepared
    }

    pub fn replay(&self) -> &[DecisionReplayCapsule] {
        &self.replay
    }

    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }
}

fn validate_decision_capsule(
    capsule: &DecisionReplayCapsule,
    timeline: &mut HashMap<DecisionRequestId, DecisionRequestRecord>,
    decision_timeline: &mut HashMap<DecisionId, DecisionRecord>,
    prepared_history: &mut HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    consumed_prepared: &mut HashSet<PreparedIntentId>,
    audits: &HashMap<&AuditEventId, &AuditEvent>,
    claimed_audits: &mut Vec<AuditEventId>,
) -> Result<(), DecisionRehydrationError> {
    let request_outcome = match &capsule.result {
        DecisionPersistenceResult::Request(outcome) => Some(outcome),
        _ => None,
    };
    let (request_id, expected_audit_code, expected_record) = match &capsule.command {
        DecisionPersistenceCommand::CreateRequest {
            id,
            subject,
            details,
            intended_owner,
            classification,
        } => {
            let outcome = request_outcome.ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            let request = DecisionRequestRecord::from_persisted_created_draft(
                id.clone(),
                subject.clone(),
                details.clone(),
                intended_owner.clone(),
                *classification,
            );
            if timeline.contains_key(id) || outcome.record != request {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            timeline.insert(id.clone(), request.clone());
            (id, "decision_request.created", request)
        }
        DecisionPersistenceCommand::SubmitRequest {
            request_id,
            expected_version,
        } => {
            let outcome = request_outcome.ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            let prior = timeline
                .get(request_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            if prior.version() != *expected_version {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let request = DecisionRequestRecord::from_persisted_submitted_open(prior.clone())
                .ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            if outcome.record != request {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            timeline.insert(request_id.clone(), request.clone());
            (request_id, "decision_request.submitted", request)
        }
        DecisionPersistenceCommand::WithdrawRequest {
            request_id,
            expected_version,
            rationale,
        } => {
            let outcome = request_outcome.ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            let prior = timeline
                .get(request_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            if prior.version() != *expected_version {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let request = DecisionRequestRecord::from_persisted_open_to_withdrawn(
                prior.clone(),
                rationale.clone(),
            )
            .ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            if outcome.record != request {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            timeline.insert(request_id.clone(), request.clone());
            (request_id, "decision_request.withdrawn", request)
        }
        DecisionPersistenceCommand::PrepareResolve {
            request_id,
            expected_version,
            statement,
            rationale,
            impact,
            evidence_ids,
            judgments,
            resulting_action_requests: command_action_requests,
        } => {
            let DecisionPersistenceResult::Prepared(intent) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let request = timeline
                .get(request_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::ResolveDecisionRequest {
                request_id: prepared_request_id,
                request_version,
                decision_id,
                decision_classification,
                statement: prepared_statement,
                rationale: prepared_rationale,
                impact: prepared_impact,
                decision_owner,
                resulting_action_requests,
                ..
            } = intent.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let Some(support) = intent.preview().support() else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let canonical_actions = resulting_action_requests
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id);
            let support_evidence_ids = support
                .evidence()
                .iter()
                .map(|item| item.id().clone())
                .collect::<Vec<_>>();
            if request.version != *expected_version
                || request.state != DecisionRequestState::Open
                || request.intended_owner.as_ref() != Some(decision_owner)
                || prepared_request_id != request_id
                || request_version != expected_version
                || decision_classification != &request.classification
                || prepared_statement != statement
                || prepared_rationale != rationale
                || prepared_impact != impact
                || decision_timeline.contains_key(decision_id)
                || intent.classification()
                    != request.classification.combine(support.classification())
                || *intent.payload_digest() != intent.preview().payload_digest()
                || !canonical_actions
                || resulting_action_requests != command_action_requests
                || support_evidence_ids != *evidence_ids
                || support.judgments() != judgments
                || !capsule.audit_event_ids.is_empty()
                || prepared_history
                    .insert(intent.id().clone(), intent.clone())
                    .is_some()
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            return Ok(());
        }
        DecisionPersistenceCommand::RejectPrepared { prepared_id, actor } => {
            let DecisionPersistenceResult::Rejected(outcome) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let prepared = prepared_history
                .get(prepared_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::ResolveDecisionRequest { request_id, .. } =
                prepared.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let audit = outcome.audit_event();
            if *actor != AuditActor::HeadOfProducts
                || outcome.prepared_intent_id() != prepared_id
                || outcome.rejected_at() != audit.occurred_at()
                || outcome.expired_at_rejection()
                    != (outcome.rejected_at() >= prepared.preview().expires_at())
                || capsule.audit_event_ids.len() != 1
                || capsule.audit_event_ids[0] != *audit.id()
                || !consumed_prepared.insert(prepared_id.clone())
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let expected = crate::work_management::prepared_intent_rejection_audit(
                audit.id().clone(),
                audit.occurred_at(),
                DECISION_PREPARED_REJECTED_AUDIT_CODE,
                AuditTarget::DecisionRequest(request_id.clone()),
                capsule.original_correlation_id.clone(),
            );
            if audits.get(audit.id()).copied() != Some(audit) || expected.as_ref() != Some(audit) {
                return Err(DecisionRehydrationError::AuditMismatch);
            }
            claimed_audits.push(audit.id().clone());
            return Ok(());
        }
        DecisionPersistenceCommand::ExecuteResolve { approval } => {
            let DecisionPersistenceResult::Resolved(outcome) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let prepared = prepared_history
                .get(approval.prepared_id())
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::ResolveDecisionRequest {
                request_id,
                request_version,
                decision_id,
                statement,
                rationale,
                impact,
                decision_owner,
                decided_at,
                resulting_action_requests,
                ..
            } = prepared.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let previous = timeline
                .get(request_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let Some(support) = prepared.preview().support() else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let mut request = previous.clone();
            request.state = DecisionRequestState::Resolved;
            request.linked_decision_id = Some(decision_id.clone());
            request.version = request
                .version
                .next()
                .ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            let decision = DecisionRecord {
                id: decision_id.clone(),
                source_request_id: Some(request_id.clone()),
                statement: statement.clone(),
                rationale: rationale.clone(),
                impact: impact.clone(),
                owner: decision_owner.clone(),
                decided_at: *decided_at,
                classification: prepared.classification(),
                state: DecisionState::Effective,
                support: support.clone(),
                resulting_action_request_ids: resulting_action_requests
                    .iter()
                    .map(|item| item.id.clone())
                    .collect(),
                supersedes_decision_id: None,
                superseded_by_decision_id: None,
                version: AggregateVersion::initial(),
            };
            if approval.idempotency_id() != capsule.idempotency_id()
                || approval.actor() != AuditActor::HeadOfProducts
                || approval.acknowledged_payload_digest() != prepared.payload_digest()
                || previous.version != *request_version
                || previous.state != DecisionRequestState::Open
                || previous.intended_owner.as_ref() != Some(decision_owner)
                || decision_timeline.contains_key(decision_id)
                || outcome.request != request
                || outcome.decision != decision
                || outcome.resulting_action_request_ids != decision.resulting_action_request_ids
                || outcome.audit_events.len() != 3
                || capsule.audit_event_ids.len() != 3
                || outcome.approval_receipt_id.as_str().is_empty()
                || !consumed_prepared.insert(approval.prepared_id().clone())
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let expected = [
                (
                    "decision_request.resolved",
                    AuditTarget::DecisionRequest(request_id.clone()),
                ),
                (
                    "decision.created",
                    AuditTarget::Decision(decision_id.clone()),
                ),
                (
                    "decision_request.decision_linked",
                    AuditTarget::DecisionRequest(request_id.clone()),
                ),
            ];
            for ((audit, audit_id), (code, target)) in outcome
                .audit_events
                .iter()
                .zip(capsule.audit_event_ids.iter())
                .zip(expected)
            {
                if audit.id() != audit_id
                    || audits.get(audit_id).copied() != Some(audit)
                    || audit.correlation_id() != capsule.original_correlation_id()
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != code
                    || audit.target() != &target
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::Approved
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != code
                {
                    return Err(DecisionRehydrationError::AuditMismatch);
                }
                claimed_audits.push(audit_id.clone());
            }
            timeline.insert(request_id.clone(), request);
            decision_timeline.insert(decision_id.clone(), decision);
            return Ok(());
        }
        DecisionPersistenceCommand::PrepareSupersede {
            decision_id,
            expected_version,
            replacement_statement,
            replacement_rationale,
            replacement_impact,
            replacement_owner,
            evidence_ids,
            judgments,
            resulting_action_requests: command_action_requests,
        } => {
            let DecisionPersistenceResult::Prepared(intent) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let decision = decision_timeline
                .get(decision_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::SupersedeDecision {
                decision_id: prepared_decision_id,
                decision_version,
                replacement_decision_id,
                replacement_decision_version,
                replacement_decision_classification,
                replacement_statement: prepared_statement,
                replacement_rationale: prepared_rationale,
                replacement_impact: prepared_impact,
                replacement_owner: prepared_owner,
                resulting_action_requests,
                ..
            } = intent.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let Some(support) = intent.preview().support() else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let canonical_actions = resulting_action_requests
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id);
            let support_evidence_ids = support
                .evidence()
                .iter()
                .map(|item| item.id().clone())
                .collect::<Vec<_>>();
            if decision.version != *expected_version
                || decision.state != DecisionState::Effective
                || prepared_decision_id != decision_id
                || decision_version != expected_version
                || prepared_statement != replacement_statement
                || prepared_rationale != replacement_rationale
                || prepared_impact != replacement_impact
                || prepared_owner != replacement_owner
                || *replacement_decision_version != AggregateVersion::initial()
                || replacement_decision_id == decision_id
                || decision_timeline.contains_key(replacement_decision_id)
                || *replacement_decision_classification
                    != decision.classification.combine(support.classification())
                || intent.classification() != decision.classification
                || *intent.payload_digest() != intent.preview().payload_digest()
                || !canonical_actions
                || resulting_action_requests != command_action_requests
                || support_evidence_ids != *evidence_ids
                || support.judgments() != judgments
                || !capsule.audit_event_ids.is_empty()
                || prepared_history
                    .insert(intent.id().clone(), intent.clone())
                    .is_some()
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            return Ok(());
        }
        DecisionPersistenceCommand::ExecuteSupersede { approval } => {
            let DecisionPersistenceResult::Superseded(outcome) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let prepared = prepared_history
                .get(approval.prepared_id())
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::SupersedeDecision {
                decision_id,
                decision_version,
                replacement_decision_id,
                replacement_decision_version,
                replacement_statement,
                replacement_rationale,
                replacement_impact,
                replacement_owner,
                replacement_decided_at,
                resulting_action_requests,
                incomplete_downstream,
                ..
            } = prepared.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let previous = decision_timeline
                .get(decision_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            if previous.version != *decision_version || previous.state != DecisionState::Effective {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let mut superseded = previous.clone();
            superseded.state = DecisionState::Superseded;
            superseded.superseded_by_decision_id = Some(replacement_decision_id.clone());
            superseded.version = superseded
                .version
                .next()
                .ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            let Some(replacement_support) = prepared.preview().support() else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let replacement = DecisionRecord {
                id: replacement_decision_id.clone(),
                source_request_id: None,
                statement: replacement_statement.clone(),
                rationale: replacement_rationale.clone(),
                impact: replacement_impact.clone(),
                owner: replacement_owner.clone(),
                decided_at: *replacement_decided_at,
                classification: prepared.classification(),
                state: DecisionState::Effective,
                support: replacement_support.clone(),
                resulting_action_request_ids: resulting_action_requests
                    .iter()
                    .map(|item| item.id.clone())
                    .collect(),
                supersedes_decision_id: Some(decision_id.clone()),
                superseded_by_decision_id: None,
                version: *replacement_decision_version,
            };
            if approval.idempotency_id() != capsule.idempotency_id()
                || approval.actor() != AuditActor::HeadOfProducts
                || approval.acknowledged_payload_digest() != prepared.payload_digest()
                || decision_timeline.contains_key(replacement_decision_id)
                || outcome.superseded != superseded
                || outcome.replacement != replacement
                || outcome.resulting_action_request_ids != replacement.resulting_action_request_ids
                || outcome.flagged_downstream != *incomplete_downstream
                || outcome.audit_events.len() != 3
                || capsule.audit_event_ids.len() != 3
                || outcome.approval_receipt_id.as_str().is_empty()
                || !consumed_prepared.insert(approval.prepared_id().clone())
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let expected = [
                (
                    "decision.superseded",
                    AuditTarget::Decision(decision_id.clone()),
                ),
                (
                    "decision.replacement_created",
                    AuditTarget::Decision(replacement_decision_id.clone()),
                ),
                (
                    "decision.replacement_linked",
                    AuditTarget::Decision(decision_id.clone()),
                ),
            ];
            for ((audit, audit_id), (code, target)) in outcome
                .audit_events
                .iter()
                .zip(capsule.audit_event_ids.iter())
                .zip(expected)
            {
                if audit.id() != audit_id
                    || audits.get(audit_id).copied() != Some(audit)
                    || audit.correlation_id() != capsule.original_correlation_id()
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != code
                    || audit.target() != &target
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::Approved
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != code
                {
                    return Err(DecisionRehydrationError::AuditMismatch);
                }
                claimed_audits.push(audit_id.clone());
            }
            decision_timeline.insert(decision_id.clone(), superseded);
            decision_timeline.insert(replacement_decision_id.clone(), replacement);
            return Ok(());
        }
        DecisionPersistenceCommand::PrepareLowerClassification {
            decision_id,
            expected_version,
            proposed_classification,
            rationale,
        } => {
            let DecisionPersistenceResult::Prepared(intent) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let decision = decision_timeline
                .get(decision_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::LowerDecisionClassification {
                decision_id: prepared_decision_id,
                decision_version,
                current_classification,
                proposed_classification: prepared_proposed,
                rationale: prepared_rationale,
            } = intent.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            if decision.version != *expected_version
                || prepared_decision_id != decision_id
                || decision_version != expected_version
                || current_classification != &decision.classification
                || prepared_proposed != proposed_classification
                || prepared_rationale != rationale
                || intent.classification() != decision.classification
                || *intent.payload_digest() != intent.preview().payload_digest()
                || !capsule.audit_event_ids.is_empty()
                || prepared_history
                    .insert(intent.id().clone(), intent.clone())
                    .is_some()
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            return Ok(());
        }
        DecisionPersistenceCommand::ExecuteLowerClassification { approval } => {
            let DecisionPersistenceResult::Lowered(outcome) = &capsule.result else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let prepared = prepared_history
                .get(approval.prepared_id())
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            let WorkManagementOperation::LowerDecisionClassification {
                decision_id,
                decision_version,
                proposed_classification,
                ..
            } = prepared.operation()
            else {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            };
            let previous = decision_timeline
                .get(decision_id)
                .ok_or(DecisionRehydrationError::OrphanRecord)?;
            if previous.version != *decision_version {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let mut decision = previous.clone();
            decision.version = decision
                .version
                .next()
                .ok_or(DecisionRehydrationError::CommandResultMismatch)?;
            decision.classification = *proposed_classification;
            if approval.idempotency_id() != capsule.idempotency_id()
                || approval.actor() != AuditActor::HeadOfProducts
                || approval.acknowledged_payload_digest() != prepared.payload_digest()
                || outcome.record != decision
                || outcome.audit_events.len() != 1
                || capsule.audit_event_ids.len() != 1
                || outcome.approval_receipt_id.is_none()
                || !consumed_prepared.insert(approval.prepared_id().clone())
            {
                return Err(DecisionRehydrationError::CommandResultMismatch);
            }
            let audit = &outcome.audit_events[0];
            let audit_id = &capsule.audit_event_ids[0];
            if audit.id() != audit_id
                || audits.get(audit_id).copied() != Some(audit)
                || audit.correlation_id() != capsule.original_correlation_id()
                || audit.actor() != AuditActor::HeadOfProducts
                || audit.module() != AuditModule::WorkManagement
                || audit.code().as_str() != "decision.classification_lowered"
                || audit.target() != &AuditTarget::Decision(decision_id.clone())
                || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                || audit.approval_outcome() != AuditApprovalOutcome::Approved
                || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                || audit.effect_scope() != AuditEffectScope::Complete
                || audit.actual_effects().len() != 1
                || audit.actual_effects()[0].as_str() != "decision.classification_lowered"
            {
                return Err(DecisionRehydrationError::AuditMismatch);
            }
            claimed_audits.push(audit_id.clone());
            decision_timeline.insert(decision_id.clone(), decision);
            return Ok(());
        }
    };
    let DecisionPersistenceResult::Request(outcome) = &capsule.result else {
        return Err(DecisionRehydrationError::CommandResultMismatch);
    };
    if outcome.approval_receipt_id.is_some()
        || outcome.audit_events.len() != 1
        || capsule.audit_event_ids.len() != 1
    {
        return Err(DecisionRehydrationError::CommandResultMismatch);
    }
    let audit = &outcome.audit_events[0];
    let audit_id = &capsule.audit_event_ids[0];
    if audit.id() != audit_id
        || audit.correlation_id() != &capsule.original_correlation_id
        || audits.get(audit_id).copied() != Some(audit)
        || audit.actor() != AuditActor::HeadOfProducts
        || audit.module() != AuditModule::WorkManagement
        || audit.code().as_str() != expected_audit_code
        || audit.target() != &AuditTarget::DecisionRequest(request_id.clone())
        || audit.policy_outcome() != AuditPolicyOutcome::Allowed
        || audit.approval_outcome() != AuditApprovalOutcome::NotRequired
        || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
        || audit.effect_scope() != AuditEffectScope::Complete
        || audit.actual_effects().len() != 1
        || audit.actual_effects()[0].as_str() != expected_audit_code
        || expected_record.id() != request_id
    {
        return Err(DecisionRehydrationError::AuditMismatch);
    }
    claimed_audits.push(audit_id.clone());
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRecord {
    id: DecisionId,
    source_request_id: Option<DecisionRequestId>,
    statement: DecisionText,
    rationale: DecisionText,
    impact: DecisionText,
    owner: StakeholderId,
    decided_at: UtcTimestamp,
    classification: DataClassification,
    state: DecisionState,
    support: SupportWitness,
    resulting_action_request_ids: Vec<ActionRequestId>,
    supersedes_decision_id: Option<DecisionId>,
    superseded_by_decision_id: Option<DecisionId>,
    version: AggregateVersion,
}
impl DecisionRecord {
    pub fn id(&self) -> &DecisionId {
        &self.id
    }
    pub fn source_request_id(&self) -> Option<&DecisionRequestId> {
        self.source_request_id.as_ref()
    }
    pub fn statement(&self) -> &DecisionText {
        &self.statement
    }
    pub fn rationale(&self) -> &DecisionText {
        &self.rationale
    }
    pub fn impact(&self) -> &DecisionText {
        &self.impact
    }
    pub fn owner(&self) -> &StakeholderId {
        &self.owner
    }
    pub const fn decided_at(&self) -> UtcTimestamp {
        self.decided_at
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn state(&self) -> DecisionState {
        self.state
    }
    pub const fn support(&self) -> &SupportWitness {
        &self.support
    }
    pub fn resulting_action_request_ids(&self) -> &[ActionRequestId] {
        &self.resulting_action_request_ids
    }
    pub fn supersedes_decision_id(&self) -> Option<&DecisionId> {
        self.supersedes_decision_id.as_ref()
    }
    pub fn superseded_by_decision_id(&self) -> Option<&DecisionId> {
        self.superseded_by_decision_id.as_ref()
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }

    /// Reconstruct the record after Decision Supersede's EXECUTE marks this
    /// Effective decision Superseded. Mirrors `execute_supersede`'s in-
    /// memory mutation of `old` exactly (state, `superseded_by_decision_id`,
    /// version advance by one) -- used only by the ledger's own read-time
    /// decode, which reconstructs `SupersededDecisionOutcome` from persisted
    /// rows without re-running domain `execute_supersede` (that call also
    /// needs live Action state, which read-time decode does not have a safe
    /// way to reproduce as-of an arbitrary historical ordinal).
    #[doc(hidden)]
    pub fn from_persisted_superseded(
        mut previous: Self,
        replacement_decision_id: DecisionId,
    ) -> Option<Self> {
        if previous.state != DecisionState::Effective {
            return None;
        }
        previous.state = DecisionState::Superseded;
        previous.superseded_by_decision_id = Some(replacement_decision_id);
        previous.version = previous.version.next()?;
        Some(previous)
    }

    /// Reconstruct the freshly created replacement decision record after
    /// Decision Supersede's EXECUTE. Mirrors `execute_supersede`'s in-memory
    /// construction of `replacement` exactly. See
    /// `from_persisted_superseded`'s doc comment for why the ledger
    /// reconstructs this instead of re-running domain `execute_supersede`.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted_replacement(
        id: DecisionId,
        statement: DecisionText,
        rationale: DecisionText,
        impact: DecisionText,
        owner: StakeholderId,
        decided_at: UtcTimestamp,
        classification: DataClassification,
        support: SupportWitness,
        resulting_action_request_ids: Vec<ActionRequestId>,
        supersedes_decision_id: DecisionId,
    ) -> Self {
        Self {
            id,
            source_request_id: None,
            statement,
            rationale,
            impact,
            owner,
            decided_at,
            classification,
            state: DecisionState::Effective,
            support,
            resulting_action_request_ids,
            supersedes_decision_id: Some(supersedes_decision_id),
            superseded_by_decision_id: None,
            version: AggregateVersion::initial(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionMutationOutcome<T> {
    pub record: T,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: Option<ApprovalReceiptId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedDecisionOutcome {
    pub request: DecisionRequestRecord,
    pub decision: DecisionRecord,
    pub resulting_action_request_ids: Vec<ActionRequestId>,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: ApprovalReceiptId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupersededDecisionOutcome {
    pub superseded: DecisionRecord,
    pub replacement: DecisionRecord,
    pub resulting_action_request_ids: Vec<ActionRequestId>,
    pub flagged_downstream: Vec<IncompleteDownstreamWork>,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: ApprovalReceiptId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDecisionRequestDraft {
    pub id: DecisionRequestId,
    pub subject: DecisionSubject,
    pub details: DecisionText,
    pub intended_owner: Option<StakeholderId>,
    pub classification: DataClassification,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitDecisionRequest {
    pub request_id: DecisionRequestId,
    pub expected_version: AggregateVersion,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawDecisionRequest {
    pub request_id: DecisionRequestId,
    pub expected_version: AggregateVersion,
    pub rationale: DecisionWithdrawalRationale,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareResolveDecisionRequest {
    pub request_id: DecisionRequestId,
    pub expected_version: AggregateVersion,
    pub statement: DecisionText,
    pub rationale: DecisionText,
    pub impact: DecisionText,
    pub evidence_ids: Vec<EvidenceReferenceId>,
    pub judgments: Vec<HumanJudgment>,
    pub resulting_action_requests: Vec<DecisionResultingActionRequest>,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareSupersedeDecision {
    pub decision_id: DecisionId,
    pub expected_version: AggregateVersion,
    pub replacement_statement: DecisionText,
    pub replacement_rationale: DecisionText,
    pub replacement_impact: DecisionText,
    pub replacement_owner: StakeholderId,
    pub evidence_ids: Vec<EvidenceReferenceId>,
    pub judgments: Vec<HumanJudgment>,
    pub resulting_action_requests: Vec<DecisionResultingActionRequest>,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteResolveDecisionRequest {
    pub approval: WorkManagementApproval,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteSupersedeDecision {
    pub approval: WorkManagementApproval,
    pub context: DecisionOperationContext,
}
/// H2a "Lower Data Classification" for Decision. Unlike
/// ResolveDecisionRequest/SupersedeDecision, this needs no Evidence-or-
/// Judgment support -- built independently of `self.prepare`/`self.validate`
/// (which hardcode a required `SupportWitness`) rather than fabricating one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerDecisionClassification {
    pub decision_id: DecisionId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: crate::work_management::WorkManagementRationale,
    pub context: DecisionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerDecisionClassification {
    pub approval: WorkManagementApproval,
    pub context: DecisionOperationContext,
}

pub trait DecisionServiceIdSource {
    fn next_decision_id(&mut self) -> Result<DecisionId, crate::DomainValueError>;
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, crate::DomainValueError>;
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, crate::DomainValueError>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionEvidenceAuthorityError {
    Unavailable,
    NotFound,
}
pub trait DecisionEvidenceAuthorityPort {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyDecisionEvidenceAuthority;
impl DecisionEvidenceAuthorityPort for DenyDecisionEvidenceAuthority {
    fn resolve(
        &self,
        _: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        Err(DecisionEvidenceAuthorityError::Unavailable)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionExecutionPolicy {
    Allowed,
    Denied,
}
pub trait DecisionExecutionPolicyPort {
    fn current_policy(&self, operation: &WorkManagementOperation) -> DecisionExecutionPolicy;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecisionError {
    NotFound,
    AlreadyExists,
    Stale,
    Illegal,
    MissingOwner,
    SupportDenied,
    EvidenceUnavailable,
    IdempotencyConflict,
    PreparedChanged,
    Unauthorized,
    PolicyDenied,
    Infrastructure,
    NotALowering,
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum Signature {
    Create(
        DecisionRequestId,
        DecisionSubject,
        DecisionText,
        Option<StakeholderId>,
        DataClassification,
    ),
    Submit(DecisionRequestId, AggregateVersion),
    Withdraw(
        DecisionRequestId,
        AggregateVersion,
        DecisionWithdrawalRationale,
    ),
    PrepareResolve(
        DecisionRequestId,
        AggregateVersion,
        DecisionText,
        DecisionText,
        DecisionText,
        Vec<EvidenceReferenceId>,
        Vec<HumanJudgment>,
        Vec<DecisionResultingActionRequest>,
    ),
    PrepareSupersede(
        DecisionId,
        AggregateVersion,
        DecisionText,
        DecisionText,
        DecisionText,
        StakeholderId,
        Vec<EvidenceReferenceId>,
        Vec<HumanJudgment>,
        Vec<DecisionResultingActionRequest>,
    ),
    Execute(
        PreparedIntentId,
        AuditActor,
        crate::work_management::WorkManagementPayloadDigest,
    ),
    RejectPrepared(PreparedIntentId, AuditActor),
    PrepareLowerClassification(
        DecisionId,
        AggregateVersion,
        DataClassification,
        crate::work_management::WorkManagementRationale,
    ),
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum ResultValue {
    Request(DecisionMutationOutcome<DecisionRequestRecord>),
    Prepared(WorkManagementPreparedIntent),
    Resolved(ResolvedDecisionOutcome),
    Superseded(SupersededDecisionOutcome),
    ClassificationLowered(DecisionMutationOutcome<DecisionRecord>),
    Rejected(RejectedPreparedIntentOutcome),
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Stored {
    signature: Signature,
    result: ResultValue,
}
#[derive(Clone, Default)]
struct Store {
    requests: HashMap<DecisionRequestId, DecisionRequestRecord>,
    decisions: HashMap<DecisionId, DecisionRecord>,
    prepared: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    idem: HashMap<IdempotencyId, Stored>,
    audits: Vec<AuditEvent>,
}

pub struct InMemoryDecisionService<C, I, Z, P, E> {
    clock: C,
    ids: I,
    authorization: Z,
    policy: P,
    evidence: E,
    state: Store,
    fail_next_commit: bool,
}

impl<
        C: Clock,
        I: DecisionServiceIdSource,
        Z: ApprovalAuthorizationPort,
        P: DecisionExecutionPolicyPort,
        E: DecisionEvidenceAuthorityPort,
    > InMemoryDecisionService<C, I, Z, P, E>
{
    pub fn new(clock: C, ids: I, authorization: Z, policy: P, evidence: E) -> Self {
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence,
            state: Store::default(),
            fail_next_commit: false,
        }
    }
    /// Rehydrate the validated typed Decision persistence boundary.
    ///
    /// This deliberately accepts only a `DecisionPersistenceSnapshot`, whose
    /// constructor has already replay-validated every stored command.  It is
    /// an adapter seam, not a general state import API.
    #[doc(hidden)]
    pub fn rehydrate(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence: E,
        snapshot: DecisionPersistenceSnapshot,
    ) -> Self {
        let mut state = Store::default();
        for request in snapshot.requests {
            state.requests.insert(request.id.clone(), request);
        }
        for decision in snapshot.decisions {
            state.decisions.insert(decision.id.clone(), decision);
        }
        for prepared in snapshot.prepared {
            state.prepared.insert(prepared.id().clone(), prepared);
        }
        state.audits = snapshot.audits;
        for capsule in snapshot.replay {
            let signature = match capsule.command {
                DecisionPersistenceCommand::CreateRequest {
                    id,
                    subject,
                    details,
                    intended_owner,
                    classification,
                } => Signature::Create(id, subject, details, intended_owner, classification),
                DecisionPersistenceCommand::SubmitRequest {
                    request_id,
                    expected_version,
                } => Signature::Submit(request_id, expected_version),
                DecisionPersistenceCommand::WithdrawRequest {
                    request_id,
                    expected_version,
                    rationale,
                } => Signature::Withdraw(request_id, expected_version, rationale),
                DecisionPersistenceCommand::PrepareResolve {
                    request_id,
                    expected_version,
                    statement,
                    rationale,
                    impact,
                    evidence_ids,
                    judgments,
                    resulting_action_requests,
                } => Signature::PrepareResolve(
                    request_id,
                    expected_version,
                    statement,
                    rationale,
                    impact,
                    evidence_ids,
                    judgments,
                    resulting_action_requests,
                ),
                DecisionPersistenceCommand::ExecuteResolve { approval } => Signature::Execute(
                    approval.prepared_id().clone(),
                    approval.actor(),
                    approval.acknowledged_payload_digest().clone(),
                ),
                DecisionPersistenceCommand::PrepareLowerClassification {
                    decision_id,
                    expected_version,
                    proposed_classification,
                    rationale,
                } => Signature::PrepareLowerClassification(
                    decision_id,
                    expected_version,
                    proposed_classification,
                    rationale,
                ),
                DecisionPersistenceCommand::ExecuteLowerClassification { approval } => {
                    Signature::Execute(
                        approval.prepared_id().clone(),
                        approval.actor(),
                        approval.acknowledged_payload_digest().clone(),
                    )
                }
                DecisionPersistenceCommand::PrepareSupersede {
                    decision_id,
                    expected_version,
                    replacement_statement,
                    replacement_rationale,
                    replacement_impact,
                    replacement_owner,
                    evidence_ids,
                    judgments,
                    resulting_action_requests,
                } => Signature::PrepareSupersede(
                    decision_id,
                    expected_version,
                    replacement_statement,
                    replacement_rationale,
                    replacement_impact,
                    replacement_owner,
                    evidence_ids,
                    judgments,
                    resulting_action_requests,
                ),
                DecisionPersistenceCommand::RejectPrepared { prepared_id, actor } => {
                    Signature::RejectPrepared(prepared_id, actor)
                }
                DecisionPersistenceCommand::ExecuteSupersede { approval } => Signature::Execute(
                    approval.prepared_id().clone(),
                    approval.actor(),
                    approval.acknowledged_payload_digest().clone(),
                ),
            };
            let result = match capsule.result {
                DecisionPersistenceResult::Request(outcome) => ResultValue::Request(outcome),
                DecisionPersistenceResult::Prepared(prepared) => ResultValue::Prepared(prepared),
                DecisionPersistenceResult::Resolved(outcome) => ResultValue::Resolved(outcome),
                DecisionPersistenceResult::Lowered(outcome) => {
                    ResultValue::ClassificationLowered(outcome)
                }
                DecisionPersistenceResult::Superseded(outcome) => ResultValue::Superseded(outcome),
                DecisionPersistenceResult::Rejected(outcome) => ResultValue::Rejected(outcome),
            };
            state
                .idem
                .insert(capsule.idempotency_id, Stored { signature, result });
        }
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence,
            state,
            fail_next_commit: false,
        }
    }
    pub fn request(&self, id: &DecisionRequestId) -> Option<&DecisionRequestRecord> {
        self.state.requests.get(id)
    }
    pub fn decision(&self, id: &DecisionId) -> Option<&DecisionRecord> {
        self.state.decisions.get(id)
    }
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.state.audits
    }
    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }
    /// Produces an isolated transactional copy for the Product Ledger composition.
    pub fn isolated_copy(&self) -> Self
    where
        C: Clone,
        I: Clone,
        Z: Clone,
        P: Clone,
        E: Clone,
    {
        Self {
            clock: self.clock.clone(),
            ids: self.ids.clone(),
            authorization: self.authorization.clone(),
            policy: self.policy.clone(),
            evidence: self.evidence.clone(),
            state: self.state.clone(),
            fail_next_commit: false,
        }
    }
    pub fn discard_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let existed = self.state.prepared.remove(id).is_some();
        if existed {
            self.state.idem.retain(|_, stored| {
                !matches!(&stored.result, ResultValue::Prepared(prepared) if prepared.id() == id)
            });
        }
        existed
    }

    pub fn create_decision_request_draft(
        &mut self,
        c: CreateDecisionRequestDraft,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.create_cause(c).map_err(|e| domain_error(e, &x))
    }
    pub fn submit_decision_request(
        &mut self,
        c: SubmitDecisionRequest,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.transition(
            c.request_id,
            c.expected_version,
            DecisionRequestState::Draft,
            DecisionRequestState::Open,
            None,
            c.context,
        )
        .map_err(|e| domain_error(e, &x))
    }
    pub fn withdraw_decision_request(
        &mut self,
        c: WithdrawDecisionRequest,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.transition(
            c.request_id,
            c.expected_version,
            DecisionRequestState::Open,
            DecisionRequestState::Withdrawn,
            Some(c.rationale),
            c.context,
        )
        .map_err(|e| domain_error(e, &x))
    }
    pub fn prepare_resolve_decision_request(
        &mut self,
        c: PrepareResolveDecisionRequest,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let x = c.context.clone();
        let target = AuditTarget::DecisionRequest(c.request_id.clone());
        match self.prepare_resolve(c) {
            Ok(prepared) => Ok(prepared),
            Err(cause) => {
                if is_h3_denial(cause) {
                    if let Err(audit_cause) = self.record_prepare_denial(target, &x) {
                        return Err(domain_error(audit_cause, &x));
                    }
                }
                Err(domain_error(cause, &x))
            }
        }
    }
    pub fn prepare_lower_decision_classification(
        &mut self,
        c: PrepareLowerDecisionClassification,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let x = c.context.clone();
        self.prepare_lower_decision_classification_cause(c)
            .map_err(|cause| domain_error(cause, &x))
    }
    fn prepare_lower_decision_classification_cause(
        &mut self,
        c: PrepareLowerDecisionClassification,
    ) -> Result<WorkManagementPreparedIntent, DecisionError> {
        let sig = Signature::PrepareLowerClassification(
            c.decision_id.clone(),
            c.expected_version,
            c.proposed_classification,
            c.rationale.clone(),
        );
        if let Some(p) = self.replay_prepared(&c.context.idempotency_id, &sig)? {
            return Ok(p);
        }
        let r = self.exact_decision(&c.decision_id, c.expected_version)?;
        if !is_genuine_lowering(r.classification, c.proposed_classification) {
            return Err(DecisionError::NotALowering);
        }
        let current_classification = r.classification;
        let op = WorkManagementOperation::LowerDecisionClassification {
            decision_id: c.decision_id,
            decision_version: c.expected_version,
            current_classification,
            proposed_classification: c.proposed_classification,
            rationale: c.rationale,
        };
        if self.policy.current_policy(&op) == DecisionExecutionPolicy::Denied {
            return Err(DecisionError::PolicyDenied);
        }
        let id = self
            .ids
            .next_prepared_intent_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let p = WorkManagementPreparedIntent::prepare(
            id,
            op,
            current_classification,
            None,
            self.clock.now(),
        )
        .map_err(map_prepare)?;
        self.state.prepared.insert(p.id().clone(), p.clone());
        self.state.idem.insert(
            c.context.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::Prepared(p.clone()),
            },
        );
        Ok(p)
    }
    pub fn prepare_supersede_decision<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &mut self,
        c: PrepareSupersedeDecision,
        actions: &InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let x = c.context.clone();
        let target = AuditTarget::Decision(c.decision_id.clone());
        match self.prepare_supersede(c, actions) {
            Ok(prepared) => Ok(prepared),
            Err(cause) => {
                if is_h3_denial(cause) {
                    if let Err(audit_cause) = self.record_prepare_denial(target, &x) {
                        return Err(domain_error(audit_cause, &x));
                    }
                }
                Err(domain_error(cause, &x))
            }
        }
    }

    fn create_cause(
        &mut self,
        c: CreateDecisionRequestDraft,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DecisionError> {
        let sig = Signature::Create(
            c.id.clone(),
            c.subject.clone(),
            c.details.clone(),
            c.intended_owner.clone(),
            c.classification,
        );
        if let Some(v) = self.replay_request(&c.context.idempotency_id, &sig)? {
            return Ok(v);
        }
        if self.state.requests.contains_key(&c.id) {
            return Err(DecisionError::AlreadyExists);
        }
        let r = DecisionRequestRecord {
            id: c.id,
            subject: c.subject,
            details: c.details,
            intended_owner: c.intended_owner,
            classification: c.classification,
            state: DecisionRequestState::Draft,
            withdrawal_rationale: None,
            linked_decision_id: None,
            version: AggregateVersion::initial(),
        };
        self.store_request(r, c.context, sig, "decision_request.created")
    }
    fn transition(
        &mut self,
        id: DecisionRequestId,
        v: AggregateVersion,
        from: DecisionRequestState,
        to: DecisionRequestState,
        rationale: Option<DecisionWithdrawalRationale>,
        ctx: DecisionOperationContext,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DecisionError> {
        let sig = if to == DecisionRequestState::Open {
            Signature::Submit(id.clone(), v)
        } else {
            Signature::Withdraw(
                id.clone(),
                v,
                rationale.clone().ok_or(DecisionError::Illegal)?,
            )
        };
        if let Some(x) = self.replay_request(&ctx.idempotency_id, &sig)? {
            return Ok(x);
        }
        let mut r = self.exact_request(&id, v)?.clone();
        if r.state != from {
            return Err(DecisionError::Illegal);
        }
        r.state = to;
        r.withdrawal_rationale = rationale;
        r.version = next(r.version)?;
        self.store_request(
            r,
            ctx,
            sig,
            if to == DecisionRequestState::Open {
                "decision_request.submitted"
            } else {
                "decision_request.withdrawn"
            },
        )
    }
    fn prepare_resolve(
        &mut self,
        c: PrepareResolveDecisionRequest,
    ) -> Result<WorkManagementPreparedIntent, DecisionError> {
        let sig = Signature::PrepareResolve(
            c.request_id.clone(),
            c.expected_version,
            c.statement.clone(),
            c.rationale.clone(),
            c.impact.clone(),
            c.evidence_ids.clone(),
            c.judgments.clone(),
            c.resulting_action_requests.clone(),
        );
        if let Some(p) = self.replay_prepared(&c.context.idempotency_id, &sig)? {
            return Ok(p);
        }
        let r = self
            .exact_request(&c.request_id, c.expected_version)?
            .clone();
        if r.state != DecisionRequestState::Open {
            return Err(DecisionError::Illegal);
        }
        let owner = r
            .intended_owner
            .clone()
            .ok_or(DecisionError::MissingOwner)?;
        let did = self
            .ids
            .next_decision_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let support = self.support(&c.evidence_ids, c.judgments.clone())?;
        let op = WorkManagementOperation::ResolveDecisionRequest {
            request_id: r.id.clone(),
            request_version: r.version,
            decision_id: did,
            decision_classification: r.classification,
            statement: c.statement,
            rationale: c.rationale,
            impact: c.impact,
            decision_owner: owner,
            decided_at: self.clock.now(),
            resulting_action_requests: c.resulting_action_requests,
        };
        self.prepare(op, r.classification, support, c.context, sig)
    }
    fn prepare_supersede<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &mut self,
        c: PrepareSupersedeDecision,
        actions: &InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Result<WorkManagementPreparedIntent, DecisionError> {
        let sig = Signature::PrepareSupersede(
            c.decision_id.clone(),
            c.expected_version,
            c.replacement_statement.clone(),
            c.replacement_rationale.clone(),
            c.replacement_impact.clone(),
            c.replacement_owner.clone(),
            c.evidence_ids.clone(),
            c.judgments.clone(),
            c.resulting_action_requests.clone(),
        );
        if let Some(p) = self.replay_prepared(&c.context.idempotency_id, &sig)? {
            return Ok(p);
        }
        let old = self
            .exact_decision(&c.decision_id, c.expected_version)?
            .clone();
        if old.state != DecisionState::Effective {
            return Err(DecisionError::Illegal);
        }
        let replacement = self
            .ids
            .next_decision_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let support = self.support(&c.evidence_ids, c.judgments.clone())?;
        let mut downstream = Vec::new();
        for r in actions.action_requests_for_decision(&old.id) {
            if matches!(
                r.state(),
                crate::work_management::ActionRequestState::Draft
                    | crate::work_management::ActionRequestState::Open
            ) {
                downstream.push(IncompleteDownstreamWork::ActionRequest(
                    r.id().clone(),
                    r.version(),
                    r.classification(),
                ));
            }
        }
        for a in actions.actions_for_decision(&old.id) {
            if matches!(
                a.state(),
                crate::work_management::ActionState::Open
                    | crate::work_management::ActionState::InProgress
            ) {
                downstream.push(IncompleteDownstreamWork::Action(
                    a.id().clone(),
                    a.version(),
                    a.classification(),
                ));
            }
        }
        let class = old.classification.combine(support.classification());
        let op = WorkManagementOperation::SupersedeDecision {
            decision_id: old.id.clone(),
            decision_version: old.version,
            replacement_decision_id: replacement,
            replacement_decision_version: AggregateVersion::initial(),
            replacement_decision_classification: class,
            replacement_statement: c.replacement_statement,
            replacement_rationale: c.replacement_rationale,
            replacement_impact: c.replacement_impact,
            replacement_owner: c.replacement_owner,
            replacement_decided_at: self.clock.now(),
            resulting_action_requests: c.resulting_action_requests,
            incomplete_downstream: downstream,
        };
        self.prepare(op, old.classification, support, c.context, sig)
    }
    fn support(
        &self,
        ids: &[EvidenceReferenceId],
        judgments: Vec<HumanJudgment>,
    ) -> Result<SupportWitness, DecisionError> {
        let evidence = ids
            .iter()
            .map(|id| {
                self.evidence
                    .resolve(id)
                    .map_err(|_| DecisionError::EvidenceUnavailable)
            })
            .collect::<Result<Vec<_>, _>>()?;
        EvidenceOrJudgment::new(evidence, judgments)
            .map_err(|_| DecisionError::SupportDenied)?
            .evaluate_evidence_or_judgment()
            .map_err(|_| DecisionError::SupportDenied)
    }
    fn prepare(
        &mut self,
        op: WorkManagementOperation,
        class: DataClassification,
        support: SupportWitness,
        ctx: DecisionOperationContext,
        sig: Signature,
    ) -> Result<WorkManagementPreparedIntent, DecisionError> {
        if self.policy.current_policy(&op) == DecisionExecutionPolicy::Denied {
            return Err(DecisionError::PolicyDenied);
        }
        let id = self
            .ids
            .next_prepared_intent_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let p =
            WorkManagementPreparedIntent::prepare(id, op, class, Some(support), self.clock.now())
                .map_err(map_prepare)?;
        self.state.prepared.insert(p.id().clone(), p.clone());
        self.state.idem.insert(
            ctx.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::Prepared(p.clone()),
            },
        );
        Ok(p)
    }

    pub fn approve_and_execute_resolve_decision_request<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &mut self,
        c: ApproveAndExecuteResolveDecisionRequest,
        actions: &mut InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Result<ResolvedDecisionOutcome, DomainError> {
        let x = c.context.clone();
        let approval = c.approval.clone();
        match self.execute_resolve(c, actions) {
            Ok(outcome) => Ok(outcome),
            Err(cause) => {
                if let Err(audit_cause) = self.record_h2_failure(&approval, &x, cause) {
                    return Err(domain_error(audit_cause, &x));
                }
                Err(domain_error(cause, &x))
            }
        }
    }
    /// H2a rejection (v45): the Head of Products refuses a pending Resolve
    /// preview. A successful command with no effect: the intent is consumed
    /// and can never be executed, one zero-effect audit is recorded, no
    /// receipt is minted. Rejecting an already-consumed intent is a
    /// conflict; an unknown one is not found; an expired-but-unconsumed
    /// preview is rejected normally and the outcome says it had expired.
    pub fn reject_decision_prepared_intent(
        &mut self,
        c: RejectDecisionPreparedIntent,
    ) -> Result<RejectedPreparedIntentOutcome, DomainError> {
        let context = c.context.clone();
        self.reject_decision_prepared_intent_cause(c)
            .map_err(|cause| domain_error(cause, &context))
    }
    fn reject_decision_prepared_intent_cause(
        &mut self,
        c: RejectDecisionPreparedIntent,
    ) -> Result<RejectedPreparedIntentOutcome, DecisionError> {
        let sig = Signature::RejectPrepared(c.prepared_id.clone(), c.actor);
        if let Some(v) = self.replay(&c.context.idempotency_id, &sig)? {
            return match v {
                ResultValue::Rejected(outcome) => Ok(outcome.clone()),
                _ => Err(DecisionError::IdempotencyConflict),
            };
        }
        if c.actor != AuditActor::HeadOfProducts || !self.authorization.authorize(c.actor) {
            return Err(DecisionError::Unauthorized);
        }
        let Some(p) = self.state.prepared.get(&c.prepared_id).cloned() else {
            let known = self.state.idem.values().any(|stored| {
                matches!(&stored.result, ResultValue::Prepared(prepared) if prepared.id() == &c.prepared_id)
            });
            return Err(if known {
                DecisionError::Illegal
            } else {
                DecisionError::NotFound
            });
        };
        let WorkManagementOperation::ResolveDecisionRequest { request_id, .. } = p.operation()
        else {
            return Err(DecisionError::Illegal);
        };
        let now = self.clock.now();
        let audit_id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let audit = crate::work_management::prepared_intent_rejection_audit(
            audit_id,
            now,
            DECISION_PREPARED_REJECTED_AUDIT_CODE,
            AuditTarget::DecisionRequest(request_id.clone()),
            c.context.correlation_id.clone(),
        )
        .ok_or(DecisionError::Infrastructure)?;
        let outcome = RejectedPreparedIntentOutcome::new(
            p.id().clone(),
            now,
            now >= p.preview().expires_at(),
            audit.clone(),
        );
        let mut staged = self.state.clone();
        staged.prepared.remove(&c.prepared_id);
        staged.audits.push(audit);
        staged.idem.insert(
            c.context.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::Rejected(outcome.clone()),
            },
        );
        self.finish(staged)?;
        Ok(outcome)
    }
    pub fn approve_and_execute_lower_decision_classification(
        &mut self,
        c: ApproveAndExecuteLowerDecisionClassification,
    ) -> Result<DecisionMutationOutcome<DecisionRecord>, DomainError> {
        let x = c.context.clone();
        let approval = c.approval.clone();
        match self.execute_lower_decision_classification(c) {
            Ok(outcome) => Ok(outcome),
            Err(cause) => {
                if let Err(audit_cause) = self.record_h2_failure(&approval, &x, cause) {
                    return Err(domain_error(audit_cause, &x));
                }
                Err(domain_error(cause, &x))
            }
        }
    }
    fn execute_lower_decision_classification(
        &mut self,
        c: ApproveAndExecuteLowerDecisionClassification,
    ) -> Result<DecisionMutationOutcome<DecisionRecord>, DecisionError> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(DecisionError::IdempotencyConflict);
        }
        let sig = Signature::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(v) = self.replay(&c.context.idempotency_id, &sig)? {
            return match v {
                ResultValue::ClassificationLowered(outcome) => Ok(outcome.clone()),
                _ => Err(DecisionError::IdempotencyConflict),
            };
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(DecisionError::PreparedChanged)?;
        let (decision_id, decision_version, proposed_classification, rationale) =
            match p.operation() {
                WorkManagementOperation::LowerDecisionClassification {
                    decision_id,
                    decision_version,
                    proposed_classification,
                    rationale,
                    ..
                } => (
                    decision_id.clone(),
                    *decision_version,
                    *proposed_classification,
                    rationale.clone(),
                ),
                _ => return Err(DecisionError::PreparedChanged),
            };
        let r = self.exact_decision(&decision_id, decision_version)?.clone();
        let current_op = WorkManagementOperation::LowerDecisionClassification {
            decision_id: decision_id.clone(),
            decision_version: r.version,
            current_classification: r.classification,
            proposed_classification,
            rationale,
        };
        if self.policy.current_policy(&current_op) == DecisionExecutionPolicy::Denied {
            return Err(DecisionError::PolicyDenied);
        }
        let receipt_id = self
            .ids
            .next_approval_receipt_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            current_op.clone(),
            r.classification,
            None,
            self.clock.now(),
        )
        .map_err(map_prepare)?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            &p,
            &c.approval,
            &snapshot,
            self.clock.now(),
            receipt_id,
            &self.authorization,
        )
        .map_err(map_validation)?;
        let mut mutated = r;
        mutated.version = next(mutated.version)?;
        mutated.classification = proposed_classification;
        let audit = self.audit(
            AuditTarget::Decision(decision_id.clone()),
            "decision.classification_lowered",
            &c.context,
            true,
        )?;
        let outcome = DecisionMutationOutcome {
            record: mutated.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: Some(receipt.id),
        };
        self.state.decisions.insert(decision_id, mutated);
        self.state.prepared.remove(c.approval.prepared_id());
        self.state.audits.push(audit);
        self.state.idem.insert(
            c.context.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::ClassificationLowered(outcome.clone()),
            },
        );
        Ok(outcome)
    }
    pub fn approve_and_execute_supersede_decision<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &mut self,
        c: ApproveAndExecuteSupersedeDecision,
        actions: &mut InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Result<SupersededDecisionOutcome, DomainError> {
        let x = c.context.clone();
        let approval = c.approval.clone();
        match self.execute_supersede(c, actions) {
            Ok(outcome) => Ok(outcome),
            Err(cause) => {
                if let Err(audit_cause) = self.record_h2_failure(&approval, &x, cause) {
                    return Err(domain_error(audit_cause, &x));
                }
                Err(domain_error(cause, &x))
            }
        }
    }

    fn execute_resolve<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &mut self,
        c: ApproveAndExecuteResolveDecisionRequest,
        actions: &mut InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Result<ResolvedDecisionOutcome, DecisionError> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(DecisionError::IdempotencyConflict);
        }
        let sig = Signature::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(v) = self.replay_resolved(&c.context.idempotency_id, &sig)? {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(DecisionError::PreparedChanged)?;
        let (rid, rv, did, statement, rationale, impact, owner, decided_at, specs) =
            match p.operation() {
                WorkManagementOperation::ResolveDecisionRequest {
                    request_id,
                    request_version,
                    decision_id,
                    statement,
                    rationale,
                    impact,
                    decision_owner,
                    decided_at,
                    resulting_action_requests,
                    ..
                } => (
                    request_id.clone(),
                    *request_version,
                    decision_id.clone(),
                    statement.clone(),
                    rationale.clone(),
                    impact.clone(),
                    decision_owner.clone(),
                    *decided_at,
                    resulting_action_requests.clone(),
                ),
                _ => return Err(DecisionError::PreparedChanged),
            };
        let mut request = self
            .exact_request(&rid, rv)
            .map_err(|error| match error {
                DecisionError::Stale => DecisionError::PreparedChanged,
                other => other,
            })?
            .clone();
        if request.state != DecisionRequestState::Open
            || request.intended_owner.as_ref() != Some(&owner)
        {
            return Err(DecisionError::PreparedChanged);
        }
        if self.state.decisions.contains_key(&did) {
            return Err(DecisionError::AlreadyExists);
        }
        let current_support = self.current_support(&p)?;
        let current_op = WorkManagementOperation::ResolveDecisionRequest {
            request_id: rid.clone(),
            request_version: request.version,
            decision_id: did.clone(),
            decision_classification: request.classification,
            statement: statement.clone(),
            rationale: rationale.clone(),
            impact: impact.clone(),
            decision_owner: owner.clone(),
            decided_at,
            resulting_action_requests: specs.clone(),
        };
        let receipt = self.validate(
            &p,
            &c.approval,
            current_op,
            request.classification,
            current_support.clone(),
        )?;
        let mut action_stage = actions.begin_decision_stage();
        for (index, spec) in specs.iter().enumerate() {
            actions
                .create_resulting_action_request_from_decision_transition(
                    &mut action_stage,
                    spec,
                    did.clone(),
                    p.classification(),
                    action_context(&c.context, index, "resolve"),
                )
                .map_err(map_action)?;
        }
        request.state = DecisionRequestState::Resolved;
        request.linked_decision_id = Some(did.clone());
        request.version = next(request.version)?;
        let decision = DecisionRecord {
            id: did.clone(),
            source_request_id: Some(rid),
            statement,
            rationale,
            impact,
            owner,
            decided_at,
            classification: p.classification(),
            state: DecisionState::Effective,
            support: current_support,
            resulting_action_request_ids: specs.iter().map(|x| x.id.clone()).collect(),
            supersedes_decision_id: None,
            superseded_by_decision_id: None,
            version: AggregateVersion::initial(),
        };
        let audits = vec![
            self.audit(
                AuditTarget::DecisionRequest(request.id.clone()),
                "decision_request.resolved",
                &c.context,
                true,
            )?,
            self.audit(
                AuditTarget::Decision(did.clone()),
                "decision.created",
                &c.context,
                true,
            )?,
            self.audit(
                AuditTarget::DecisionRequest(request.id.clone()),
                "decision_request.decision_linked",
                &c.context,
                true,
            )?,
        ];
        let outcome = ResolvedDecisionOutcome {
            request: request.clone(),
            decision: decision.clone(),
            resulting_action_request_ids: decision.resulting_action_request_ids.clone(),
            audit_events: audits.clone(),
            approval_receipt_id: receipt,
        };
        let mut staged = self.state.clone();
        staged.requests.insert(request.id.clone(), request);
        staged.decisions.insert(did, decision);
        staged.prepared.remove(c.approval.prepared_id());
        staged.audits.extend(audits);
        staged.idem.insert(
            c.context.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::Resolved(outcome.clone()),
            },
        );
        actions
            .preflight_decision_stage_commit()
            .map_err(map_action)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(DecisionError::Infrastructure);
        }
        self.state = staged;
        actions.apply_decision_stage_unchecked(action_stage);
        Ok(outcome)
    }

    fn execute_supersede<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &mut self,
        c: ApproveAndExecuteSupersedeDecision,
        actions: &mut InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Result<SupersededDecisionOutcome, DecisionError> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(DecisionError::IdempotencyConflict);
        }
        let sig = Signature::Execute(
            c.approval.prepared_id().clone(),
            c.approval.actor(),
            c.approval.acknowledged_payload_digest().clone(),
        );
        if let Some(v) = self.replay_superseded(&c.context.idempotency_id, &sig)? {
            return Ok(v);
        }
        let p = self
            .state
            .prepared
            .get(c.approval.prepared_id())
            .cloned()
            .ok_or(DecisionError::PreparedChanged)?;
        let (
            old_id,
            old_v,
            new_id,
            statement,
            rationale,
            impact,
            owner,
            decided_at,
            specs,
            downstream,
        ) = match p.operation() {
            WorkManagementOperation::SupersedeDecision {
                decision_id,
                decision_version,
                replacement_decision_id,
                replacement_statement,
                replacement_rationale,
                replacement_impact,
                replacement_owner,
                replacement_decided_at,
                resulting_action_requests,
                incomplete_downstream,
                ..
            } => (
                decision_id.clone(),
                *decision_version,
                replacement_decision_id.clone(),
                replacement_statement.clone(),
                replacement_rationale.clone(),
                replacement_impact.clone(),
                replacement_owner.clone(),
                *replacement_decided_at,
                resulting_action_requests.clone(),
                incomplete_downstream.clone(),
            ),
            _ => return Err(DecisionError::PreparedChanged),
        };
        let mut old = self
            .exact_decision(&old_id, old_v)
            .map_err(|error| match error {
                DecisionError::Stale => DecisionError::PreparedChanged,
                other => other,
            })?
            .clone();
        if old.state != DecisionState::Effective || self.state.decisions.contains_key(&new_id) {
            return Err(DecisionError::PreparedChanged);
        }
        let current_support = self.current_support(&p)?;
        let current_downstream = self.downstream(&old_id, actions);
        if current_downstream != downstream {
            return Err(DecisionError::PreparedChanged);
        }
        let current_op = WorkManagementOperation::SupersedeDecision {
            decision_id: old_id.clone(),
            decision_version: old.version,
            replacement_decision_id: new_id.clone(),
            replacement_decision_version: AggregateVersion::initial(),
            replacement_decision_classification: old
                .classification
                .combine(current_support.classification()),
            replacement_statement: statement.clone(),
            replacement_rationale: rationale.clone(),
            replacement_impact: impact.clone(),
            replacement_owner: owner.clone(),
            replacement_decided_at: decided_at,
            resulting_action_requests: specs.clone(),
            incomplete_downstream: downstream.clone(),
        };
        let receipt = self.validate(
            &p,
            &c.approval,
            current_op,
            old.classification,
            current_support.clone(),
        )?;
        let mut action_stage = actions.begin_decision_stage();
        let mut offset = 0;
        for spec in &specs {
            actions
                .create_resulting_action_request_from_decision_transition(
                    &mut action_stage,
                    spec,
                    new_id.clone(),
                    p.classification(),
                    action_context(&c.context, offset, "supersede-create"),
                )
                .map_err(map_action)?;
            offset += 1;
        }
        for item in &downstream {
            match item {
                IncompleteDownstreamWork::ActionRequest(id, v, _) => {
                    actions
                        .mark_action_request_superseded_premise_from_decision(
                            &mut action_stage,
                            id,
                            *v,
                            &old_id,
                            p.classification(),
                            action_context(&c.context, offset, "supersede-request"),
                        )
                        .map_err(map_action)?;
                }
                IncompleteDownstreamWork::Action(id, v, _) => {
                    actions
                        .mark_action_superseded_premise_from_decision(
                            &mut action_stage,
                            id,
                            *v,
                            &old_id,
                            p.classification(),
                            action_context(&c.context, offset, "supersede-action"),
                        )
                        .map_err(map_action)?;
                }
            }
            offset += 1;
        }
        old.state = DecisionState::Superseded;
        old.superseded_by_decision_id = Some(new_id.clone());
        old.version = next(old.version)?;
        let replacement = DecisionRecord {
            id: new_id.clone(),
            source_request_id: None,
            statement,
            rationale,
            impact,
            owner,
            decided_at,
            classification: p.classification(),
            state: DecisionState::Effective,
            support: current_support,
            resulting_action_request_ids: specs.iter().map(|x| x.id.clone()).collect(),
            supersedes_decision_id: Some(old_id.clone()),
            superseded_by_decision_id: None,
            version: AggregateVersion::initial(),
        };
        let audits = vec![
            self.audit(
                AuditTarget::Decision(old_id.clone()),
                "decision.superseded",
                &c.context,
                true,
            )?,
            self.audit(
                AuditTarget::Decision(new_id.clone()),
                "decision.replacement_created",
                &c.context,
                true,
            )?,
            self.audit(
                AuditTarget::Decision(old_id.clone()),
                "decision.replacement_linked",
                &c.context,
                true,
            )?,
        ];
        let outcome = SupersededDecisionOutcome {
            superseded: old.clone(),
            replacement: replacement.clone(),
            resulting_action_request_ids: replacement.resulting_action_request_ids.clone(),
            flagged_downstream: downstream,
            audit_events: audits.clone(),
            approval_receipt_id: receipt,
        };
        let mut staged = self.state.clone();
        staged.decisions.insert(old_id, old);
        staged.decisions.insert(new_id, replacement);
        staged.prepared.remove(c.approval.prepared_id());
        staged.audits.extend(audits);
        staged.idem.insert(
            c.context.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::Superseded(outcome.clone()),
            },
        );
        actions
            .preflight_decision_stage_commit()
            .map_err(map_action)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(DecisionError::Infrastructure);
        }
        self.state = staged;
        actions.apply_decision_stage_unchecked(action_stage);
        Ok(outcome)
    }

    fn current_support(
        &self,
        p: &WorkManagementPreparedIntent,
    ) -> Result<SupportWitness, DecisionError> {
        let prior = p.preview().support().ok_or(DecisionError::SupportDenied)?;
        let ids = prior
            .evidence()
            .iter()
            .map(|x| x.id().clone())
            .collect::<Vec<_>>();
        self.support(&ids, prior.judgments().to_vec())
    }
    fn downstream<
        AC: Clock,
        AI: ActionServiceIdSource,
        AZ: ApprovalAuthorizationPort,
        AP: ActionExecutionPolicyPort,
        AE: ActionEvidenceAuthorityPort,
    >(
        &self,
        id: &DecisionId,
        actions: &InMemoryActionService<AC, AI, AZ, AP, AE>,
    ) -> Vec<IncompleteDownstreamWork> {
        let mut x = Vec::new();
        for r in actions.action_requests_for_decision(id) {
            if matches!(
                r.state(),
                crate::work_management::ActionRequestState::Draft
                    | crate::work_management::ActionRequestState::Open
            ) {
                x.push(IncompleteDownstreamWork::ActionRequest(
                    r.id().clone(),
                    r.version(),
                    r.classification(),
                ));
            }
        }
        for a in actions.actions_for_decision(id) {
            if matches!(
                a.state(),
                crate::work_management::ActionState::Open
                    | crate::work_management::ActionState::InProgress
            ) {
                x.push(IncompleteDownstreamWork::Action(
                    a.id().clone(),
                    a.version(),
                    a.classification(),
                ));
            }
        }
        x.sort_by(|a, b| downstream_key(a).cmp(&downstream_key(b)));
        x
    }
    fn validate(
        &mut self,
        p: &WorkManagementPreparedIntent,
        a: &WorkManagementApproval,
        op: WorkManagementOperation,
        class: DataClassification,
        support: SupportWitness,
    ) -> Result<ApprovalReceiptId, DecisionError> {
        if self.policy.current_policy(&op) == DecisionExecutionPolicy::Denied {
            return Err(DecisionError::PolicyDenied);
        }
        let id = self
            .ids
            .next_approval_receipt_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let fresh = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            op.clone(),
            class,
            Some(support.clone()),
            self.clock.now(),
        )
        .map_err(map_prepare)?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: Some(support),
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt = validate_and_mint_work_management_h2a_receipt(
            p,
            a,
            &snapshot,
            self.clock.now(),
            id.clone(),
            &self.authorization,
        )
        .map_err(map_validation)?;
        Ok(receipt.id)
    }

    fn exact_request(
        &self,
        id: &DecisionRequestId,
        v: AggregateVersion,
    ) -> Result<&DecisionRequestRecord, DecisionError> {
        let r = self.state.requests.get(id).ok_or(DecisionError::NotFound)?;
        if r.version != v {
            return Err(DecisionError::Stale);
        }
        Ok(r)
    }
    fn exact_decision(
        &self,
        id: &DecisionId,
        v: AggregateVersion,
    ) -> Result<&DecisionRecord, DecisionError> {
        let r = self
            .state
            .decisions
            .get(id)
            .ok_or(DecisionError::NotFound)?;
        if r.version != v {
            return Err(DecisionError::Stale);
        }
        Ok(r)
    }
    fn replay(
        &self,
        id: &IdempotencyId,
        sig: &Signature,
    ) -> Result<Option<&ResultValue>, DecisionError> {
        match self.state.idem.get(id) {
            None => Ok(None),
            Some(x) if &x.signature == sig => Ok(Some(&x.result)),
            Some(_) => Err(DecisionError::IdempotencyConflict),
        }
    }
    fn replay_request(
        &self,
        id: &IdempotencyId,
        sig: &Signature,
    ) -> Result<Option<DecisionMutationOutcome<DecisionRequestRecord>>, DecisionError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(ResultValue::Request(x)) => Ok(Some(x.clone())),
            _ => Err(DecisionError::IdempotencyConflict),
        }
    }
    fn replay_prepared(
        &self,
        id: &IdempotencyId,
        sig: &Signature,
    ) -> Result<Option<WorkManagementPreparedIntent>, DecisionError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(ResultValue::Prepared(x)) => Ok(Some(x.clone())),
            _ => Err(DecisionError::IdempotencyConflict),
        }
    }
    fn replay_resolved(
        &self,
        id: &IdempotencyId,
        sig: &Signature,
    ) -> Result<Option<ResolvedDecisionOutcome>, DecisionError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(ResultValue::Resolved(x)) => Ok(Some(x.clone())),
            _ => Err(DecisionError::IdempotencyConflict),
        }
    }
    fn replay_superseded(
        &self,
        id: &IdempotencyId,
        sig: &Signature,
    ) -> Result<Option<SupersededDecisionOutcome>, DecisionError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(ResultValue::Superseded(x)) => Ok(Some(x.clone())),
            _ => Err(DecisionError::IdempotencyConflict),
        }
    }
    fn store_request(
        &mut self,
        r: DecisionRequestRecord,
        ctx: DecisionOperationContext,
        sig: Signature,
        code: &str,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DecisionError> {
        let audit = self.audit(
            AuditTarget::DecisionRequest(r.id.clone()),
            code,
            &ctx,
            false,
        )?;
        let out = DecisionMutationOutcome {
            record: r.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        let mut s = self.state.clone();
        s.requests.insert(r.id.clone(), r);
        s.audits.push(audit);
        s.idem.insert(
            ctx.idempotency_id,
            Stored {
                signature: sig,
                result: ResultValue::Request(out.clone()),
            },
        );
        self.finish(s)?;
        Ok(out)
    }
    fn finish(&mut self, s: Store) -> Result<(), DecisionError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(DecisionError::Infrastructure);
        }
        self.state = s;
        Ok(())
    }
    fn audit(
        &mut self,
        target: AuditTarget,
        code: &str,
        ctx: &DecisionOperationContext,
        approved: bool,
    ) -> Result<AuditEvent, DecisionError> {
        let id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| DecisionError::Infrastructure)?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(code).map_err(|_| DecisionError::Infrastructure)?,
            target,
        );
        let disposition = AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            if approved {
                AuditApprovalOutcome::Approved
            } else {
                AuditApprovalOutcome::NotRequired
            },
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse(code).map_err(|_| DecisionError::Infrastructure)?],
        )
        .map_err(|_| DecisionError::Infrastructure)?;
        Ok(AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            action,
            ctx.correlation_id.clone(),
            disposition,
        ))
    }

    fn record_h2_failure(
        &mut self,
        approval: &WorkManagementApproval,
        context: &DecisionOperationContext,
        cause: DecisionError,
    ) -> Result<(), DecisionError> {
        let Some(target) = self
            .state
            .prepared
            .get(approval.prepared_id())
            .map(|prepared| match prepared.operation() {
                WorkManagementOperation::ResolveDecisionRequest { request_id, .. } => {
                    AuditTarget::DecisionRequest(request_id.clone())
                }
                WorkManagementOperation::SupersedeDecision { decision_id, .. }
                | WorkManagementOperation::LowerDecisionClassification { decision_id, .. } => {
                    AuditTarget::Decision(decision_id.clone())
                }
                _ => unreachable!("Decision service stores only Decision prepared intents"),
            })
        else {
            return Ok(());
        };
        let denied = matches!(
            cause,
            DecisionError::PolicyDenied
                | DecisionError::SupportDenied
                | DecisionError::EvidenceUnavailable
                | DecisionError::Unauthorized
        );
        let infrastructure = matches!(cause, DecisionError::Infrastructure);
        let disposition = AuditDisposition::new(
            if denied {
                AuditPolicyOutcome::Denied
            } else {
                AuditPolicyOutcome::Allowed
            },
            if denied {
                AuditApprovalOutcome::NotRequired
            } else if infrastructure {
                AuditApprovalOutcome::Approved
            } else {
                AuditApprovalOutcome::Rejected
            },
            if infrastructure {
                AuditExecutionOutcome::Failed
            } else {
                AuditExecutionOutcome::NotAttempted
            },
            AuditEffectScope::None,
            vec![],
        )
        .map_err(|_| DecisionError::Infrastructure)?;
        let event = AuditEvent::new(
            self.ids
                .next_audit_event_id()
                .map_err(|_| DecisionError::Infrastructure)?,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse(if denied {
                    "decision.approval_denied"
                } else if infrastructure {
                    "decision.execution_failed"
                } else {
                    "decision.approval_rejected"
                })
                .map_err(|_| DecisionError::Infrastructure)?,
                target,
            ),
            context.correlation_id.clone(),
            disposition,
        );
        self.state.audits.push(event);
        Ok(())
    }

    fn record_prepare_denial(
        &mut self,
        target: AuditTarget,
        context: &DecisionOperationContext,
    ) -> Result<(), DecisionError> {
        let disposition = AuditDisposition::new(
            AuditPolicyOutcome::Denied,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::NotAttempted,
            AuditEffectScope::None,
            vec![],
        )
        .map_err(|_| DecisionError::Infrastructure)?;
        self.state.audits.push(AuditEvent::new(
            self.ids
                .next_audit_event_id()
                .map_err(|_| DecisionError::Infrastructure)?,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse("decision.prepare_denied")
                    .map_err(|_| DecisionError::Infrastructure)?,
                target,
            ),
            context.correlation_id.clone(),
            disposition,
        ));
        Ok(())
    }
}

fn next(v: AggregateVersion) -> Result<AggregateVersion, DecisionError> {
    v.next().ok_or(DecisionError::Infrastructure)
}
/// True only for a genuine lowering (`proposed` strictly less restrictive
/// than `current`). See `portfolio::is_genuine_lowering` -- same check,
/// duplicated per module rather than shared.
fn is_genuine_lowering(current: DataClassification, proposed: DataClassification) -> bool {
    proposed.combine(current) != proposed
}
fn is_h3_denial(error: DecisionError) -> bool {
    matches!(
        error,
        DecisionError::PolicyDenied
            | DecisionError::SupportDenied
            | DecisionError::EvidenceUnavailable
    )
}
fn map_prepare(e: PreparedIntentError) -> DecisionError {
    match e {
        PreparedIntentError::UnclassifiedBinding | PreparedIntentError::InvalidSupport => {
            DecisionError::SupportDenied
        }
        _ => DecisionError::Illegal,
    }
}
fn map_validation(e: WorkManagementApprovalValidationError) -> DecisionError {
    match e {
        WorkManagementApprovalValidationError::Unauthorized => DecisionError::Unauthorized,
        WorkManagementApprovalValidationError::DigestMismatch
        | WorkManagementApprovalValidationError::Expired
        | WorkManagementApprovalValidationError::PreparedIntentMismatch
        | WorkManagementApprovalValidationError::PreviewChanged => DecisionError::PreparedChanged,
    }
}
fn map_action(e: ActionServiceError) -> DecisionError {
    match e {
        ActionServiceError::InfrastructureFailure => DecisionError::Infrastructure,
        ActionServiceError::StaleVersion => DecisionError::Stale,
        ActionServiceError::IdempotencyConflict => DecisionError::IdempotencyConflict,
        _ => DecisionError::PreparedChanged,
    }
}
fn domain_error(e: DecisionError, c: &DecisionOperationContext) -> DomainError {
    let (code, key, retry) = match e {
        DecisionError::NotFound => (ErrorCode::DomainNotFound, "decision.not_found", false),
        DecisionError::Stale
        | DecisionError::Illegal
        | DecisionError::AlreadyExists
        | DecisionError::MissingOwner
        | DecisionError::IdempotencyConflict => {
            (ErrorCode::DomainConflict, "decision.conflict", false)
        }
        DecisionError::SupportDenied
        | DecisionError::EvidenceUnavailable
        | DecisionError::Unauthorized
        | DecisionError::PolicyDenied => (
            ErrorCode::SecurityPolicyDenied,
            "decision.approval_denied",
            false,
        ),
        DecisionError::PreparedChanged => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "decision.preview_changed",
            false,
        ),
        DecisionError::Infrastructure => (ErrorCode::PlatformInternal, "decision.internal", true),
        DecisionError::NotALowering => (
            ErrorCode::DomainConflict,
            "decision.classification_lowering_not_a_lowering",
            false,
        ),
    };
    DomainError::new(
        code,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        c.correlation_id.clone(),
        retry,
    )
}
fn action_context(
    c: &DecisionOperationContext,
    index: usize,
    kind: &str,
) -> ActionOperationContext {
    let mut digest = Sha256::new();
    for field in [c.idempotency_id.as_str().as_bytes(), kind.as_bytes()] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.update((index as u64).to_be_bytes());
    let digest = digest.finalize();
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(format!("decision-stage-v1-{digest:x}"))
            .unwrap_or_else(|_| unreachable!()),
        correlation_id: c.correlation_id.clone(),
    }
}
fn downstream_key(x: &IncompleteDownstreamWork) -> (&str, &str) {
    match x {
        IncompleteDownstreamWork::ActionRequest(id, ..) => (id.as_str(), "request"),
        IncompleteDownstreamWork::Action(id, ..) => (id.as_str(), "action"),
    }
}
