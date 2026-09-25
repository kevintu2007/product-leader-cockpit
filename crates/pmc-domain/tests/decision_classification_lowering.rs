//! H2a "Lower Data Classification" for Decision, one of the four
//! work-management aggregate families (Action, Decision, Risk, Issue) that
//! support it. Like Action, `InMemoryDecisionService`
//! already had H2a machinery baked into its constructor. Unlike Action (and
//! unlike Portfolio), Decision's own internal `prepare`/`validate` helpers
//! hardcode a *required* `SupportWitness` (every existing Decision H2a
//! operation needs Evidence-or-Judgment) -- this operation needs none, so
//! it inlines its own copies of that plumbing with `None` support rather
//! than fabricating a witness just to satisfy the wrapper.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::decisions::*;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceReferenceMetadata, HumanJudgment,
    HumanJudgmentDisposition, WorkManagementApproval, WorkManagementOperation,
    WorkManagementPreparedIntent, WorkManagementRationale,
};

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

struct TestIds(u64);
impl DecisionServiceIdSource for TestIds {
    fn next_decision_id(&mut self) -> Result<DecisionId, pmc_domain::DomainValueError> {
        self.0 += 1;
        DecisionId::parse(format!("decision-{}", self.0))
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct AllowHeadOfProducts;
impl ApprovalAuthorizationPort for AllowHeadOfProducts {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
impl DecisionExecutionPolicyPort for AllowHeadOfProducts {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        DecisionExecutionPolicy::Allowed
    }
}

#[derive(Clone, Default)]
struct EmptyEvidenceAuthority(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl DecisionEvidenceAuthorityPort for EmptyEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        self.0
            .borrow()
            .get(id)
            .cloned()
            .ok_or(DecisionEvidenceAuthorityError::NotFound)
    }
}

type Service = InMemoryDecisionService<
    TestClock,
    TestIds,
    AllowHeadOfProducts,
    AllowHeadOfProducts,
    EmptyEvidenceAuthority,
>;

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn rationale(value: &str) -> WorkManagementRationale {
    WorkManagementRationale::parse(value).unwrap()
}
fn ctx(id: &str) -> DecisionOperationContext {
    DecisionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn owner() -> StakeholderId {
    StakeholderId::parse("owner-product").unwrap()
}
fn judgment(class: DataClassification) -> HumanJudgment {
    HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Synthetic owner judgment",
        class,
    )
    .unwrap()
}
fn service() -> Service {
    InMemoryDecisionService::new(
        TestClock(Rc::new(Cell::new(100))),
        TestIds(0),
        AllowHeadOfProducts,
        AllowHeadOfProducts,
        EmptyEvidenceAuthority::default(),
    )
}
fn approval(prepared: &WorkManagementPreparedIntent, id: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(id).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

/// A restricted Decision, created through the real Resolve flow (not a
/// synthetic shortcut). This service alone has no linked Action service,
/// so it must resolve with zero resulting Action Requests.
fn restricted_decision(s: &mut Service, id: &str) -> DecisionRecord {
    let request = s
        .create_decision_request_draft(CreateDecisionRequestDraft {
            id: DecisionRequestId::parse(format!("request-{id}")).unwrap(),
            subject: text("Choose synthetic direction"),
            details: text("Synthetic decision details"),
            intended_owner: Some(owner()),
            classification: DataClassification::Restricted,
            context: ctx(&format!("create-{id}")),
        })
        .unwrap()
        .record;
    let request = s
        .submit_decision_request(SubmitDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx(&format!("submit-{id}")),
        })
        .unwrap()
        .record;
    let prepared = s
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            statement: text("Proceed with synthetic option A"),
            rationale: text("Best synthetic tradeoff"),
            impact: text("Improves synthetic delivery"),
            evidence_ids: vec![],
            judgments: vec![judgment(DataClassification::Restricted)],
            resulting_action_requests: vec![],
            context: ctx(&format!("resolve-prepare-{id}")),
        })
        .unwrap();
    s.approve_and_execute_resolve_decision_request(
        ApproveAndExecuteResolveDecisionRequest {
            approval: approval(&prepared, &format!("resolve-{id}")),
            context: ctx(&format!("resolve-{id}")),
        },
        &mut synthetic_action_service(),
    )
    .unwrap()
    .decision
}

/// Only used to satisfy `approve_and_execute_resolve_decision_request`'s
/// generic `actions` parameter for a resolution that creates zero
/// resulting Action Requests -- never actually touched.
fn synthetic_action_service() -> pmc_domain::actions::InMemoryActionService<
    TestClock,
    SyntheticActionIds,
    AllowHeadOfProducts,
    SyntheticActionPolicy,
    EmptyActionEvidence,
> {
    pmc_domain::actions::InMemoryActionService::new(
        TestClock(Rc::new(Cell::new(100))),
        SyntheticActionIds(0),
        AllowHeadOfProducts,
        SyntheticActionPolicy,
        EmptyActionEvidence::default(),
    )
}
struct SyntheticActionIds(u64);
impl pmc_domain::actions::ActionServiceIdSource for SyntheticActionIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ActionId::parse(format!("synthetic-action-{}", self.0))
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("synthetic-prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("synthetic-receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("synthetic-audit-{}", self.0))
    }
}
#[derive(Clone, Copy)]
struct SyntheticActionPolicy;
impl pmc_domain::actions::ActionExecutionPolicyPort for SyntheticActionPolicy {
    fn current_policy(
        &self,
        _: &WorkManagementOperation,
    ) -> pmc_domain::actions::ActionExecutionPolicy {
        pmc_domain::actions::ActionExecutionPolicy::Allowed
    }
}
#[derive(Clone, Default)]
struct EmptyActionEvidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl pmc_domain::actions::ActionEvidenceAuthorityPort for EmptyActionEvidence {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, pmc_domain::actions::ActionEvidenceAuthorityError> {
        self.0
            .borrow()
            .get(id)
            .cloned()
            .ok_or(pmc_domain::actions::ActionEvidenceAuthorityError::NotFound)
    }
}

#[test]
fn prepare_then_approve_atomically_lowers_classification_and_bumps_version() {
    let mut s = service();
    let decision = restricted_decision(&mut s, "alpha");

    let prepared = s
        .prepare_lower_decision_classification(PrepareLowerDecisionClassification {
            decision_id: decision.id().clone(),
            expected_version: decision.version(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-prepare"),
        })
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_decision_classification(
            ApproveAndExecuteLowerDecisionClassification {
                approval: approval(&prepared, "lower-approve"),
                context: ctx("lower-approve"),
            },
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.version(), decision.version().next().unwrap());
}

#[test]
fn prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let decision = restricted_decision(&mut s, "beta");

    let stale = s
        .prepare_lower_decision_classification(PrepareLowerDecisionClassification {
            decision_id: decision.id().clone(),
            expected_version: AggregateVersion::initial().next().unwrap(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-stale"),
        })
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_decision_classification(PrepareLowerDecisionClassification {
            decision_id: decision.id().clone(),
            expected_version: decision.version(),
            proposed_classification: DataClassification::Restricted,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-unchanged"),
        })
        .unwrap_err();
    assert_eq!(unchanged.code(), ErrorCode::DomainConflict);
}

#[test]
fn prepare_is_idempotent_and_execute_replays_the_original_outcome() {
    let mut s = service();
    let decision = restricted_decision(&mut s, "gamma");

    let intent = PrepareLowerDecisionClassification {
        decision_id: decision.id().clone(),
        expected_version: decision.version(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale("Synthetic rationale."),
        context: ctx("lower-idem"),
    };
    let first = s
        .prepare_lower_decision_classification(intent.clone())
        .unwrap_or_else(|error| panic!("first prepare: {error}"));
    let replay = s
        .prepare_lower_decision_classification(intent)
        .unwrap_or_else(|error| panic!("replay prepare: {error}"));
    assert_eq!(first.id(), replay.id());

    let command = ApproveAndExecuteLowerDecisionClassification {
        approval: approval(&first, "lower-exec-replay"),
        context: ctx("lower-exec-replay"),
    };
    let executed = s
        .approve_and_execute_lower_decision_classification(command.clone())
        .unwrap_or_else(|error| panic!("first approve: {error}"));
    let replayed = s
        .approve_and_execute_lower_decision_classification(command)
        .unwrap_or_else(|error| panic!("replay approve: {error}"));
    assert_eq!(executed, replayed);
}

/// Regression test for a real panic risk found and fixed in this same
/// slice: `record_h2_failure`'s target-resolution match used to end in
/// `_ => unreachable!(...)`, correct only because a DecisionService's own
/// `state.prepared` could previously hold just two operation kinds. This
/// drives a genuine execute-time failure (a mismatched acknowledged
/// digest) through that exact path for `LowerDecisionClassification` and
/// asserts a clean `Err`, not a panic -- proving the fix, not just
/// asserting it in a comment.
#[test]
fn approve_failure_is_recorded_without_panicking_on_a_mismatched_digest() {
    let mut s = service();
    let decision = restricted_decision(&mut s, "epsilon");

    let prepared = s
        .prepare_lower_decision_classification(PrepareLowerDecisionClassification {
            decision_id: decision.id().clone(),
            expected_version: decision.version(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-digest-prepare"),
        })
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let wrong_digest_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted("b".repeat(64))
            .unwrap(),
        IdempotencyId::parse("lower-digest").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    let error = s
        .approve_and_execute_lower_decision_classification(
            ApproveAndExecuteLowerDecisionClassification {
                approval: wrong_digest_approval,
                context: ctx("lower-digest"),
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.decision(decision.id()).unwrap().classification(),
        DataClassification::Restricted
    );
}
