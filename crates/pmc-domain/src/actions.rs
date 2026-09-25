//! Governed Action Request and Action lifecycle service.
#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};

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
    ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, CorrelationId, DecisionId,
    EvidenceReferenceId, IdempotencyId, PreparedIntentId, StakeholderId,
};
use crate::time::{Clock, UtcTimestamp};
use crate::value::BoundedText;
use crate::work_management::{
    validate_and_mint_work_management_h2a_receipt, ActionReopenMode, ActionRequestState,
    ActionState, ApprovalAuthorizationPort, DecisionResultingActionRequest,
    EvidenceClassificationBinding, EvidenceOrJudgment, EvidenceReferenceMetadata,
    EvidenceVerification, HumanJudgment, PreparedIntentError, RejectedPreparedIntentOutcome,
    SupportWitness, WorkManagementApproval, WorkManagementApprovalValidationError,
    WorkManagementAuthoritativeSnapshot, WorkManagementCurrentPolicy, WorkManagementOperation,
    WorkManagementPayloadDigest, WorkManagementPreparedIntent, WorkManagementRationale,
};

pub type ActionTitle = BoundedText<240>;
pub type ActionDetails = BoundedText<2_000>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionTransitionRecord {
    from: ActionState,
    to: ActionState,
    reason: Option<ActionDetails>,
    occurred_at: UtcTimestamp,
    support: Option<SupportWitness>,
    approval_receipt_id: Option<ApprovalReceiptId>,
}
impl ActionTransitionRecord {
    /// Sibling work to H2a Lower Data Classification persistence: SQLite
    /// persistence for Action Cancel/Complete/Reopen. Additive-only
    /// rehydration constructor -- mirrors the spirit of
    /// `ActionRecord::from_persisted_accepted_request_with_lowered_classification`,
    /// letting the ledger reconstruct a durable transition-history entry
    /// without exposing this struct's private fields.
    #[doc(hidden)]
    pub fn from_persisted(
        from: ActionState,
        to: ActionState,
        reason: Option<ActionDetails>,
        occurred_at: UtcTimestamp,
        support: Option<SupportWitness>,
        approval_receipt_id: Option<ApprovalReceiptId>,
    ) -> Self {
        Self {
            from,
            to,
            reason,
            occurred_at,
            support,
            approval_receipt_id,
        }
    }
    pub const fn from(&self) -> ActionState {
        self.from
    }
    pub const fn to(&self) -> ActionState {
        self.to
    }
    pub fn reason(&self) -> Option<&ActionDetails> {
        self.reason.as_ref()
    }
    pub const fn occurred_at(&self) -> UtcTimestamp {
        self.occurred_at
    }
    pub const fn support(&self) -> Option<&SupportWitness> {
        self.support.as_ref()
    }
    pub const fn approval_receipt_id(&self) -> Option<&ApprovalReceiptId> {
        self.approval_receipt_id.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionOperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequestRecord {
    id: ActionRequestId,
    title: ActionTitle,
    details: ActionDetails,
    intended_owner: Option<StakeholderId>,
    response_due_at: Option<UtcTimestamp>,
    intended_action_due_at: Option<UtcTimestamp>,
    classification: DataClassification,
    state: ActionRequestState,
    terminal_rationale: Option<ActionDetails>,
    linked_action_id: Option<ActionId>,
    source_decision_id: Option<DecisionId>,
    superseded_premise: bool,
    version: AggregateVersion,
}
impl ActionRequestRecord {
    /// Reconstruct the canonical initial record emitted by `CreateRequest`.
    ///
    /// This is intentionally narrower than a general persistence constructor:
    /// callers cannot provide state, version, terminal data, or relationships.
    /// Persistence adapters must supply already validated domain values.
    #[doc(hidden)]
    pub fn from_persisted_created_draft(
        id: ActionRequestId,
        title: ActionTitle,
        details: ActionDetails,
        intended_owner: Option<StakeholderId>,
        response_due_at: Option<UtcTimestamp>,
        intended_action_due_at: Option<UtcTimestamp>,
        classification: DataClassification,
    ) -> Self {
        Self {
            id,
            title,
            details,
            intended_owner,
            response_due_at,
            intended_action_due_at,
            classification,
            state: ActionRequestState::Draft,
            terminal_rationale: None,
            linked_action_id: None,
            source_decision_id: None,
            superseded_premise: false,
            version: AggregateVersion::initial(),
        }
    }

    /// Reconstruct the canonical record emitted after `SubmitActionRequest`.
    ///
    /// This is deliberately narrower than a general persistence constructor:
    /// the adapter may provide only the fields carried forward from the draft.
    /// Submission always produces the one legal `Open`/version `2` shape and
    /// clears all terminal and relationship fields.  In particular, callers
    /// cannot smuggle in an accepted action, decision origin, or superseded
    /// premise while decoding a submitted request.
    #[doc(hidden)]
    pub fn from_persisted_submitted_open(
        id: ActionRequestId,
        title: ActionTitle,
        details: ActionDetails,
        intended_owner: Option<StakeholderId>,
        response_due_at: Option<UtcTimestamp>,
        intended_action_due_at: Option<UtcTimestamp>,
        classification: DataClassification,
    ) -> Self {
        Self {
            id,
            title,
            details,
            intended_owner,
            response_due_at,
            intended_action_due_at,
            classification,
            state: ActionRequestState::Open,
            terminal_rationale: None,
            linked_action_id: None,
            source_decision_id: None,
            superseded_premise: false,
            // AggregateVersion starts at one for Create and advances once on
            // Submit. `initial().next()` is infallible for this fixed value;
            // retain a total constructor rather than exposing a raw version.
            version: AggregateVersion::initial()
                .next()
                .unwrap_or(AggregateVersion::initial()),
        }
    }

    /// Reconstruct the canonical record emitted after `DeclineActionRequest`.
    ///
    /// Decline is the terminal H1 transition from Open/version 2 to
    /// Declined/version 3.  The rationale is a required bounded domain value,
    /// so an adapter cannot represent a terminal record without its reason or
    /// smuggle in a linked Action, Decision origin, or superseded premise.
    #[doc(hidden)]
    pub fn from_persisted_open_to_declined(
        mut open: Self,
        terminal_rationale: ActionDetails,
    ) -> Option<Self> {
        if open.state != ActionRequestState::Open
            || open.version
                != AggregateVersion::initial()
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || open.linked_action_id.is_some()
            || open.source_decision_id.is_some()
            || open.superseded_premise
        {
            return None;
        }

        open.state = ActionRequestState::Declined;
        open.terminal_rationale = Some(terminal_rationale);
        open.version = AggregateVersion::initial()
            .next()
            .and_then(|version| version.next())
            .unwrap_or(AggregateVersion::initial());
        Some(open)
    }

    /// Reconstruct the canonical record emitted after `WithdrawActionRequest`.
    ///
    /// Withdraw is a terminal H1 transition from Open/version 2 to
    /// Withdrawn/version 3.  Keep this constructor as strict as Decline's:
    /// only a verified Open record can cross the seam, and the resulting
    /// record cannot carry an Action, Decision origin, or superseded premise.
    #[doc(hidden)]
    pub fn from_persisted_open_to_withdrawn(
        mut open: Self,
        terminal_rationale: ActionDetails,
    ) -> Option<Self> {
        if open.state != ActionRequestState::Open
            || open.version
                != AggregateVersion::initial()
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || open.linked_action_id.is_some()
            || open.source_decision_id.is_some()
            || open.superseded_premise
        {
            return None;
        }

        open.state = ActionRequestState::Withdrawn;
        open.terminal_rationale = Some(terminal_rationale);
        open.version = AggregateVersion::initial()
            .next()
            .and_then(|version| version.next())
            .unwrap_or(AggregateVersion::initial());
        Some(open)
    }

    /// Reconstruct the canonical H2a result of accepting an Open request.
    ///
    /// This is intentionally constrained to the one accepted lifecycle edge:
    /// Open/version 2 gains exactly one linked Action and advances to
    /// Accepted/version 3. It is not a general persistence state setter.
    #[doc(hidden)]
    pub fn from_persisted_open_to_accepted(
        mut open: Self,
        linked_action_id: ActionId,
    ) -> Option<Self> {
        if open.state != ActionRequestState::Open
            || open.version
                != AggregateVersion::initial()
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || open.linked_action_id.is_some()
            || open.terminal_rationale.is_some()
        {
            return None;
        }
        open.state = ActionRequestState::Accepted;
        open.linked_action_id = Some(linked_action_id);
        open.version = AggregateVersion::initial()
            .next()
            .and_then(|version| version.next())
            .unwrap_or(AggregateVersion::initial());
        Some(open)
    }

    /// Reconstruct the canonical H2a result of accepting a Decision-created
    /// Open request.
    ///
    /// Unlike [`Self::from_persisted_open_to_accepted`], the pre-accept
    /// version is not hardcoded to 2: a Decision-created request starts at
    /// Open/version 1 (never marked) or Open/version 2 (marked superseded
    /// once by a later Decision) before Accept advances it by exactly one
    /// version -- so this takes the expected pre-accept version as an
    /// explicit parameter rather than assuming the ordinary H1
    /// Draft-then-Submit shape. Still constrained to the one accepted
    /// lifecycle edge: only a Decision-created record may cross this seam.
    #[doc(hidden)]
    pub fn from_persisted_open_to_accepted_from_decision(
        mut open: Self,
        expected_open_version: AggregateVersion,
        linked_action_id: ActionId,
    ) -> Option<Self> {
        if open.state != ActionRequestState::Open
            || open.version != expected_open_version
            || open.source_decision_id.is_none()
            || open.linked_action_id.is_some()
            || open.terminal_rationale.is_some()
        {
            return None;
        }
        open.state = ActionRequestState::Accepted;
        open.linked_action_id = Some(linked_action_id);
        open.version = open.version.next()?;
        Some(open)
    }

    /// Reconstruct the canonical initial record created by a Decision
    /// transition (Resolve/Supersede) rather than the ordinary H1
    /// `CreateRequest` -> `SubmitActionRequest` path.
    ///
    /// Always Open/version 1 with no response due date -- mirrors
    /// `InMemoryActionService::create_resulting_action_request_from_decision_transition`'s
    /// in-memory construction exactly. Not a general persistence
    /// constructor: an adapter cannot represent any other shape through it.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted_created_from_decision(
        id: ActionRequestId,
        title: ActionTitle,
        details: ActionDetails,
        intended_owner: StakeholderId,
        intended_action_due_at: UtcTimestamp,
        classification: DataClassification,
        source_decision_id: DecisionId,
    ) -> Self {
        Self {
            id,
            title,
            details,
            intended_owner: Some(intended_owner),
            response_due_at: None,
            intended_action_due_at: Some(intended_action_due_at),
            classification,
            state: ActionRequestState::Open,
            terminal_rationale: None,
            linked_action_id: None,
            source_decision_id: Some(source_decision_id),
            superseded_premise: false,
            version: AggregateVersion::initial(),
        }
    }

    /// Reconstruct the record after a Decision Supersede marks this
    /// still-incomplete request's premise superseded.
    ///
    /// Mirrors
    /// `InMemoryActionService::mark_action_request_superseded_premise_from_decision`'s
    /// in-memory mutation exactly: requires the same originating Decision and
    /// a Draft/Open record at the expected version, combines classification,
    /// and advances the version by exactly one. Marking an already-marked
    /// record is legal (the live domain method has no such guard -- a chain
    /// of Decisions can supersede overlapping downstream work more than
    /// once), so this constructor does not reject `previous.superseded_premise`.
    #[doc(hidden)]
    pub fn from_persisted_superseded_premise_marked(
        mut previous: Self,
        expected_version: AggregateVersion,
        source_decision_id: &DecisionId,
        decided_classification: DataClassification,
    ) -> Option<Self> {
        if previous.version != expected_version
            || previous.source_decision_id.as_ref() != Some(source_decision_id)
            || !matches!(
                previous.state,
                ActionRequestState::Draft | ActionRequestState::Open
            )
        {
            return None;
        }
        previous.classification = previous.classification.combine(decided_classification);
        previous.superseded_premise = true;
        previous.version = previous.version.next()?;
        Some(previous)
    }

    pub fn id(&self) -> &ActionRequestId {
        &self.id
    }
    pub fn title(&self) -> &ActionTitle {
        &self.title
    }
    pub fn details(&self) -> &ActionDetails {
        &self.details
    }
    pub fn intended_owner(&self) -> Option<&StakeholderId> {
        self.intended_owner.as_ref()
    }
    pub const fn response_due_at(&self) -> Option<UtcTimestamp> {
        self.response_due_at
    }
    pub const fn intended_action_due_at(&self) -> Option<UtcTimestamp> {
        self.intended_action_due_at
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn state(&self) -> ActionRequestState {
        self.state
    }
    pub fn terminal_rationale(&self) -> Option<&ActionDetails> {
        self.terminal_rationale.as_ref()
    }
    pub fn linked_action_id(&self) -> Option<&ActionId> {
        self.linked_action_id.as_ref()
    }
    pub fn source_decision_id(&self) -> Option<&DecisionId> {
        self.source_decision_id.as_ref()
    }
    pub const fn has_superseded_premise(&self) -> bool {
        self.superseded_premise
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRecord {
    id: ActionId,
    source_request_id: ActionRequestId,
    title: ActionTitle,
    details: ActionDetails,
    owner: StakeholderId,
    due_at: UtcTimestamp,
    classification: DataClassification,
    state: ActionState,
    completion_evidence: Vec<EvidenceReferenceId>,
    commitment_classification: DataClassification,
    support: Option<SupportWitness>,
    transition_reason: Option<ActionDetails>,
    transition_history: Vec<ActionTransitionRecord>,
    source_decision_id: Option<DecisionId>,
    superseded_premise: bool,
    version: AggregateVersion,
}
impl ActionRecord {
    /// Reconstruct the Action atomically created by an accepted Action Request.
    ///
    /// The caller can supply only the immutable fields of the accepted
    /// contract. Lifecycle mutation, support, completion evidence, and
    /// transition history remain unavailable at this persistence seam.
    #[doc(hidden)]
    pub fn from_persisted_accepted_request(
        request: &ActionRequestRecord,
        prepared: &WorkManagementPreparedIntent,
    ) -> Option<Self> {
        let WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            action_classification,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
        } = prepared.operation()
        else {
            return None;
        };
        if request.state != ActionRequestState::Accepted
            || request.id != *request_id
            || request.version
                != request_version
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || request.linked_action_id.as_ref() != Some(action_id)
            || request.title != *action_subject
            || request.details != *commitment_details
            || request.intended_owner.as_ref() != Some(intended_owner)
            || request.intended_action_due_at != Some(*intended_due_at)
            || request.classification != *action_classification
        {
            return None;
        }
        Some(Self {
            id: action_id.clone(),
            source_request_id: request_id.clone(),
            title: action_subject.clone(),
            details: commitment_details.clone(),
            owner: intended_owner.clone(),
            due_at: *intended_due_at,
            classification: prepared.classification(),
            state: ActionState::Open,
            completion_evidence: Vec::new(),
            commitment_classification: *action_classification,
            support: None,
            transition_reason: None,
            transition_history: Vec::new(),
            source_decision_id: request.source_decision_id.clone(),
            superseded_premise: request.superseded_premise,
            version: AggregateVersion::initial(),
        })
    }
    /// Rehydrate an Open Action that has never transitioned lifecycle state
    /// but has had its classification governed-lowered one or more times
    /// (H2a `LowerActionClassification`), which advances
    /// `version` and `classification` without leaving `ActionState::Open`.
    /// Unlike [`Self::from_persisted_accepted_request`], `classification`
    /// and `version` are taken as explicit parameters rather than derived
    /// from the Accept preview alone -- every other field is still derived
    /// from `request`/`prepared` exactly as the base constructor does, and
    /// every other narrow shape (no accumulated completion evidence/
    /// transition history/support) still applies unchanged.
    pub fn from_persisted_accepted_request_with_lowered_classification(
        request: &ActionRequestRecord,
        prepared: &WorkManagementPreparedIntent,
        classification: DataClassification,
        version: AggregateVersion,
    ) -> Option<Self> {
        let WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            action_classification,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
        } = prepared.operation()
        else {
            return None;
        };
        if request.state != ActionRequestState::Accepted
            || request.id != *request_id
            || request.version
                != request_version
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || request.linked_action_id.as_ref() != Some(action_id)
            || request.title != *action_subject
            || request.details != *commitment_details
            || request.intended_owner.as_ref() != Some(intended_owner)
            || request.intended_action_due_at != Some(*intended_due_at)
            || request.classification != *action_classification
        {
            return None;
        }
        Some(Self {
            id: action_id.clone(),
            source_request_id: request_id.clone(),
            title: action_subject.clone(),
            details: commitment_details.clone(),
            owner: intended_owner.clone(),
            due_at: *intended_due_at,
            classification,
            state: ActionState::Open,
            completion_evidence: Vec::new(),
            commitment_classification: *action_classification,
            support: None,
            transition_reason: None,
            transition_history: Vec::new(),
            source_decision_id: request.source_decision_id.clone(),
            superseded_premise: request.superseded_premise,
            version,
        })
    }

    /// Sibling work to H2a Lower Data Classification: SQLite persistence
    /// for Action Cancel/Complete/Reopen. Mirrors
    /// `from_persisted_accepted_request_with_lowered_classification`'s exact
    /// accept-time cross-checks, generalized to also carry the lifecycle
    /// `state`, `transition_reason`, `transition_history`, `support`, and
    /// `completion_evidence` a transition can leave behind -- Lower
    /// Classification never touches any of those (it never leaves Open, mints
    /// no transition record, and never touches evidence/support), so that
    /// constructor hardcodes them; this one lets the ledger supply the
    /// replayed values instead.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted_accepted_request_with_transition(
        request: &ActionRequestRecord,
        prepared: &WorkManagementPreparedIntent,
        state: ActionState,
        classification: DataClassification,
        version: AggregateVersion,
        transition_reason: Option<ActionDetails>,
        transition_history: Vec<ActionTransitionRecord>,
        support: Option<SupportWitness>,
        completion_evidence: Vec<EvidenceReferenceId>,
    ) -> Option<Self> {
        let WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            action_classification,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
        } = prepared.operation()
        else {
            return None;
        };
        if request.state != ActionRequestState::Accepted
            || request.id != *request_id
            || request.version
                != request_version
                    .next()
                    .unwrap_or(AggregateVersion::initial())
            || request.linked_action_id.as_ref() != Some(action_id)
            || request.title != *action_subject
            || request.details != *commitment_details
            || request.intended_owner.as_ref() != Some(intended_owner)
            || request.intended_action_due_at != Some(*intended_due_at)
            || request.classification != *action_classification
        {
            return None;
        }
        Some(Self {
            id: action_id.clone(),
            source_request_id: request_id.clone(),
            title: action_subject.clone(),
            details: commitment_details.clone(),
            owner: intended_owner.clone(),
            due_at: *intended_due_at,
            classification,
            state,
            completion_evidence,
            commitment_classification: *action_classification,
            support,
            transition_reason,
            transition_history,
            source_decision_id: request.source_decision_id.clone(),
            superseded_premise: request.superseded_premise,
            version,
        })
    }

    /// Reconstruct the record after `StartAction` moves it Open -> InProgress.
    ///
    /// Mirrors `InMemoryActionService::start_action_cause`'s in-memory
    /// mutation exactly: no classification change, one new
    /// `ActionTransitionRecord`, version advances by one.
    #[doc(hidden)]
    pub fn from_persisted_started(
        mut previous: Self,
        expected_version: AggregateVersion,
        occurred_at: UtcTimestamp,
    ) -> Option<Self> {
        if previous.version != expected_version || previous.state != ActionState::Open {
            return None;
        }
        previous
            .transition_history
            .push(ActionTransitionRecord::from_persisted(
                ActionState::Open,
                ActionState::InProgress,
                None,
                occurred_at,
                None,
                None,
            ));
        previous.state = ActionState::InProgress;
        previous.version = previous.version.next()?;
        Some(previous)
    }

    /// Reconstruct the record after `LinkActionCompletionEvidence` links one
    /// more completion-evidence reference.
    ///
    /// Mirrors `InMemoryActionService::link_action_completion_evidence_cause`'s
    /// in-memory mutation exactly: requires InProgress, rejects a duplicate
    /// evidence id, combines classification, version advances by one. No new
    /// `ActionTransitionRecord` -- linking evidence is not itself a state
    /// transition.
    #[doc(hidden)]
    pub fn from_persisted_completion_evidence_linked(
        mut previous: Self,
        expected_version: AggregateVersion,
        evidence_id: EvidenceReferenceId,
        evidence_classification: DataClassification,
    ) -> Option<Self> {
        if previous.version != expected_version
            || previous.state != ActionState::InProgress
            || previous.completion_evidence.contains(&evidence_id)
        {
            return None;
        }
        previous.classification = previous.classification.combine(evidence_classification);
        previous.completion_evidence.push(evidence_id);
        previous.version = previous.version.next()?;
        Some(previous)
    }

    /// Reconstruct the record after `ApproveAndExecuteLowerActionClassification`
    /// lowers its classification.
    ///
    /// Mirrors `InMemoryActionService::approve_and_execute_lower_action_classification_cause`'s
    /// in-memory mutation exactly: no state check beyond the version match
    /// (Lower Classification's own PREPARE already validated the state was
    /// legal), no new `ActionTransitionRecord` (a lowering is not itself a
    /// state transition, same as linking evidence), `classification` is
    /// simply overwritten with `proposed_classification` (not combined --
    /// `is_genuine_lowering` already guarantees monotonic decrease at PREPARE
    /// time), version advances by one. Added because, unlike
    /// `from_persisted_started`/`from_persisted_completion_evidence_linked`,
    /// no persisted-rehydration constructor for this operation existed
    /// before -- the SQLite decoder instead always re-derived a fresh
    /// accept-time record via `from_persisted_accepted_request_with_lowered_classification`,
    /// which cannot apply a lowering on top of any other prior mutation
    /// (Start, Link, Complete, Cancel, Reopen).
    #[doc(hidden)]
    pub fn from_persisted_classification_lowered(
        mut previous: Self,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
    ) -> Option<Self> {
        if previous.version != expected_version {
            return None;
        }
        previous.classification = proposed_classification;
        previous.version = previous.version.next()?;
        Some(previous)
    }

    /// Reconstruct the record after a Decision Supersede marks this
    /// still-incomplete Action's premise superseded.
    ///
    /// Mirrors
    /// `InMemoryActionService::mark_action_superseded_premise_from_decision`'s
    /// in-memory mutation exactly: requires the same originating Decision and
    /// an Open/InProgress record at the expected version, combines both
    /// classification fields, and advances the version by exactly one.
    /// Marking an already-marked record is legal (the live domain method has
    /// no such guard -- e.g. an Action inherits `superseded_premise=true`
    /// from an already-marked source request at Accept time, then can still
    /// be marked again directly), so this constructor does not reject
    /// `previous.superseded_premise`.
    #[doc(hidden)]
    pub fn from_persisted_superseded_premise_marked(
        mut previous: Self,
        expected_version: AggregateVersion,
        source_decision_id: &DecisionId,
        decided_classification: DataClassification,
    ) -> Option<Self> {
        if previous.version != expected_version
            || previous.source_decision_id.as_ref() != Some(source_decision_id)
            || !matches!(previous.state, ActionState::Open | ActionState::InProgress)
        {
            return None;
        }
        previous.classification = previous.classification.combine(decided_classification);
        previous.commitment_classification = previous
            .commitment_classification
            .combine(decided_classification);
        previous.superseded_premise = true;
        previous.version = previous.version.next()?;
        Some(previous)
    }

    pub fn id(&self) -> &ActionId {
        &self.id
    }
    pub fn source_request_id(&self) -> &ActionRequestId {
        &self.source_request_id
    }
    pub fn title(&self) -> &ActionTitle {
        &self.title
    }
    pub fn details(&self) -> &ActionDetails {
        &self.details
    }
    pub fn owner(&self) -> &StakeholderId {
        &self.owner
    }
    pub const fn due_at(&self) -> UtcTimestamp {
        self.due_at
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn commitment_classification(&self) -> DataClassification {
        self.commitment_classification
    }
    pub const fn state(&self) -> ActionState {
        self.state
    }
    pub fn completion_evidence(&self) -> &[EvidenceReferenceId] {
        &self.completion_evidence
    }
    pub const fn support(&self) -> Option<&SupportWitness> {
        self.support.as_ref()
    }
    pub fn transition_reason(&self) -> Option<&ActionDetails> {
        self.transition_reason.as_ref()
    }
    pub fn transition_history(&self) -> &[ActionTransitionRecord] {
        &self.transition_history
    }
    pub fn source_decision_id(&self) -> Option<&DecisionId> {
        self.source_decision_id.as_ref()
    }
    pub const fn has_superseded_premise(&self) -> bool {
        self.superseded_premise
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateActionRequestDraft {
    pub id: ActionRequestId,
    pub title: ActionTitle,
    pub details: ActionDetails,
    pub intended_owner: Option<StakeholderId>,
    pub response_due_at: Option<UtcTimestamp>,
    pub intended_action_due_at: Option<UtcTimestamp>,
    pub classification: DataClassification,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitActionRequest {
    pub request_id: ActionRequestId,
    pub expected_version: AggregateVersion,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclineActionRequest {
    pub request_id: ActionRequestId,
    pub expected_version: AggregateVersion,
    pub rationale: ActionDetails,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawActionRequest {
    pub request_id: ActionRequestId,
    pub expected_version: AggregateVersion,
    pub rationale: ActionDetails,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartAction {
    pub action_id: ActionId,
    pub expected_version: AggregateVersion,
    pub context: ActionOperationContext,
}
/// H2a rejection (v45): the Head of Products refuses a pending Accept/
/// Complete/Cancel/Reopen preview. See
/// [`InMemoryActionService::reject_action_prepared_intent`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectActionPreparedIntent {
    pub prepared_id: PreparedIntentId,
    pub actor: AuditActor,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkActionCompletionEvidence {
    pub action_id: ActionId,
    pub expected_version: AggregateVersion,
    pub evidence_id: EvidenceReferenceId,
    pub context: ActionOperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareAcceptActionRequest {
    pub request_id: ActionRequestId,
    pub expected_version: AggregateVersion,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareCompleteAction {
    pub action_id: ActionId,
    pub expected_version: AggregateVersion,
    pub judgment: Option<HumanJudgment>,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareCancelAction {
    pub action_id: ActionId,
    pub expected_version: AggregateVersion,
    pub reason: ActionDetails,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareReopenAction {
    pub action_id: ActionId,
    pub expected_version: AggregateVersion,
    pub mode: ActionReopenMode,
    pub reason: ActionDetails,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteAcceptActionRequest {
    pub approval: WorkManagementApproval,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteCompleteAction {
    pub approval: WorkManagementApproval,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteCancelAction {
    pub approval: WorkManagementApproval,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteReopenAction {
    pub approval: WorkManagementApproval,
    pub context: ActionOperationContext,
}
/// H2a "Lower Data Classification" for Action, mirroring
/// `PrepareCompleteAction`/`ApproveAndExecuteCompleteAction`'s shape but
/// built as its own independent method pair rather than folding into the
/// shared `execute_h2` dispatcher Complete/Cancel/Reopen already use --
/// deliberately avoiding touching that dispatcher's shared, already-tested
/// state-transition logic for an operation that isn't a lifecycle
/// transition at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerActionClassification {
    pub action_id: ActionId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: ActionOperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerActionClassification {
    pub approval: WorkManagementApproval,
    pub context: ActionOperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionMutationOutcome<T> {
    pub record: T,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: Option<ApprovalReceiptId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedActionOutcome {
    pub request: ActionRequestRecord,
    pub action: ActionRecord,
    pub audit_events: Vec<AuditEvent>,
    pub approval_receipt_id: ApprovalReceiptId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionServiceError {
    NotFound,
    AlreadyExists,
    IllegalTransition,
    StaleVersion,
    MissingOwner,
    MissingDueDate,
    InvalidEvidence,
    EvidenceUnavailable,
    EvidencePolicyDenied,
    ClassificationUnresolved,
    InvalidReopenMode,
    IdempotencyConflict,
    PreparedIntentNotFound,
    PreparedIntentChanged,
    Unauthorized,
    DigestMismatch,
    Expired,
    PolicyDenied,
    InfrastructureFailure,
    NotALowering,
}

pub trait ActionServiceIdSource {
    fn next_action_id(&mut self) -> Result<ActionId, crate::DomainValueError>;
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, crate::DomainValueError>;
    fn next_audit_event_id(
        &mut self,
    ) -> Result<crate::identity::AuditEventId, crate::DomainValueError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionExecutionPolicy {
    Allowed,
    Denied,
}

pub trait ActionExecutionPolicyPort {
    fn current_policy(&self, operation: &WorkManagementOperation) -> ActionExecutionPolicy;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionEvidenceAuthorityError {
    Unavailable,
    NotFound,
}

pub trait ActionEvidenceAuthorityPort {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DenyActionEvidenceAuthority;
impl ActionEvidenceAuthorityPort for DenyActionEvidenceAuthority {
    fn resolve(
        &self,
        _: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        Err(ActionEvidenceAuthorityError::Unavailable)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredResult {
    Request(ActionMutationOutcome<ActionRequestRecord>),
    Action(ActionMutationOutcome<ActionRecord>),
    Accepted(AcceptedActionOutcome),
    Prepared(WorkManagementPreparedIntent),
    Terminal(ActionTerminalFailure),
    Rejected(RejectedPreparedIntentOutcome),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActionTerminalFailure {
    cause: ActionServiceError,
    persisted_cause: ActionPersistenceTerminalCause,
    error: DomainError,
    prepared_disposition: PreparedDisposition,
    audit: AuditEvent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommandIdentity {
    CreateRequestFromDecision {
        request: DecisionResultingActionRequest,
        source_decision_id: DecisionId,
        classification: DataClassification,
    },
    MarkRequestSupersededPremise {
        request_id: ActionRequestId,
        expected_version: AggregateVersion,
        source_decision_id: DecisionId,
        classification: DataClassification,
    },
    MarkActionSupersededPremise {
        action_id: ActionId,
        expected_version: AggregateVersion,
        source_decision_id: DecisionId,
        classification: DataClassification,
    },
    CreateRequest {
        id: ActionRequestId,
        title: ActionTitle,
        details: ActionDetails,
        intended_owner: Option<StakeholderId>,
        response_due_at: Option<UtcTimestamp>,
        intended_action_due_at: Option<UtcTimestamp>,
        classification: DataClassification,
    },
    TransitionRequest {
        request_id: ActionRequestId,
        expected_version: AggregateVersion,
        target_state: ActionRequestState,
        rationale: Option<ActionDetails>,
    },
    StartAction {
        action_id: ActionId,
        expected_version: AggregateVersion,
    },
    LinkCompletionEvidence {
        action_id: ActionId,
        expected_version: AggregateVersion,
        evidence_id: EvidenceReferenceId,
        evidence_classification: Option<DataClassification>,
    },
    ExecuteAccept(ApprovalBusinessIdentity),
    ExecuteAction(PreparedActionTransitionKind, ApprovalBusinessIdentity),
    RejectPrepared {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
    },
    PrepareAccept {
        request_id: ActionRequestId,
        expected_version: AggregateVersion,
    },
    PrepareComplete {
        action_id: ActionId,
        expected_version: AggregateVersion,
        judgment: Option<HumanJudgment>,
    },
    PrepareCancel {
        action_id: ActionId,
        expected_version: AggregateVersion,
        reason: ActionDetails,
    },
    PrepareReopen {
        action_id: ActionId,
        expected_version: AggregateVersion,
        mode: ActionReopenMode,
        reason: ActionDetails,
    },
    PrepareLowerActionClassification {
        action_id: ActionId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ExecuteLowerActionClassification(ApprovalBusinessIdentity),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApprovalBusinessIdentity {
    prepared_id: PreparedIntentId,
    actor: AuditActor,
    digest: WorkManagementPayloadDigest,
}
fn approval_business_identity(approval: &WorkManagementApproval) -> ApprovalBusinessIdentity {
    ApprovalBusinessIdentity {
        prepared_id: approval.prepared_id().clone(),
        actor: approval.actor(),
        digest: approval.acknowledged_payload_digest().clone(),
    }
}

#[derive(Clone)]
enum ErrorTarget {
    Request(ActionRequestId),
    Action(ActionId),
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Stored {
    signature: CommandIdentity,
    result: StoredResult,
    original_correlation_id: CorrelationId,
    operation_ordinal: u64,
}
#[derive(Clone, Default)]
struct Store {
    requests: HashMap<ActionRequestId, ActionRequestRecord>,
    actions: HashMap<ActionId, ActionRecord>,
    prepared: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    discarded_prepared: Vec<WorkManagementPreparedIntent>,
    idem: HashMap<IdempotencyId, Stored>,
    audit_events: Vec<AuditEvent>,
    next_operation_ordinal: u64,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionPersistenceCommand {
    CreateRequest {
        id: ActionRequestId,
        title: ActionTitle,
        details: ActionDetails,
        intended_owner: Option<StakeholderId>,
        response_due_at: Option<UtcTimestamp>,
        intended_action_due_at: Option<UtcTimestamp>,
        classification: DataClassification,
    },
    /// A Decision transition (Resolve/Supersede) creating a resulting Action
    /// Request. Mirrors `CommandIdentity::CreateRequestFromDecision` field
    /// for field -- see
    /// [`InMemoryActionService::create_resulting_action_request_from_decision_transition`].
    CreateRequestFromDecision {
        request: crate::work_management::DecisionResultingActionRequest,
        source_decision_id: DecisionId,
        classification: DataClassification,
    },
    /// A Decision Supersede marking an existing, still-incomplete Action
    /// Request's premise superseded. Mirrors
    /// `CommandIdentity::MarkRequestSupersededPremise` field for field -- see
    /// [`InMemoryActionService::mark_action_request_superseded_premise_from_decision`].
    MarkRequestSupersededPremise {
        request_id: ActionRequestId,
        expected_version: AggregateVersion,
        source_decision_id: DecisionId,
        classification: DataClassification,
    },
    /// A Decision Supersede marking an existing, still-incomplete Action's
    /// premise superseded. Mirrors `CommandIdentity::MarkActionSupersededPremise`
    /// field for field -- see
    /// [`InMemoryActionService::mark_action_superseded_premise_from_decision`].
    MarkActionSupersededPremise {
        action_id: ActionId,
        expected_version: AggregateVersion,
        source_decision_id: DecisionId,
        classification: DataClassification,
    },
    TransitionRequest {
        request_id: ActionRequestId,
        expected_version: AggregateVersion,
        target_state: ActionRequestState,
        rationale: Option<ActionDetails>,
    },
    PrepareAccept {
        request_id: ActionRequestId,
        expected_version: AggregateVersion,
    },
    ExecuteAccept {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_digest: WorkManagementPayloadDigest,
    },
    PrepareComplete {
        action_id: ActionId,
        expected_version: AggregateVersion,
        judgment: Option<HumanJudgment>,
    },
    PrepareCancel {
        action_id: ActionId,
        expected_version: AggregateVersion,
        reason: ActionDetails,
    },
    PrepareReopen {
        action_id: ActionId,
        expected_version: AggregateVersion,
        mode: ActionReopenMode,
        reason: ActionDetails,
    },
    ExecuteAction {
        kind: ActionPersistenceTransitionKind,
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_digest: WorkManagementPayloadDigest,
    },
    StartAction {
        action_id: ActionId,
        expected_version: AggregateVersion,
    },
    LinkCompletionEvidence {
        action_id: ActionId,
        expected_version: AggregateVersion,
        evidence_id: EvidenceReferenceId,
        evidence_classification: DataClassification,
    },
    /// H2a "Lower Data Classification" for Action.
    PrepareLowerClassification {
        action_id: ActionId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: crate::work_management::WorkManagementRationale,
    },
    ExecuteLowerClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_digest: WorkManagementPayloadDigest,
    },
    /// v45: the Head of Products' explicit refusal of a pending Accept/
    /// Complete/Cancel/Reopen preview. Consumes the intent; no effects.
    RejectPrepared {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
    },
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionPersistenceTransitionKind {
    Complete,
    Cancel,
    Reopen,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionPersistenceResult {
    Request(ActionMutationOutcome<ActionRequestRecord>),
    Action(ActionMutationOutcome<ActionRecord>),
    Prepared(WorkManagementPreparedIntent),
    Accepted(AcceptedActionOutcome),
    Rejected(RejectedPreparedIntentOutcome),
    Terminal {
        command: ActionPersistenceCommand,
        cause: ActionPersistenceTerminalCause,
        error: DomainError,
        prepared_disposition: PreparedDisposition,
        audit: AuditEvent,
    },
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedDisposition {
    NotApplicable,
    Retained,
    ConsumedAndDiscarded,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionPersistenceTerminalCause {
    NotFound,
    AlreadyExists,
    IllegalTransition,
    StaleVersion,
    MissingOwner,
    MissingDueDate,
    InvalidEvidence,
    InvalidReopenMode,
    IdempotencyConflict,
    PreparedIntentNotFound,
    PreparedIntentChanged,
    Unauthorized,
    DigestMismatch {
        attempted_digest: WorkManagementPayloadDigest,
    },
    Expired,
    PolicyDenied,
    InfrastructureFailure,
    H3Denied(ActionPersistenceH3DenialCause),
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionPersistenceH3DenialCause {
    MissingCompletionEvidence,
    CompletionEvidenceNotFound,
    UnverifiedCompletionEvidence,
    UnclassifiedCompletionEvidence,
    EvidenceUnavailable,
    ClassificationUnresolved,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionReplayCapsule {
    idempotency_id: IdempotencyId,
    original_correlation_id: CorrelationId,
    operation_ordinal: u64,
    command: ActionPersistenceCommand,
    result: ActionPersistenceResult,
    audit_event_ids: Vec<crate::identity::AuditEventId>,
}

impl ActionReplayCapsule {
    /// Construct a replay capsule from already validated domain-typed values.
    ///
    /// Persistence adapters must decode identifiers, timestamps, enums,
    /// commands, results, errors, and audit membership into these types before
    /// calling this constructor.  No raw row, text, SQL, or generic payload is
    /// accepted at this boundary; the owning snapshot validator performs the
    /// cross-record and terminal-topology checks.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        idempotency_id: IdempotencyId,
        original_correlation_id: CorrelationId,
        operation_ordinal: u64,
        command: ActionPersistenceCommand,
        result: ActionPersistenceResult,
        audit_event_ids: Vec<crate::identity::AuditEventId>,
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

    pub const fn command(&self) -> &ActionPersistenceCommand {
        &self.command
    }

    pub const fn result(&self) -> &ActionPersistenceResult {
        &self.result
    }

    pub fn audit_event_ids(&self) -> &[crate::identity::AuditEventId] {
        &self.audit_event_ids
    }
}

/// Opaque, Action-only input boundary for a persistence adapter.
///
/// The input owns immutable, domain-typed values.  `decode` delegates to the
/// canonical [`ActionPersistenceSnapshot::try_new`] validator, so malformed
/// identifiers/text/timestamps/enums and inconsistent command/result,
/// terminal/error, prepared/discarded, or audit topology fail closed before
/// an authoritative snapshot can be returned.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionPersistenceDecodeInput {
    requests: Vec<ActionRequestRecord>,
    actions: Vec<ActionRecord>,
    prepared: Vec<WorkManagementPreparedIntent>,
    discarded_prepared: Vec<WorkManagementPreparedIntent>,
    replay: Vec<ActionReplayCapsule>,
    audits: Vec<AuditEvent>,
}

impl ActionPersistenceDecodeInput {
    /// Create an adapter input from typed Action-domain components only.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        requests: Vec<ActionRequestRecord>,
        actions: Vec<ActionRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
        discarded_prepared: Vec<WorkManagementPreparedIntent>,
        replay: Vec<ActionReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Self {
        Self {
            requests,
            actions,
            prepared,
            discarded_prepared,
            replay,
            audits,
        }
    }

    /// Validate and consume the input into the authoritative Action snapshot.
    pub fn decode(self) -> Result<ActionPersistenceSnapshot, ActionRehydrationError> {
        ActionPersistenceSnapshot::try_new(
            self.requests,
            self.actions,
            self.prepared,
            self.discarded_prepared,
            self.replay,
            self.audits,
        )
    }

    /// Alias making the ownership transfer explicit at adapter call sites.
    pub fn try_into_snapshot(self) -> Result<ActionPersistenceSnapshot, ActionRehydrationError> {
        self.decode()
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionRehydrationError {
    UnsupportedState,
    DuplicateRecord,
    DuplicatePreparedIntent,
    DuplicateIdempotency,
    DuplicateAuditEvent,
    OperationOrderMismatch,
    CommandResultMismatch,
    AuditMismatch,
    OrphanRecord,
    OrphanAuditEvent,
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ActionPersistenceSnapshot {
    requests: Vec<ActionRequestRecord>,
    actions: Vec<ActionRecord>,
    prepared: Vec<WorkManagementPreparedIntent>,
    discarded_prepared: Vec<WorkManagementPreparedIntent>,
    replay: Vec<ActionReplayCapsule>,
    audits: Vec<AuditEvent>,
}

impl ActionPersistenceSnapshot {
    /// Construct and validate a complete, lossless persistence boundary.
    pub fn try_new(
        requests: Vec<ActionRequestRecord>,
        actions: Vec<ActionRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
        discarded_prepared: Vec<WorkManagementPreparedIntent>,
        replay: Vec<ActionReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, ActionRehydrationError> {
        Self::validate_with_discarded(
            requests,
            actions,
            prepared,
            discarded_prepared,
            replay,
            audits,
        )
    }

    pub fn requests(&self) -> &[ActionRequestRecord] {
        &self.requests
    }

    pub fn actions(&self) -> &[ActionRecord] {
        &self.actions
    }

    pub fn prepared(&self) -> &[WorkManagementPreparedIntent] {
        &self.prepared
    }

    pub fn discarded_prepared(&self) -> &[WorkManagementPreparedIntent] {
        &self.discarded_prepared
    }

    pub fn replay(&self) -> &[ActionReplayCapsule] {
        &self.replay
    }

    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }

    fn validate_with_discarded(
        requests: Vec<ActionRequestRecord>,
        actions: Vec<ActionRecord>,
        prepared: Vec<WorkManagementPreparedIntent>,
        discarded_prepared: Vec<WorkManagementPreparedIntent>,
        replay: Vec<ActionReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, ActionRehydrationError> {
        let mut request_map = HashMap::new();
        for record in &requests {
            if request_map.insert(record.id(), record).is_some() {
                return Err(ActionRehydrationError::DuplicateRecord);
            }
        }
        let mut action_map = HashMap::new();
        for record in &actions {
            if action_map.insert(record.id(), record).is_some() {
                return Err(ActionRehydrationError::DuplicateRecord);
            }
        }
        let mut audit_map = HashMap::new();
        for audit in &audits {
            if audit_map.insert(audit.id(), audit).is_some() {
                return Err(ActionRehydrationError::DuplicateAuditEvent);
            }
        }
        let mut prepared_map = HashMap::new();
        for intent in &prepared {
            if prepared_map.insert(intent.id(), intent).is_some() {
                return Err(ActionRehydrationError::DuplicatePreparedIntent);
            }
        }
        let mut idempotency_ids = HashSet::new();
        if replay
            .iter()
            .any(|capsule| !idempotency_ids.insert(capsule.idempotency_id.clone()))
        {
            return Err(ActionRehydrationError::DuplicateIdempotency);
        }

        let mut ordered: Vec<_> = replay.iter().collect();
        ordered.sort_by_key(|capsule| capsule.operation_ordinal);
        if ordered.iter().enumerate().any(|(index, capsule)| {
            capsule.operation_ordinal != u64::try_from(index).unwrap_or(u64::MAX)
        }) {
            return Err(ActionRehydrationError::OperationOrderMismatch);
        }

        let mut timeline: HashMap<ActionRequestId, ActionRequestRecord> = HashMap::new();
        let mut action_timeline: HashMap<ActionId, ActionRecord> = HashMap::new();
        let mut prepared_history: HashMap<PreparedIntentId, WorkManagementPreparedIntent> =
            HashMap::new();
        let mut discarded_map = HashMap::new();
        for intent in &discarded_prepared {
            if prepared_map.contains_key(intent.id())
                || discarded_map.insert(intent.id().clone(), intent).is_some()
            {
                return Err(ActionRehydrationError::DuplicatePreparedIntent);
            }
        }
        let mut consumed_prepared = HashSet::new();
        let mut receipt_ids = HashSet::new();
        let mut claimed_audits = Vec::new();
        for capsule in ordered {
            if let ActionPersistenceResult::Terminal {
                command,
                cause,
                error,
                prepared_disposition,
                audit,
            } = &capsule.result
            {
                validate_typed_terminal_capsule(
                    capsule,
                    command,
                    cause,
                    error,
                    *prepared_disposition,
                    audit,
                    &prepared_history,
                    &prepared_map,
                    &discarded_map,
                    &timeline,
                    &action_timeline,
                    &audit_map,
                    &mut consumed_prepared,
                    &mut claimed_audits,
                )?;
                continue;
            }
            if let (
                ActionPersistenceCommand::RejectPrepared { prepared_id, actor },
                ActionPersistenceResult::Rejected(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(intent) = prepared_history.get(prepared_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(target) = rejection_audit_target(intent) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let audit = outcome.audit_event();
                if *actor != AuditActor::HeadOfProducts
                    || outcome.prepared_intent_id() != prepared_id
                    || outcome.rejected_at() != audit.occurred_at()
                    || outcome.expired_at_rejection()
                        != (outcome.rejected_at() >= intent.preview().expires_at())
                    || !discarded_map.contains_key(prepared_id)
                    || !consumed_prepared.insert(prepared_id.clone())
                    || capsule.audit_event_ids.len() != 1
                    || capsule.audit_event_ids[0] != *audit.id()
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let expected = crate::work_management::prepared_intent_rejection_audit(
                    audit.id().clone(),
                    audit.occurred_at(),
                    crate::work_management::ACTION_PREPARED_REJECTED_AUDIT_CODE,
                    target,
                    capsule.original_correlation_id.clone(),
                );
                if audit_map.get(audit.id()).copied() != Some(audit)
                    || expected.as_ref() != Some(audit)
                {
                    return Err(ActionRehydrationError::AuditMismatch);
                }
                claimed_audits.push(audit.id().clone());
                continue;
            }
            if let (
                ActionPersistenceCommand::StartAction {
                    action_id,
                    expected_version,
                },
                ActionPersistenceResult::Action(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(previous) = action_timeline.get(action_id).cloned() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(start_audit) = outcome.audit_events.first() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(authoritative_start_audit) = audit_map.get(start_audit.id()).copied()
                else {
                    return Err(ActionRehydrationError::AuditMismatch);
                };
                let Some(transition) = outcome.record.transition_history.last() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let mut expected = previous.clone();
                expected.state = ActionState::InProgress;
                expected.version = expected
                    .version
                    .next()
                    .ok_or(ActionRehydrationError::CommandResultMismatch)?;
                expected.transition_history.push(ActionTransitionRecord {
                    from: ActionState::Open,
                    to: ActionState::InProgress,
                    reason: None,
                    occurred_at: authoritative_start_audit.occurred_at(),
                    support: None,
                    approval_receipt_id: None,
                });
                if previous.version != *expected_version
                    || previous.state != ActionState::Open
                    || transition.from != ActionState::Open
                    || transition.to != ActionState::InProgress
                    || transition.reason.is_some()
                    || transition.support.is_some()
                    || transition.approval_receipt_id.is_some()
                    || outcome.record != expected
                    || outcome.approval_receipt_id.is_some()
                    || outcome.audit_events.len() != 1
                    || capsule.audit_event_ids.len() != 1
                    || capsule.audit_event_ids[0] != *outcome.audit_events[0].id()
                    || outcome.audit_events[0].correlation_id() != &capsule.original_correlation_id
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let audit = &outcome.audit_events[0];
                if audit_map.get(audit.id()).copied() != Some(audit)
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != "action.started"
                    || audit.target() != &AuditTarget::Action(action_id.clone())
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::NotRequired
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != "action.started"
                {
                    return Err(ActionRehydrationError::AuditMismatch);
                }
                claimed_audits.push(audit.id().clone());
                action_timeline.insert(action_id.clone(), expected);
                continue;
            }
            if let (
                ActionPersistenceCommand::LinkCompletionEvidence {
                    action_id,
                    expected_version,
                    evidence_id,
                    evidence_classification,
                },
                ActionPersistenceResult::Action(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(previous) = action_timeline.get(action_id).cloned() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                if previous
                    .completion_evidence
                    .iter()
                    .any(|id| id == evidence_id)
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let mut expected = previous.clone();
                expected.classification = previous.classification.combine(*evidence_classification);
                expected.completion_evidence.push(evidence_id.clone());
                expected.version = expected
                    .version
                    .next()
                    .ok_or(ActionRehydrationError::CommandResultMismatch)?;
                if previous.version != *expected_version
                    || previous.state != ActionState::InProgress
                    || outcome.record != expected
                    || outcome.approval_receipt_id.is_some()
                    || outcome.audit_events.len() != 1
                    || capsule.audit_event_ids.len() != 1
                    || capsule.audit_event_ids[0] != *outcome.audit_events[0].id()
                    || outcome.audit_events[0].correlation_id() != &capsule.original_correlation_id
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let audit = &outcome.audit_events[0];
                if audit_map.get(audit.id()).copied() != Some(audit)
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != "action.completion_evidence_linked"
                    || audit.target() != &AuditTarget::Action(action_id.clone())
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::NotRequired
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != "action.completion_evidence_linked"
                {
                    return Err(ActionRehydrationError::AuditMismatch);
                }
                claimed_audits.push(audit.id().clone());
                action_timeline.insert(action_id.clone(), expected);
                continue;
            }
            if let (
                ActionPersistenceCommand::PrepareAccept {
                    request_id,
                    expected_version,
                },
                ActionPersistenceResult::Prepared(intent),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(request) = timeline.get(request_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let operation_matches = matches!(
                    intent.operation(),
                    WorkManagementOperation::AcceptActionRequest {
                        request_id: persisted_request_id,
                        request_version,
                        action_classification,
                        action_subject,
                        commitment_details,
                        intended_owner,
                        intended_due_at,
                        ..
                    } if persisted_request_id == request_id
                        && request_version == expected_version
                        && *action_classification == request.classification
                        && action_subject == &request.title
                        && commitment_details == &request.details
                        && request.intended_owner.as_ref() == Some(intended_owner)
                        && request.intended_action_due_at == Some(*intended_due_at)
                );
                if request.version != *expected_version
                    || request.state != ActionRequestState::Open
                    || request.intended_owner.is_none()
                    || request.intended_action_due_at.is_none()
                    || intent.classification() != request.classification
                    || intent.preview().payload_digest() != *intent.payload_digest()
                    || intent.preview().support().is_some()
                    || !operation_matches
                    || !capsule.audit_event_ids.is_empty()
                    || prepared_history
                        .insert(intent.id().clone(), intent.clone())
                        .is_some()
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                continue;
            }
            if let (
                ActionPersistenceCommand::PrepareComplete {
                    action_id,
                    expected_version,
                    judgment,
                },
                ActionPersistenceResult::Prepared(intent),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(action) = action_timeline.get(action_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let operation_matches = matches!(
                    intent.operation(),
                    WorkManagementOperation::CompleteAction {
                        action_id: persisted_action_id,
                        action_version,
                    } if persisted_action_id == action_id && action_version == expected_version
                );
                let support_judgments = intent
                    .preview()
                    .support()
                    .map_or_else(Vec::new, |support| support.judgments().to_vec());
                let requested_judgments = judgment.clone().into_iter().collect::<Vec<_>>();
                let support_is_canonical = intent.preview().support().is_some_and(|support| {
                    let mut support_ids: Vec<_> = support
                        .evidence()
                        .iter()
                        .map(|evidence| evidence.id().clone())
                        .collect();
                    let mut action_ids = action.completion_evidence.clone();
                    support_ids.sort();
                    action_ids.sort();
                    support_ids == action_ids
                        && support.evidence().windows(2).all(|pair| {
                            pair[0].id() < pair[1].id()
                        })
                        && support.evidence().iter().all(|evidence| {
                            evidence.role()
                                == crate::work_management::EvidenceRole::ActionCompletion
                                && !matches!(
                                    evidence.verification(),
                                    crate::work_management::EvidenceVerification::Unverified
                                        | crate::work_management::EvidenceVerification::IntegrityMismatch
                                )
                        })
                        && support
                            .evidence()
                            .iter()
                            .map(EvidenceReferenceMetadata::classification)
                            .chain(
                                support
                                    .judgments()
                                    .iter()
                                    .map(HumanJudgment::classification),
                            )
                            .reduce(DataClassification::combine)
                            == Some(support.classification())
                });
                let Some(support) = intent.preview().support() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let expected_current_classification = support
                    .evidence()
                    .iter()
                    .map(EvidenceReferenceMetadata::classification)
                    .fold(
                        if action.support.is_some() {
                            action.classification
                        } else {
                            action.commitment_classification
                        },
                        DataClassification::combine,
                    );
                let expected_intent_classification =
                    expected_current_classification.combine(support.classification());
                if action.version != *expected_version
                    || action.state != ActionState::InProgress
                    || action.completion_evidence.is_empty()
                    || !operation_matches
                    || intent.classification() == DataClassification::Unclassified
                    || intent.classification() != expected_intent_classification
                    || action
                        .classification
                        .combine(expected_current_classification)
                        != expected_current_classification
                    || *intent.payload_digest() != intent.preview().payload_digest()
                    || !support_is_canonical
                    || support_judgments != requested_judgments
                    || !capsule.audit_event_ids.is_empty()
                    || prepared_history
                        .insert(intent.id().clone(), intent.clone())
                        .is_some()
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                continue;
            }
            if let (
                ActionPersistenceCommand::PrepareCancel {
                    action_id,
                    expected_version,
                    reason,
                },
                ActionPersistenceResult::Prepared(intent),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(action) = action_timeline.get(action_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let operation_matches = matches!(
                    intent.operation(),
                    WorkManagementOperation::CancelAction {
                        action_id: persisted_action_id,
                        action_version,
                        reason: persisted_reason,
                        evidence_classifications,
                    } if persisted_action_id == action_id
                        && action_version == expected_version
                        && persisted_reason.as_str() == reason.as_str()
                        && evidence_classifications.windows(2).all(|pair| pair[0].evidence_id() < pair[1].evidence_id())
                );
                let has_support = intent.preview().support().is_some();
                let classification = match intent.operation() {
                    WorkManagementOperation::CancelAction {
                        evidence_classifications,
                        ..
                    } => evidence_classifications.iter().fold(
                        if action.support.is_some() {
                            action.classification
                        } else {
                            action.commitment_classification
                        },
                        |value, binding| value.combine(binding.classification()),
                    ),
                    _ => DataClassification::Unclassified,
                };
                let bindings_are_canonical = match intent.operation() {
                    WorkManagementOperation::CancelAction {
                        evidence_classifications,
                        ..
                    } => {
                        let mut binding_ids: Vec<_> = evidence_classifications
                            .iter()
                            .map(|binding| binding.evidence_id())
                            .cloned()
                            .collect();
                        let mut action_ids = action.completion_evidence.clone();
                        binding_ids.sort();
                        action_ids.sort();
                        binding_ids == action_ids
                            && evidence_classifications.iter().all(|binding| {
                                binding.classification() != DataClassification::Unclassified
                            })
                    }
                    _ => false,
                };
                if !matches!(action.state, ActionState::Open | ActionState::InProgress)
                    || action.version != *expected_version
                    || !operation_matches
                    || !bindings_are_canonical
                    || has_support
                    || intent.classification() != classification
                    || (action.classification != DataClassification::Unclassified
                        && action.classification.combine(classification) != classification)
                    || *intent.payload_digest() != intent.preview().payload_digest()
                    || !capsule.audit_event_ids.is_empty()
                    || prepared_history
                        .insert(intent.id().clone(), intent.clone())
                        .is_some()
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                continue;
            }
            if let (
                ActionPersistenceCommand::PrepareReopen {
                    action_id,
                    expected_version,
                    mode,
                    reason,
                },
                ActionPersistenceResult::Prepared(intent),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(action) = action_timeline.get(action_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let operation_matches = matches!(
                    intent.operation(),
                    WorkManagementOperation::ReopenAction {
                        action_id: persisted_action_id,
                        action_version,
                        mode: persisted_mode,
                        reason: persisted_reason,
                        evidence_classifications,
                    } if persisted_action_id == action_id
                        && action_version == expected_version
                        && persisted_mode == mode
                        && persisted_reason.as_str() == reason.as_str()
                        && evidence_classifications.windows(2).all(|pair| pair[0].evidence_id() < pair[1].evidence_id())
                );
                let classification = match intent.operation() {
                    WorkManagementOperation::ReopenAction {
                        evidence_classifications,
                        ..
                    } => evidence_classifications.iter().fold(
                        if action.support.is_some() {
                            action.classification
                        } else {
                            action.commitment_classification
                        },
                        |value, binding| value.combine(binding.classification()),
                    ),
                    _ => DataClassification::Unclassified,
                };
                let bindings_are_canonical = match intent.operation() {
                    WorkManagementOperation::ReopenAction {
                        evidence_classifications,
                        ..
                    } => {
                        let mut binding_ids: Vec<_> = evidence_classifications
                            .iter()
                            .map(|binding| binding.evidence_id())
                            .cloned()
                            .collect();
                        let mut action_ids = action.completion_evidence.clone();
                        binding_ids.sort();
                        action_ids.sort();
                        binding_ids == action_ids
                            && evidence_classifications.iter().all(|binding| {
                                binding.classification() != DataClassification::Unclassified
                            })
                    }
                    _ => false,
                };
                let legal_mode = matches!(
                    (action.state, mode),
                    (ActionState::Completed, ActionReopenMode::ReopenCompleted)
                        | (ActionState::Cancelled, ActionReopenMode::RestartCancelled)
                );
                if !legal_mode
                    || action.version != *expected_version
                    || !operation_matches
                    || !bindings_are_canonical
                    || intent.preview().support().is_some()
                    || intent.classification() != classification
                    || (action.classification != DataClassification::Unclassified
                        && action.classification.combine(classification) != classification)
                    || *intent.payload_digest() != intent.preview().payload_digest()
                    || !capsule.audit_event_ids.is_empty()
                    || prepared_history
                        .insert(intent.id().clone(), intent.clone())
                        .is_some()
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                continue;
            }
            if let (
                ActionPersistenceCommand::ExecuteAccept {
                    prepared_id,
                    actor,
                    acknowledged_digest,
                },
                ActionPersistenceResult::Accepted(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(intent) = prepared_history.get(prepared_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let WorkManagementOperation::AcceptActionRequest {
                    request_id,
                    request_version,
                    action_id,
                    action_classification,
                    action_subject,
                    commitment_details,
                    intended_owner,
                    intended_due_at,
                } = intent.operation()
                else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(previous_request) = timeline.get(request_id).cloned() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                if *actor != AuditActor::HeadOfProducts
                    || acknowledged_digest != intent.payload_digest()
                    || previous_request.version != *request_version
                    || previous_request.state != ActionRequestState::Open
                    || previous_request.title != *action_subject
                    || previous_request.details != *commitment_details
                    || previous_request.intended_owner.as_ref() != Some(intended_owner)
                    || previous_request.intended_action_due_at != Some(*intended_due_at)
                    || previous_request.classification != *action_classification
                    || action_timeline.contains_key(action_id)
                    || !consumed_prepared.insert(prepared_id.clone())
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let mut expected_request = previous_request.clone();
                expected_request.state = ActionRequestState::Accepted;
                expected_request.linked_action_id = Some(action_id.clone());
                expected_request.version = expected_request
                    .version
                    .next()
                    .ok_or(ActionRehydrationError::CommandResultMismatch)?;
                let expected_action = ActionRecord {
                    id: action_id.clone(),
                    source_request_id: request_id.clone(),
                    title: action_subject.clone(),
                    details: commitment_details.clone(),
                    owner: intended_owner.clone(),
                    due_at: *intended_due_at,
                    classification: intent.classification(),
                    commitment_classification: *action_classification,
                    state: ActionState::Open,
                    completion_evidence: Vec::new(),
                    support: None,
                    transition_reason: None,
                    transition_history: Vec::new(),
                    source_decision_id: previous_request.source_decision_id.clone(),
                    superseded_premise: previous_request.superseded_premise,
                    version: AggregateVersion::initial(),
                };
                if outcome.request != expected_request
                    || outcome.action != expected_action
                    || outcome.audit_events.len() != 3
                    || capsule.audit_event_ids
                        != outcome
                            .audit_events
                            .iter()
                            .map(|audit| audit.id().clone())
                            .collect::<Vec<_>>()
                    || outcome
                        .audit_events
                        .iter()
                        .any(|audit| audit.correlation_id() != &capsule.original_correlation_id)
                    || outcome
                        .audit_events
                        .iter()
                        .any(|audit| audit.occurred_at() >= intent.preview().expires_at())
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                if !receipt_ids.insert(outcome.approval_receipt_id.clone()) {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let expected_audits = [
                    (
                        "action_request.accepted",
                        AuditTarget::ActionRequest(request_id.clone()),
                    ),
                    (
                        "action.created_from_request",
                        AuditTarget::Action(action_id.clone()),
                    ),
                    (
                        "action_request.action_linked",
                        AuditTarget::ActionRequest(request_id.clone()),
                    ),
                ];
                for (audit, (code, target)) in
                    outcome.audit_events.iter().zip(expected_audits.iter())
                {
                    if audit_map.get(audit.id()).copied() != Some(audit)
                        || audit.actor() != AuditActor::HeadOfProducts
                        || audit.module() != AuditModule::WorkManagement
                        || audit.code().as_str() != *code
                        || audit.target() != target
                        || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                        || audit.approval_outcome() != AuditApprovalOutcome::Approved
                        || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                        || audit.effect_scope() != AuditEffectScope::Complete
                        || audit.actual_effects().len() != 1
                        || audit.actual_effects()[0].as_str() != *code
                    {
                        return Err(ActionRehydrationError::AuditMismatch);
                    }
                    claimed_audits.push(audit.id().clone());
                }
                timeline.insert(request_id.clone(), expected_request);
                action_timeline.insert(action_id.clone(), expected_action);
                continue;
            }
            if let (
                ActionPersistenceCommand::PrepareLowerClassification {
                    action_id,
                    expected_version,
                    proposed_classification,
                    rationale,
                },
                ActionPersistenceResult::Prepared(intent),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(action) = action_timeline.get(action_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let operation_matches = matches!(
                    intent.operation(),
                    WorkManagementOperation::LowerActionClassification {
                        action_id: persisted_action_id,
                        action_version,
                        current_classification,
                        proposed_classification: persisted_proposed,
                        rationale: persisted_rationale,
                    } if persisted_action_id == action_id
                        && action_version == expected_version
                        && *current_classification == action.classification
                        && persisted_proposed == proposed_classification
                        && persisted_rationale.as_str() == rationale.as_str()
                );
                if action.version != *expected_version
                    || !operation_matches
                    || intent.preview().support().is_some()
                    || intent.classification() != action.classification
                    || *intent.payload_digest() != intent.preview().payload_digest()
                    || !capsule.audit_event_ids.is_empty()
                    || prepared_history
                        .insert(intent.id().clone(), intent.clone())
                        .is_some()
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                continue;
            }
            if let (
                ActionPersistenceCommand::ExecuteLowerClassification {
                    prepared_id,
                    actor,
                    acknowledged_digest,
                },
                ActionPersistenceResult::Action(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(intent) = prepared_history.get(prepared_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let (action_id, action_version, proposed_classification) = match intent.operation()
                {
                    WorkManagementOperation::LowerActionClassification {
                        action_id,
                        action_version,
                        proposed_classification,
                        ..
                    } => (action_id, *action_version, *proposed_classification),
                    _ => return Err(ActionRehydrationError::CommandResultMismatch),
                };
                let Some(previous) = action_timeline.get(action_id).cloned() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(audit) = outcome.audit_events.first() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let mut expected = previous.clone();
                expected.classification = proposed_classification;
                let receipt_id = outcome.approval_receipt_id.clone();
                let Some(receipt_id) = receipt_id else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                expected.version = expected
                    .version
                    .next()
                    .ok_or(ActionRehydrationError::CommandResultMismatch)?;
                let audit_code = "action.classification_lowered";
                if previous.version != action_version
                    || *actor != AuditActor::HeadOfProducts
                    || acknowledged_digest != intent.payload_digest()
                    || audit.occurred_at() >= intent.preview().expires_at()
                    || !receipt_ids.insert(receipt_id)
                    || *intent.payload_digest() != intent.preview().payload_digest()
                    || outcome.record != expected
                    || outcome.audit_events.len() != 1
                    || capsule.audit_event_ids.len() != 1
                    || capsule.audit_event_ids[0] != *audit.id()
                    || audit.correlation_id() != &capsule.original_correlation_id
                    || audit_map.get(audit.id()).copied() != Some(audit)
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != audit_code
                    || audit.target() != &AuditTarget::Action(action_id.clone())
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::Approved
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != audit_code
                    || !consumed_prepared.insert(prepared_id.clone())
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                claimed_audits.push(audit.id().clone());
                action_timeline.insert(action_id.clone(), expected);
                continue;
            }
            if let (
                ActionPersistenceCommand::MarkActionSupersededPremise {
                    action_id,
                    expected_version,
                    source_decision_id,
                    classification,
                },
                ActionPersistenceResult::Action(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(previous) = action_timeline.get(action_id).cloned() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(expected) = ActionRecord::from_persisted_superseded_premise_marked(
                    previous,
                    *expected_version,
                    source_decision_id,
                    *classification,
                ) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let audit_code = "action.superseded_premise_marked";
                if outcome.record != expected
                    || outcome.approval_receipt_id.is_some()
                    || outcome.audit_events.len() != 1
                    || capsule.audit_event_ids.len() != 1
                    || capsule.audit_event_ids[0] != *outcome.audit_events[0].id()
                    || outcome.audit_events[0].correlation_id() != &capsule.original_correlation_id
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                let audit = &outcome.audit_events[0];
                if audit_map.get(audit.id()).copied() != Some(audit)
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != audit_code
                    || audit.target() != &AuditTarget::Action(action_id.clone())
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::Approved
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != audit_code
                {
                    return Err(ActionRehydrationError::AuditMismatch);
                }
                claimed_audits.push(audit.id().clone());
                action_timeline.insert(action_id.clone(), expected);
                continue;
            }
            if let (
                ActionPersistenceCommand::ExecuteAction {
                    kind,
                    prepared_id,
                    actor,
                    acknowledged_digest,
                },
                ActionPersistenceResult::Action(outcome),
            ) = (&capsule.command, &capsule.result)
            {
                let Some(intent) = prepared_history.get(prepared_id) else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let (operation_kind, action_id, action_version, reason, bindings) =
                    match intent.operation() {
                        WorkManagementOperation::CompleteAction {
                            action_id,
                            action_version,
                        } => (
                            ActionPersistenceTransitionKind::Complete,
                            action_id,
                            *action_version,
                            None,
                            None,
                        ),
                        WorkManagementOperation::CancelAction {
                            action_id,
                            action_version,
                            reason,
                            evidence_classifications,
                        } => (
                            ActionPersistenceTransitionKind::Cancel,
                            action_id,
                            *action_version,
                            Some(reason.as_str()),
                            Some(evidence_classifications.as_slice()),
                        ),
                        WorkManagementOperation::ReopenAction {
                            action_id,
                            action_version,
                            reason,
                            evidence_classifications,
                            ..
                        } => (
                            ActionPersistenceTransitionKind::Reopen,
                            action_id,
                            *action_version,
                            Some(reason.as_str()),
                            Some(evidence_classifications.as_slice()),
                        ),
                        _ => return Err(ActionRehydrationError::CommandResultMismatch),
                    };
                let Some(previous) = action_timeline.get(action_id).cloned() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let Some(audit) = outcome.audit_events.first() else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                let target_state = match kind {
                    ActionPersistenceTransitionKind::Complete => ActionState::Completed,
                    ActionPersistenceTransitionKind::Cancel => ActionState::Cancelled,
                    ActionPersistenceTransitionKind::Reopen => ActionState::InProgress,
                };
                let legal = match kind {
                    ActionPersistenceTransitionKind::Complete => {
                        previous.state == ActionState::InProgress
                    }
                    ActionPersistenceTransitionKind::Cancel => {
                        matches!(previous.state, ActionState::Open | ActionState::InProgress)
                    }
                    ActionPersistenceTransitionKind::Reopen => matches!(
                        previous.state,
                        ActionState::Completed | ActionState::Cancelled
                    ),
                };
                let expected_reason =
                    reason.and_then(|value| ActionDetails::parse(value.to_owned()).ok());
                let expected_classification = bindings.map_or_else(
                    || {
                        intent
                            .preview()
                            .support()
                            .map_or(previous.classification, |support| {
                                support
                                    .evidence()
                                    .iter()
                                    .map(EvidenceReferenceMetadata::classification)
                                    .chain(
                                        support
                                            .judgments()
                                            .iter()
                                            .map(HumanJudgment::classification),
                                    )
                                    .fold(previous.classification, DataClassification::combine)
                            })
                    },
                    |items| {
                        items.iter().fold(previous.classification, |value, item| {
                            value.combine(item.classification())
                        })
                    },
                );
                let mut expected = previous.clone();
                expected.state = target_state;
                expected.transition_reason = expected_reason.clone();
                expected.classification = expected_classification;
                if let Some(support) = intent.preview().support() {
                    expected.support = Some(support.clone());
                }
                let receipt_id = outcome.approval_receipt_id.clone();
                let Some(receipt_id) = receipt_id else {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                };
                expected.transition_history.push(ActionTransitionRecord {
                    from: previous.state,
                    to: target_state,
                    reason: expected_reason,
                    occurred_at: audit.occurred_at(),
                    support: intent.preview().support().cloned(),
                    approval_receipt_id: Some(receipt_id.clone()),
                });
                expected.version = expected
                    .version
                    .next()
                    .ok_or(ActionRehydrationError::CommandResultMismatch)?;
                let audit_code = match kind {
                    ActionPersistenceTransitionKind::Complete => "action.completed",
                    ActionPersistenceTransitionKind::Cancel => "action.cancelled",
                    ActionPersistenceTransitionKind::Reopen => "action.reopened",
                };
                if audit.occurred_at() >= intent.preview().expires_at()
                    || !receipt_ids.insert(receipt_id)
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                if *kind != operation_kind
                    || *actor != AuditActor::HeadOfProducts
                    || acknowledged_digest != intent.payload_digest()
                    || previous.version != action_version
                    || !legal
                    || *intent.payload_digest() != intent.preview().payload_digest()
                    || intent.classification()
                        != intent
                            .preview()
                            .support()
                            .map_or(expected_classification, |support| {
                                expected_classification.combine(support.classification())
                            })
                    || previous.classification.combine(expected_classification)
                        != expected_classification
                    || outcome.record != expected
                    || outcome.audit_events.len() != 1
                    || capsule.audit_event_ids.len() != 1
                    || capsule.audit_event_ids[0] != *audit.id()
                    || audit.correlation_id() != &capsule.original_correlation_id
                    || audit_map.get(audit.id()).copied() != Some(audit)
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::WorkManagement
                    || audit.code().as_str() != audit_code
                    || audit.target() != &AuditTarget::Action(action_id.clone())
                    || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                    || audit.approval_outcome() != AuditApprovalOutcome::Approved
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str() != audit_code
                    || !consumed_prepared.insert(prepared_id.clone())
                {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                claimed_audits.push(audit.id().clone());
                action_timeline.insert(action_id.clone(), expected);
                continue;
            }
            let ActionPersistenceResult::Request(outcome) = &capsule.result else {
                return Err(ActionRehydrationError::CommandResultMismatch);
            };
            let (expected_record, audit_code, expected_approval_outcome) = match &capsule.command {
                ActionPersistenceCommand::CreateRequest {
                    id,
                    title,
                    details,
                    intended_owner,
                    response_due_at,
                    intended_action_due_at,
                    classification,
                } => {
                    if timeline.contains_key(id) {
                        return Err(ActionRehydrationError::DuplicateRecord);
                    }
                    (
                        ActionRequestRecord::from_persisted_created_draft(
                            id.clone(),
                            title.clone(),
                            details.clone(),
                            intended_owner.clone(),
                            *response_due_at,
                            *intended_action_due_at,
                            *classification,
                        ),
                        "action_request.created",
                        AuditApprovalOutcome::NotRequired,
                    )
                }
                ActionPersistenceCommand::TransitionRequest {
                    request_id,
                    expected_version,
                    target_state,
                    rationale,
                } => {
                    let Some(mut previous) = timeline.get(request_id).cloned() else {
                        return Err(ActionRehydrationError::CommandResultMismatch);
                    };
                    if previous.version() != *expected_version
                        || !valid_persisted_request_transition(
                            previous.state(),
                            *target_state,
                            rationale.as_ref(),
                        )
                    {
                        return Err(ActionRehydrationError::CommandResultMismatch);
                    }
                    previous.state = *target_state;
                    previous.terminal_rationale = rationale.clone();
                    previous.version = previous
                        .version
                        .next()
                        .ok_or(ActionRehydrationError::CommandResultMismatch)?;
                    let code = match target_state {
                        ActionRequestState::Open => "action_request.submitted",
                        ActionRequestState::Declined => "action_request.declined",
                        ActionRequestState::Withdrawn => "action_request.withdrawn",
                        ActionRequestState::Draft | ActionRequestState::Accepted => {
                            return Err(ActionRehydrationError::CommandResultMismatch);
                        }
                    };
                    (previous, code, AuditApprovalOutcome::NotRequired)
                }
                ActionPersistenceCommand::CreateRequestFromDecision {
                    request,
                    source_decision_id,
                    classification,
                } => {
                    if timeline.contains_key(&request.id) {
                        return Err(ActionRehydrationError::DuplicateRecord);
                    }
                    (
                        ActionRequestRecord::from_persisted_created_from_decision(
                            request.id.clone(),
                            request.subject.clone(),
                            request.details.clone(),
                            request.intended_owner.clone(),
                            request.due_at,
                            *classification,
                            source_decision_id.clone(),
                        ),
                        "action_request.created_from_decision",
                        AuditApprovalOutcome::Approved,
                    )
                }
                ActionPersistenceCommand::MarkRequestSupersededPremise {
                    request_id,
                    expected_version,
                    source_decision_id,
                    classification,
                } => {
                    let Some(previous) = timeline.get(request_id).cloned() else {
                        return Err(ActionRehydrationError::CommandResultMismatch);
                    };
                    let Some(marked) =
                        ActionRequestRecord::from_persisted_superseded_premise_marked(
                            previous,
                            *expected_version,
                            source_decision_id,
                            *classification,
                        )
                    else {
                        return Err(ActionRehydrationError::CommandResultMismatch);
                    };
                    (
                        marked,
                        "action_request.superseded_premise_marked",
                        AuditApprovalOutcome::Approved,
                    )
                }
                ActionPersistenceCommand::PrepareAccept { .. } => {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                ActionPersistenceCommand::ExecuteAccept { .. } => {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                ActionPersistenceCommand::PrepareComplete { .. }
                | ActionPersistenceCommand::PrepareCancel { .. }
                | ActionPersistenceCommand::PrepareReopen { .. }
                | ActionPersistenceCommand::ExecuteAction { .. }
                | ActionPersistenceCommand::RejectPrepared { .. } => {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
                ActionPersistenceCommand::StartAction { .. }
                | ActionPersistenceCommand::LinkCompletionEvidence { .. }
                | ActionPersistenceCommand::PrepareLowerClassification { .. }
                | ActionPersistenceCommand::ExecuteLowerClassification { .. }
                | ActionPersistenceCommand::MarkActionSupersededPremise { .. } => {
                    return Err(ActionRehydrationError::CommandResultMismatch);
                }
            };
            if outcome.record != expected_record
                || outcome.approval_receipt_id.is_some()
                || outcome.audit_events.len() != 1
                || capsule.audit_event_ids.len() != 1
                || capsule.audit_event_ids[0] != *outcome.audit_events[0].id()
                || outcome.audit_events[0].correlation_id() != &capsule.original_correlation_id
            {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            timeline.insert(expected_record.id.clone(), expected_record.clone());
            let audit = &outcome.audit_events[0];
            if audit_map.get(audit.id()).copied() != Some(audit)
                || audit.actor() != AuditActor::HeadOfProducts
                || audit.module() != AuditModule::WorkManagement
                || audit.code().as_str() != audit_code
                || audit.target() != &AuditTarget::ActionRequest(expected_record.id.clone())
                || audit.policy_outcome() != AuditPolicyOutcome::Allowed
                || audit.approval_outcome() != expected_approval_outcome
                || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                || audit.effect_scope() != AuditEffectScope::Complete
                || audit.actual_effects().len() != 1
                || audit.actual_effects()[0].as_str() != audit_code
            {
                return Err(ActionRehydrationError::AuditMismatch);
            }
            claimed_audits.push(audit.id().clone());
        }
        if timeline.len() != requests.len()
            || timeline
                .iter()
                .any(|(id, record)| request_map.get(id).copied() != Some(record))
        {
            return Err(ActionRehydrationError::OrphanRecord);
        }
        if action_timeline.len() != actions.len()
            || action_timeline
                .iter()
                .any(|(id, record)| action_map.get(id).copied() != Some(record))
        {
            return Err(ActionRehydrationError::OrphanRecord);
        }
        let expected_pending: HashSet<_> = prepared_history
            .keys()
            .filter(|id| !consumed_prepared.contains(*id))
            .cloned()
            .collect();
        if expected_pending.len() != prepared.len()
            || expected_pending
                .iter()
                .any(|id| prepared_map.get(id).copied() != prepared_history.get(id))
        {
            return Err(ActionRehydrationError::OrphanRecord);
        }
        if claimed_audits.len() != audits.len()
            || claimed_audits
                .iter()
                .zip(audits.iter().map(AuditEvent::id))
                .any(|(claimed, actual)| claimed != actual)
        {
            return Err(ActionRehydrationError::OrphanAuditEvent);
        }

        Ok(Self {
            requests,
            actions,
            prepared,
            discarded_prepared,
            replay,
            audits,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_typed_terminal_capsule(
    capsule: &ActionReplayCapsule,
    terminal_command: &ActionPersistenceCommand,
    cause: &ActionPersistenceTerminalCause,
    error: &DomainError,
    prepared_disposition: PreparedDisposition,
    audit: &AuditEvent,
    prepared_history: &HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    prepared: &HashMap<&PreparedIntentId, &WorkManagementPreparedIntent>,
    discarded: &HashMap<PreparedIntentId, &WorkManagementPreparedIntent>,
    request_timeline: &HashMap<ActionRequestId, ActionRequestRecord>,
    action_timeline: &HashMap<ActionId, ActionRecord>,
    audit_map: &HashMap<&crate::identity::AuditEventId, &AuditEvent>,
    consumed: &mut HashSet<PreparedIntentId>,
    claimed_audits: &mut Vec<crate::identity::AuditEventId>,
) -> Result<(), ActionRehydrationError> {
    if terminal_command != &capsule.command {
        return Err(ActionRehydrationError::CommandResultMismatch);
    }
    let (target, actor, is_h2, prepared_id) = match &capsule.command {
        ActionPersistenceCommand::ExecuteAccept {
            prepared_id,
            actor,
            acknowledged_digest,
        } => {
            let Some(intent) = terminal_prepared_intent(
                prepared_id,
                prepared_disposition,
                prepared_history,
                prepared,
                discarded,
            ) else {
                return Err(ActionRehydrationError::CommandResultMismatch);
            };
            let target = match intent.operation() {
                WorkManagementOperation::AcceptActionRequest { request_id, .. } => {
                    AuditTarget::ActionRequest(request_id.clone())
                }
                _ => return Err(ActionRehydrationError::CommandResultMismatch),
            };
            validate_terminal_digest(cause, acknowledged_digest, intent)?;
            (target, *actor, true, Some(prepared_id.clone()))
        }
        ActionPersistenceCommand::ExecuteAction {
            kind,
            prepared_id,
            actor,
            acknowledged_digest,
        } => {
            let Some(intent) = terminal_prepared_intent(
                prepared_id,
                prepared_disposition,
                prepared_history,
                prepared,
                discarded,
            ) else {
                return Err(ActionRehydrationError::CommandResultMismatch);
            };
            let (operation_kind, action_id) = match intent.operation() {
                WorkManagementOperation::CompleteAction { action_id, .. } => {
                    (ActionPersistenceTransitionKind::Complete, action_id)
                }
                WorkManagementOperation::CancelAction { action_id, .. } => {
                    (ActionPersistenceTransitionKind::Cancel, action_id)
                }
                WorkManagementOperation::ReopenAction { action_id, .. } => {
                    (ActionPersistenceTransitionKind::Reopen, action_id)
                }
                _ => return Err(ActionRehydrationError::CommandResultMismatch),
            };
            if *kind != operation_kind {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            validate_terminal_digest(cause, acknowledged_digest, intent)?;
            (
                AuditTarget::Action(action_id.clone()),
                *actor,
                true,
                Some(prepared_id.clone()),
            )
        }
        ActionPersistenceCommand::PrepareComplete {
            action_id,
            expected_version,
            ..
        } => {
            if !action_timeline.get(action_id).is_some_and(|a| {
                a.version == *expected_version && a.state == ActionState::InProgress
            }) {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            let ActionPersistenceTerminalCause::H3Denied(denial) = cause else {
                return Err(ActionRehydrationError::CommandResultMismatch);
            };
            let action = action_timeline
                .get(action_id)
                .ok_or(ActionRehydrationError::CommandResultMismatch)?;
            if !valid_h3_command_cause(ActionPersistenceTransitionKind::Complete, action, *denial) {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            (
                AuditTarget::Action(action_id.clone()),
                AuditActor::HeadOfProducts,
                false,
                None,
            )
        }
        ActionPersistenceCommand::PrepareCancel {
            action_id,
            expected_version,
            ..
        } => {
            if !action_timeline.get(action_id).is_some_and(|a| {
                a.version == *expected_version
                    && matches!(a.state, ActionState::Open | ActionState::InProgress)
            }) {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            let ActionPersistenceTerminalCause::H3Denied(denial) = cause else {
                return Err(ActionRehydrationError::CommandResultMismatch);
            };
            let action = action_timeline
                .get(action_id)
                .ok_or(ActionRehydrationError::CommandResultMismatch)?;
            if !valid_h3_command_cause(ActionPersistenceTransitionKind::Cancel, action, *denial) {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            (
                AuditTarget::Action(action_id.clone()),
                AuditActor::HeadOfProducts,
                false,
                None,
            )
        }
        ActionPersistenceCommand::PrepareReopen {
            action_id,
            expected_version,
            mode,
            ..
        } => {
            if !action_timeline.get(action_id).is_some_and(|a| {
                a.version == *expected_version
                    && matches!(
                        (a.state, mode),
                        (ActionState::Completed, ActionReopenMode::ReopenCompleted)
                            | (ActionState::Cancelled, ActionReopenMode::RestartCancelled)
                    )
            }) {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            let ActionPersistenceTerminalCause::H3Denied(denial) = cause else {
                return Err(ActionRehydrationError::CommandResultMismatch);
            };
            let action = action_timeline
                .get(action_id)
                .ok_or(ActionRehydrationError::CommandResultMismatch)?;
            if !valid_h3_command_cause(ActionPersistenceTransitionKind::Reopen, action, *denial) {
                return Err(ActionRehydrationError::CommandResultMismatch);
            }
            (
                AuditTarget::Action(action_id.clone()),
                AuditActor::HeadOfProducts,
                false,
                None,
            )
        }
        _ => return Err(ActionRehydrationError::CommandResultMismatch),
    };
    let expected_code = if !is_h2 {
        "action.evidence_policy_denied"
    } else if matches!(cause, ActionPersistenceTerminalCause::InfrastructureFailure) {
        "action.execution_failed"
    } else if matches!(
        cause,
        ActionPersistenceTerminalCause::PolicyDenied | ActionPersistenceTerminalCause::Unauthorized
    ) {
        "action.approval_denied"
    } else {
        "action.approval_rejected"
    };
    if !is_h2 && prepared_disposition != PreparedDisposition::NotApplicable
        || is_h2
            && matches!(cause, ActionPersistenceTerminalCause::PreparedIntentChanged)
            && prepared_disposition != PreparedDisposition::Retained
        || is_h2
            && !matches!(cause, ActionPersistenceTerminalCause::PreparedIntentChanged)
            && prepared_disposition != PreparedDisposition::ConsumedAndDiscarded
    {
        return Err(ActionRehydrationError::CommandResultMismatch);
    }
    if capsule.audit_event_ids.len() != 1
        || capsule.audit_event_ids[0] != *audit.id()
        || audit_map.get(audit.id()).copied() != Some(audit)
        || audit.correlation_id() != &capsule.original_correlation_id
        || audit.target() != &target
        || audit.actor() != actor
        || actor != AuditActor::HeadOfProducts
        || audit.module() != AuditModule::WorkManagement
        || audit.code().as_str() != expected_code
    {
        return Err(ActionRehydrationError::AuditMismatch);
    }
    let truthful = match cause {
        ActionPersistenceTerminalCause::PolicyDenied
        | ActionPersistenceTerminalCause::Unauthorized
        | ActionPersistenceTerminalCause::H3Denied(_) => {
            audit.policy_outcome() == AuditPolicyOutcome::Denied
                && audit.approval_outcome() == AuditApprovalOutcome::NotRequired
                && audit.execution_outcome() == AuditExecutionOutcome::NotAttempted
        }
        ActionPersistenceTerminalCause::InfrastructureFailure => {
            audit.policy_outcome() == AuditPolicyOutcome::Allowed
                && audit.approval_outcome() == AuditApprovalOutcome::Approved
                && audit.execution_outcome() == AuditExecutionOutcome::Failed
        }
        _ => {
            audit.policy_outcome() == AuditPolicyOutcome::Allowed
                && audit.approval_outcome() == AuditApprovalOutcome::Rejected
                && audit.execution_outcome() == AuditExecutionOutcome::NotAttempted
        }
    };
    if !truthful
        || audit.effect_scope() != AuditEffectScope::None
        || !audit.actual_effects().is_empty()
    {
        return Err(ActionRehydrationError::AuditMismatch);
    }
    if matches!(cause, ActionPersistenceTerminalCause::Expired) {
        let Some(id) = prepared_id.as_ref() else {
            return Err(ActionRehydrationError::CommandResultMismatch);
        };
        let Some(intent) = terminal_prepared_intent(
            id,
            prepared_disposition,
            prepared_history,
            prepared,
            discarded,
        ) else {
            return Err(ActionRehydrationError::CommandResultMismatch);
        };
        if audit.occurred_at() < intent.preview().expires_at() {
            return Err(ActionRehydrationError::AuditMismatch);
        }
    }
    let expected_error = domain_error_for_persisted_target(
        terminal_cause_to_service(cause),
        capsule.original_correlation_id.clone(),
        &target,
        request_timeline,
        action_timeline,
    );
    if *error != expected_error {
        return Err(ActionRehydrationError::CommandResultMismatch);
    }
    if prepared_disposition == PreparedDisposition::ConsumedAndDiscarded {
        let Some(id) = prepared_id else {
            return Err(ActionRehydrationError::CommandResultMismatch);
        };
        if !consumed.insert(id.clone()) {
            return Err(ActionRehydrationError::CommandResultMismatch);
        }
    }
    claimed_audits.push(audit.id().clone());
    Ok(())
}

fn terminal_prepared_intent<'a>(
    id: &PreparedIntentId,
    disposition: PreparedDisposition,
    history: &'a HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    prepared: &'a HashMap<&PreparedIntentId, &WorkManagementPreparedIntent>,
    discarded: &'a HashMap<PreparedIntentId, &WorkManagementPreparedIntent>,
) -> Option<&'a WorkManagementPreparedIntent> {
    let intent = match disposition {
        PreparedDisposition::Retained => prepared
            .get(id)
            .copied()
            .or_else(|| discarded.get(id).copied())
            .or_else(|| history.get(id)),
        PreparedDisposition::ConsumedAndDiscarded => discarded.get(id).copied(),
        PreparedDisposition::NotApplicable => None,
    }?;
    if history.get(id).is_some_and(|original| original != intent) {
        return None;
    }
    Some(intent)
}

fn validate_terminal_digest(
    cause: &ActionPersistenceTerminalCause,
    acknowledged: &WorkManagementPayloadDigest,
    intent: &WorkManagementPreparedIntent,
) -> Result<(), ActionRehydrationError> {
    match cause {
        ActionPersistenceTerminalCause::DigestMismatch { attempted_digest }
            if attempted_digest == acknowledged && acknowledged != intent.payload_digest() =>
        {
            Ok(())
        }
        ActionPersistenceTerminalCause::DigestMismatch { .. }
        | ActionPersistenceTerminalCause::H3Denied(_) => {
            Err(ActionRehydrationError::CommandResultMismatch)
        }
        _ if acknowledged == intent.payload_digest() => Ok(()),
        _ => Err(ActionRehydrationError::CommandResultMismatch),
    }
}

fn valid_h3_command_cause(
    kind: ActionPersistenceTransitionKind,
    action: &ActionRecord,
    cause: ActionPersistenceH3DenialCause,
) -> bool {
    let has_evidence = !action.completion_evidence.is_empty();
    let unresolved = action.classification == DataClassification::Unclassified
        || action.commitment_classification == DataClassification::Unclassified;
    match (kind, cause) {
        (
            ActionPersistenceTransitionKind::Complete,
            ActionPersistenceH3DenialCause::MissingCompletionEvidence,
        ) => !has_evidence,
        (
            ActionPersistenceTransitionKind::Complete,
            ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence,
        ) => has_evidence && !unresolved,
        (_, ActionPersistenceH3DenialCause::CompletionEvidenceNotFound) => has_evidence,
        (_, ActionPersistenceH3DenialCause::UnclassifiedCompletionEvidence) => has_evidence,
        (_, ActionPersistenceH3DenialCause::ClassificationUnresolved) => unresolved,
        (_, ActionPersistenceH3DenialCause::EvidenceUnavailable) => has_evidence,
        _ => false,
    }
}

fn domain_error_for_persisted_target(
    cause: ActionServiceError,
    correlation_id: CorrelationId,
    target: &AuditTarget,
    requests: &HashMap<ActionRequestId, ActionRequestRecord>,
    actions: &HashMap<ActionId, ActionRecord>,
) -> DomainError {
    match target {
        AuditTarget::ActionRequest(id) => domain_error_with_snapshot(
            cause,
            correlation_id,
            requests.get(id).map(|record| {
                (
                    record.version(),
                    record.state().as_persisted(),
                    request_allowed_intents(record.state()),
                )
            }),
        ),
        AuditTarget::Action(id) => domain_error_with_snapshot(
            cause,
            correlation_id,
            actions.get(id).map(|record| {
                (
                    record.version(),
                    record.state().as_persisted(),
                    action_allowed_intents(record.state()),
                )
            }),
        ),
        _ => domain_error_with_snapshot(cause, correlation_id, None),
    }
}

fn domain_error_for_runtime_target(
    cause: ActionServiceError,
    correlation_id: CorrelationId,
    target: &ErrorTarget,
    requests: &HashMap<ActionRequestId, ActionRequestRecord>,
    actions: &HashMap<ActionId, ActionRecord>,
) -> DomainError {
    match target {
        ErrorTarget::Request(id) => domain_error_with_snapshot(
            cause,
            correlation_id,
            requests.get(id).map(|record| {
                (
                    record.version(),
                    record.state().as_persisted(),
                    request_allowed_intents(record.state()),
                )
            }),
        ),
        ErrorTarget::Action(id) => domain_error_with_snapshot(
            cause,
            correlation_id,
            actions.get(id).map(|record| {
                (
                    record.version(),
                    record.state().as_persisted(),
                    action_allowed_intents(record.state()),
                )
            }),
        ),
        ErrorTarget::Unknown => domain_error_with_snapshot(cause, correlation_id, None),
    }
}

fn domain_error_with_snapshot(
    cause: ActionServiceError,
    correlation_id: CorrelationId,
    snapshot: Option<(AggregateVersion, &'static str, &'static [&'static str])>,
) -> DomainError {
    let (code, key) = match cause {
        ActionServiceError::NotFound => (ErrorCode::DomainNotFound, "action.not_found"),
        ActionServiceError::IdempotencyConflict => (
            ErrorCode::DomainIdempotencyConflict,
            "action.idempotency_conflict",
        ),
        ActionServiceError::PolicyDenied
        | ActionServiceError::Unauthorized
        | ActionServiceError::EvidencePolicyDenied
        | ActionServiceError::ClassificationUnresolved
        | ActionServiceError::EvidenceUnavailable => {
            (ErrorCode::SecurityPolicyDenied, "action.approval_denied")
        }
        ActionServiceError::DigestMismatch
        | ActionServiceError::Expired
        | ActionServiceError::PreparedIntentChanged
        | ActionServiceError::PreparedIntentNotFound => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "action.preview_expired_or_changed",
        ),
        ActionServiceError::InfrastructureFailure => {
            (ErrorCode::PlatformInternal, "action.internal")
        }
        ActionServiceError::MissingOwner
        | ActionServiceError::MissingDueDate
        | ActionServiceError::InvalidEvidence => (
            ErrorCode::ValidationInvalidField,
            "action.validation_failed",
        ),
        ActionServiceError::AlreadyExists
        | ActionServiceError::IllegalTransition
        | ActionServiceError::StaleVersion
        | ActionServiceError::InvalidReopenMode => {
            (ErrorCode::DomainConflict, "action.domain_conflict")
        }
        ActionServiceError::NotALowering => (
            ErrorCode::DomainConflict,
            "action.classification_lowering_not_a_lowering",
        ),
    };
    let mut error = DomainError::new(
        code,
        static_message_key(key),
        correlation_id,
        matches!(
            cause,
            ActionServiceError::InfrastructureFailure | ActionServiceError::EvidenceUnavailable
        ),
    );
    if let Some((version, state, allowed)) = snapshot {
        error = error.with_extension(SafeErrorExtension::CurrentVersion(version));
        if let Ok(param) =
            MessageParam::new("current_state", SafeParamValue::FieldKey(state.to_owned()))
        {
            error = error.with_param(param);
        }
        for (index, intent) in allowed.iter().enumerate() {
            if let Ok(param) = MessageParam::new(
                format!("allowed_next_intent_{}", index + 1),
                SafeParamValue::FieldKey((*intent).to_owned()),
            ) {
                error = error.with_param(param);
            }
        }
    }
    error
}

fn valid_persisted_request_transition(
    from: ActionRequestState,
    to: ActionRequestState,
    rationale: Option<&ActionDetails>,
) -> bool {
    matches!(
        (from, to, rationale.is_some()),
        (ActionRequestState::Draft, ActionRequestState::Open, false)
            | (ActionRequestState::Open, ActionRequestState::Declined, true)
            | (
                ActionRequestState::Open,
                ActionRequestState::Withdrawn,
                true
            )
    )
}

#[derive(Clone)]
pub(crate) struct ActionDecisionStage {
    state: Store,
}

#[allow(dead_code)]
impl ActionDecisionStage {
    pub(crate) fn request(&self, id: &ActionRequestId) -> Option<&ActionRequestRecord> {
        self.state.requests.get(id)
    }

    pub(crate) fn action(&self, id: &ActionId) -> Option<&ActionRecord> {
        self.state.actions.get(id)
    }

    pub(crate) fn action_requests_for_decision(
        &self,
        source_decision_id: &DecisionId,
    ) -> Vec<ActionRequestRecord> {
        decision_requests_from(&self.state, source_decision_id)
    }

    pub(crate) fn actions_for_decision(
        &self,
        source_decision_id: &DecisionId,
    ) -> Vec<ActionRecord> {
        decision_actions_from(&self.state, source_decision_id)
    }
}

pub struct InMemoryActionService<C, I, Z, P, E> {
    clock: C,
    ids: I,
    authorization: Z,
    policy: P,
    evidence_authority: E,
    state: Store,
    fail_next_commit: bool,
}

impl<
        C: Clock,
        I: ActionServiceIdSource,
        Z: ApprovalAuthorizationPort,
        P: ActionExecutionPolicyPort,
        E: ActionEvidenceAuthorityPort,
    > InMemoryActionService<C, I, Z, P, E>
{
    pub fn new(clock: C, ids: I, authorization: Z, policy: P, evidence_authority: E) -> Self {
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence_authority,
            state: Store::default(),
            fail_next_commit: false,
        }
    }

    #[doc(hidden)]
    pub fn persistence_snapshot(
        &self,
    ) -> Result<ActionPersistenceSnapshot, ActionRehydrationError> {
        let mut entries: Vec<_> = self.state.idem.iter().collect();
        entries.sort_by_key(|(_, stored)| stored.operation_ordinal);
        let mut replay = Vec::with_capacity(entries.len());
        for (idempotency_id, stored) in entries {
            let command = match &stored.signature {
                CommandIdentity::CreateRequest {
                    id,
                    title,
                    details,
                    intended_owner,
                    response_due_at,
                    intended_action_due_at,
                    classification,
                } => ActionPersistenceCommand::CreateRequest {
                    id: id.clone(),
                    title: title.clone(),
                    details: details.clone(),
                    intended_owner: intended_owner.clone(),
                    response_due_at: *response_due_at,
                    intended_action_due_at: *intended_action_due_at,
                    classification: *classification,
                },
                CommandIdentity::TransitionRequest {
                    request_id,
                    expected_version,
                    target_state,
                    rationale,
                } => ActionPersistenceCommand::TransitionRequest {
                    request_id: request_id.clone(),
                    expected_version: *expected_version,
                    target_state: *target_state,
                    rationale: rationale.clone(),
                },
                CommandIdentity::PrepareAccept {
                    request_id,
                    expected_version,
                } => ActionPersistenceCommand::PrepareAccept {
                    request_id: request_id.clone(),
                    expected_version: *expected_version,
                },
                CommandIdentity::ExecuteAccept(identity) => {
                    ActionPersistenceCommand::ExecuteAccept {
                        prepared_id: identity.prepared_id.clone(),
                        actor: identity.actor,
                        acknowledged_digest: identity.digest.clone(),
                    }
                }
                CommandIdentity::PrepareComplete {
                    action_id,
                    expected_version,
                    judgment,
                } => ActionPersistenceCommand::PrepareComplete {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                    judgment: judgment.clone(),
                },
                CommandIdentity::PrepareCancel {
                    action_id,
                    expected_version,
                    reason,
                } => ActionPersistenceCommand::PrepareCancel {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                    reason: reason.clone(),
                },
                CommandIdentity::PrepareReopen {
                    action_id,
                    expected_version,
                    mode,
                    reason,
                } => ActionPersistenceCommand::PrepareReopen {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                    mode: *mode,
                    reason: reason.clone(),
                },
                CommandIdentity::ExecuteAction(kind, identity) => {
                    ActionPersistenceCommand::ExecuteAction {
                        kind: persistence_transition_kind(*kind),
                        prepared_id: identity.prepared_id.clone(),
                        actor: identity.actor,
                        acknowledged_digest: identity.digest.clone(),
                    }
                }
                CommandIdentity::StartAction {
                    action_id,
                    expected_version,
                } => ActionPersistenceCommand::StartAction {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                },
                CommandIdentity::LinkCompletionEvidence {
                    action_id,
                    expected_version,
                    evidence_id,
                    evidence_classification: Some(evidence_classification),
                } => ActionPersistenceCommand::LinkCompletionEvidence {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                    evidence_id: evidence_id.clone(),
                    evidence_classification: *evidence_classification,
                },
                CommandIdentity::CreateRequestFromDecision {
                    request,
                    source_decision_id,
                    classification,
                } => ActionPersistenceCommand::CreateRequestFromDecision {
                    request: request.clone(),
                    source_decision_id: source_decision_id.clone(),
                    classification: *classification,
                },
                CommandIdentity::MarkRequestSupersededPremise {
                    request_id,
                    expected_version,
                    source_decision_id,
                    classification,
                } => ActionPersistenceCommand::MarkRequestSupersededPremise {
                    request_id: request_id.clone(),
                    expected_version: *expected_version,
                    source_decision_id: source_decision_id.clone(),
                    classification: *classification,
                },
                CommandIdentity::MarkActionSupersededPremise {
                    action_id,
                    expected_version,
                    source_decision_id,
                    classification,
                } => ActionPersistenceCommand::MarkActionSupersededPremise {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                    source_decision_id: source_decision_id.clone(),
                    classification: *classification,
                },
                // H2a Lower Data Classification. These are the
                // exact inverse of the mappings `rehydrate()` already performs,
                // so a capsule that was decoded can be re-exported. Without
                // them, any Action that had ever been lower-classified could
                // never be Cancelled, Completed or Reopened again, because the
                // re-export of its own history failed.
                CommandIdentity::PrepareLowerActionClassification {
                    action_id,
                    expected_version,
                    proposed_classification,
                    rationale,
                } => ActionPersistenceCommand::PrepareLowerClassification {
                    action_id: action_id.clone(),
                    expected_version: *expected_version,
                    proposed_classification: *proposed_classification,
                    rationale: rationale.clone(),
                },
                CommandIdentity::ExecuteLowerActionClassification(approval) => {
                    ActionPersistenceCommand::ExecuteLowerClassification {
                        prepared_id: approval.prepared_id.clone(),
                        actor: approval.actor,
                        acknowledged_digest: approval.digest.clone(),
                    }
                }
                // Deliberately still refused: this is the pre-resolution
                // identity used only for idempotency lookup before the
                // Evidence classification is known. A successful link stores
                // the `Some(..)` form the arm above exports, so reaching here
                // means the classification is genuinely unknown -- and
                // inventing one would be unsafe.
                CommandIdentity::RejectPrepared { prepared_id, actor } => {
                    ActionPersistenceCommand::RejectPrepared {
                        prepared_id: prepared_id.clone(),
                        actor: *actor,
                    }
                }
                CommandIdentity::LinkCompletionEvidence { .. } => {
                    return Err(ActionRehydrationError::UnsupportedState);
                }
            };
            let (result, audit_event_ids) = match &stored.result {
                StoredResult::Request(outcome) => (
                    ActionPersistenceResult::Request(outcome.clone()),
                    outcome
                        .audit_events
                        .iter()
                        .map(|audit| audit.id().clone())
                        .collect(),
                ),
                StoredResult::Prepared(intent)
                    if matches!(
                        &command,
                        ActionPersistenceCommand::PrepareAccept { .. }
                            | ActionPersistenceCommand::PrepareComplete { .. }
                            | ActionPersistenceCommand::PrepareCancel { .. }
                            | ActionPersistenceCommand::PrepareReopen { .. }
                            | ActionPersistenceCommand::PrepareLowerClassification { .. }
                    ) =>
                {
                    (
                        ActionPersistenceResult::Prepared(intent.clone()),
                        Vec::new(),
                    )
                }
                StoredResult::Accepted(outcome)
                    if matches!(&command, ActionPersistenceCommand::ExecuteAccept { .. }) =>
                {
                    (
                        ActionPersistenceResult::Accepted(outcome.clone()),
                        outcome
                            .audit_events
                            .iter()
                            .map(|audit| audit.id().clone())
                            .collect(),
                    )
                }
                StoredResult::Action(outcome)
                    if matches!(
                        &command,
                        ActionPersistenceCommand::StartAction { .. }
                            | ActionPersistenceCommand::LinkCompletionEvidence { .. }
                            | ActionPersistenceCommand::ExecuteAction { .. }
                            | ActionPersistenceCommand::MarkActionSupersededPremise { .. }
                            | ActionPersistenceCommand::ExecuteLowerClassification { .. }
                    ) =>
                {
                    (
                        ActionPersistenceResult::Action(outcome.clone()),
                        outcome
                            .audit_events
                            .iter()
                            .map(|audit| audit.id().clone())
                            .collect(),
                    )
                }
                StoredResult::Rejected(outcome)
                    if matches!(&command, ActionPersistenceCommand::RejectPrepared { .. }) =>
                {
                    (
                        ActionPersistenceResult::Rejected(outcome.clone()),
                        vec![outcome.audit_event().id().clone()],
                    )
                }
                StoredResult::Terminal(failure) => (
                    ActionPersistenceResult::Terminal {
                        command: command.clone(),
                        cause: failure.persisted_cause.clone(),
                        error: failure.error.clone(),
                        prepared_disposition: failure.prepared_disposition,
                        audit: failure.audit.clone(),
                    },
                    vec![failure.audit.id().clone()],
                ),
                StoredResult::Action(_)
                | StoredResult::Accepted(_)
                | StoredResult::Prepared(_)
                | StoredResult::Rejected(_) => {
                    return Err(ActionRehydrationError::UnsupportedState);
                }
            };
            replay.push(ActionReplayCapsule {
                idempotency_id: idempotency_id.clone(),
                original_correlation_id: stored.original_correlation_id.clone(),
                operation_ordinal: stored.operation_ordinal,
                command,
                result,
                audit_event_ids,
            });
        }
        replay.sort_by_key(ActionReplayCapsule::operation_ordinal);

        let mut requests: Vec<_> = self.state.requests.values().cloned().collect();
        requests.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        let mut actions: Vec<_> = self.state.actions.values().cloned().collect();
        actions.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        let mut prepared: Vec<_> = self.state.prepared.values().cloned().collect();
        prepared.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        ActionPersistenceSnapshot::try_new(
            requests,
            actions,
            prepared,
            self.state.discarded_prepared.clone(),
            replay,
            self.state.audit_events.clone(),
        )
    }

    #[doc(hidden)]
    pub fn rehydrate(
        clock: C,
        ids: I,
        authorization: Z,
        policy: P,
        evidence_authority: E,
        snapshot: ActionPersistenceSnapshot,
    ) -> Self {
        let requests = snapshot
            .requests
            .iter()
            .cloned()
            .map(|record| (record.id().clone(), record))
            .collect();
        let actions = snapshot
            .actions
            .iter()
            .cloned()
            .map(|record| (record.id().clone(), record))
            .collect();
        let prepared: HashMap<PreparedIntentId, WorkManagementPreparedIntent> = snapshot
            .prepared
            .iter()
            .cloned()
            .map(|intent| (intent.id().clone(), intent))
            .collect();
        let idem = snapshot
            .replay
            .iter()
            .map(|capsule| {
                let signature = match &capsule.command {
                    ActionPersistenceCommand::CreateRequest {
                        id,
                        title,
                        details,
                        intended_owner,
                        response_due_at,
                        intended_action_due_at,
                        classification,
                    } => CommandIdentity::CreateRequest {
                        id: id.clone(),
                        title: title.clone(),
                        details: details.clone(),
                        intended_owner: intended_owner.clone(),
                        response_due_at: *response_due_at,
                        intended_action_due_at: *intended_action_due_at,
                        classification: *classification,
                    },
                    ActionPersistenceCommand::TransitionRequest {
                        request_id,
                        expected_version,
                        target_state,
                        rationale,
                    } => CommandIdentity::TransitionRequest {
                        request_id: request_id.clone(),
                        expected_version: *expected_version,
                        target_state: *target_state,
                        rationale: rationale.clone(),
                    },
                    ActionPersistenceCommand::PrepareAccept {
                        request_id,
                        expected_version,
                    } => CommandIdentity::PrepareAccept {
                        request_id: request_id.clone(),
                        expected_version: *expected_version,
                    },
                    ActionPersistenceCommand::ExecuteAccept {
                        prepared_id,
                        actor,
                        acknowledged_digest,
                    } => CommandIdentity::ExecuteAccept(ApprovalBusinessIdentity {
                        prepared_id: prepared_id.clone(),
                        actor: *actor,
                        digest: acknowledged_digest.clone(),
                    }),
                    ActionPersistenceCommand::PrepareComplete {
                        action_id,
                        expected_version,
                        judgment,
                    } => CommandIdentity::PrepareComplete {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                        judgment: judgment.clone(),
                    },
                    ActionPersistenceCommand::PrepareCancel {
                        action_id,
                        expected_version,
                        reason,
                    } => CommandIdentity::PrepareCancel {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                        reason: reason.clone(),
                    },
                    ActionPersistenceCommand::PrepareReopen {
                        action_id,
                        expected_version,
                        mode,
                        reason,
                    } => CommandIdentity::PrepareReopen {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                        mode: *mode,
                        reason: reason.clone(),
                    },
                    ActionPersistenceCommand::ExecuteAction {
                        kind,
                        prepared_id,
                        actor,
                        acknowledged_digest,
                    } => CommandIdentity::ExecuteAction(
                        runtime_transition_kind(*kind),
                        ApprovalBusinessIdentity {
                            prepared_id: prepared_id.clone(),
                            actor: *actor,
                            digest: acknowledged_digest.clone(),
                        },
                    ),
                    ActionPersistenceCommand::StartAction {
                        action_id,
                        expected_version,
                    } => CommandIdentity::StartAction {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                    },
                    ActionPersistenceCommand::RejectPrepared { prepared_id, actor } => {
                        CommandIdentity::RejectPrepared {
                            prepared_id: prepared_id.clone(),
                            actor: *actor,
                        }
                    }
                    ActionPersistenceCommand::LinkCompletionEvidence {
                        action_id,
                        expected_version,
                        evidence_id,
                        evidence_classification,
                    } => CommandIdentity::LinkCompletionEvidence {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                        evidence_id: evidence_id.clone(),
                        evidence_classification: Some(*evidence_classification),
                    },
                    ActionPersistenceCommand::PrepareLowerClassification {
                        action_id,
                        expected_version,
                        proposed_classification,
                        rationale,
                    } => CommandIdentity::PrepareLowerActionClassification {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                        proposed_classification: *proposed_classification,
                        rationale: rationale.clone(),
                    },
                    ActionPersistenceCommand::ExecuteLowerClassification {
                        prepared_id,
                        actor,
                        acknowledged_digest,
                    } => CommandIdentity::ExecuteLowerActionClassification(
                        ApprovalBusinessIdentity {
                            prepared_id: prepared_id.clone(),
                            actor: *actor,
                            digest: acknowledged_digest.clone(),
                        },
                    ),
                    ActionPersistenceCommand::CreateRequestFromDecision {
                        request,
                        source_decision_id,
                        classification,
                    } => CommandIdentity::CreateRequestFromDecision {
                        request: request.clone(),
                        source_decision_id: source_decision_id.clone(),
                        classification: *classification,
                    },
                    ActionPersistenceCommand::MarkRequestSupersededPremise {
                        request_id,
                        expected_version,
                        source_decision_id,
                        classification,
                    } => CommandIdentity::MarkRequestSupersededPremise {
                        request_id: request_id.clone(),
                        expected_version: *expected_version,
                        source_decision_id: source_decision_id.clone(),
                        classification: *classification,
                    },
                    ActionPersistenceCommand::MarkActionSupersededPremise {
                        action_id,
                        expected_version,
                        source_decision_id,
                        classification,
                    } => CommandIdentity::MarkActionSupersededPremise {
                        action_id: action_id.clone(),
                        expected_version: *expected_version,
                        source_decision_id: source_decision_id.clone(),
                        classification: *classification,
                    },
                };
                let result = match &capsule.result {
                    ActionPersistenceResult::Request(outcome) => {
                        StoredResult::Request(outcome.clone())
                    }
                    ActionPersistenceResult::Action(outcome) => {
                        StoredResult::Action(outcome.clone())
                    }
                    ActionPersistenceResult::Prepared(intent) => {
                        StoredResult::Prepared(intent.clone())
                    }
                    ActionPersistenceResult::Accepted(outcome) => {
                        StoredResult::Accepted(outcome.clone())
                    }
                    ActionPersistenceResult::Rejected(outcome) => {
                        StoredResult::Rejected(outcome.clone())
                    }
                    ActionPersistenceResult::Terminal {
                        command: _terminal_command,
                        cause,
                        error,
                        prepared_disposition,
                        audit,
                    } => StoredResult::Terminal(ActionTerminalFailure {
                        cause: terminal_cause_to_service(cause),
                        persisted_cause: cause.clone(),
                        error: error.clone(),
                        prepared_disposition: *prepared_disposition,
                        audit: audit.clone(),
                    }),
                };
                (
                    capsule.idempotency_id.clone(),
                    Stored {
                        signature,
                        result,
                        original_correlation_id: capsule.original_correlation_id.clone(),
                        operation_ordinal: capsule.operation_ordinal,
                    },
                )
            })
            .collect();
        Self {
            clock,
            ids,
            authorization,
            policy,
            evidence_authority,
            state: Store {
                requests,
                actions,
                prepared,
                discarded_prepared: snapshot.discarded_prepared,
                idem,
                audit_events: snapshot.audits,
                next_operation_ordinal: u64::try_from(snapshot.replay.len()).unwrap_or(u64::MAX),
            },
            fail_next_commit: false,
        }
    }
    pub fn request(&self, id: &ActionRequestId) -> Option<&ActionRequestRecord> {
        self.state.requests.get(id)
    }
    pub fn action(&self, id: &ActionId) -> Option<&ActionRecord> {
        self.state.actions.get(id)
    }
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.state.audit_events
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
            evidence_authority: self.evidence_authority.clone(),
            state: self.state.clone(),
            fail_next_commit: false,
        }
    }
    pub fn discard_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let discarded = self.state.prepared.get(id).cloned();
        let removed_ordinal = self.state.idem.values().find_map(|stored| {
            matches!(&stored.result, StoredResult::Prepared(prepared) if prepared.id() == id)
                .then_some(stored.operation_ordinal)
        });
        let existed = self.state.prepared.remove(id).is_some();
        if existed {
            if let Some(discarded) = discarded {
                self.state.discarded_prepared.push(discarded);
            }
            self.state.idem.retain(|_, stored| {
                !matches!(&stored.result, StoredResult::Prepared(prepared) if prepared.id() == id)
            });
            if let Some(removed_ordinal) = removed_ordinal {
                for stored in self.state.idem.values_mut() {
                    if stored.operation_ordinal > removed_ordinal {
                        stored.operation_ordinal -= 1;
                    }
                }
                self.state.next_operation_ordinal =
                    self.state.next_operation_ordinal.saturating_sub(1);
            }
        }
        existed
    }
    pub fn action_requests_for_decision(
        &self,
        source_decision_id: &DecisionId,
    ) -> Vec<ActionRequestRecord> {
        decision_requests_from(&self.state, source_decision_id)
    }
    pub fn actions_for_decision(&self, source_decision_id: &DecisionId) -> Vec<ActionRecord> {
        decision_actions_from(&self.state, source_decision_id)
    }

    #[allow(dead_code)]
    pub(crate) fn begin_decision_stage(&self) -> ActionDecisionStage {
        ActionDecisionStage {
            state: self.state.clone(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn commit_decision_stage(
        &mut self,
        stage: ActionDecisionStage,
    ) -> Result<(), ActionServiceError> {
        self.finish(stage.state)
    }

    #[allow(dead_code)]
    pub(crate) fn preflight_decision_stage_commit(&mut self) -> Result<(), ActionServiceError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(ActionServiceError::InfrastructureFailure);
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn apply_decision_stage_unchecked(&mut self, stage: ActionDecisionStage) {
        self.state = stage.state;
    }

    #[allow(dead_code)]
    pub(crate) fn create_resulting_action_request_from_decision_transition(
        &mut self,
        stage: &mut ActionDecisionStage,
        spec: &DecisionResultingActionRequest,
        source_decision_id: DecisionId,
        decided_classification: DataClassification,
        context: ActionOperationContext,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        let classification = decided_classification.combine(spec.classification);
        let signature = CommandIdentity::CreateRequestFromDecision {
            request: spec.clone(),
            source_decision_id: source_decision_id.clone(),
            classification,
        };
        if let Some(record) =
            replay_request_from(&stage.state, &context.idempotency_id, &signature)?
        {
            return Ok(record);
        }
        if stage.state.requests.contains_key(&spec.id) {
            return Err(ActionServiceError::AlreadyExists);
        }
        let record = ActionRequestRecord {
            id: spec.id.clone(),
            title: spec.subject.clone(),
            details: spec.details.clone(),
            intended_owner: Some(spec.intended_owner.clone()),
            response_due_at: None,
            intended_action_due_at: Some(spec.due_at),
            classification,
            state: ActionRequestState::Open,
            terminal_rationale: None,
            linked_action_id: None,
            source_decision_id: Some(source_decision_id),
            superseded_premise: false,
            version: AggregateVersion::initial(),
        };
        self.stage_decision_request(
            stage,
            record,
            context,
            signature,
            "action_request.created_from_decision",
        )
    }

    #[allow(dead_code)]
    pub(crate) fn mark_action_request_superseded_premise_from_decision(
        &mut self,
        stage: &mut ActionDecisionStage,
        id: &ActionRequestId,
        expected_version: AggregateVersion,
        source_decision_id: &DecisionId,
        decided_classification: DataClassification,
        context: ActionOperationContext,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        let signature = CommandIdentity::MarkRequestSupersededPremise {
            request_id: id.clone(),
            expected_version,
            source_decision_id: source_decision_id.clone(),
            classification: decided_classification,
        };
        if let Some(record) =
            replay_request_from(&stage.state, &context.idempotency_id, &signature)?
        {
            return Ok(record);
        }
        let mut record = exact_request_from(&stage.state, id, expected_version)?.clone();
        if record.source_decision_id() != Some(source_decision_id)
            || !matches!(
                record.state,
                ActionRequestState::Draft | ActionRequestState::Open
            )
        {
            return Err(ActionServiceError::IllegalTransition);
        }
        record.classification = record.classification.combine(decided_classification);
        record.superseded_premise = true;
        record.version = next(record.version)?;
        self.stage_decision_request(
            stage,
            record,
            context,
            signature,
            "action_request.superseded_premise_marked",
        )
    }

    #[allow(dead_code)]
    pub(crate) fn mark_action_superseded_premise_from_decision(
        &mut self,
        stage: &mut ActionDecisionStage,
        id: &ActionId,
        expected_version: AggregateVersion,
        source_decision_id: &DecisionId,
        decided_classification: DataClassification,
        context: ActionOperationContext,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        let signature = CommandIdentity::MarkActionSupersededPremise {
            action_id: id.clone(),
            expected_version,
            source_decision_id: source_decision_id.clone(),
            classification: decided_classification,
        };
        if let Some(record) = replay_action_from(&stage.state, &context.idempotency_id, &signature)?
        {
            return Ok(record);
        }
        let mut record = exact_action_from(&stage.state, id, expected_version)?.clone();
        if record.source_decision_id() != Some(source_decision_id)
            || !matches!(record.state, ActionState::Open | ActionState::InProgress)
        {
            return Err(ActionServiceError::IllegalTransition);
        }
        record.classification = record.classification.combine(decided_classification);
        record.commitment_classification = record
            .commitment_classification
            .combine(decided_classification);
        record.superseded_premise = true;
        record.version = next(record.version)?;
        self.stage_decision_action(
            stage,
            record,
            context,
            signature,
            None,
            "action.superseded_premise_marked",
        )
    }

    fn stage_decision_request(
        &mut self,
        stage: &mut ActionDecisionStage,
        record: ActionRequestRecord,
        context: ActionOperationContext,
        signature: CommandIdentity,
        code: &str,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        let audit = self.audit(
            AuditTarget::ActionRequest(record.id.clone()),
            code,
            &context,
            true,
        )?;
        let outcome = ActionMutationOutcome {
            record: record.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        let operation_ordinal = allocate_operation_ordinal(&mut stage.state)?;
        stage.state.requests.insert(record.id.clone(), record);
        stage.state.audit_events.push(audit);
        stage.state.idem.insert(
            context.idempotency_id.clone(),
            Stored {
                signature,
                result: StoredResult::Request(outcome.clone()),
                original_correlation_id: context.correlation_id,
                operation_ordinal,
            },
        );
        Ok(outcome)
    }

    fn stage_decision_action(
        &mut self,
        stage: &mut ActionDecisionStage,
        record: ActionRecord,
        context: ActionOperationContext,
        signature: CommandIdentity,
        receipt: Option<ApprovalReceiptId>,
        code: &str,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        let audit = self.audit(AuditTarget::Action(record.id.clone()), code, &context, true)?;
        let outcome = ActionMutationOutcome {
            record: record.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: receipt,
        };
        let operation_ordinal = allocate_operation_ordinal(&mut stage.state)?;
        stage.state.actions.insert(record.id.clone(), record);
        stage.state.audit_events.push(audit);
        stage.state.idem.insert(
            context.idempotency_id.clone(),
            Stored {
                signature,
                result: StoredResult::Action(outcome.clone()),
                original_correlation_id: context.correlation_id,
                operation_ordinal,
            },
        );
        Ok(outcome)
    }

    pub fn create_action_request_draft(
        &mut self,
        command: CreateActionRequestDraft,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Request(command.id.clone());
        self.create_action_request_draft_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    pub fn submit_action_request(
        &mut self,
        command: SubmitActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Request(command.request_id.clone());
        self.submit_action_request_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    pub fn decline_action_request(
        &mut self,
        command: DeclineActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Request(command.request_id.clone());
        self.decline_action_request_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    pub fn withdraw_action_request(
        &mut self,
        command: WithdrawActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Request(command.request_id.clone());
        self.withdraw_action_request_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    pub fn start_action(
        &mut self,
        command: StartAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Action(command.action_id.clone());
        self.start_action_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    /// H2a rejection (v45): the Head of Products refuses a pending Accept/
    /// Complete/Cancel/Reopen preview. A successful command with no effect:
    /// the intent is consumed (it leaves `prepared` for `discarded_prepared`
    /// and can never be executed), one zero-effect audit is recorded, no
    /// receipt is minted. Rejecting an already-consumed intent (executed,
    /// terminally discarded or already rejected) is a conflict; an unknown
    /// intent is not found; an expired-but-unconsumed preview is rejected
    /// normally and the outcome says it had expired.
    pub fn reject_action_prepared_intent(
        &mut self,
        command: RejectActionPreparedIntent,
    ) -> Result<RejectedPreparedIntentOutcome, DomainError> {
        let context = command.context.clone();
        let target = self.prepared_intent_error_target(&command.prepared_id);
        self.reject_action_prepared_intent_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    fn prepared_intent_error_target(&self, prepared_id: &PreparedIntentId) -> ErrorTarget {
        let intent = self.state.prepared.get(prepared_id).or_else(|| {
            self.state
                .discarded_prepared
                .iter()
                .find(|intent| intent.id() == prepared_id)
        });
        match intent.map(WorkManagementPreparedIntent::operation) {
            Some(WorkManagementOperation::AcceptActionRequest { request_id, .. }) => {
                ErrorTarget::Request(request_id.clone())
            }
            Some(
                WorkManagementOperation::CompleteAction { action_id, .. }
                | WorkManagementOperation::CancelAction { action_id, .. }
                | WorkManagementOperation::ReopenAction { action_id, .. },
            ) => ErrorTarget::Action(action_id.clone()),
            _ => ErrorTarget::Unknown,
        }
    }
    fn reject_action_prepared_intent_cause(
        &mut self,
        c: RejectActionPreparedIntent,
    ) -> Result<RejectedPreparedIntentOutcome, ActionServiceError> {
        let signature = CommandIdentity::RejectPrepared {
            prepared_id: c.prepared_id.clone(),
            actor: c.actor,
        };
        if let Some(stored) = self.replay(&c.context.idempotency_id, &signature)? {
            return match stored {
                StoredResult::Rejected(outcome) => Ok(outcome.clone()),
                _ => Err(ActionServiceError::IdempotencyConflict),
            };
        }
        if c.actor != AuditActor::HeadOfProducts || !self.authorization.authorize(c.actor) {
            return Err(ActionServiceError::Unauthorized);
        }
        let Some(intent) = self.state.prepared.get(&c.prepared_id).cloned() else {
            let known = self
                .state
                .discarded_prepared
                .iter()
                .any(|intent| intent.id() == &c.prepared_id)
                || self.state.idem.values().any(|stored| {
                    matches!(&stored.result, StoredResult::Prepared(prepared) if prepared.id() == &c.prepared_id)
                });
            return Err(if known {
                ActionServiceError::IllegalTransition
            } else {
                ActionServiceError::NotFound
            });
        };
        let target =
            rejection_audit_target(&intent).ok_or(ActionServiceError::IllegalTransition)?;
        let now = self.clock.now();
        let audit_id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let audit = crate::work_management::prepared_intent_rejection_audit(
            audit_id,
            now,
            crate::work_management::ACTION_PREPARED_REJECTED_AUDIT_CODE,
            target,
            c.context.correlation_id.clone(),
        )
        .ok_or(ActionServiceError::InfrastructureFailure)?;
        let outcome = RejectedPreparedIntentOutcome::new(
            intent.id().clone(),
            now,
            now >= intent.preview().expires_at(),
            audit.clone(),
        );
        let mut staged = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut staged)?;
        staged.prepared.remove(&c.prepared_id);
        staged.discarded_prepared.push(intent);
        staged.audit_events.push(audit);
        staged.idem.insert(
            c.context.idempotency_id,
            Stored {
                signature,
                result: StoredResult::Rejected(outcome.clone()),
                original_correlation_id: c.context.correlation_id,
                operation_ordinal,
            },
        );
        self.finish(staged)?;
        Ok(outcome)
    }
    pub fn link_action_completion_evidence(
        &mut self,
        command: LinkActionCompletionEvidence,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Action(command.action_id.clone());
        self.link_action_completion_evidence_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    pub fn prepare_accept_action_request(
        &mut self,
        command: PrepareAcceptActionRequest,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = command.context.clone();
        let target = ErrorTarget::Request(command.request_id.clone());
        self.prepare_accept_action_request_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, target))
    }
    pub fn prepare_complete_action(
        &mut self,
        command: PrepareCompleteAction,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = command.context.clone();
        let action_id = command.action_id.clone();
        let expected_version = command.expected_version;
        let signature = CommandIdentity::PrepareComplete {
            action_id: action_id.clone(),
            expected_version,
            judgment: command.judgment.clone(),
        };
        let target = ErrorTarget::Action(action_id.clone());
        match self.replay_terminal_error(&context.idempotency_id, &signature) {
            Ok(Some(error)) => return Err(error),
            Err(cause) => return Err(self.domain_error(cause, &context, target)),
            Ok(None) => {}
        }
        match self.prepare_complete_action_cause(command) {
            Ok(value) => Ok(value),
            Err(cause) => {
                if is_h3_denial(cause) {
                    let denial = self.h3_denial_cause_for_action(&action_id, cause);
                    let cause = h3_terminal_service_cause(denial);
                    if let Err(audit_cause) = self.record_h3_denial(
                        AuditTarget::Action(action_id),
                        &context,
                        signature.clone(),
                        cause,
                        denial,
                    ) {
                        return Err(self.domain_error(audit_cause, &context, target));
                    }
                    return Err(self.domain_error(cause, &context, target));
                }
                Err(self.domain_error(cause, &context, target))
            }
        }
    }
    pub fn prepare_cancel_action(
        &mut self,
        command: PrepareCancelAction,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = command.context.clone();
        let action_id = command.action_id.clone();
        let expected_version = command.expected_version;
        let signature = CommandIdentity::PrepareCancel {
            action_id: action_id.clone(),
            expected_version,
            reason: command.reason.clone(),
        };
        let target = ErrorTarget::Action(action_id.clone());
        match self.replay_terminal_error(&context.idempotency_id, &signature) {
            Ok(Some(error)) => return Err(error),
            Err(cause) => return Err(self.domain_error(cause, &context, target)),
            Ok(None) => {}
        }
        match self.prepare_cancel_action_cause(command) {
            Ok(value) => Ok(value),
            Err(cause) => {
                if is_h3_denial(cause) {
                    let denial = self.h3_denial_cause_for_action(&action_id, cause);
                    let cause = h3_terminal_service_cause(denial);
                    if let Err(audit_cause) = self.record_h3_denial(
                        AuditTarget::Action(action_id),
                        &context,
                        signature.clone(),
                        cause,
                        denial,
                    ) {
                        return Err(self.domain_error(audit_cause, &context, target));
                    }
                    return Err(self.domain_error(cause, &context, target));
                }
                Err(self.domain_error(cause, &context, target))
            }
        }
    }
    pub fn prepare_reopen_action(
        &mut self,
        command: PrepareReopenAction,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = command.context.clone();
        let action_id = command.action_id.clone();
        let expected_version = command.expected_version;
        let mode = command.mode;
        let signature = CommandIdentity::PrepareReopen {
            action_id: action_id.clone(),
            expected_version,
            mode,
            reason: command.reason.clone(),
        };
        let target = ErrorTarget::Action(action_id.clone());
        match self.replay_terminal_error(&context.idempotency_id, &signature) {
            Ok(Some(error)) => return Err(error),
            Err(cause) => return Err(self.domain_error(cause, &context, target)),
            Ok(None) => {}
        }
        match self.prepare_reopen_action_cause(command) {
            Ok(value) => Ok(value),
            Err(cause) => {
                if is_h3_denial(cause) {
                    let denial = self.h3_denial_cause_for_action(&action_id, cause);
                    let cause = h3_terminal_service_cause(denial);
                    if let Err(audit_cause) = self.record_h3_denial(
                        AuditTarget::Action(action_id),
                        &context,
                        signature.clone(),
                        cause,
                        denial,
                    ) {
                        return Err(self.domain_error(audit_cause, &context, target));
                    }
                    return Err(self.domain_error(cause, &context, target));
                }
                Err(self.domain_error(cause, &context, target))
            }
        }
    }
    pub fn prepare_lower_action_classification(
        &mut self,
        command: PrepareLowerActionClassification,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let context = command.context.clone();
        let action_id = command.action_id.clone();
        let expected_version = command.expected_version;
        let signature = CommandIdentity::PrepareLowerActionClassification {
            action_id: action_id.clone(),
            expected_version,
            proposed_classification: command.proposed_classification,
            rationale: command.rationale.clone(),
        };
        let target = ErrorTarget::Action(action_id.clone());
        match self.replay_terminal_error(&context.idempotency_id, &signature) {
            Ok(Some(error)) => return Err(error),
            Err(cause) => return Err(self.domain_error(cause, &context, target)),
            Ok(None) => {}
        }
        match self.prepare_lower_action_classification_cause(command) {
            Ok(value) => Ok(value),
            Err(cause) => {
                if is_h3_denial(cause) {
                    let denial = self.h3_denial_cause_for_action(&action_id, cause);
                    let cause = h3_terminal_service_cause(denial);
                    if let Err(audit_cause) = self.record_h3_denial(
                        AuditTarget::Action(action_id),
                        &context,
                        signature.clone(),
                        cause,
                        denial,
                    ) {
                        return Err(self.domain_error(audit_cause, &context, target));
                    }
                    return Err(self.domain_error(cause, &context, target));
                }
                Err(self.domain_error(cause, &context, target))
            }
        }
    }
    pub fn approve_and_execute_accept_action_request(
        &mut self,
        command: ApproveAndExecuteAcceptActionRequest,
    ) -> Result<AcceptedActionOutcome, DomainError> {
        let context = command.context.clone();
        let approval = command.approval.clone();
        match self.approve_and_execute_accept_action_request_cause(command) {
            Ok(value) => Ok(value),
            Err(cause) => {
                if !is_retryable_execution_failure(cause) {
                    self.record_h2_failure(&approval, &context, cause)
                        .map_err(|audit_cause| {
                            self.domain_error(audit_cause, &context, self.failure_target(&approval))
                        })?;
                } else if cause == ActionServiceError::PreparedIntentChanged {
                    self.record_retryable_preview_failure(&approval, &context, cause)
                        .map_err(|audit_cause| {
                            self.domain_error(audit_cause, &context, self.failure_target(&approval))
                        })?;
                }
                Err(self.domain_error(cause, &context, self.failure_target(&approval)))
            }
        }
    }
    pub fn approve_and_execute_complete_action(
        &mut self,
        command: ApproveAndExecuteCompleteAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let context = command.context.clone();
        let approval = command.approval.clone();
        self.execute_action_command(
            command,
            approval,
            context,
            PreparedActionTransitionKind::Complete,
        )
    }
    pub fn approve_and_execute_lower_action_classification(
        &mut self,
        command: ApproveAndExecuteLowerActionClassification,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        // Deliberately does NOT reuse `failed_action_execution` /
        // `record_h2_failure` / `record_retryable_preview_failure`: those
        // helpers hardcode a `CommandIdentity::ExecuteAccept` or
        // `ExecuteAction(self.failure_transition_kind(approval), ..)`
        // signature when persisting a terminal failure for retry/replay,
        // and `failure_transition_kind` only recognizes
        // Cancel/Reopen/Complete's operations -- reusing them here would
        // silently persist a *wrong* signature for this operation's
        // failures. A failed attempt here is simply not remembered for
        // fast-path replay (it re-validates on retry instead), which is a
        // safe simplification, not a partial correctness bug.
        let context = command.context.clone();
        let approval = command.approval.clone();
        self.approve_and_execute_lower_action_classification_cause(command)
            .map_err(|cause| self.domain_error(cause, &context, self.failure_target(&approval)))
    }
    pub fn approve_and_execute_cancel_action(
        &mut self,
        command: ApproveAndExecuteCancelAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let context = command.context.clone();
        let approval = command.approval.clone();
        match self.approve_and_execute_cancel_action_cause(command) {
            Ok(v) => Ok(v),
            Err(cause) => self.failed_action_execution(&approval, &context, cause),
        }
    }
    pub fn approve_and_execute_reopen_action(
        &mut self,
        command: ApproveAndExecuteReopenAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let context = command.context.clone();
        let approval = command.approval.clone();
        match self.approve_and_execute_reopen_action_cause(command) {
            Ok(v) => Ok(v),
            Err(cause) => self.failed_action_execution(&approval, &context, cause),
        }
    }

    fn execute_action_command(
        &mut self,
        command: ApproveAndExecuteCompleteAction,
        approval: WorkManagementApproval,
        context: ActionOperationContext,
        _: PreparedActionTransitionKind,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        match self.approve_and_execute_complete_action_cause(command) {
            Ok(v) => Ok(v),
            Err(cause) => self.failed_action_execution(&approval, &context, cause),
        }
    }
    fn failed_action_execution(
        &mut self,
        approval: &WorkManagementApproval,
        context: &ActionOperationContext,
        cause: ActionServiceError,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        if !is_retryable_execution_failure(cause) {
            self.record_h2_failure(approval, context, cause)
                .map_err(|audit_cause| {
                    self.domain_error(audit_cause, context, self.failure_target(approval))
                })?;
        } else if cause == ActionServiceError::PreparedIntentChanged {
            self.record_retryable_preview_failure(approval, context, cause)
                .map_err(|audit_cause| {
                    self.domain_error(audit_cause, context, self.failure_target(approval))
                })?;
        }
        Err(self.domain_error(cause, context, self.failure_target(approval)))
    }

    fn create_action_request_draft_cause(
        &mut self,
        c: CreateActionRequestDraft,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        let sig = CommandIdentity::CreateRequest {
            id: c.id.clone(),
            title: c.title.clone(),
            details: c.details.clone(),
            intended_owner: c.intended_owner.clone(),
            response_due_at: c.response_due_at,
            intended_action_due_at: c.intended_action_due_at,
            classification: c.classification,
        };
        if let Some(r) = self.replay_request(&c.context.idempotency_id, &sig)? {
            return Ok(r);
        }
        if self.state.requests.contains_key(&c.id) {
            return Err(ActionServiceError::AlreadyExists);
        }
        let r = ActionRequestRecord {
            id: c.id.clone(),
            title: c.title,
            details: c.details,
            intended_owner: c.intended_owner,
            response_due_at: c.response_due_at,
            intended_action_due_at: c.intended_action_due_at,
            classification: c.classification,
            state: ActionRequestState::Draft,
            terminal_rationale: None,
            linked_action_id: None,
            source_decision_id: None,
            superseded_premise: false,
            version: AggregateVersion::initial(),
        };
        self.store_request(r, c.context, sig, "action_request.created")
    }
    fn submit_action_request_cause(
        &mut self,
        c: SubmitActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        self.request_transition(
            c.request_id,
            c.expected_version,
            None,
            ActionRequestState::Draft,
            ActionRequestState::Open,
            c.context,
        )
    }
    fn decline_action_request_cause(
        &mut self,
        c: DeclineActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        self.request_transition(
            c.request_id,
            c.expected_version,
            Some(c.rationale),
            ActionRequestState::Open,
            ActionRequestState::Declined,
            c.context,
        )
    }
    fn withdraw_action_request_cause(
        &mut self,
        c: WithdrawActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        self.request_transition(
            c.request_id,
            c.expected_version,
            Some(c.rationale),
            ActionRequestState::Open,
            ActionRequestState::Withdrawn,
            c.context,
        )
    }
    fn start_action_cause(
        &mut self,
        c: StartAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        let sig = CommandIdentity::StartAction {
            action_id: c.action_id.clone(),
            expected_version: c.expected_version,
        };
        if let Some(r) = self.replay_action(&c.context.idempotency_id, &sig)? {
            return Ok(r);
        }
        let mut r = self.exact_action(&c.action_id, c.expected_version)?.clone();
        if r.state != ActionState::Open {
            return Err(ActionServiceError::IllegalTransition);
        }
        r.transition_history.push(ActionTransitionRecord {
            from: r.state,
            to: ActionState::InProgress,
            reason: None,
            occurred_at: self.clock.now(),
            support: None,
            approval_receipt_id: None,
        });
        r.state = ActionState::InProgress;
        r.version = next(r.version)?;
        self.store_action(r, c.context, sig, None, "action.started")
    }
    fn link_action_completion_evidence_cause(
        &mut self,
        c: LinkActionCompletionEvidence,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        let sig = CommandIdentity::LinkCompletionEvidence {
            action_id: c.action_id.clone(),
            expected_version: c.expected_version,
            evidence_id: c.evidence_id.clone(),
            evidence_classification: None,
        };
        if let Some(r) = self.replay_link_action(&c.context.idempotency_id, &sig)? {
            return Ok(r);
        }
        let mut r = self.exact_action(&c.action_id, c.expected_version)?.clone();
        if r.state != ActionState::InProgress {
            return Err(ActionServiceError::IllegalTransition);
        }
        let evidence = self.resolve_evidence(&c.evidence_id)?;
        if evidence.role() != crate::work_management::EvidenceRole::ActionCompletion {
            return Err(ActionServiceError::InvalidEvidence);
        }
        if r.completion_evidence.iter().any(|e| e == &c.evidence_id) {
            return Err(ActionServiceError::AlreadyExists);
        }
        let evidence_classification = evidence.classification();
        r.classification = r.classification.combine(evidence_classification);
        r.completion_evidence.push(c.evidence_id);
        r.version = next(r.version)?;
        let sig = CommandIdentity::LinkCompletionEvidence {
            action_id: c.action_id,
            expected_version: c.expected_version,
            evidence_id: r
                .completion_evidence
                .last()
                .cloned()
                .ok_or(ActionServiceError::InfrastructureFailure)?,
            evidence_classification: Some(evidence_classification),
        };
        self.store_action(r, c.context, sig, None, "action.completion_evidence_linked")
    }

    fn prepare_accept_action_request_cause(
        &mut self,
        c: PrepareAcceptActionRequest,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let signature = CommandIdentity::PrepareAccept {
            request_id: c.request_id.clone(),
            expected_version: c.expected_version,
        };
        if let Some(prepared) = self.replay_prepared(&c.context.idempotency_id, &signature)? {
            return Ok(prepared);
        }
        let idempotency_id = c.context.idempotency_id.clone();
        let original_correlation_id = c.context.correlation_id.clone();
        let r = self.exact_request(&c.request_id, c.expected_version)?;
        if r.state != ActionRequestState::Open {
            return Err(ActionServiceError::IllegalTransition);
        }
        if r.intended_owner.is_none() {
            return Err(ActionServiceError::MissingOwner);
        }
        if r.intended_action_due_at.is_none() {
            return Err(ActionServiceError::MissingDueDate);
        }
        let classification = r.classification;
        let subject = r.title.clone();
        let details = r.details.clone();
        let owner = r
            .intended_owner
            .clone()
            .ok_or(ActionServiceError::MissingOwner)?;
        let due = r
            .intended_action_due_at
            .ok_or(ActionServiceError::MissingDueDate)?;
        let action_id = self
            .ids
            .next_action_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        if self.state.actions.contains_key(&action_id) {
            return Err(ActionServiceError::AlreadyExists);
        }
        let prepared = self.prepare(
            WorkManagementOperation::AcceptActionRequest {
                request_id: c.request_id,
                request_version: c.expected_version,
                action_id,
                action_classification: classification,
                action_subject: subject,
                commitment_details: details,
                intended_owner: owner,
                intended_due_at: due,
            },
            classification,
            None,
        )?;
        self.store_prepared(prepared, idempotency_id, original_correlation_id, signature)
    }
    fn prepare_complete_action_cause(
        &mut self,
        c: PrepareCompleteAction,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let signature = CommandIdentity::PrepareComplete {
            action_id: c.action_id.clone(),
            expected_version: c.expected_version,
            judgment: c.judgment.clone(),
        };
        if let Some(prepared) = self.replay_prepared(&c.context.idempotency_id, &signature)? {
            return Ok(prepared);
        }
        let idempotency_id = c.context.idempotency_id.clone();
        let original_correlation_id = c.context.correlation_id.clone();
        let r = self.exact_action(&c.action_id, c.expected_version)?;
        if r.state != ActionState::InProgress {
            return Err(ActionServiceError::IllegalTransition);
        }
        if r.completion_evidence.is_empty() {
            return Err(ActionServiceError::EvidencePolicyDenied);
        }
        let (current_classification, _bindings, evidence) =
            self.authoritative_action_snapshot(r)
                .map_err(|_| ActionServiceError::EvidencePolicyDenied)?;
        let mut evidence = evidence;
        evidence.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        let support = EvidenceOrJudgment::new(evidence, c.judgment.into_iter().collect())
            .map_err(|_| ActionServiceError::EvidencePolicyDenied)?
            .evaluate_evidence_required()
            .map_err(|_| ActionServiceError::EvidencePolicyDenied)?;
        let prepared = self.prepare(
            WorkManagementOperation::CompleteAction {
                action_id: c.action_id,
                action_version: c.expected_version,
            },
            current_classification.combine(support.classification()),
            Some(support),
        )?;
        self.store_prepared(prepared, idempotency_id, original_correlation_id, signature)
    }
    fn prepare_cancel_action_cause(
        &mut self,
        c: PrepareCancelAction,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let signature = CommandIdentity::PrepareCancel {
            action_id: c.action_id.clone(),
            expected_version: c.expected_version,
            reason: c.reason.clone(),
        };
        if let Some(prepared) = self.replay_prepared(&c.context.idempotency_id, &signature)? {
            return Ok(prepared);
        }
        let idempotency_id = c.context.idempotency_id.clone();
        let original_correlation_id = c.context.correlation_id.clone();
        let r = self.exact_action(&c.action_id, c.expected_version)?;
        if !matches!(r.state, ActionState::Open | ActionState::InProgress) {
            return Err(ActionServiceError::IllegalTransition);
        }
        let (current_classification, bindings, _) = self.authoritative_action_snapshot(r)?;
        if r.classification != DataClassification::Unclassified
            && r.classification.combine(current_classification) != current_classification
        {
            return Err(ActionServiceError::ClassificationUnresolved);
        }
        let prepared = self.prepare(
            WorkManagementOperation::CancelAction {
                action_id: c.action_id,
                action_version: c.expected_version,
                reason: c.reason,
                evidence_classifications: bindings,
            },
            current_classification,
            None,
        )?;
        self.store_prepared(prepared, idempotency_id, original_correlation_id, signature)
    }
    fn prepare_reopen_action_cause(
        &mut self,
        c: PrepareReopenAction,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let signature = CommandIdentity::PrepareReopen {
            action_id: c.action_id.clone(),
            expected_version: c.expected_version,
            mode: c.mode,
            reason: c.reason.clone(),
        };
        if let Some(prepared) = self.replay_prepared(&c.context.idempotency_id, &signature)? {
            return Ok(prepared);
        }
        let idempotency_id = c.context.idempotency_id.clone();
        let original_correlation_id = c.context.correlation_id.clone();
        let r = self.exact_action(&c.action_id, c.expected_version)?;
        if !matches!(
            (r.state, c.mode),
            (ActionState::Completed, ActionReopenMode::ReopenCompleted)
                | (ActionState::Cancelled, ActionReopenMode::RestartCancelled)
        ) {
            return Err(ActionServiceError::InvalidReopenMode);
        }
        let (current_classification, bindings, _) = self.authoritative_action_snapshot(r)?;
        if r.classification != DataClassification::Unclassified
            && r.classification.combine(current_classification) != current_classification
        {
            return Err(ActionServiceError::ClassificationUnresolved);
        }
        let prepared = self.prepare(
            WorkManagementOperation::ReopenAction {
                action_id: c.action_id,
                action_version: c.expected_version,
                mode: c.mode,
                reason: c.reason,
                evidence_classifications: bindings,
            },
            current_classification,
            None,
        )?;
        self.store_prepared(prepared, idempotency_id, original_correlation_id, signature)
    }

    fn prepare_lower_action_classification_cause(
        &mut self,
        c: PrepareLowerActionClassification,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let signature = CommandIdentity::PrepareLowerActionClassification {
            action_id: c.action_id.clone(),
            expected_version: c.expected_version,
            proposed_classification: c.proposed_classification,
            rationale: c.rationale.clone(),
        };
        if let Some(prepared) = self.replay_prepared(&c.context.idempotency_id, &signature)? {
            return Ok(prepared);
        }
        let idempotency_id = c.context.idempotency_id.clone();
        let original_correlation_id = c.context.correlation_id.clone();
        let r = self.exact_action(&c.action_id, c.expected_version)?;
        if !is_genuine_lowering(r.classification, c.proposed_classification) {
            return Err(ActionServiceError::NotALowering);
        }
        let current_classification = r.classification;
        let prepared = self.prepare(
            WorkManagementOperation::LowerActionClassification {
                action_id: c.action_id,
                action_version: c.expected_version,
                current_classification,
                proposed_classification: c.proposed_classification,
                rationale: c.rationale,
            },
            current_classification,
            None,
        )?;
        self.store_prepared(prepared, idempotency_id, original_correlation_id, signature)
    }

    fn approve_and_execute_accept_action_request_cause(
        &mut self,
        c: ApproveAndExecuteAcceptActionRequest,
    ) -> Result<AcceptedActionOutcome, ActionServiceError> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(ActionServiceError::IdempotencyConflict);
        }
        let sig = CommandIdentity::ExecuteAccept(approval_business_identity(&c.approval));
        if let Some(r) = self.replay_accepted(&c.context.idempotency_id, &sig)? {
            return Ok(r);
        }
        if let Some(discarded) = self
            .state
            .discarded_prepared
            .iter()
            .find(|intent| intent.id() == c.approval.prepared_id())
        {
            if discarded.payload_digest() != c.approval.acknowledged_payload_digest() {
                return Err(ActionServiceError::DigestMismatch);
            }
        }
        let p = self.pending(&c.approval)?.clone();
        let (rid, rv, aid) = match p.operation() {
            WorkManagementOperation::AcceptActionRequest {
                request_id,
                request_version,
                action_id,
                ..
            } => (request_id.clone(), *request_version, action_id.clone()),
            _ => return Err(ActionServiceError::PreparedIntentChanged),
        };
        let mut request = self
            .exact_request(&rid, rv)
            .map_err(map_post_prepare_state)?
            .clone();
        if request.state != ActionRequestState::Open {
            return Err(ActionServiceError::IllegalTransition);
        }
        let owner = request
            .intended_owner
            .clone()
            .ok_or(ActionServiceError::MissingOwner)?;
        let due = request
            .intended_action_due_at
            .ok_or(ActionServiceError::MissingDueDate)?;
        if self.state.actions.contains_key(&aid) {
            return Err(ActionServiceError::AlreadyExists);
        }
        let current_accept = WorkManagementOperation::AcceptActionRequest {
            request_id: request.id.clone(),
            request_version: request.version,
            action_id: aid.clone(),
            action_classification: request.classification,
            action_subject: request.title.clone(),
            commitment_details: request.details.clone(),
            intended_owner: owner.clone(),
            intended_due_at: due,
        };
        let receipt = self.validate(
            &p,
            &c.approval,
            current_accept,
            request.classification,
            None,
        )?;
        request.state = ActionRequestState::Accepted;
        request.linked_action_id = Some(aid.clone());
        request.version = next(request.version)?;
        let action = ActionRecord {
            id: aid,
            source_request_id: rid,
            title: request.title.clone(),
            details: request.details.clone(),
            owner,
            due_at: due,
            classification: p.classification(),
            commitment_classification: request.classification,
            state: ActionState::Open,
            completion_evidence: vec![],
            support: None,
            transition_reason: None,
            transition_history: vec![],
            source_decision_id: request.source_decision_id.clone(),
            superseded_premise: request.superseded_premise,
            version: AggregateVersion::initial(),
        };
        let audits = vec![
            self.audit(
                AuditTarget::ActionRequest(request.id.clone()),
                "action_request.accepted",
                &c.context,
                true,
            )?,
            self.audit(
                AuditTarget::Action(action.id.clone()),
                "action.created_from_request",
                &c.context,
                true,
            )?,
            self.audit(
                AuditTarget::ActionRequest(request.id.clone()),
                "action_request.action_linked",
                &c.context,
                true,
            )?,
        ];
        let result = AcceptedActionOutcome {
            request: request.clone(),
            action: action.clone(),
            audit_events: audits.clone(),
            approval_receipt_id: receipt.id.clone(),
        };
        let mut staged = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut staged)?;
        staged.requests.insert(request.id.clone(), request);
        staged.actions.insert(action.id.clone(), action);
        staged.prepared.remove(c.approval.prepared_id());
        staged.audit_events.extend(audits);
        staged.idem.insert(
            c.context.idempotency_id.clone(),
            Stored {
                signature: sig,
                result: StoredResult::Accepted(result.clone()),
                original_correlation_id: c.context.correlation_id,
                operation_ordinal,
            },
        );
        self.finish(staged)?;
        Ok(result)
    }
    fn approve_and_execute_complete_action_cause(
        &mut self,
        c: ApproveAndExecuteCompleteAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        self.execute_h2(
            c.approval,
            c.context,
            PreparedActionTransitionKind::Complete,
        )
    }
    fn approve_and_execute_cancel_action_cause(
        &mut self,
        c: ApproveAndExecuteCancelAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        self.execute_h2(c.approval, c.context, PreparedActionTransitionKind::Cancel)
    }
    fn approve_and_execute_reopen_action_cause(
        &mut self,
        c: ApproveAndExecuteReopenAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        self.execute_h2(c.approval, c.context, PreparedActionTransitionKind::Reopen)
    }

    /// H2a step 2 for Lower Data Classification. Deliberately independent
    /// of `execute_h2` -- see `PrepareLowerActionClassification`'s doc
    /// comment -- but reuses every other internal seam (`pending`,
    /// `exact_action`, `validate`, `audit`, `finish`) exactly as `execute_h2`
    /// does, so it stays consistent with the rest of this service.
    fn approve_and_execute_lower_action_classification_cause(
        &mut self,
        c: ApproveAndExecuteLowerActionClassification,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        if c.approval.idempotency_id() != &c.context.idempotency_id {
            return Err(ActionServiceError::IdempotencyConflict);
        }
        let sig = CommandIdentity::ExecuteLowerActionClassification(approval_business_identity(
            &c.approval,
        ));
        if let Some(r) = self.replay_action(&c.context.idempotency_id, &sig)? {
            return Ok(r);
        }
        let p = self.pending(&c.approval)?.clone();
        let (action_id, action_version, proposed_classification, rationale) = match p.operation() {
            WorkManagementOperation::LowerActionClassification {
                action_id,
                action_version,
                proposed_classification,
                rationale,
                ..
            } => (
                action_id.clone(),
                *action_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => return Err(ActionServiceError::PreparedIntentChanged),
        };
        let r = self
            .exact_action(&action_id, action_version)
            .map_err(map_post_prepare_state)?
            .clone();
        let current_classification = r.classification;
        let current_op = WorkManagementOperation::LowerActionClassification {
            action_id: action_id.clone(),
            action_version: r.version,
            current_classification,
            proposed_classification,
            rationale,
        };
        let receipt = self.validate(&p, &c.approval, current_op, current_classification, None)?;
        let mut mutated = r;
        mutated.version = next(mutated.version)?;
        mutated.classification = proposed_classification;
        let audit = self.audit(
            AuditTarget::Action(action_id.clone()),
            "action.classification_lowered",
            &c.context,
            true,
        )?;
        let outcome = ActionMutationOutcome {
            record: mutated.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: Some(receipt.id.clone()),
        };
        let mut staged = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut staged)?;
        staged.actions.insert(action_id, mutated);
        staged.prepared.remove(c.approval.prepared_id());
        staged.audit_events.push(audit);
        staged.idem.insert(
            c.context.idempotency_id.clone(),
            Stored {
                signature: sig,
                result: StoredResult::Action(outcome.clone()),
                original_correlation_id: c.context.correlation_id,
                operation_ordinal,
            },
        );
        self.finish(staged)?;
        Ok(outcome)
    }

    fn execute_h2(
        &mut self,
        a: WorkManagementApproval,
        c: ActionOperationContext,
        kind: PreparedActionTransitionKind,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        if a.idempotency_id() != &c.idempotency_id {
            return Err(ActionServiceError::IdempotencyConflict);
        }
        let sig = CommandIdentity::ExecuteAction(kind, approval_business_identity(&a));
        if let Some(r) = self.replay_action(&c.idempotency_id, &sig)? {
            return Ok(r);
        }
        let p = self.pending(&a)?.clone();
        let (id, v, reason) = match (kind, p.operation()) {
            (
                PreparedActionTransitionKind::Complete,
                WorkManagementOperation::CompleteAction {
                    action_id,
                    action_version,
                },
            ) => (action_id.clone(), *action_version, None),
            (
                PreparedActionTransitionKind::Cancel,
                WorkManagementOperation::CancelAction {
                    action_id,
                    action_version,
                    reason,
                    ..
                },
            ) => (
                action_id.clone(),
                *action_version,
                Some(to_details(reason)?),
            ),
            (
                PreparedActionTransitionKind::Reopen,
                WorkManagementOperation::ReopenAction {
                    action_id,
                    action_version,
                    reason,
                    ..
                },
            ) => (
                action_id.clone(),
                *action_version,
                Some(to_details(reason)?),
            ),
            _ => return Err(ActionServiceError::PreparedIntentChanged),
        };
        let mut r = self
            .exact_action(&id, v)
            .map_err(map_post_prepare_state)?
            .clone();
        let legal = match kind {
            PreparedActionTransitionKind::Complete => r.state == ActionState::InProgress,
            PreparedActionTransitionKind::Cancel => {
                matches!(r.state, ActionState::Open | ActionState::InProgress)
            }
            PreparedActionTransitionKind::Reopen => {
                matches!(r.state, ActionState::Completed | ActionState::Cancelled)
            }
        };
        if !legal {
            return Err(ActionServiceError::IllegalTransition);
        }
        let (current_classification, current_bindings, evidence) = self
            .authoritative_action_snapshot(&r)
            .map_err(|_| ActionServiceError::PreparedIntentChanged)?;
        let prepared_bindings = match p.operation() {
            WorkManagementOperation::CancelAction {
                evidence_classifications,
                ..
            }
            | WorkManagementOperation::ReopenAction {
                evidence_classifications,
                ..
            } => Some(evidence_classifications.as_slice()),
            _ => None,
        };
        if prepared_bindings.is_some_and(|bindings| bindings != current_bindings.as_slice()) {
            return Err(ActionServiceError::PreparedIntentChanged);
        }
        let current_support = if kind == PreparedActionTransitionKind::Complete {
            let judgments = p
                .preview()
                .support()
                .map_or_else(Vec::new, |support| support.judgments().to_vec());
            let support = EvidenceOrJudgment::new(evidence, judgments)
                .map_err(|_| ActionServiceError::PreparedIntentChanged)?
                .evaluate_evidence_required()
                .map_err(|_| ActionServiceError::PreparedIntentChanged)?;
            Some(support)
        } else {
            None
        };
        let receipt = self.validate(
            &p,
            &a,
            p.operation().clone(),
            if kind == PreparedActionTransitionKind::Complete {
                p.classification()
            } else {
                current_classification
            },
            current_support,
        )?;
        let target_state = match kind {
            PreparedActionTransitionKind::Complete => ActionState::Completed,
            PreparedActionTransitionKind::Cancel => ActionState::Cancelled,
            PreparedActionTransitionKind::Reopen => ActionState::InProgress,
        };
        r.transition_history.push(ActionTransitionRecord {
            from: r.state,
            to: target_state,
            reason: reason.clone(),
            occurred_at: self.clock.now(),
            support: p.preview().support().cloned(),
            approval_receipt_id: Some(receipt.id.clone()),
        });
        r.state = target_state;
        r.transition_reason = reason;
        if let Some(support) = p.preview().support().cloned() {
            r.support = Some(support);
        }
        r.classification = if kind == PreparedActionTransitionKind::Complete {
            current_classification.combine(p.preview().support().map_or(
                DataClassification::Unclassified,
                SupportWitness::classification,
            ))
        } else {
            current_classification
        };
        r.version = next(r.version)?;
        let audit = self.audit(
            AuditTarget::Action(id.clone()),
            match kind {
                PreparedActionTransitionKind::Complete => "action.completed",
                PreparedActionTransitionKind::Cancel => "action.cancelled",
                PreparedActionTransitionKind::Reopen => "action.reopened",
            },
            &c,
            true,
        )?;
        let outcome = ActionMutationOutcome {
            record: r.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: Some(receipt.id.clone()),
        };
        let mut staged = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut staged)?;
        staged.actions.insert(id, r);
        staged.prepared.remove(a.prepared_id());
        staged.audit_events.push(audit);
        staged.idem.insert(
            c.idempotency_id.clone(),
            Stored {
                signature: sig,
                result: StoredResult::Action(outcome.clone()),
                original_correlation_id: c.correlation_id,
                operation_ordinal,
            },
        );
        self.finish(staged)?;
        Ok(outcome)
    }
    fn request_transition(
        &mut self,
        id: ActionRequestId,
        v: AggregateVersion,
        reason: Option<ActionDetails>,
        from: ActionRequestState,
        to: ActionRequestState,
        c: ActionOperationContext,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        let sig = CommandIdentity::TransitionRequest {
            request_id: id.clone(),
            expected_version: v,
            target_state: to,
            rationale: reason.clone(),
        };
        if let Some(r) = self.replay_request(&c.idempotency_id, &sig)? {
            return Ok(r);
        }
        let mut r = self.exact_request(&id, v)?.clone();
        if r.state != from {
            return Err(ActionServiceError::IllegalTransition);
        }
        r.state = to;
        r.terminal_rationale = reason;
        r.version = next(r.version)?;
        let code = match to {
            ActionRequestState::Open => "action_request.submitted",
            ActionRequestState::Declined => "action_request.declined",
            ActionRequestState::Withdrawn => "action_request.withdrawn",
            ActionRequestState::Draft | ActionRequestState::Accepted => "action_request.mutated",
        };
        self.store_request(r, c, sig, code)
    }
    fn prepare(
        &mut self,
        op: WorkManagementOperation,
        class: DataClassification,
        support: Option<SupportWitness>,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let id = self
            .ids
            .next_prepared_intent_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let p = WorkManagementPreparedIntent::prepare(id, op, class, support, self.clock.now())
            .map_err(map_prepare)?;
        Ok(p)
    }
    fn pending(
        &self,
        a: &WorkManagementApproval,
    ) -> Result<&WorkManagementPreparedIntent, ActionServiceError> {
        self.state
            .prepared
            .get(a.prepared_id())
            .ok_or(ActionServiceError::PreparedIntentNotFound)
    }
    fn validate(
        &mut self,
        p: &WorkManagementPreparedIntent,
        a: &WorkManagementApproval,
        current_operation: WorkManagementOperation,
        target_classification: DataClassification,
        current_support: Option<SupportWitness>,
    ) -> Result<crate::work_management::WorkManagementApprovalReceipt, ActionServiceError> {
        if self.policy.current_policy(p.operation()) == ActionExecutionPolicy::Denied {
            return Err(ActionServiceError::PolicyDenied);
        }
        let current_prepared = WorkManagementPreparedIntent::prepare(
            p.id().clone(),
            current_operation,
            target_classification,
            current_support,
            self.clock.now(),
        )
        .map_err(map_prepare)?;
        let s = WorkManagementAuthoritativeSnapshot {
            operation: current_prepared.operation().clone(),
            classification: current_prepared.classification(),
            classification_sources: current_prepared.preview().classification_sources().to_vec(),
            support: current_prepared.preview().support().cloned(),
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let id = self
            .ids
            .next_approval_receipt_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        validate_and_mint_work_management_h2a_receipt(
            p,
            a,
            &s,
            self.clock.now(),
            id,
            &self.authorization,
        )
        .map_err(map_validation)
    }
    fn exact_request(
        &self,
        id: &ActionRequestId,
        v: AggregateVersion,
    ) -> Result<&ActionRequestRecord, ActionServiceError> {
        let r = self
            .state
            .requests
            .get(id)
            .ok_or(ActionServiceError::NotFound)?;
        if r.version != v {
            return Err(ActionServiceError::StaleVersion);
        }
        Ok(r)
    }
    fn exact_action(
        &self,
        id: &ActionId,
        v: AggregateVersion,
    ) -> Result<&ActionRecord, ActionServiceError> {
        let r = self
            .state
            .actions
            .get(id)
            .ok_or(ActionServiceError::NotFound)?;
        if r.version != v {
            return Err(ActionServiceError::StaleVersion);
        }
        Ok(r)
    }
    fn resolve_evidence(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionServiceError> {
        self.evidence_authority
            .resolve(id)
            .map_err(|error| match error {
                ActionEvidenceAuthorityError::Unavailable => {
                    ActionServiceError::EvidenceUnavailable
                }
                ActionEvidenceAuthorityError::NotFound => ActionServiceError::InvalidEvidence,
            })
    }
    fn resolve_action_evidence(
        &self,
        action: &ActionRecord,
    ) -> Result<Vec<EvidenceReferenceMetadata>, ActionServiceError> {
        action
            .completion_evidence
            .iter()
            .map(|id| self.resolve_evidence(id))
            .collect()
    }
    fn authoritative_action_snapshot(
        &self,
        action: &ActionRecord,
    ) -> Result<
        (
            DataClassification,
            Vec<EvidenceClassificationBinding>,
            Vec<EvidenceReferenceMetadata>,
        ),
        ActionServiceError,
    > {
        if action.commitment_classification == DataClassification::Unclassified {
            return Err(ActionServiceError::ClassificationUnresolved);
        }
        let evidence = self.resolve_action_evidence(action)?;
        if evidence
            .iter()
            .any(|item| item.classification() == DataClassification::Unclassified)
        {
            return Err(ActionServiceError::ClassificationUnresolved);
        }
        let mut evidence = evidence;
        evidence.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        let base_classification = if action.support.is_some() {
            action.classification
        } else {
            action.commitment_classification
        };
        let classification = evidence.iter().fold(base_classification, |current, item| {
            current.combine(item.classification())
        });
        let mut bindings: Vec<_> = evidence
            .iter()
            .map(|item| {
                EvidenceClassificationBinding::new(item.id().clone(), item.classification())
            })
            .collect();
        bindings.sort_by(|left, right| {
            left.evidence_id()
                .as_str()
                .cmp(right.evidence_id().as_str())
        });
        Ok((classification, bindings, evidence))
    }
    fn store_request(
        &mut self,
        r: ActionRequestRecord,
        context: ActionOperationContext,
        sig: CommandIdentity,
        code: &str,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionServiceError> {
        let audit = self.audit(
            AuditTarget::ActionRequest(r.id.clone()),
            code,
            &context,
            false,
        )?;
        let outcome = ActionMutationOutcome {
            record: r.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        let mut s = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut s)?;
        s.requests.insert(r.id.clone(), r);
        s.audit_events.push(audit);
        s.idem.insert(
            context.idempotency_id.clone(),
            Stored {
                signature: sig,
                result: StoredResult::Request(outcome.clone()),
                original_correlation_id: context.correlation_id,
                operation_ordinal,
            },
        );
        self.finish(s)?;
        Ok(outcome)
    }
    fn store_action(
        &mut self,
        r: ActionRecord,
        context: ActionOperationContext,
        sig: CommandIdentity,
        receipt: Option<ApprovalReceiptId>,
        code: &str,
    ) -> Result<ActionMutationOutcome<ActionRecord>, ActionServiceError> {
        let audit = self.audit(AuditTarget::Action(r.id.clone()), code, &context, false)?;
        let outcome = ActionMutationOutcome {
            record: r.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: receipt,
        };
        let mut s = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut s)?;
        s.actions.insert(r.id.clone(), r);
        s.audit_events.push(audit);
        s.idem.insert(
            context.idempotency_id.clone(),
            Stored {
                signature: sig,
                result: StoredResult::Action(outcome.clone()),
                original_correlation_id: context.correlation_id,
                operation_ordinal,
            },
        );
        self.finish(s)?;
        Ok(outcome)
    }
    fn finish(&mut self, s: Store) -> Result<(), ActionServiceError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(ActionServiceError::InfrastructureFailure);
        }
        self.state = s;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn store_terminal_failure(
        &mut self,
        idempotency_id: IdempotencyId,
        original_correlation_id: CorrelationId,
        signature: CommandIdentity,
        cause: ActionServiceError,
        persisted_cause: ActionPersistenceTerminalCause,
        error: DomainError,
        prepared_disposition: PreparedDisposition,
        audit: AuditEvent,
    ) -> Result<(), ActionServiceError> {
        let mut staged = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut staged)?;
        let consumed_id = match &signature {
            CommandIdentity::ExecuteAccept(identity)
            | CommandIdentity::ExecuteAction(_, identity) => Some(identity.prepared_id.clone()),
            _ => None,
        };
        if prepared_disposition == PreparedDisposition::ConsumedAndDiscarded {
            let Some(prepared_id) = consumed_id else {
                return Err(ActionServiceError::InfrastructureFailure);
            };
            if let Some(intent) = staged.prepared.remove(&prepared_id) {
                staged.discarded_prepared.push(intent);
            }
        }
        staged.audit_events.push(audit.clone());
        staged.idem.insert(
            idempotency_id,
            Stored {
                signature,
                result: StoredResult::Terminal(ActionTerminalFailure {
                    cause,
                    persisted_cause,
                    error,
                    prepared_disposition,
                    audit,
                }),
                original_correlation_id,
                operation_ordinal,
            },
        );
        self.finish(staged)
    }
    fn replay_request(
        &self,
        id: &IdempotencyId,
        sig: &CommandIdentity,
    ) -> Result<Option<ActionMutationOutcome<ActionRequestRecord>>, ActionServiceError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(StoredResult::Request(r)) => Ok(Some(r.clone())),
            Some(_) => Err(ActionServiceError::IdempotencyConflict),
        }
    }
    fn replay_action(
        &self,
        id: &IdempotencyId,
        sig: &CommandIdentity,
    ) -> Result<Option<ActionMutationOutcome<ActionRecord>>, ActionServiceError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(StoredResult::Action(r)) => Ok(Some(r.clone())),
            Some(StoredResult::Terminal(failure)) => Err(failure.cause),
            Some(_) => Err(ActionServiceError::IdempotencyConflict),
        }
    }
    fn replay_link_action(
        &self,
        id: &IdempotencyId,
        sig: &CommandIdentity,
    ) -> Result<Option<ActionMutationOutcome<ActionRecord>>, ActionServiceError> {
        let Some(stored) = self.state.idem.get(id) else {
            return Ok(None);
        };
        let same_business_command = match (&stored.signature, sig) {
            (
                CommandIdentity::LinkCompletionEvidence {
                    action_id: stored_action_id,
                    expected_version: stored_version,
                    evidence_id: stored_evidence_id,
                    ..
                },
                CommandIdentity::LinkCompletionEvidence {
                    action_id,
                    expected_version,
                    evidence_id,
                    ..
                },
            ) => {
                stored_action_id == action_id
                    && stored_version == expected_version
                    && stored_evidence_id == evidence_id
            }
            _ => false,
        };
        if !same_business_command {
            return Err(ActionServiceError::IdempotencyConflict);
        }
        match &stored.result {
            StoredResult::Action(outcome) => Ok(Some(outcome.clone())),
            _ => Err(ActionServiceError::IdempotencyConflict),
        }
    }
    fn replay_accepted(
        &self,
        id: &IdempotencyId,
        sig: &CommandIdentity,
    ) -> Result<Option<AcceptedActionOutcome>, ActionServiceError> {
        match self.replay(id, sig)? {
            None => Ok(None),
            Some(StoredResult::Accepted(r)) => Ok(Some(r.clone())),
            Some(StoredResult::Terminal(failure)) => Err(failure.cause),
            Some(_) => Err(ActionServiceError::IdempotencyConflict),
        }
    }
    fn replay_prepared(
        &self,
        id: &IdempotencyId,
        signature: &CommandIdentity,
    ) -> Result<Option<WorkManagementPreparedIntent>, ActionServiceError> {
        match self.replay(id, signature)? {
            None => Ok(None),
            Some(StoredResult::Prepared(prepared)) => Ok(Some(prepared.clone())),
            Some(StoredResult::Terminal(failure)) => Err(failure.cause),
            Some(_) => Err(ActionServiceError::IdempotencyConflict),
        }
    }
    fn store_prepared(
        &mut self,
        prepared: WorkManagementPreparedIntent,
        idempotency_id: IdempotencyId,
        original_correlation_id: CorrelationId,
        signature: CommandIdentity,
    ) -> Result<WorkManagementPreparedIntent, ActionServiceError> {
        let mut staged = self.state.clone();
        let operation_ordinal = allocate_operation_ordinal(&mut staged)?;
        staged
            .prepared
            .insert(prepared.id().clone(), prepared.clone());
        staged.idem.insert(
            idempotency_id,
            Stored {
                signature,
                result: StoredResult::Prepared(prepared.clone()),
                original_correlation_id,
                operation_ordinal,
            },
        );
        self.state = staged;
        Ok(prepared)
    }
    fn replay(
        &self,
        id: &IdempotencyId,
        signature: &CommandIdentity,
    ) -> Result<Option<&StoredResult>, ActionServiceError> {
        match self.state.idem.get(id) {
            None => Ok(None),
            Some(stored) if &stored.signature == signature => Ok(Some(&stored.result)),
            Some(_) => Err(ActionServiceError::IdempotencyConflict),
        }
    }
    fn audit(
        &mut self,
        target: AuditTarget,
        code: &str,
        c: &ActionOperationContext,
        approved: bool,
    ) -> Result<AuditEvent, ActionServiceError> {
        let id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(code).map_err(|_| ActionServiceError::InfrastructureFailure)?,
            target.clone(),
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
            vec![AuditEffectCode::parse(code)
                .map_err(|_| ActionServiceError::InfrastructureFailure)?],
        )
        .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        Ok(AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            action,
            c.correlation_id.clone(),
            disposition,
        ))
    }
    /// Whether `prepared_id` was consumed by a v45 rejection. Such an intent
    /// sits in `discarded_prepared`, but an approval attempted against it is
    /// refused like one against any other consumed preview: no terminal
    /// capsule is recorded (the rejection already consumed the intent, and a
    /// second consumption would never rehydrate) and no audit is written.
    fn was_rejected(&self, prepared_id: &PreparedIntentId) -> bool {
        self.state.idem.values().any(|stored| {
            matches!(&stored.result, StoredResult::Rejected(outcome) if outcome.prepared_intent_id() == prepared_id)
        })
    }
    fn failure_target(&self, approval: &WorkManagementApproval) -> ErrorTarget {
        let target = self
            .state
            .prepared
            .get(approval.prepared_id())
            .or_else(|| {
                self.state
                    .discarded_prepared
                    .iter()
                    .find(|prepared| prepared.id() == approval.prepared_id())
                    .filter(|_| !self.was_rejected(approval.prepared_id()))
            })
            .map_or(ErrorTarget::Unknown, |prepared| {
                match prepared.operation() {
                    WorkManagementOperation::AcceptActionRequest { request_id, .. } => {
                        ErrorTarget::Request(request_id.clone())
                    }
                    WorkManagementOperation::CompleteAction { action_id, .. }
                    | WorkManagementOperation::CancelAction { action_id, .. }
                    | WorkManagementOperation::ReopenAction { action_id, .. }
                    | WorkManagementOperation::LowerActionClassification { action_id, .. } => {
                        ErrorTarget::Action(action_id.clone())
                    }
                    _ => ErrorTarget::Unknown,
                }
            });
        if !matches!(target, ErrorTarget::Unknown) {
            return target;
        }
        match self.state.idem.get(approval.idempotency_id()) {
            Some(Stored {
                result: StoredResult::Terminal(failure),
                ..
            }) => match failure.audit.target() {
                AuditTarget::ActionRequest(id) => ErrorTarget::Request(id.clone()),
                AuditTarget::Action(id) => ErrorTarget::Action(id.clone()),
                _ => ErrorTarget::Unknown,
            },
            _ => ErrorTarget::Unknown,
        }
    }
    fn failure_transition_kind(
        &self,
        approval: &WorkManagementApproval,
    ) -> PreparedActionTransitionKind {
        let prepared = self.state.prepared.get(approval.prepared_id()).or_else(|| {
            self.state
                .discarded_prepared
                .iter()
                .find(|p| p.id() == approval.prepared_id())
        });
        match prepared.map(WorkManagementPreparedIntent::operation) {
            Some(WorkManagementOperation::CancelAction { .. }) => {
                PreparedActionTransitionKind::Cancel
            }
            Some(WorkManagementOperation::ReopenAction { .. }) => {
                PreparedActionTransitionKind::Reopen
            }
            _ => PreparedActionTransitionKind::Complete,
        }
    }
    fn h3_denial_cause_for_action(
        &self,
        action_id: &ActionId,
        cause: ActionServiceError,
    ) -> ActionPersistenceH3DenialCause {
        let Some(action) = self.state.actions.get(action_id) else {
            return ActionPersistenceH3DenialCause::ClassificationUnresolved;
        };
        if action.commitment_classification == DataClassification::Unclassified {
            return ActionPersistenceH3DenialCause::ClassificationUnresolved;
        }
        if action.completion_evidence.is_empty() {
            return ActionPersistenceH3DenialCause::MissingCompletionEvidence;
        }
        for evidence_id in &action.completion_evidence {
            match self.evidence_authority.resolve(evidence_id) {
                Err(ActionEvidenceAuthorityError::Unavailable) => {
                    return ActionPersistenceH3DenialCause::EvidenceUnavailable;
                }
                Err(ActionEvidenceAuthorityError::NotFound) => {
                    return ActionPersistenceH3DenialCause::CompletionEvidenceNotFound;
                }
                Ok(metadata) if metadata.classification() == DataClassification::Unclassified => {
                    return ActionPersistenceH3DenialCause::UnclassifiedCompletionEvidence;
                }
                Ok(metadata)
                    if matches!(
                        metadata.verification(),
                        EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch
                    ) =>
                {
                    return ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence;
                }
                Ok(_) => {}
            }
        }
        match cause {
            ActionServiceError::EvidenceUnavailable => {
                ActionPersistenceH3DenialCause::EvidenceUnavailable
            }
            ActionServiceError::ClassificationUnresolved => {
                ActionPersistenceH3DenialCause::ClassificationUnresolved
            }
            ActionServiceError::InvalidEvidence => {
                ActionPersistenceH3DenialCause::MissingCompletionEvidence
            }
            _ => ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence,
        }
    }
    fn domain_error(
        &self,
        cause: ActionServiceError,
        context: &ActionOperationContext,
        target: ErrorTarget,
    ) -> DomainError {
        if let Some(Stored {
            result: StoredResult::Terminal(failure),
            ..
        }) = self.state.idem.get(&context.idempotency_id)
        {
            if failure.cause == cause {
                return failure.error.clone();
            }
        }
        domain_error_for_runtime_target(
            cause,
            context.correlation_id.clone(),
            &target,
            &self.state.requests,
            &self.state.actions,
        )
    }
    fn replay_terminal_error(
        &self,
        id: &IdempotencyId,
        signature: &CommandIdentity,
    ) -> Result<Option<DomainError>, ActionServiceError> {
        match self.replay(id, signature)? {
            Some(StoredResult::Terminal(failure)) => Ok(Some(failure.error.clone())),
            Some(_) | None => Ok(None),
        }
    }
    fn record_retryable_preview_failure(
        &mut self,
        approval: &WorkManagementApproval,
        context: &ActionOperationContext,
        cause: ActionServiceError,
    ) -> Result<(), ActionServiceError> {
        if self.state.idem.contains_key(&context.idempotency_id) {
            return Ok(());
        }
        let error_target = self.failure_target(approval);
        let target = match &error_target {
            ErrorTarget::Request(id) => AuditTarget::ActionRequest(id.clone()),
            ErrorTarget::Action(id) => AuditTarget::Action(id.clone()),
            ErrorTarget::Unknown => return Ok(()),
        }
        .clone();
        let disposition = AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Rejected,
            AuditExecutionOutcome::NotAttempted,
            AuditEffectScope::None,
            vec![],
        )
        .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let audit = AuditEvent::new(
            self.ids
                .next_audit_event_id()
                .map_err(|_| ActionServiceError::InfrastructureFailure)?,
            self.clock.now(),
            approval.actor(),
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse("action.approval_rejected")
                    .map_err(|_| ActionServiceError::InfrastructureFailure)?,
                target,
            ),
            context.correlation_id.clone(),
            disposition,
        );
        let is_accept = self
            .state
            .prepared
            .get(approval.prepared_id())
            .or_else(|| {
                self.state
                    .discarded_prepared
                    .iter()
                    .find(|intent| intent.id() == approval.prepared_id())
            })
            .is_some_and(|intent| {
                matches!(
                    intent.operation(),
                    WorkManagementOperation::AcceptActionRequest { .. }
                )
            });
        let signature = if is_accept {
            CommandIdentity::ExecuteAccept(approval_business_identity(approval))
        } else {
            CommandIdentity::ExecuteAction(
                self.failure_transition_kind(approval),
                approval_business_identity(approval),
            )
        };
        let error = domain_error_for_runtime_target(
            cause,
            context.correlation_id.clone(),
            &error_target,
            &self.state.requests,
            &self.state.actions,
        );
        self.store_terminal_failure(
            context.idempotency_id.clone(),
            context.correlation_id.clone(),
            signature,
            cause,
            persistence_terminal_cause(cause, approval.acknowledged_payload_digest()),
            error,
            PreparedDisposition::Retained,
            audit,
        )
    }

    fn record_h2_failure(
        &mut self,
        approval: &WorkManagementApproval,
        context: &ActionOperationContext,
        cause: ActionServiceError,
    ) -> Result<(), ActionServiceError> {
        if self.state.idem.contains_key(&context.idempotency_id) {
            return Ok(());
        }
        let error_target = self.failure_target(approval);
        let target = match &error_target {
            ErrorTarget::Request(id) => AuditTarget::ActionRequest(id.clone()),
            ErrorTarget::Action(id) => AuditTarget::Action(id.clone()),
            ErrorTarget::Unknown => return Ok(()),
        };
        let denied = matches!(
            cause,
            ActionServiceError::PolicyDenied | ActionServiceError::Unauthorized
        );
        let failed = matches!(cause, ActionServiceError::InfrastructureFailure);
        let disposition = AuditDisposition::new(
            if denied {
                AuditPolicyOutcome::Denied
            } else {
                AuditPolicyOutcome::Allowed
            },
            if denied {
                AuditApprovalOutcome::NotRequired
            } else if failed {
                AuditApprovalOutcome::Approved
            } else {
                AuditApprovalOutcome::Rejected
            },
            if failed {
                AuditExecutionOutcome::Failed
            } else {
                AuditExecutionOutcome::NotAttempted
            },
            AuditEffectScope::None,
            vec![],
        )
        .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(if denied {
                "action.approval_denied"
            } else if failed {
                "action.execution_failed"
            } else {
                "action.approval_rejected"
            })
            .map_err(|_| ActionServiceError::InfrastructureFailure)?,
            target.clone(),
        );
        let audit = AuditEvent::new(
            id,
            self.clock.now(),
            approval.actor(),
            action,
            context.correlation_id.clone(),
            disposition,
        );
        let signature = if self
            .state
            .prepared
            .get(approval.prepared_id())
            .or_else(|| {
                self.state
                    .discarded_prepared
                    .iter()
                    .find(|intent| intent.id() == approval.prepared_id())
            })
            .is_some_and(|intent| {
                matches!(
                    intent.operation(),
                    WorkManagementOperation::AcceptActionRequest { .. }
                )
            }) {
            CommandIdentity::ExecuteAccept(approval_business_identity(approval))
        } else {
            CommandIdentity::ExecuteAction(
                self.failure_transition_kind(approval),
                approval_business_identity(approval),
            )
        };
        let error = domain_error_for_runtime_target(
            cause,
            context.correlation_id.clone(),
            &error_target,
            &self.state.requests,
            &self.state.actions,
        );
        self.store_terminal_failure(
            context.idempotency_id.clone(),
            context.correlation_id.clone(),
            signature,
            cause,
            persistence_terminal_cause(cause, approval.acknowledged_payload_digest()),
            error,
            PreparedDisposition::ConsumedAndDiscarded,
            audit,
        )
    }
    fn record_h3_denial(
        &mut self,
        target: AuditTarget,
        context: &ActionOperationContext,
        signature: CommandIdentity,
        cause: ActionServiceError,
        denial_cause: ActionPersistenceH3DenialCause,
    ) -> Result<(), ActionServiceError> {
        if self.state.idem.contains_key(&context.idempotency_id) {
            return Ok(());
        }
        let disposition = AuditDisposition::new(
            AuditPolicyOutcome::Denied,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::NotAttempted,
            AuditEffectScope::None,
            vec![],
        )
        .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let id = self
            .ids
            .next_audit_event_id()
            .map_err(|_| ActionServiceError::InfrastructureFailure)?;
        let action = AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("action.evidence_policy_denied")
                .map_err(|_| ActionServiceError::InfrastructureFailure)?,
            target.clone(),
        );
        let audit = AuditEvent::new(
            id,
            self.clock.now(),
            AuditActor::HeadOfProducts,
            action,
            context.correlation_id.clone(),
            disposition,
        );
        let error_target = match &target {
            AuditTarget::ActionRequest(id) => ErrorTarget::Request(id.clone()),
            AuditTarget::Action(id) => ErrorTarget::Action(id.clone()),
            _ => ErrorTarget::Unknown,
        };
        let error = domain_error_for_runtime_target(
            cause,
            context.correlation_id.clone(),
            &error_target,
            &self.state.requests,
            &self.state.actions,
        );
        self.store_terminal_failure(
            context.idempotency_id.clone(),
            context.correlation_id.clone(),
            signature,
            cause,
            ActionPersistenceTerminalCause::H3Denied(denial_cause),
            error,
            PreparedDisposition::NotApplicable,
            audit,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreparedActionTransitionKind {
    Complete,
    Cancel,
    Reopen,
}

const fn persistence_transition_kind(
    kind: PreparedActionTransitionKind,
) -> ActionPersistenceTransitionKind {
    match kind {
        PreparedActionTransitionKind::Complete => ActionPersistenceTransitionKind::Complete,
        PreparedActionTransitionKind::Cancel => ActionPersistenceTransitionKind::Cancel,
        PreparedActionTransitionKind::Reopen => ActionPersistenceTransitionKind::Reopen,
    }
}

const fn runtime_transition_kind(
    kind: ActionPersistenceTransitionKind,
) -> PreparedActionTransitionKind {
    match kind {
        ActionPersistenceTransitionKind::Complete => PreparedActionTransitionKind::Complete,
        ActionPersistenceTransitionKind::Cancel => PreparedActionTransitionKind::Cancel,
        ActionPersistenceTransitionKind::Reopen => PreparedActionTransitionKind::Reopen,
    }
}
fn next(v: AggregateVersion) -> Result<AggregateVersion, ActionServiceError> {
    v.next().ok_or(ActionServiceError::InfrastructureFailure)
}
/// The audit target a rejection of this intent records: the Action Request
/// for an Accept preview, the Action for Complete/Cancel/Reopen. `None` for
/// any other operation -- Action rejection is scoped to those four kinds.
fn rejection_audit_target(intent: &WorkManagementPreparedIntent) -> Option<AuditTarget> {
    match intent.operation() {
        WorkManagementOperation::AcceptActionRequest { request_id, .. } => {
            Some(AuditTarget::ActionRequest(request_id.clone()))
        }
        WorkManagementOperation::CompleteAction { action_id, .. }
        | WorkManagementOperation::CancelAction { action_id, .. }
        | WorkManagementOperation::ReopenAction { action_id, .. } => {
            Some(AuditTarget::Action(action_id.clone()))
        }
        _ => None,
    }
}
fn allocate_operation_ordinal(store: &mut Store) -> Result<u64, ActionServiceError> {
    let ordinal = store.next_operation_ordinal;
    store.next_operation_ordinal = ordinal
        .checked_add(1)
        .ok_or(ActionServiceError::InfrastructureFailure)?;
    Ok(ordinal)
}
fn to_details(
    r: &crate::work_management::WorkManagementRationale,
) -> Result<ActionDetails, ActionServiceError> {
    ActionDetails::parse(r.as_str().to_owned())
        .map_err(|_| ActionServiceError::InfrastructureFailure)
}
fn map_prepare(_: PreparedIntentError) -> ActionServiceError {
    ActionServiceError::PreparedIntentChanged
}
/// True only for a genuine lowering (`proposed` strictly less restrictive
/// than `current`). See `portfolio::is_genuine_lowering` -- same check,
/// duplicated per module rather than shared, matching how
/// `ensure_classification_not_lowered`-style guards are already
/// duplicated per aggregate family in this codebase.
fn is_genuine_lowering(current: DataClassification, proposed: DataClassification) -> bool {
    proposed.combine(current) != proposed
}
fn map_validation(e: WorkManagementApprovalValidationError) -> ActionServiceError {
    match e {
        WorkManagementApprovalValidationError::PreparedIntentMismatch => {
            ActionServiceError::PreparedIntentNotFound
        }
        WorkManagementApprovalValidationError::Unauthorized => ActionServiceError::Unauthorized,
        WorkManagementApprovalValidationError::DigestMismatch => ActionServiceError::DigestMismatch,
        WorkManagementApprovalValidationError::Expired => ActionServiceError::Expired,
        WorkManagementApprovalValidationError::PreviewChanged => {
            ActionServiceError::PreparedIntentChanged
        }
    }
}
fn map_post_prepare_state(error: ActionServiceError) -> ActionServiceError {
    if error == ActionServiceError::StaleVersion {
        ActionServiceError::PreparedIntentChanged
    } else {
        error
    }
}
fn is_h3_denial(error: ActionServiceError) -> bool {
    matches!(
        error,
        ActionServiceError::EvidencePolicyDenied
            | ActionServiceError::ClassificationUnresolved
            | ActionServiceError::EvidenceUnavailable
            | ActionServiceError::InvalidEvidence
    )
}

fn is_retryable_execution_failure(error: ActionServiceError) -> bool {
    matches!(
        error,
        ActionServiceError::PreparedIntentChanged | ActionServiceError::InfrastructureFailure
    )
}

fn h3_terminal_service_cause(cause: ActionPersistenceH3DenialCause) -> ActionServiceError {
    match cause {
        ActionPersistenceH3DenialCause::EvidenceUnavailable => {
            ActionServiceError::EvidenceUnavailable
        }
        ActionPersistenceH3DenialCause::ClassificationUnresolved
        | ActionPersistenceH3DenialCause::UnclassifiedCompletionEvidence => {
            ActionServiceError::ClassificationUnresolved
        }
        ActionPersistenceH3DenialCause::MissingCompletionEvidence
        | ActionPersistenceH3DenialCause::CompletionEvidenceNotFound
        | ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence => {
            ActionServiceError::EvidencePolicyDenied
        }
    }
}

fn persistence_terminal_cause(
    cause: ActionServiceError,
    digest: &WorkManagementPayloadDigest,
) -> ActionPersistenceTerminalCause {
    match cause {
        ActionServiceError::NotFound => ActionPersistenceTerminalCause::NotFound,
        ActionServiceError::AlreadyExists => ActionPersistenceTerminalCause::AlreadyExists,
        ActionServiceError::IllegalTransition => ActionPersistenceTerminalCause::IllegalTransition,
        ActionServiceError::StaleVersion => ActionPersistenceTerminalCause::StaleVersion,
        ActionServiceError::MissingOwner => ActionPersistenceTerminalCause::MissingOwner,
        ActionServiceError::MissingDueDate => ActionPersistenceTerminalCause::MissingDueDate,
        ActionServiceError::InvalidEvidence => ActionPersistenceTerminalCause::InvalidEvidence,
        ActionServiceError::InvalidReopenMode => ActionPersistenceTerminalCause::InvalidReopenMode,
        ActionServiceError::IdempotencyConflict => {
            ActionPersistenceTerminalCause::IdempotencyConflict
        }
        ActionServiceError::PreparedIntentNotFound => {
            ActionPersistenceTerminalCause::PreparedIntentNotFound
        }
        ActionServiceError::PreparedIntentChanged => {
            ActionPersistenceTerminalCause::PreparedIntentChanged
        }
        ActionServiceError::Unauthorized => ActionPersistenceTerminalCause::Unauthorized,
        ActionServiceError::DigestMismatch => ActionPersistenceTerminalCause::DigestMismatch {
            attempted_digest: digest.clone(),
        },
        ActionServiceError::Expired => ActionPersistenceTerminalCause::Expired,
        ActionServiceError::PolicyDenied => ActionPersistenceTerminalCause::PolicyDenied,
        ActionServiceError::InfrastructureFailure => {
            ActionPersistenceTerminalCause::InfrastructureFailure
        }
        ActionServiceError::EvidenceUnavailable => ActionPersistenceTerminalCause::H3Denied(
            ActionPersistenceH3DenialCause::EvidenceUnavailable,
        ),
        ActionServiceError::ClassificationUnresolved => ActionPersistenceTerminalCause::H3Denied(
            ActionPersistenceH3DenialCause::ClassificationUnresolved,
        ),
        ActionServiceError::EvidencePolicyDenied => ActionPersistenceTerminalCause::H3Denied(
            ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence,
        ),
        // H2a Lower Data Classification: unreachable in practice -- `persistence_terminal_cause`
        // is only invoked along the H3-denial path (gated by `is_h3_denial`,
        // which `NotALowering` never satisfies) and from
        // `record_retryable_preview_failure`/`record_h2_failure`, neither of
        // which this operation's execute wrapper calls (see its own doc
        // comment). Mapped to the nearest existing bucket only to satisfy
        // exhaustiveness.
        ActionServiceError::NotALowering => ActionPersistenceTerminalCause::IllegalTransition,
    }
}

fn terminal_cause_to_service(cause: &ActionPersistenceTerminalCause) -> ActionServiceError {
    match cause {
        ActionPersistenceTerminalCause::NotFound => ActionServiceError::NotFound,
        ActionPersistenceTerminalCause::AlreadyExists => ActionServiceError::AlreadyExists,
        ActionPersistenceTerminalCause::IllegalTransition => ActionServiceError::IllegalTransition,
        ActionPersistenceTerminalCause::StaleVersion => ActionServiceError::StaleVersion,
        ActionPersistenceTerminalCause::MissingOwner => ActionServiceError::MissingOwner,
        ActionPersistenceTerminalCause::MissingDueDate => ActionServiceError::MissingDueDate,
        ActionPersistenceTerminalCause::InvalidEvidence => ActionServiceError::InvalidEvidence,
        ActionPersistenceTerminalCause::InvalidReopenMode => ActionServiceError::InvalidReopenMode,
        ActionPersistenceTerminalCause::IdempotencyConflict => {
            ActionServiceError::IdempotencyConflict
        }
        ActionPersistenceTerminalCause::PreparedIntentNotFound => {
            ActionServiceError::PreparedIntentNotFound
        }
        ActionPersistenceTerminalCause::PreparedIntentChanged => {
            ActionServiceError::PreparedIntentChanged
        }
        ActionPersistenceTerminalCause::Unauthorized => ActionServiceError::Unauthorized,
        ActionPersistenceTerminalCause::DigestMismatch { .. } => ActionServiceError::DigestMismatch,
        ActionPersistenceTerminalCause::Expired => ActionServiceError::Expired,
        ActionPersistenceTerminalCause::PolicyDenied => ActionServiceError::PolicyDenied,
        ActionPersistenceTerminalCause::InfrastructureFailure => {
            ActionServiceError::InfrastructureFailure
        }
        ActionPersistenceTerminalCause::H3Denied(cause) => h3_terminal_service_cause(*cause),
    }
}

fn decision_requests_from(store: &Store, source: &DecisionId) -> Vec<ActionRequestRecord> {
    let mut records: Vec<_> = store
        .requests
        .values()
        .filter(|record| record.source_decision_id() == Some(source))
        .cloned()
        .collect();
    records.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
    records
}

fn decision_actions_from(store: &Store, source: &DecisionId) -> Vec<ActionRecord> {
    let mut records: Vec<_> = store
        .actions
        .values()
        .filter(|record| record.source_decision_id() == Some(source))
        .cloned()
        .collect();
    records.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
    records
}

fn exact_request_from<'a>(
    store: &'a Store,
    id: &ActionRequestId,
    version: AggregateVersion,
) -> Result<&'a ActionRequestRecord, ActionServiceError> {
    let record = store.requests.get(id).ok_or(ActionServiceError::NotFound)?;
    if record.version != version {
        return Err(ActionServiceError::StaleVersion);
    }
    Ok(record)
}

fn exact_action_from<'a>(
    store: &'a Store,
    id: &ActionId,
    version: AggregateVersion,
) -> Result<&'a ActionRecord, ActionServiceError> {
    let record = store.actions.get(id).ok_or(ActionServiceError::NotFound)?;
    if record.version != version {
        return Err(ActionServiceError::StaleVersion);
    }
    Ok(record)
}

fn replay_request_from(
    store: &Store,
    id: &IdempotencyId,
    signature: &CommandIdentity,
) -> Result<Option<ActionMutationOutcome<ActionRequestRecord>>, ActionServiceError> {
    match store.idem.get(id) {
        None => Ok(None),
        Some(stored) if &stored.signature != signature => {
            Err(ActionServiceError::IdempotencyConflict)
        }
        Some(Stored {
            result: StoredResult::Request(record),
            ..
        }) => Ok(Some(record.clone())),
        Some(_) => Err(ActionServiceError::IdempotencyConflict),
    }
}

fn replay_action_from(
    store: &Store,
    id: &IdempotencyId,
    signature: &CommandIdentity,
) -> Result<Option<ActionMutationOutcome<ActionRecord>>, ActionServiceError> {
    match store.idem.get(id) {
        None => Ok(None),
        Some(stored) if &stored.signature != signature => {
            Err(ActionServiceError::IdempotencyConflict)
        }
        Some(Stored {
            result: StoredResult::Action(record),
            ..
        }) => Ok(Some(record.clone())),
        Some(_) => Err(ActionServiceError::IdempotencyConflict),
    }
}

fn static_message_key(value: &str) -> MessageKey {
    match MessageKey::parse(value) {
        Ok(key) => key,
        Err(_) => unreachable!("static message key is valid"),
    }
}
/// What an Action Request's current state admits.
///
/// Public so the Work Queue reads the same table the safe-error diagnostics
/// use, rather than a second copy that would drift from it. See
/// [`crate::state_intents`] for what this does and does not claim.
pub const fn request_allowed_intents(state: ActionRequestState) -> &'static [&'static str] {
    match state {
        ActionRequestState::Draft => &["submit_action_request"],
        ActionRequestState::Open => &[
            "prepare_accept_action_request",
            "decline_action_request",
            "withdraw_action_request",
        ],
        ActionRequestState::Accepted
        | ActionRequestState::Declined
        | ActionRequestState::Withdrawn => &[],
    }
}
/// What an Action's current state admits.
///
/// Includes `link_action_completion_evidence`, which is state-dependent
/// without being a lifecycle transition. That is the established convention
/// of this table and is kept rather than narrowed, so the Work Queue and the
/// safe errors describe the same set. See [`crate::state_intents`].
pub const fn action_allowed_intents(state: ActionState) -> &'static [&'static str] {
    match state {
        ActionState::Open => &["start_action", "prepare_cancel_action"],
        ActionState::InProgress => &[
            "link_action_completion_evidence",
            "prepare_complete_action",
            "prepare_cancel_action",
        ],
        ActionState::Completed | ActionState::Cancelled => &["prepare_reopen_action"],
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod decision_integration_seam_tests {
    use std::cell::Cell;

    use super::*;
    use crate::audit::AuditActor;
    use crate::work_management::{ApprovalConfirmation, WorkManagementApproval};

    #[derive(Clone)]
    struct TestClock(Cell<i64>);
    impl Clock for TestClock {
        fn now(&self) -> UtcTimestamp {
            UtcTimestamp::from_unix_millis(self.0.get())
        }
    }
    struct TestIds(u64);
    impl ActionServiceIdSource for TestIds {
        fn next_action_id(&mut self) -> Result<ActionId, crate::DomainValueError> {
            self.0 += 1;
            ActionId::parse(format!("action-decision-{}", self.0))
        }
        fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError> {
            self.0 += 1;
            PreparedIntentId::parse(format!("prepared-decision-{}", self.0))
        }
        fn next_approval_receipt_id(
            &mut self,
        ) -> Result<ApprovalReceiptId, crate::DomainValueError> {
            self.0 += 1;
            ApprovalReceiptId::parse(format!("receipt-decision-{}", self.0))
        }
        fn next_audit_event_id(
            &mut self,
        ) -> Result<crate::identity::AuditEventId, crate::DomainValueError> {
            self.0 += 1;
            crate::identity::AuditEventId::parse(format!("audit-decision-{}", self.0))
        }
    }
    struct Allow;
    impl ApprovalAuthorizationPort for Allow {
        fn authorize(&self, actor: AuditActor) -> bool {
            actor == AuditActor::HeadOfProducts
        }
    }
    impl ActionExecutionPolicyPort for Allow {
        fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
            ActionExecutionPolicy::Allowed
        }
    }
    type Service =
        InMemoryActionService<TestClock, TestIds, Allow, Allow, DenyActionEvidenceAuthority>;

    fn service() -> Service {
        InMemoryActionService::new(
            TestClock(Cell::new(100)),
            TestIds(0),
            Allow,
            Allow,
            DenyActionEvidenceAuthority,
        )
    }
    fn context(id: &str) -> ActionOperationContext {
        ActionOperationContext {
            idempotency_id: IdempotencyId::parse(id).unwrap(),
            correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
        }
    }
    fn decision(id: &str) -> DecisionId {
        DecisionId::parse(id).unwrap()
    }
    fn spec(id: &str) -> DecisionResultingActionRequest {
        DecisionResultingActionRequest {
            id: ActionRequestId::parse(id).unwrap(),
            subject: BoundedText::parse("Decision follow-up".to_owned()).unwrap(),
            details: BoundedText::parse("Execute the decided synthetic follow-up".to_owned())
                .unwrap(),
            intended_owner: StakeholderId::parse("owner-decision").unwrap(),
            due_at: UtcTimestamp::from_unix_millis(900),
            classification: DataClassification::Internal,
        }
    }

    #[test]
    fn resulting_request_is_open_exact_and_acceptance_inherits_origin_and_attention() {
        let mut service = service();
        let source = decision("decision-source");
        let mut stage = service.begin_decision_stage();
        let created = service
            .create_resulting_action_request_from_decision_transition(
                &mut stage,
                &spec("request-resulting"),
                source.clone(),
                DataClassification::Confidential,
                context("create-resulting"),
            )
            .unwrap()
            .record;
        assert!(service.request(created.id()).is_none());
        service.commit_decision_stage(stage).unwrap();
        assert_eq!(created.state(), ActionRequestState::Open);
        assert_eq!(created.source_decision_id(), Some(&source));
        assert_eq!(created.classification(), DataClassification::Confidential);
        assert_eq!(
            created.intended_action_due_at(),
            Some(UtcTimestamp::from_unix_millis(900))
        );
        assert!(!created.has_superseded_premise());
        let mut stage = service.begin_decision_stage();
        let marked = service
            .mark_action_request_superseded_premise_from_decision(
                &mut stage,
                created.id(),
                created.version(),
                &source,
                DataClassification::Restricted,
                context("mark-resulting"),
            )
            .unwrap()
            .record;
        assert_eq!(stage.request(marked.id()), Some(&marked));
        service.commit_decision_stage(stage).unwrap();
        assert_eq!(marked.state(), ActionRequestState::Open);
        assert!(marked.has_superseded_premise());
        assert_eq!(marked.classification(), DataClassification::Restricted);
        let prepared = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: marked.id().clone(),
                expected_version: marked.version(),
                context: context("prepare-resulting"),
            })
            .unwrap();
        let approval = WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("accept-resulting").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval,
                context: context("accept-resulting"),
            })
            .unwrap();
        assert_eq!(accepted.action.source_decision_id(), Some(&source));
        assert!(accepted.action.has_superseded_premise());
        assert_eq!(accepted.action.state(), ActionState::Open);
        let mut stage = service.begin_decision_stage();
        let marked_action = service
            .mark_action_superseded_premise_from_decision(
                &mut stage,
                accepted.action.id(),
                accepted.action.version(),
                &source,
                DataClassification::Restricted,
                context("mark-resulting-action"),
            )
            .unwrap()
            .record;
        assert_eq!(stage.action(marked_action.id()), Some(&marked_action));
        assert_eq!(stage.actions_for_decision(&source).len(), 1);
        service.commit_decision_stage(stage).unwrap();
        assert_eq!(marked_action.state(), ActionState::Open);
        assert!(marked_action.has_superseded_premise());
        assert_eq!(service.action_requests_for_decision(&source).len(), 1);
        assert_eq!(service.actions_for_decision(&source).len(), 1);
        let mut stage = service.begin_decision_stage();
        assert!(matches!(
            service.mark_action_request_superseded_premise_from_decision(
                &mut stage,
                accepted.request.id(),
                accepted.request.version(),
                &source,
                DataClassification::Restricted,
                context("mark-accepted"),
            ),
            Err(ActionServiceError::IllegalTransition)
        ));
        service
            .state
            .actions
            .get_mut(marked_action.id())
            .unwrap()
            .state = ActionState::Cancelled;
        let mut stage = service.begin_decision_stage();
        assert!(matches!(
            service.mark_action_superseded_premise_from_decision(
                &mut stage,
                marked_action.id(),
                marked_action.version(),
                &source,
                DataClassification::Restricted,
                context("mark-cancelled-action"),
            ),
            Err(ActionServiceError::IllegalTransition)
        ));
        service
            .state
            .actions
            .get_mut(marked_action.id())
            .unwrap()
            .state = ActionState::Completed;
        let mut stage = service.begin_decision_stage();
        assert!(matches!(
            service.mark_action_superseded_premise_from_decision(
                &mut stage,
                marked_action.id(),
                marked_action.version(),
                &source,
                DataClassification::Restricted,
                context("mark-completed-action"),
            ),
            Err(ActionServiceError::IllegalTransition)
        ));
    }

    /// The ledger persistence gap fix (2026-09): `CreateRequestFromDecision`/
    /// `MarkRequestSupersededPremise`/`MarkActionSupersededPremise` must
    /// round-trip through the same frozen `persistence_snapshot()` ->
    /// `rehydrate()` seam every other Action command already does, instead of
    /// hitting `ActionRehydrationError::UnsupportedState`.
    #[test]
    fn decision_triggered_mutations_persist_snapshot_and_rehydrate_exactly() {
        let mut service = service();
        let source = decision("decision-persist");
        let mut stage = service.begin_decision_stage();
        let created = service
            .create_resulting_action_request_from_decision_transition(
                &mut stage,
                &spec("request-persist"),
                source.clone(),
                DataClassification::Confidential,
                context("create-persist"),
            )
            .unwrap()
            .record;
        service.commit_decision_stage(stage).unwrap();

        let mut stage = service.begin_decision_stage();
        let marked_request = service
            .mark_action_request_superseded_premise_from_decision(
                &mut stage,
                created.id(),
                created.version(),
                &source,
                DataClassification::Restricted,
                context("mark-request-persist"),
            )
            .unwrap()
            .record;
        service.commit_decision_stage(stage).unwrap();

        let prepared = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: marked_request.id().clone(),
                expected_version: marked_request.version(),
                context: context("prepare-persist"),
            })
            .unwrap();
        let approval = WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("accept-persist").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval,
                context: context("accept-persist"),
            })
            .unwrap();

        let mut stage = service.begin_decision_stage();
        let marked_action = service
            .mark_action_superseded_premise_from_decision(
                &mut stage,
                accepted.action.id(),
                accepted.action.version(),
                &source,
                DataClassification::Restricted,
                context("mark-action-persist"),
            )
            .unwrap()
            .record;
        service.commit_decision_stage(stage).unwrap();

        // Must now export instead of hitting `ActionRehydrationError::UnsupportedState`.
        let snapshot = service.persistence_snapshot().unwrap();
        assert_eq!(snapshot.requests().len(), 1);
        assert_eq!(snapshot.requests()[0].id(), marked_request.id());
        assert_eq!(snapshot.actions().len(), 1);
        assert_eq!(snapshot.actions()[0].id(), marked_action.id());
        // create-from-decision, mark-request-superseded, prepare-accept,
        // execute-accept, mark-action-superseded -- every command this
        // service has ever executed, not just the 3 decision-triggered ones.
        assert_eq!(snapshot.replay().len(), 5);
        // 1 (create) + 1 (mark-request) + 3 (accept: request accepted,
        // action created, request linked) + 1 (mark-action) = 6.
        assert_eq!(snapshot.audits().len(), 6);

        let rehydrated = InMemoryActionService::rehydrate(
            TestClock(Cell::new(200)),
            TestIds(1_000),
            Allow,
            Allow,
            DenyActionEvidenceAuthority,
            snapshot,
        );
        assert_eq!(rehydrated.request(created.id()), Some(&accepted.request));
        let rehydrated_action = rehydrated.action(marked_action.id()).unwrap();
        assert_eq!(rehydrated_action, &marked_action);
        assert!(rehydrated_action.has_superseded_premise());
        assert_eq!(rehydrated_action.source_decision_id(), Some(&source));
    }

    #[test]
    fn stale_wrong_origin_terminal_and_commit_failure_have_zero_effect() {
        let mut service = service();
        let source = decision("decision-atomic");
        let mut stage = service.begin_decision_stage();
        let created = service
            .create_resulting_action_request_from_decision_transition(
                &mut stage,
                &spec("request-atomic"),
                source.clone(),
                DataClassification::Internal,
                context("create-atomic"),
            )
            .unwrap()
            .record;
        service.commit_decision_stage(stage).unwrap();
        let baseline = created.clone();
        let mut invalid_stage = service.begin_decision_stage();
        assert!(service
            .mark_action_request_superseded_premise_from_decision(
                &mut invalid_stage,
                created.id(),
                AggregateVersion::initial().next().unwrap(),
                &source,
                DataClassification::Restricted,
                context("stale-atomic"),
            )
            .is_err());
        let mut invalid_stage = service.begin_decision_stage();
        assert!(service
            .mark_action_request_superseded_premise_from_decision(
                &mut invalid_stage,
                created.id(),
                created.version(),
                &decision("decision-wrong"),
                DataClassification::Restricted,
                context("wrong-origin"),
            )
            .is_err());
        assert_eq!(service.request(created.id()), Some(&baseline));
        let mut abandoned_stage = service.begin_decision_stage();
        let first = service
            .create_resulting_action_request_from_decision_transition(
                &mut abandoned_stage,
                &spec("request-staged-first"),
                source.clone(),
                DataClassification::Internal,
                context("stage-first"),
            )
            .unwrap()
            .record;
        service
            .create_resulting_action_request_from_decision_transition(
                &mut abandoned_stage,
                &spec("request-staged-second"),
                source.clone(),
                DataClassification::Internal,
                context("stage-second"),
            )
            .unwrap();
        assert_eq!(
            abandoned_stage.action_requests_for_decision(&source).len(),
            3
        );
        assert!(service
            .mark_action_request_superseded_premise_from_decision(
                &mut abandoned_stage,
                first.id(),
                first.version(),
                &decision("decision-stage-wrong"),
                DataClassification::Restricted,
                context("stage-later-failure"),
            )
            .is_err());
        assert!(service
            .request(&ActionRequestId::parse("request-staged-first").unwrap())
            .is_none());
        drop(abandoned_stage);
        let mut failed_stage = service.begin_decision_stage();
        service
            .mark_action_request_superseded_premise_from_decision(
                &mut failed_stage,
                created.id(),
                created.version(),
                &source,
                DataClassification::Restricted,
                context("commit-failure"),
            )
            .unwrap();
        service.inject_next_commit_failure();
        assert!(matches!(
            service.commit_decision_stage(failed_stage),
            Err(ActionServiceError::InfrastructureFailure)
        ));
        assert_eq!(service.request(created.id()), Some(&baseline));
        let mut final_stage = service.begin_decision_stage();
        let finally_marked = service
            .mark_action_request_superseded_premise_from_decision(
                &mut final_stage,
                created.id(),
                created.version(),
                &source,
                DataClassification::Restricted,
                context("preflight-then-apply"),
            )
            .unwrap()
            .record;
        service.preflight_decision_stage_commit().unwrap();
        service.apply_decision_stage_unchecked(final_stage);
        assert_eq!(service.request(created.id()), Some(&finally_marked));
    }

    #[test]
    fn prepare_accept_ordinal_exhaustion_leaves_no_pending_or_idempotency_state() {
        let mut service = service();
        let source = decision("decision-ordinal-exhaustion");
        let mut stage = service.begin_decision_stage();
        let request = service
            .create_resulting_action_request_from_decision_transition(
                &mut stage,
                &spec("request-ordinal-exhaustion"),
                source,
                DataClassification::Internal,
                context("create-ordinal-exhaustion"),
            )
            .unwrap()
            .record;
        service.commit_decision_stage(stage).unwrap();
        service.state.next_operation_ordinal = u64::MAX;

        let before = service.request(request.id()).cloned().unwrap();
        let prepared_count = service.state.prepared.len();
        let idempotency_count = service.state.idem.len();
        assert!(service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: request.id().clone(),
                expected_version: request.version(),
                context: context("prepare-ordinal-exhaustion"),
            })
            .is_err());

        assert_eq!(service.request(request.id()), Some(&before));
        assert_eq!(service.state.prepared.len(), prepared_count);
        assert_eq!(service.state.idem.len(), idempotency_count);
        assert_eq!(service.state.next_operation_ordinal, u64::MAX);
    }

    fn ordinary_started_snapshot() -> ActionPersistenceSnapshot {
        let mut service = InMemoryActionService::new(
            TestClock(Cell::new(100)),
            TestIds(0),
            Allow,
            Allow,
            DenyActionEvidenceAuthority,
        );
        let created = service
            .create_action_request_draft(CreateActionRequestDraft {
                id: ActionRequestId::parse("request-persistence-tamper").unwrap(),
                title: BoundedText::parse("Synthetic persistence tamper".to_owned()).unwrap(),
                details: BoundedText::parse("Synthetic persistence details".to_owned()).unwrap(),
                intended_owner: Some(StakeholderId::parse("owner-persistence-tamper").unwrap()),
                response_due_at: None,
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
                classification: DataClassification::Internal,
                context: context("tamper-create"),
            })
            .unwrap();
        let opened = service
            .submit_action_request(SubmitActionRequest {
                request_id: created.record.id().clone(),
                expected_version: created.record.version(),
                context: context("tamper-submit"),
            })
            .unwrap();
        let prepared = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: opened.record.id().clone(),
                expected_version: opened.record.version(),
                context: context("tamper-prepare"),
            })
            .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("tamper-accept").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("tamper-accept"),
            })
            .unwrap();
        service
            .start_action(StartAction {
                action_id: accepted.action.id().clone(),
                expected_version: accepted.action.version(),
                context: context("tamper-start"),
            })
            .unwrap();
        service.persistence_snapshot().unwrap()
    }

    #[test]
    fn ordinary_start_persistence_rejects_timestamp_version_shape_and_audit_tampering() {
        let snapshot = ordinary_started_snapshot();
        let start_index = snapshot
            .replay
            .iter()
            .position(|capsule| {
                matches!(
                    capsule.command,
                    ActionPersistenceCommand::StartAction { .. }
                )
            })
            .unwrap();

        let mut timestamp_tampered = snapshot.replay.clone();
        let ActionPersistenceResult::Action(outcome) = &mut timestamp_tampered[start_index].result
        else {
            panic!("start must persist an action outcome");
        };
        outcome
            .record
            .transition_history
            .last_mut()
            .unwrap()
            .occurred_at = UtcTimestamp::from_unix_millis(999);
        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            timestamp_tampered,
            snapshot.audits.clone(),
        )
        .is_err());

        let mut version_tampered = snapshot.replay.clone();
        let ActionPersistenceCommand::StartAction {
            expected_version, ..
        } = &mut version_tampered[start_index].command
        else {
            panic!("start command must be persisted");
        };
        *expected_version = AggregateVersion::initial().next().unwrap();
        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            version_tampered,
            snapshot.audits.clone(),
        )
        .is_err());

        let mut audit_id_tampered = snapshot.replay.clone();
        audit_id_tampered[start_index].audit_event_ids[0] = snapshot.audits[0].id().clone();
        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            audit_id_tampered,
            snapshot.audits.clone(),
        )
        .is_err());

        let mut audit_order_tampered = snapshot.audits.clone();
        audit_order_tampered.reverse();
        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests,
            snapshot.actions,
            snapshot.prepared,
            snapshot.discarded_prepared,
            snapshot.replay,
            audit_order_tampered,
        )
        .is_err());
    }

    #[derive(Clone)]
    struct CompletionEvidence;
    impl ActionEvidenceAuthorityPort for CompletionEvidence {
        fn resolve(
            &self,
            id: &EvidenceReferenceId,
        ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
            if id.as_str() != "evidence-link" && id.as_str() != "evidence-link-2" {
                return Err(ActionEvidenceAuthorityError::NotFound);
            }
            Ok(EvidenceReferenceMetadata::new(
                id.clone(),
                AggregateVersion::initial(),
                DataClassification::Confidential,
                crate::work_management::EvidenceRole::ActionCompletion,
                crate::work_management::EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(110),
                    integrity_digest: crate::work_management::IntegrityDigest::parse(
                        "a".repeat(64),
                    )
                    .unwrap(),
                },
            ))
        }
    }

    #[test]
    fn ordinary_link_persistence_rejects_source_and_duplicate_evidence_tampering() {
        type EvidenceService =
            InMemoryActionService<TestClock, TestIds, Allow, Allow, CompletionEvidence>;
        let mut service = EvidenceService::new(
            TestClock(Cell::new(100)),
            TestIds(0),
            Allow,
            Allow,
            CompletionEvidence,
        );
        let created = service
            .create_action_request_draft(CreateActionRequestDraft {
                id: ActionRequestId::parse("request-link-tamper").unwrap(),
                title: BoundedText::parse("Synthetic link tamper".to_owned()).unwrap(),
                details: BoundedText::parse("Synthetic link details".to_owned()).unwrap(),
                intended_owner: Some(StakeholderId::parse("owner-link-tamper").unwrap()),
                response_due_at: None,
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
                classification: DataClassification::Internal,
                context: context("link-create"),
            })
            .unwrap();
        let opened = service
            .submit_action_request(SubmitActionRequest {
                request_id: created.record.id().clone(),
                expected_version: created.record.version(),
                context: context("link-submit"),
            })
            .unwrap();
        let prepared = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: opened.record.id().clone(),
                expected_version: opened.record.version(),
                context: context("link-prepare"),
            })
            .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("link-accept").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("link-accept"),
            })
            .unwrap();
        let started = service
            .start_action(StartAction {
                action_id: accepted.action.id().clone(),
                expected_version: accepted.action.version(),
                context: context("link-start"),
            })
            .unwrap();
        service
            .link_action_completion_evidence(LinkActionCompletionEvidence {
                action_id: started.record.id().clone(),
                expected_version: started.record.version(),
                evidence_id: EvidenceReferenceId::parse("evidence-link").unwrap(),
                context: context("link-first"),
            })
            .unwrap();
        service
            .link_action_completion_evidence(LinkActionCompletionEvidence {
                action_id: started.record.id().clone(),
                expected_version: started.record.version().next().unwrap(),
                evidence_id: EvidenceReferenceId::parse("evidence-link-2").unwrap(),
                context: context("link-second"),
            })
            .unwrap();
        let snapshot = service.persistence_snapshot().unwrap();
        let link_indices: Vec<_> = snapshot
            .replay
            .iter()
            .enumerate()
            .filter_map(|(index, capsule)| {
                matches!(
                    capsule.command,
                    ActionPersistenceCommand::LinkCompletionEvidence { .. }
                )
                .then_some(index)
            })
            .collect();
        assert_eq!(link_indices.len(), 2);

        let mut source_tampered = snapshot.replay.clone();
        let ActionPersistenceCommand::LinkCompletionEvidence {
            evidence_classification,
            ..
        } = &mut source_tampered[link_indices[0]].command
        else {
            panic!("link command must persist source classification");
        };
        *evidence_classification = DataClassification::Restricted;
        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            source_tampered,
            snapshot.audits.clone(),
        )
        .is_err());

        let mut duplicate_tampered = snapshot.replay.clone();
        let first_evidence = match &duplicate_tampered[link_indices[0]].command {
            ActionPersistenceCommand::LinkCompletionEvidence { evidence_id, .. } => {
                evidence_id.clone()
            }
            _ => unreachable!(),
        };
        let ActionPersistenceCommand::LinkCompletionEvidence { evidence_id, .. } =
            &mut duplicate_tampered[link_indices[1]].command
        else {
            panic!("link command must persist evidence identity");
        };
        *evidence_id = first_evidence;
        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests,
            snapshot.actions,
            snapshot.prepared,
            snapshot.discarded_prepared,
            duplicate_tampered,
            snapshot.audits,
        )
        .is_err());
    }

    fn cancelled_h2_snapshot() -> ActionPersistenceSnapshot {
        let mut service = InMemoryActionService::new(
            TestClock(Cell::new(100)),
            TestIds(0),
            Allow,
            Allow,
            DenyActionEvidenceAuthority,
        );
        let created = service
            .create_action_request_draft(CreateActionRequestDraft {
                id: ActionRequestId::parse("request-h2-persistence").unwrap(),
                title: BoundedText::parse("Synthetic H2 persistence".to_owned()).unwrap(),
                details: BoundedText::parse("Synthetic H2 persistence details".to_owned()).unwrap(),
                intended_owner: Some(StakeholderId::parse("owner-h2-persistence").unwrap()),
                response_due_at: None,
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
                classification: DataClassification::Internal,
                context: context("h2-create"),
            })
            .unwrap();
        let opened = service
            .submit_action_request(SubmitActionRequest {
                request_id: created.record.id().clone(),
                expected_version: created.record.version(),
                context: context("h2-submit"),
            })
            .unwrap();
        let prepared_accept = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: opened.record.id().clone(),
                expected_version: opened.record.version(),
                context: context("h2-prepare-accept"),
            })
            .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared_accept.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared_accept.payload_digest().clone(),
                    IdempotencyId::parse("h2-accept").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("h2-accept"),
            })
            .unwrap();
        let prepared_cancel = service
            .prepare_cancel_action(PrepareCancelAction {
                action_id: accepted.action.id().clone(),
                expected_version: accepted.action.version(),
                reason: BoundedText::parse("Synthetic cancellation".to_owned()).unwrap(),
                context: context("h2-prepare-cancel"),
            })
            .unwrap();
        service
            .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
                approval: WorkManagementApproval::new(
                    prepared_cancel.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared_cancel.payload_digest().clone(),
                    IdempotencyId::parse("h2-cancel").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("h2-cancel"),
            })
            .unwrap();
        service.persistence_snapshot().unwrap()
    }

    fn preview_changed_terminal_snapshot() -> ActionPersistenceSnapshot {
        let mut service = service();
        let created = service
            .create_action_request_draft(CreateActionRequestDraft {
                id: ActionRequestId::parse("request-terminal-tamper").unwrap(),
                title: BoundedText::parse("Synthetic terminal tamper".to_owned()).unwrap(),
                details: BoundedText::parse("Synthetic terminal tamper details".to_owned())
                    .unwrap(),
                intended_owner: Some(StakeholderId::parse("owner-terminal-tamper").unwrap()),
                response_due_at: None,
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
                classification: DataClassification::Internal,
                context: context("terminal-tamper-create"),
            })
            .unwrap();
        let opened = service
            .submit_action_request(SubmitActionRequest {
                request_id: created.record.id().clone(),
                expected_version: created.record.version(),
                context: context("terminal-tamper-submit"),
            })
            .unwrap();
        let prepared_accept = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: opened.record.id().clone(),
                expected_version: opened.record.version(),
                context: context("terminal-tamper-prepare-accept"),
            })
            .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared_accept.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared_accept.payload_digest().clone(),
                    IdempotencyId::parse("terminal-tamper-accept").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("terminal-tamper-accept"),
            })
            .unwrap();
        let prepared_cancel = service
            .prepare_cancel_action(PrepareCancelAction {
                action_id: accepted.action.id().clone(),
                expected_version: accepted.action.version(),
                reason: BoundedText::parse("Synthetic terminal cancellation".to_owned()).unwrap(),
                context: context("terminal-tamper-prepare-cancel"),
            })
            .unwrap();
        service
            .start_action(StartAction {
                action_id: accepted.action.id().clone(),
                expected_version: accepted.action.version(),
                context: context("terminal-tamper-start"),
            })
            .unwrap();
        service
            .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
                approval: WorkManagementApproval::new(
                    prepared_cancel.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared_cancel.payload_digest().clone(),
                    IdempotencyId::parse("terminal-tamper-execute").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("terminal-tamper-execute"),
            })
            .unwrap_err();
        service.persistence_snapshot().unwrap()
    }

    fn retime_audit(audit: &AuditEvent, occurred_at: UtcTimestamp) -> AuditEvent {
        AuditEvent::new(
            audit.id().clone(),
            occurred_at,
            audit.actor(),
            AuditAction::new(audit.module(), audit.code().clone(), audit.target().clone()),
            audit.correlation_id().clone(),
            AuditDisposition::new(
                audit.policy_outcome(),
                audit.approval_outcome(),
                audit.execution_outcome(),
                audit.effect_scope(),
                audit.actual_effects().to_vec(),
            )
            .unwrap(),
        )
    }

    fn snapshot_parts_reject(
        snapshot: &ActionPersistenceSnapshot,
        replay: Vec<ActionReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> bool {
        ActionPersistenceSnapshot::try_new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            replay,
            audits,
        )
        .is_err()
    }

    #[test]
    fn typed_decode_input_rehydrates_exactly_and_rejects_terminal_or_discard_tampering() {
        let snapshot = preview_changed_terminal_snapshot();
        let Ok(decoded) = ActionPersistenceDecodeInput::new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            snapshot.replay.clone(),
            snapshot.audits.clone(),
        )
        .decode() else {
            panic!("typed Action decode must use the canonical snapshot validator");
        };
        assert_eq!(decoded.requests, snapshot.requests);
        assert_eq!(decoded.actions, snapshot.actions);
        assert_eq!(decoded.prepared, snapshot.prepared);
        assert_eq!(decoded.discarded_prepared, snapshot.discarded_prepared);
        assert_eq!(decoded.replay, snapshot.replay);
        assert_eq!(decoded.audits, snapshot.audits);

        let Some(terminal_index) = snapshot
            .replay
            .iter()
            .position(|capsule| matches!(capsule.result, ActionPersistenceResult::Terminal { .. }))
        else {
            panic!("fixture must contain a terminal result");
        };
        let mut terminal_tampered = snapshot.replay.clone();
        let ActionPersistenceResult::Terminal { error, .. } =
            &mut terminal_tampered[terminal_index].result
        else {
            unreachable!();
        };
        *error = error
            .clone()
            .with_param(MessageParam::new("tampered", SafeParamValue::Boolean(true)).unwrap());
        assert!(ActionPersistenceDecodeInput::new(
            snapshot.requests.clone(),
            snapshot.actions.clone(),
            snapshot.prepared.clone(),
            snapshot.discarded_prepared.clone(),
            terminal_tampered,
            snapshot.audits.clone(),
        )
        .decode()
        .is_err());

        let mut discarded_tampered = snapshot.discarded_prepared.clone();
        let Some(retained) = snapshot.prepared.first() else {
            panic!("fixture must retain the prepared intent");
        };
        discarded_tampered.push(retained.clone());
        assert!(ActionPersistenceDecodeInput::new(
            snapshot.requests,
            snapshot.actions,
            snapshot.prepared,
            discarded_tampered,
            snapshot.replay,
            snapshot.audits,
        )
        .decode()
        .is_err());
    }

    #[test]
    fn terminal_failure_snapshot_binds_kind_digest_actor_error_and_audit_contract() {
        let snapshot = preview_changed_terminal_snapshot();
        let terminal_index = snapshot
            .replay
            .iter()
            .position(|capsule| matches!(capsule.result, ActionPersistenceResult::Terminal { .. }))
            .unwrap();
        let other_digest = snapshot
            .replay
            .iter()
            .find_map(|capsule| match &capsule.result {
                ActionPersistenceResult::Prepared(intent) => Some(intent.payload_digest().clone()),
                _ => None,
            })
            .unwrap();

        let mut kind_tampered = snapshot.replay.clone();
        let ActionPersistenceCommand::ExecuteAction { kind, .. } =
            &mut kind_tampered[terminal_index].command
        else {
            unreachable!();
        };
        *kind = ActionPersistenceTransitionKind::Complete;
        assert!(snapshot_parts_reject(
            &snapshot,
            kind_tampered,
            snapshot.audits.clone()
        ));

        let mut digest_tampered = snapshot.replay.clone();
        let ActionPersistenceCommand::ExecuteAction {
            acknowledged_digest,
            ..
        } = &mut digest_tampered[terminal_index].command
        else {
            unreachable!();
        };
        *acknowledged_digest = other_digest;
        assert!(snapshot_parts_reject(
            &snapshot,
            digest_tampered,
            snapshot.audits.clone()
        ));

        let mut actor_tampered = snapshot.replay.clone();
        let ActionPersistenceCommand::ExecuteAction { actor, .. } =
            &mut actor_tampered[terminal_index].command
        else {
            unreachable!();
        };
        *actor = AuditActor::PolicyAuthorizedSystem;
        let ActionPersistenceResult::Terminal { audit, .. } =
            &mut actor_tampered[terminal_index].result
        else {
            unreachable!();
        };
        let altered_actor_audit = AuditEvent::new(
            audit.id().clone(),
            audit.occurred_at(),
            AuditActor::PolicyAuthorizedSystem,
            AuditAction::new(audit.module(), audit.code().clone(), audit.target().clone()),
            audit.correlation_id().clone(),
            AuditDisposition::new(
                audit.policy_outcome(),
                audit.approval_outcome(),
                audit.execution_outcome(),
                audit.effect_scope(),
                audit.actual_effects().to_vec(),
            )
            .unwrap(),
        );
        *audit = altered_actor_audit.clone();
        let mut actor_audits = snapshot.audits.clone();
        let altered_actor_audit_id = altered_actor_audit.id().clone();
        *actor_audits
            .iter_mut()
            .find(|item| item.id() == &altered_actor_audit_id)
            .unwrap() = altered_actor_audit;
        assert!(snapshot_parts_reject(
            &snapshot,
            actor_tampered,
            actor_audits
        ));

        let mut error_tampered = snapshot.replay.clone();
        let ActionPersistenceResult::Terminal { error, audit, .. } =
            &mut error_tampered[terminal_index].result
        else {
            unreachable!();
        };
        *error = domain_error_with_snapshot(
            ActionServiceError::PolicyDenied,
            audit.correlation_id().clone(),
            None,
        );
        assert!(snapshot_parts_reject(
            &snapshot,
            error_tampered,
            snapshot.audits.clone()
        ));

        let mut module_tampered = snapshot.replay.clone();
        let ActionPersistenceResult::Terminal { audit, .. } =
            &mut module_tampered[terminal_index].result
        else {
            unreachable!();
        };
        let altered_module_audit = AuditEvent::new(
            audit.id().clone(),
            audit.occurred_at(),
            audit.actor(),
            AuditAction::new(
                AuditModule::Portfolio,
                audit.code().clone(),
                audit.target().clone(),
            ),
            audit.correlation_id().clone(),
            AuditDisposition::new(
                audit.policy_outcome(),
                audit.approval_outcome(),
                audit.execution_outcome(),
                audit.effect_scope(),
                audit.actual_effects().to_vec(),
            )
            .unwrap(),
        );
        *audit = altered_module_audit.clone();
        let mut module_audits = snapshot.audits.clone();
        let altered_module_audit_id = altered_module_audit.id().clone();
        *module_audits
            .iter_mut()
            .find(|item| item.id() == &altered_module_audit_id)
            .unwrap() = altered_module_audit;
        assert!(snapshot_parts_reject(
            &snapshot,
            module_tampered,
            module_audits
        ));
    }

    #[test]
    fn h3_terminal_snapshot_rejects_command_cause_substitution() {
        let started = ordinary_started_snapshot();
        let action = started.actions[0].clone();
        let mut service = Service::rehydrate(
            TestClock(Cell::new(100)),
            TestIds(100),
            Allow,
            Allow,
            DenyActionEvidenceAuthority,
            started,
        );
        service
            .prepare_complete_action(PrepareCompleteAction {
                action_id: action.id().clone(),
                expected_version: action.version(),
                judgment: None,
                context: context("h3-cause-tamper"),
            })
            .unwrap_err();
        let snapshot = service.persistence_snapshot().unwrap();
        let mut replay = snapshot.replay.clone();
        let terminal = replay
            .iter_mut()
            .find(|capsule| matches!(capsule.result, ActionPersistenceResult::Terminal { .. }))
            .unwrap();
        let ActionPersistenceResult::Terminal { cause, .. } = &mut terminal.result else {
            unreachable!();
        };
        *cause = ActionPersistenceTerminalCause::H3Denied(
            ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence,
        );
        assert!(snapshot_parts_reject(
            &snapshot,
            replay,
            snapshot.audits.clone()
        ));
    }

    #[test]
    fn terminal_h2_snapshot_rejects_self_consistent_expired_execution_audit() {
        let snapshot = cancelled_h2_snapshot();
        let execute_index = snapshot
            .replay
            .iter()
            .position(|capsule| {
                matches!(
                    capsule.command,
                    ActionPersistenceCommand::ExecuteAction {
                        kind: ActionPersistenceTransitionKind::Cancel,
                        ..
                    }
                )
            })
            .unwrap();
        let prepared_id = match &snapshot.replay[execute_index].command {
            ActionPersistenceCommand::ExecuteAction { prepared_id, .. } => prepared_id,
            _ => unreachable!(),
        };
        let expires_at = snapshot
            .replay
            .iter()
            .find_map(|capsule| match &capsule.result {
                ActionPersistenceResult::Prepared(intent) if intent.id() == prepared_id => {
                    Some(intent.preview().expires_at())
                }
                _ => None,
            })
            .unwrap();

        let mut replay = snapshot.replay.clone();
        let mut actions = snapshot.actions.clone();
        let mut audits = snapshot.audits.clone();
        let ActionPersistenceResult::Action(outcome) = &mut replay[execute_index].result else {
            unreachable!();
        };
        let audit_id = outcome.audit_events[0].id().clone();
        let retimed = retime_audit(&outcome.audit_events[0], expires_at);
        outcome.audit_events[0] = retimed.clone();
        outcome
            .record
            .transition_history
            .last_mut()
            .unwrap()
            .occurred_at = expires_at;
        actions[0]
            .transition_history
            .last_mut()
            .unwrap()
            .occurred_at = expires_at;
        *audits
            .iter_mut()
            .find(|audit| audit.id() == &audit_id)
            .unwrap() = retimed;

        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests,
            actions,
            snapshot.prepared,
            snapshot.discarded_prepared,
            replay,
            audits,
        )
        .is_err());
    }

    #[test]
    fn expired_terminal_snapshot_rejects_audit_before_preview_expiry() {
        let mut service = service();
        let created = service
            .create_action_request_draft(CreateActionRequestDraft {
                id: ActionRequestId::parse("request-expired-terminal-tamper").unwrap(),
                title: BoundedText::parse("Synthetic expired terminal tamper".to_owned()).unwrap(),
                details: BoundedText::parse("Synthetic expired terminal tamper details".to_owned())
                    .unwrap(),
                intended_owner: Some(StakeholderId::parse("owner-expired-terminal").unwrap()),
                response_due_at: None,
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
                classification: DataClassification::Internal,
                context: context("expired-terminal-create"),
            })
            .unwrap();
        let opened = service
            .submit_action_request(SubmitActionRequest {
                request_id: created.record.id().clone(),
                expected_version: created.record.version(),
                context: context("expired-terminal-submit"),
            })
            .unwrap();
        let prepared_accept = service
            .prepare_accept_action_request(PrepareAcceptActionRequest {
                request_id: opened.record.id().clone(),
                expected_version: opened.record.version(),
                context: context("expired-terminal-prepare-accept"),
            })
            .unwrap();
        let accepted = service
            .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared_accept.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared_accept.payload_digest().clone(),
                    IdempotencyId::parse("expired-terminal-accept").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("expired-terminal-accept"),
            })
            .unwrap();
        let prepared_cancel = service
            .prepare_cancel_action(PrepareCancelAction {
                action_id: accepted.action.id().clone(),
                expected_version: accepted.action.version(),
                reason: BoundedText::parse("Synthetic expired terminal cancellation".to_owned())
                    .unwrap(),
                context: context("expired-terminal-prepare-cancel"),
            })
            .unwrap();
        let expires_at = prepared_cancel.preview().expires_at();
        service.clock.0.set(expires_at.unix_millis());
        let approval = WorkManagementApproval::new(
            prepared_cancel.id().clone(),
            AuditActor::HeadOfProducts,
            prepared_cancel.payload_digest().clone(),
            IdempotencyId::parse("expired-terminal-execute").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap();
        service
            .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
                approval,
                context: context("expired-terminal-execute"),
            })
            .unwrap_err();

        let snapshot = service.persistence_snapshot().unwrap();
        let terminal_index = snapshot
            .replay
            .iter()
            .position(|capsule| {
                matches!(
                    &capsule.result,
                    ActionPersistenceResult::Terminal {
                        cause: ActionPersistenceTerminalCause::Expired,
                        ..
                    }
                )
            })
            .unwrap();
        let mut replay = snapshot.replay.clone();
        let mut audits = snapshot.audits.clone();
        let ActionPersistenceResult::Terminal { audit, .. } = &mut replay[terminal_index].result
        else {
            unreachable!();
        };
        let audit_id = audit.id().clone();
        let retimed = retime_audit(
            audit,
            UtcTimestamp::from_unix_millis(expires_at.unix_millis() - 1),
        );
        *audit = retimed.clone();
        *audits
            .iter_mut()
            .find(|item| item.id() == &audit_id)
            .unwrap() = retimed;

        let result = ActionPersistenceSnapshot::try_new(
            snapshot.requests,
            snapshot.actions,
            snapshot.prepared,
            snapshot.discarded_prepared,
            replay,
            audits,
        );
        assert!(matches!(result, Err(ActionRehydrationError::AuditMismatch)));
    }

    #[test]
    fn terminal_h2_snapshot_rejects_self_consistent_duplicate_receipt() {
        let mut snapshot = cancelled_h2_snapshot();
        let action_id = snapshot.actions[0].id.clone();
        let cancelled = match &snapshot.replay.iter().find(|capsule| {
            matches!(
                capsule.command,
                ActionPersistenceCommand::ExecuteAction {
                    kind: ActionPersistenceTransitionKind::Cancel,
                    ..
                }
            )
        }) {
            Some(ActionReplayCapsule {
                result: ActionPersistenceResult::Action(outcome),
                ..
            }) => outcome.clone(),
            _ => unreachable!(),
        };

        let mut service = InMemoryActionService::rehydrate(
            TestClock(Cell::new(100)),
            TestIds(100),
            Allow,
            Allow,
            DenyActionEvidenceAuthority,
            snapshot.clone(),
        );
        let prepared_reopen = service
            .prepare_reopen_action(PrepareReopenAction {
                action_id,
                expected_version: cancelled.record.version(),
                mode: ActionReopenMode::RestartCancelled,
                reason: BoundedText::parse("Synthetic restart".to_owned()).unwrap(),
                context: context("h2-prepare-reopen"),
            })
            .unwrap();
        service
            .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
                approval: WorkManagementApproval::new(
                    prepared_reopen.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared_reopen.payload_digest().clone(),
                    IdempotencyId::parse("h2-reopen").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: context("h2-reopen"),
            })
            .unwrap();
        snapshot = service.persistence_snapshot().unwrap();

        let first_receipt = match snapshot.replay.iter().find(|capsule| {
            matches!(
                capsule.command,
                ActionPersistenceCommand::ExecuteAction {
                    kind: ActionPersistenceTransitionKind::Cancel,
                    ..
                }
            )
        }) {
            Some(ActionReplayCapsule {
                result: ActionPersistenceResult::Action(outcome),
                ..
            }) => outcome.approval_receipt_id.clone().unwrap(),
            _ => unreachable!(),
        };
        let reopen_index = snapshot
            .replay
            .iter()
            .position(|capsule| {
                matches!(
                    capsule.command,
                    ActionPersistenceCommand::ExecuteAction {
                        kind: ActionPersistenceTransitionKind::Reopen,
                        ..
                    }
                )
            })
            .unwrap();
        let ActionPersistenceResult::Action(outcome) = &mut snapshot.replay[reopen_index].result
        else {
            unreachable!();
        };
        outcome.approval_receipt_id = Some(first_receipt.clone());
        outcome
            .record
            .transition_history
            .last_mut()
            .unwrap()
            .approval_receipt_id = Some(first_receipt.clone());
        snapshot.actions[0]
            .transition_history
            .last_mut()
            .unwrap()
            .approval_receipt_id = Some(first_receipt);

        assert!(ActionPersistenceSnapshot::try_new(
            snapshot.requests,
            snapshot.actions,
            snapshot.prepared,
            snapshot.discarded_prepared,
            snapshot.replay,
            snapshot.audits,
        )
        .is_err());
    }
}
