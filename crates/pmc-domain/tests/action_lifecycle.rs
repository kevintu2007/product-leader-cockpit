use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pmc_domain::actions::*;
use pmc_domain::audit::{
    AuditActor, AuditApprovalOutcome, AuditEffectScope, AuditExecutionOutcome,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::{ErrorCode, SafeErrorExtension};
use pmc_domain::identity::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ActionReopenMode, ActionRequestState, ActionState, ApprovalAuthorizationPort,
    ApprovalConfirmation, EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification,
    HumanJudgment, HumanJudgmentDisposition, IntegrityDigest, RejectedPreparedIntentOutcome,
    WorkManagementApproval, WorkManagementOperation,
};

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

struct TestIds {
    counter: u64,
    control: Rc<Cell<i64>>,
}
impl ActionServiceIdSource for TestIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        self.counter += 1;
        ActionId::parse(format!("action-{}", self.counter))
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.counter += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.counter))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.counter += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.counter))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        if self.control.get() == -3 {
            return AuditEventId::parse("");
        }
        self.counter += 1;
        AuditEventId::parse(format!("audit-{}", self.counter))
    }
}

#[derive(Clone)]
struct TestAuthorization(Rc<Cell<i64>>);
impl ApprovalAuthorizationPort for TestAuthorization {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts && self.0.get() != -2
    }
}
#[derive(Clone)]
struct TestPolicy(Rc<Cell<i64>>);
impl ActionExecutionPolicyPort for TestPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        if self.0.get() == -1 {
            ActionExecutionPolicy::Denied
        } else {
            ActionExecutionPolicy::Allowed
        }
    }
}

#[derive(Clone, Default)]
struct TestEvidenceAuthority(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl ActionEvidenceAuthorityPort for TestEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        self.0
            .borrow()
            .get(id)
            .cloned()
            .ok_or(ActionEvidenceAuthorityError::NotFound)
    }
}
impl TestEvidenceAuthority {
    fn put(&self, metadata: EvidenceReferenceMetadata) {
        self.0.borrow_mut().insert(metadata.id().clone(), metadata);
    }
}

type Service =
    InMemoryActionService<TestClock, TestIds, TestAuthorization, TestPolicy, TestEvidenceAuthority>;
fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn ctx(id: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn ctx_corr(id: &str, corr: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(corr).unwrap(),
    }
}
fn service() -> (Service, Rc<Cell<i64>>, TestEvidenceAuthority) {
    let now = Rc::new(Cell::new(100));
    let evidence = TestEvidenceAuthority::default();
    (
        InMemoryActionService::new(
            TestClock(now.clone()),
            TestIds {
                counter: 0,
                control: now.clone(),
            },
            TestAuthorization(now.clone()),
            TestPolicy(now.clone()),
            evidence.clone(),
        ),
        now,
        evidence,
    )
}
fn owner() -> StakeholderId {
    StakeholderId::parse("owner-product-lead").unwrap()
}
fn draft(s: &mut Service, id: &str, has_owner: bool, has_due: bool) -> ActionRequestRecord {
    s.create_action_request_draft(CreateActionRequestDraft {
        id: ActionRequestId::parse(id).unwrap(),
        title: text("Prepare synthetic launch"),
        details: text("Synthetic commitment details"),
        intended_owner: has_owner.then(owner),
        response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
        intended_action_due_at: has_due.then(|| UtcTimestamp::from_unix_millis(900)),
        classification: DataClassification::Internal,
        context: ctx(&format!("create-{id}")),
    })
    .unwrap()
    .record
}
fn open(s: &mut Service, id: &str) -> ActionRequestRecord {
    let record = draft(s, id, true, true);
    s.submit_action_request(SubmitActionRequest {
        request_id: record.id().clone(),
        expected_version: record.version(),
        context: ctx(&format!("submit-{id}")),
    })
    .unwrap()
    .record
}
fn approval(
    prepared: &pmc_domain::work_management::WorkManagementPreparedIntent,
    id: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(id).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}
fn accepted_action(s: &mut Service, id: &str) -> AcceptedActionOutcome {
    let request = open(s, id);
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx(&format!("prepare-{id}")),
        })
        .unwrap();
    s.approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
        approval: approval(&prepared, &format!("accept-{id}")),
        context: ctx(&format!("accept-{id}")),
    })
    .unwrap()
}
fn verified(id: &str, class: DataClassification) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse(id).unwrap(),
        AggregateVersion::initial(),
        class,
        EvidenceRole::ActionCompletion,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(90),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    )
}

#[test]
fn canonical_states_round_trip_and_request_terminals_fail_with_safe_context() {
    assert_eq!(
        ActionRequestState::from_persisted("accepted").unwrap(),
        ActionRequestState::Accepted
    );
    assert_eq!(
        ActionState::from_persisted("in_progress").unwrap(),
        ActionState::InProgress
    );
    let (mut s, _, _) = service();
    let request = open(&mut s, "request-terminal");
    let declined = s
        .decline_action_request(DeclineActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            rationale: text("Capacity is unavailable"),
            context: ctx("decline"),
        })
        .unwrap()
        .record;
    let error = s
        .submit_action_request(SubmitActionRequest {
            request_id: declined.id().clone(),
            expected_version: declined.version(),
            context: ctx("illegal-terminal"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::DomainConflict);
    assert_eq!(error.correlation_id().as_str(), "corr-illegal-terminal");
    assert!(error
        .extensions()
        .contains(&SafeErrorExtension::CurrentVersion(declined.version())));
    assert!(error.params().iter().any(|p| p.key() == "current_state"));
}

#[test]
fn user_created_request_and_accepted_action_have_no_decision_origin_or_attention_flag() {
    let (mut service, _, _) = service();
    let request = open(&mut service, "request-user-origin");
    assert_eq!(request.source_decision_id(), None);
    assert!(!request.has_superseded_premise());
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-user-origin"),
        })
        .unwrap();
    let accepted = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval(&prepared, "accept-user-origin"),
            context: ctx("accept-user-origin"),
        })
        .unwrap();
    assert_eq!(accepted.request.source_decision_id(), None);
    assert!(!accepted.request.has_superseded_premise());
    assert_eq!(accepted.action.source_decision_id(), None);
    assert!(!accepted.action.has_superseded_premise());
}

#[test]
fn h1_business_idempotency_replays_across_correlation_and_rejects_changed_payload() {
    let (mut s, _, _) = service();
    let command = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-idempotent").unwrap(),
        title: text("Stable synthetic request"),
        details: text("Stable business fields"),
        intended_owner: Some(owner()),
        response_due_at: None,
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
        classification: DataClassification::Internal,
        context: ctx_corr("create-business-key", "corr-create-first"),
    };
    let first = s.create_action_request_draft(command.clone()).unwrap();
    let mut replay = command.clone();
    replay.context = ctx_corr("create-business-key", "corr-create-retry");
    assert_eq!(s.create_action_request_draft(replay).unwrap(), first);
    let mut changed = command;
    changed.title = text("Changed synthetic request");
    changed.context = ctx_corr("create-business-key", "corr-create-changed");
    let error = s.create_action_request_draft(changed).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);
}

#[test]
fn accept_preview_binds_exact_commitment_and_execution_creates_and_links_once() {
    let (mut s, _, _) = service();
    let request = open(&mut s, "request-accept");
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-accept"),
        })
        .unwrap();
    let prepare_replay = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx_corr("prepare-accept", "corr-prepare-retry"),
        })
        .unwrap();
    assert_eq!(prepared, prepare_replay);
    match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
            ..
        } => {
            assert_eq!(request_id, request.id());
            assert_eq!(*request_version, request.version());
            assert_eq!(action_subject.as_str(), request.title().as_str());
            assert_eq!(commitment_details.as_str(), request.details().as_str());
            assert_eq!(intended_owner, request.intended_owner().unwrap());
            assert_eq!(*intended_due_at, request.intended_action_due_at().unwrap());
        }
        _ => panic!("wrong operation"),
    }
    let approval = approval(&prepared, "accept-key");
    let first = s
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval.clone(),
            context: ctx_corr("accept-key", "corr-first"),
        })
        .unwrap();
    let replay = s
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval,
            context: ctx_corr("accept-key", "corr-retry"),
        })
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.request.state(), ActionRequestState::Accepted);
    assert_eq!(first.action.state(), ActionState::Open);
    assert_eq!(first.request.linked_action_id(), Some(first.action.id()));
    assert_eq!(first.audit_events.len(), 3);
    assert_eq!(
        first.audit_events[2].code().as_str(),
        "action_request.action_linked"
    );
    assert!(first
        .audit_events
        .iter()
        .all(|e| e.approval_outcome() == AuditApprovalOutcome::Approved
            && e.effect_scope() == AuditEffectScope::Complete));
}

#[test]
fn evidence_is_resolved_from_authority_and_mutation_after_prepare_has_zero_effect() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-evidence");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-evidence"),
        })
        .unwrap()
        .record;
    let metadata = verified("evidence-authoritative", DataClassification::Internal);
    authority.put(metadata.clone());
    let linked = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: metadata.id().clone(),
            context: ctx("link-evidence"),
        })
        .unwrap()
        .record;
    let prepared = s
        .prepare_complete_action(PrepareCompleteAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            judgment: None,
            context: ctx("prepare-complete"),
        })
        .unwrap();
    authority.put(EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("evidence-authoritative").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::ActionCompletion,
        EvidenceVerification::IntegrityMismatch,
    ));
    let approval = approval(&prepared, "complete-mutation");
    let before = s.action(linked.id()).unwrap().clone();
    let verification_error = s
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: approval.clone(),
            context: ctx("complete-mutation"),
        })
        .unwrap_err();
    assert_eq!(
        verification_error.code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(s.action(linked.id()).unwrap(), &before);
    authority.put(verified(
        "evidence-authoritative",
        DataClassification::Restricted,
    ));
    let error = s
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval,
            context: ctx("complete-mutation"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(s.action(linked.id()).unwrap(), &before);
    assert_eq!(
        s.audit_events().last().unwrap().effect_scope(),
        AuditEffectScope::None
    );
}

#[test]
fn degraded_completion_support_and_receipt_remain_in_immutable_history_after_reopen() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-history");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-history"),
        })
        .unwrap()
        .record;
    let evidence = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("evidence-degraded").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Confidential,
        EvidenceRole::ActionCompletion,
        EvidenceVerification::DegradedLastVerified {
            last_verified_at: UtcTimestamp::from_unix_millis(80),
            integrity_digest: IntegrityDigest::parse("b".repeat(64)).unwrap(),
        },
    );
    authority.put(evidence.clone());
    let linked = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: evidence.id().clone(),
            context: ctx("link-degraded"),
        })
        .unwrap()
        .record;
    let judgment = HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Synthetic degraded-mode judgment",
        DataClassification::Confidential,
    )
    .unwrap();
    let complete = s
        .prepare_complete_action(PrepareCompleteAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            judgment: Some(judgment),
            context: ctx("prepare-degraded"),
        })
        .unwrap();
    let completed = s
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: approval(&complete, "complete-degraded"),
            context: ctx("complete-degraded"),
        })
        .unwrap()
        .record;
    let reopen = s
        .prepare_reopen_action(PrepareReopenAction {
            action_id: completed.id().clone(),
            expected_version: completed.version(),
            mode: ActionReopenMode::ReopenCompleted,
            reason: text("New synthetic evidence"),
            context: ctx("prepare-reopen"),
        })
        .unwrap();
    let reopened = s
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: approval(&reopen, "reopen-degraded"),
            context: ctx("reopen-degraded"),
        })
        .unwrap()
        .record;
    let completion = &reopened.transition_history()[1];
    assert_eq!(completion.to(), ActionState::Completed);
    assert!(completion.support().is_some());
    assert!(completion.approval_receipt_id().is_some());
    assert_eq!(
        reopened.transition_history()[2].reason().unwrap().as_str(),
        "New synthetic evidence"
    );
}

#[test]
fn stale_illegal_and_policy_denied_errors_and_audits_are_safe_and_truthful() {
    let (mut s, control, _) = service();
    let request = open(&mut s, "request-errors");
    let stale = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: AggregateVersion::initial(),
            context: ctx("stale"),
        })
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);
    assert_eq!(stale.message_key().as_str(), "action.domain_conflict");
    assert!(stale
        .extensions()
        .contains(&SafeErrorExtension::CurrentVersion(request.version())));
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-policy"),
        })
        .unwrap();
    control.set(-1);
    let error = s
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval(&prepared, "policy-denied"),
            context: ctx("policy-denied"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPolicyDenied);
    let audit = s.audit_events().last().unwrap();
    assert_eq!(
        audit.policy_outcome(),
        pmc_domain::audit::AuditPolicyOutcome::Denied
    );
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());
    assert_eq!(
        s.request(request.id()).unwrap().state(),
        ActionRequestState::Open
    );
}

#[test]
fn all_h2_action_transitions_are_atomic_and_business_idempotency_ignores_new_correlation() {
    let (mut s, _, _) = service();
    let accepted = accepted_action(&mut s, "request-rollback");
    let cancel = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            reason: text("Synthetic reprioritization"),
            context: ctx("prepare-cancel"),
        })
        .unwrap();
    let approval = approval(&cancel, "cancel-key");
    s.inject_next_commit_failure();
    let failure = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: approval.clone(),
            context: ctx_corr("cancel-key", "corr-cancel-first"),
        })
        .unwrap_err();
    assert_eq!(failure.code(), ErrorCode::PlatformInternal);
    assert_eq!(
        s.action(accepted.action.id()).unwrap().state(),
        ActionState::Open
    );
    let cancelled = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: approval.clone(),
            context: ctx_corr("cancel-key", "corr-cancel-retry"),
        })
        .unwrap();
    let replay = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval,
            context: ctx_corr("cancel-key", "corr-cancel-third"),
        })
        .unwrap();
    assert_eq!(cancelled, replay);
    assert_eq!(cancelled.record.state(), ActionState::Cancelled);
    let wrong = s
        .prepare_reopen_action(PrepareReopenAction {
            action_id: cancelled.record.id().clone(),
            expected_version: cancelled.record.version(),
            mode: ActionReopenMode::ReopenCompleted,
            reason: text("Wrong mode"),
            context: ctx("wrong-mode"),
        })
        .unwrap_err();
    assert_eq!(wrong.code(), ErrorCode::DomainConflict);
}

#[test]
fn default_evidence_authority_fails_closed_without_accepting_caller_metadata() {
    let now = Rc::new(Cell::new(100));
    let mut service = InMemoryActionService::new(
        TestClock(now.clone()),
        TestIds {
            counter: 0,
            control: now.clone(),
        },
        TestAuthorization(now.clone()),
        TestPolicy(now),
        DenyActionEvidenceAuthority,
    );
    let request = open_deny(&mut service);
    let accepted = accept_deny(&mut service, request);
    let started = service
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("deny-start"),
        })
        .unwrap()
        .record;
    let error = service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: EvidenceReferenceId::parse("unavailable-evidence").unwrap(),
            context: ctx("deny-link"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPolicyDenied);
    assert!(error.retryable());
}

type DenyService = InMemoryActionService<
    TestClock,
    TestIds,
    TestAuthorization,
    TestPolicy,
    DenyActionEvidenceAuthority,
>;
fn open_deny(s: &mut DenyService) -> ActionRequestRecord {
    let draft = s
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("deny-request").unwrap(),
            title: text("Synthetic deny"),
            details: text("No caller evidence metadata"),
            intended_owner: Some(owner()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: ctx("deny-create"),
        })
        .unwrap()
        .record;
    s.submit_action_request(SubmitActionRequest {
        request_id: draft.id().clone(),
        expected_version: draft.version(),
        context: ctx("deny-submit"),
    })
    .unwrap()
    .record
}
fn accept_deny(s: &mut DenyService, request: ActionRequestRecord) -> AcceptedActionOutcome {
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("deny-prepare"),
        })
        .unwrap();
    s.approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
        approval: approval(&prepared, "deny-accept"),
        context: ctx("deny-accept"),
    })
    .unwrap()
}

#[test]
fn post_prepare_request_and_action_versions_are_preview_changed_not_domain_stale() {
    let (mut s, _, authority) = service();
    let request = open(&mut s, "request-post-prepare");
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-request-stale"),
        })
        .unwrap();
    let declined = s
        .decline_action_request(DeclineActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            rationale: text("Synthetic change after preview"),
            context: ctx("decline-after-preview"),
        })
        .unwrap()
        .record;
    let error = s
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval(&prepared, "accept-after-decline"),
            context: ctx("accept-after-decline"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.request(declined.id()).unwrap().state(),
        ActionRequestState::Declined
    );
    let accepted = accepted_action(&mut s, "action-post-prepare");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-post-prepare"),
        })
        .unwrap()
        .record;
    let cancel = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: started.id().clone(),
            expected_version: started.version(),
            reason: text("Preview before evidence link"),
            context: ctx("prepare-cancel-stale"),
        })
        .unwrap();
    let evidence = verified("evidence-post-prepare", DataClassification::Internal);
    authority.put(evidence.clone());
    let linked = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: evidence.id().clone(),
            context: ctx("link-after-preview"),
        })
        .unwrap()
        .record;
    let error = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: approval(&cancel, "cancel-after-link"),
            context: ctx("cancel-after-link"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.action(linked.id()).unwrap().state(),
        ActionState::InProgress
    );
}

#[test]
fn completion_evidence_never_valid_at_prepare_is_h3_denied_with_zero_effect_audit() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-h3");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-h3"),
        })
        .unwrap()
        .record;
    let missing = s
        .prepare_complete_action(PrepareCompleteAction {
            action_id: started.id().clone(),
            expected_version: started.version(),
            judgment: None,
            context: ctx("prepare-missing-evidence"),
        })
        .unwrap_err();
    assert_eq!(missing.code(), ErrorCode::SecurityPolicyDenied);
    let audit = s.audit_events().last().unwrap();
    assert_eq!(
        audit.policy_outcome(),
        pmc_domain::audit::AuditPolicyOutcome::Denied
    );
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::NotRequired);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    let invalid = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("evidence-invalid-at-prepare").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::ActionCompletion,
        EvidenceVerification::IntegrityMismatch,
    );
    authority.put(invalid.clone());
    let linked = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: invalid.id().clone(),
            context: ctx("link-invalid"),
        })
        .unwrap()
        .record;
    let denied = s
        .prepare_complete_action(PrepareCompleteAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            judgment: None,
            context: ctx("prepare-invalid"),
        })
        .unwrap_err();
    assert_eq!(denied.code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(
        s.action(linked.id()).unwrap().state(),
        ActionState::InProgress
    );
    let audit = s.audit_events().last().unwrap();
    assert_eq!(
        audit.policy_outcome(),
        pmc_domain::audit::AuditPolicyOutcome::Denied
    );
    assert!(audit.actual_effects().is_empty());
}

#[test]
fn cancel_and_reopen_bind_every_authoritative_evidence_classification_source() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-class-binding");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-class-binding"),
        })
        .unwrap()
        .record;
    let evidence = verified("evidence-class-binding", DataClassification::Internal);
    authority.put(evidence.clone());
    let linked = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: evidence.id().clone(),
            context: ctx("link-class-binding"),
        })
        .unwrap()
        .record;
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Restricted,
    ));
    let restricted_prepare = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            reason: text("Restricted preview"),
            context: ctx("prepare-cancel-restricted"),
        })
        .unwrap();
    assert_eq!(
        restricted_prepare.classification(),
        DataClassification::Restricted
    );
    assert!(s.discard_prepared_intent(restricted_prepare.id()));
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Unclassified,
    ));
    let denied = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            reason: text("Unclassified preview"),
            context: ctx("prepare-cancel-unclassified"),
        })
        .unwrap_err();
    assert_eq!(denied.code(), ErrorCode::SecurityPolicyDenied);
    authority.put(evidence.clone());
    let cancel = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            reason: text("Synthetic cancel"),
            context: ctx("prepare-cancel-binding"),
        })
        .unwrap();
    match cancel.operation() {
        WorkManagementOperation::CancelAction {
            evidence_classifications,
            ..
        } => {
            assert_eq!(evidence_classifications.len(), 1);
            assert_eq!(evidence_classifications[0].evidence_id(), evidence.id());
            assert_eq!(
                evidence_classifications[0].classification(),
                DataClassification::Internal
            );
        }
        _ => panic!("expected cancel operation"),
    }
    let cancel_approval = approval(&cancel, "cancel-binding");
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Restricted,
    ));
    let preview_error = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: cancel_approval.clone(),
            context: ctx("cancel-binding"),
        })
        .unwrap_err();
    assert_eq!(
        preview_error.code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Unclassified,
    ));
    let replayed_preview_error = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: cancel_approval.clone(),
            context: ctx("cancel-binding"),
        })
        .unwrap_err();
    assert_eq!(replayed_preview_error, preview_error);
    authority.put(evidence.clone());
    let retry_approval = approval(&cancel, "cancel-binding-retry");
    let cancelled = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: retry_approval,
            context: ctx("cancel-binding-retry"),
        })
        .unwrap()
        .record;
    s.persistence_snapshot()
        .expect("cancel preview failure followed by fresh execution must round-trip");
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Restricted,
    ));
    let restricted_reopen = s
        .prepare_reopen_action(PrepareReopenAction {
            action_id: cancelled.id().clone(),
            expected_version: cancelled.version(),
            mode: ActionReopenMode::RestartCancelled,
            reason: text("Restricted restart preview"),
            context: ctx("prepare-reopen-restricted"),
        })
        .unwrap();
    assert_eq!(
        restricted_reopen.classification(),
        DataClassification::Restricted
    );
    assert!(s.discard_prepared_intent(restricted_reopen.id()));
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Unclassified,
    ));
    let denied = s
        .prepare_reopen_action(PrepareReopenAction {
            action_id: cancelled.id().clone(),
            expected_version: cancelled.version(),
            mode: ActionReopenMode::RestartCancelled,
            reason: text("Unclassified restart preview"),
            context: ctx("prepare-reopen-unclassified"),
        })
        .unwrap_err();
    assert_eq!(denied.code(), ErrorCode::SecurityPolicyDenied);
    authority.put(evidence.clone());
    let reopen = s
        .prepare_reopen_action(PrepareReopenAction {
            action_id: cancelled.id().clone(),
            expected_version: cancelled.version(),
            mode: ActionReopenMode::RestartCancelled,
            reason: text("Synthetic restart"),
            context: ctx("prepare-reopen-binding"),
        })
        .unwrap();
    let reopen_approval = approval(&reopen, "reopen-binding");
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Restricted,
    ));
    let reopen_preview_error = s
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval.clone(),
            context: ctx("reopen-binding"),
        })
        .unwrap_err();
    assert_eq!(
        reopen_preview_error.code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    authority.put(verified(
        "evidence-class-binding",
        DataClassification::Unclassified,
    ));
    let replayed_reopen_error = s
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: reopen_approval.clone(),
            context: ctx("reopen-binding"),
        })
        .unwrap_err();
    assert_eq!(replayed_reopen_error, reopen_preview_error);
    authority.put(evidence);
    let retry_reopen_approval = approval(&reopen, "reopen-binding-retry");
    let reopened = s
        .approve_and_execute_reopen_action(ApproveAndExecuteReopenAction {
            approval: retry_reopen_approval,
            context: ctx("reopen-binding-retry"),
        })
        .unwrap()
        .record;
    assert_eq!(reopened.state(), ActionState::InProgress);
    s.persistence_snapshot()
        .expect("terminal preview failure followed by fresh execution must round-trip");
}

#[test]
fn cancel_preview_binds_each_source_even_when_resolved_classification_is_unchanged() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-source-binding");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-source-binding"),
        })
        .unwrap()
        .record;
    let restricted = verified("evidence-source-restricted", DataClassification::Restricted);
    let internal = verified("evidence-source-internal", DataClassification::Internal);
    authority.put(restricted.clone());
    authority.put(internal.clone());
    let linked_once = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: restricted.id().clone(),
            context: ctx("link-source-restricted"),
        })
        .unwrap()
        .record;
    let linked_twice = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: linked_once.id().clone(),
            expected_version: linked_once.version(),
            evidence_id: internal.id().clone(),
            context: ctx("link-source-internal"),
        })
        .unwrap()
        .record;
    let prepared = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked_twice.id().clone(),
            expected_version: linked_twice.version(),
            reason: text("Bind each source"),
            context: ctx("prepare-source-binding"),
        })
        .unwrap();
    assert_eq!(prepared.classification(), DataClassification::Restricted);
    authority.put(verified(
        "evidence-source-internal",
        DataClassification::Restricted,
    ));
    let error = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: approval(&prepared, "execute-source-binding"),
            context: ctx("execute-source-binding"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.action(linked_twice.id()).unwrap().state(),
        ActionState::InProgress
    );
}

#[test]
fn reverse_inserted_evidence_is_canonicalized_for_prepare_and_execute() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-reverse-evidence");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-reverse-evidence"),
        })
        .unwrap()
        .record;
    let later = verified("evidence-z-later", DataClassification::Internal);
    let earlier = verified("evidence-a-earlier", DataClassification::Confidential);
    authority.put(later.clone());
    authority.put(earlier.clone());
    let linked_later = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: later.id().clone(),
            context: ctx("link-z-first"),
        })
        .unwrap()
        .record;
    let linked_reverse = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: linked_later.id().clone(),
            expected_version: linked_later.version(),
            evidence_id: earlier.id().clone(),
            context: ctx("link-a-second"),
        })
        .unwrap()
        .record;
    let prepared = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked_reverse.id().clone(),
            expected_version: linked_reverse.version(),
            reason: text("Canonical evidence order"),
            context: ctx("prepare-reverse-evidence"),
        })
        .unwrap();
    match prepared.operation() {
        WorkManagementOperation::CancelAction {
            evidence_classifications,
            ..
        } => {
            assert_eq!(evidence_classifications.len(), 2);
            assert_eq!(evidence_classifications[0].evidence_id(), earlier.id());
            assert_eq!(evidence_classifications[1].evidence_id(), later.id());
        }
        _ => panic!("expected cancel operation"),
    }
    let cancelled = s
        .approve_and_execute_cancel_action(ApproveAndExecuteCancelAction {
            approval: approval(&prepared, "execute-reverse-evidence"),
            context: ctx("execute-reverse-evidence"),
        })
        .unwrap()
        .record;
    assert_eq!(cancelled.state(), ActionState::Cancelled);
}

#[test]
fn unclassified_evidence_links_and_blocks_h2_until_authority_classifies_it() {
    let (mut s, _, authority) = service();
    let accepted = accepted_action(&mut s, "request-unclassified-link");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-unclassified-link"),
        })
        .unwrap()
        .record;
    let evidence = verified(
        "evidence-unclassified-link",
        DataClassification::Unclassified,
    );
    authority.put(evidence.clone());
    let linked = s
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id: evidence.id().clone(),
            context: ctx("link-unclassified"),
        })
        .unwrap()
        .record;
    assert_eq!(linked.classification(), DataClassification::Unclassified);
    let audit_count = s.audit_events().len();
    let denied = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            reason: text("Must wait for classification"),
            context: ctx("prepare-unclassified-denied"),
        })
        .unwrap_err();
    assert_eq!(denied.code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(s.audit_events().len(), audit_count + 1);
    authority.put(verified(
        "evidence-unclassified-link",
        DataClassification::Internal,
    ));
    let prepared = s
        .prepare_cancel_action(PrepareCancelAction {
            action_id: linked.id().clone(),
            expected_version: linked.version(),
            reason: text("Authority classified evidence"),
            context: ctx("prepare-after-classification"),
        })
        .unwrap();
    assert_eq!(prepared.classification(), DataClassification::Internal);
}

#[test]
fn mandatory_h3_denial_audit_id_failure_is_platform_internal_without_business_effect() {
    let (mut s, control, _) = service();
    let accepted = accepted_action(&mut s, "request-audit-failure");
    let started = s
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: ctx("start-audit-failure"),
        })
        .unwrap()
        .record;
    let audit_count = s.audit_events().len();
    control.set(-3);
    let error = s
        .prepare_complete_action(PrepareCompleteAction {
            action_id: started.id().clone(),
            expected_version: started.version(),
            judgment: None,
            context: ctx("prepare-audit-failure"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::PlatformInternal);
    assert_eq!(s.audit_events().len(), audit_count);
    let unchanged = s.action(started.id()).unwrap();
    assert_eq!(unchanged.state(), ActionState::InProgress);
    assert_eq!(unchanged.version(), started.version());
}

fn reject(
    prepared: &pmc_domain::work_management::WorkManagementPreparedIntent,
    id: &str,
) -> RejectActionPreparedIntent {
    RejectActionPreparedIntent {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        context: ctx(id),
    }
}

/// v45: a rejection is a successful command with no effect. It consumes the
/// preview, records one zero-effect audit, replays exactly, refuses a second
/// rejection and a later approval, and survives the persistence boundary.
#[test]
fn rejecting_a_pending_accept_preview_consumes_it_without_effects_and_replays() {
    let (mut s, now, _) = service();
    let request = open(&mut s, "request-reject");
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-reject"),
        })
        .unwrap();
    now.set(150);
    let outcome = s
        .reject_action_prepared_intent(reject(&prepared, "reject-1"))
        .unwrap();
    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(150));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.code().as_str(), "action.prepared_rejected");
    assert_eq!(audit.correlation_id().as_str(), "corr-reject-1");
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());
    assert_eq!(s.request(request.id()), Some(&request));
    assert_eq!(s.audit_events().last(), Some(audit));

    assert_eq!(
        s.reject_action_prepared_intent(reject(&prepared, "reject-1"))
            .unwrap(),
        outcome
    );
    let again = s
        .reject_action_prepared_intent(reject(&prepared, "reject-2"))
        .unwrap_err();
    assert_eq!(again.code(), ErrorCode::DomainConflict);
    let executed = s
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: approval(&prepared, "accept-after-reject"),
            context: ctx("accept-after-reject"),
        })
        .unwrap_err();
    assert_eq!(executed.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.request(request.id()).map(ActionRequestRecord::state),
        Some(ActionRequestState::Open)
    );
    let unknown = s
        .reject_action_prepared_intent(RejectActionPreparedIntent {
            prepared_id: PreparedIntentId::parse("prepared-missing").unwrap(),
            actor: AuditActor::HeadOfProducts,
            context: ctx("reject-missing"),
        })
        .unwrap_err();
    assert_eq!(unknown.code(), ErrorCode::DomainNotFound);

    let snapshot = s.persistence_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    assert_eq!(snapshot.discarded_prepared(), &[prepared.clone()]);
    let rejection = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id().as_str() == "reject-1")
        .unwrap();
    assert_eq!(
        rejection.result(),
        &ActionPersistenceResult::Rejected(outcome.clone())
    );
    assert_eq!(rejection.audit_event_ids(), &[audit.id().clone()]);
    let rehydrated = InMemoryActionService::rehydrate(
        TestClock(now.clone()),
        TestIds {
            counter: 50,
            control: now.clone(),
        },
        TestAuthorization(now.clone()),
        TestPolicy(now.clone()),
        TestEvidenceAuthority::default(),
        snapshot.clone(),
    )
    .persistence_snapshot()
    .unwrap();
    assert_eq!(rehydrated.replay(), snapshot.replay());
    assert_eq!(rehydrated.audits(), snapshot.audits());
    assert_eq!(
        rehydrated.discarded_prepared(),
        snapshot.discarded_prepared()
    );

    // The validator refuses a rejection whose recorded expiry flag disagrees
    // with the audit instant, and one whose intent was not discarded.
    let index = snapshot
        .replay()
        .iter()
        .position(|capsule| capsule.idempotency_id().as_str() == "reject-1")
        .unwrap();
    let mut flipped = snapshot.replay().to_vec();
    flipped[index] = ActionReplayCapsule::new(
        rejection.idempotency_id().clone(),
        rejection.original_correlation_id().clone(),
        rejection.operation_ordinal(),
        rejection.command().clone(),
        ActionPersistenceResult::Rejected(RejectedPreparedIntentOutcome::new(
            outcome.prepared_intent_id().clone(),
            outcome.rejected_at(),
            !outcome.expired_at_rejection(),
            outcome.audit_event().clone(),
        )),
        rejection.audit_event_ids().to_vec(),
    );
    assert!(ActionPersistenceSnapshot::try_new(
        snapshot.requests().to_vec(),
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        snapshot.discarded_prepared().to_vec(),
        flipped,
        snapshot.audits().to_vec(),
    )
    .is_err());
    assert!(ActionPersistenceSnapshot::try_new(
        snapshot.requests().to_vec(),
        snapshot.actions().to_vec(),
        snapshot.prepared().to_vec(),
        Vec::new(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .is_err());
}

#[test]
fn rejecting_after_execute_conflicts_and_an_expired_preview_is_still_rejectable() {
    let (mut s, now, _) = service();
    let request = open(&mut s, "request-executed");
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-executed"),
        })
        .unwrap();
    s.approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
        approval: approval(&prepared, "accept-executed"),
        context: ctx("accept-executed"),
    })
    .unwrap();
    let late = s
        .reject_action_prepared_intent(reject(&prepared, "reject-late"))
        .unwrap_err();
    assert_eq!(late.code(), ErrorCode::DomainConflict);

    let request = open(&mut s, "request-expired");
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx("prepare-expired"),
        })
        .unwrap();
    now.set(prepared.preview().expires_at().unix_millis() + 1);
    let outcome = s
        .reject_action_prepared_intent(reject(&prepared, "reject-expired"))
        .unwrap();
    assert!(outcome.expired_at_rejection());
    assert!(s.persistence_snapshot().unwrap().prepared().is_empty());
}
