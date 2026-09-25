//! H2a "Lower Data Classification" for Action -- the
//! first of the four remaining aggregate families (Action, Decision, Risk,
//! Issue). Unlike the Portfolio family, `InMemoryActionService` already had
//! full H2a Prepare/ApproveAndExecute machinery baked into its constructor
//! from day one, so this needed no method-level-generic workaround; it
//! reuses the service's existing internal seams (`pending`, `exact_action`,
//! `validate`, `audit`, `finish`) directly, built as an independent method
//! pair rather than folding into the shared `execute_h2` dispatcher
//! Complete/Cancel/Reopen use.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pmc_domain::actions::*;
use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceReferenceMetadata,
    WorkManagementApproval, WorkManagementOperation, WorkManagementPreparedIntent,
    WorkManagementRationale,
};

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

struct TestIds(u64);
impl ActionServiceIdSource for TestIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ActionId::parse(format!("action-{}", self.0))
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

#[derive(Clone, Copy)]
struct AlwaysAllowedPolicy;
impl ActionExecutionPolicyPort for AlwaysAllowedPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}

#[derive(Clone, Default)]
struct EmptyEvidenceAuthority(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl ActionEvidenceAuthorityPort for EmptyEvidenceAuthority {
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

type Service = InMemoryActionService<
    TestClock,
    TestIds,
    AllowHeadOfProducts,
    AlwaysAllowedPolicy,
    EmptyEvidenceAuthority,
>;

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn rationale(value: &str) -> WorkManagementRationale {
    WorkManagementRationale::parse(value).unwrap()
}
fn ctx(id: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn owner() -> StakeholderId {
    StakeholderId::parse("owner-product-lead").unwrap()
}
fn service() -> Service {
    InMemoryActionService::new(
        TestClock(Rc::new(Cell::new(100))),
        TestIds(0),
        AllowHeadOfProducts,
        AlwaysAllowedPolicy,
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

/// A restricted Action, created through the real Accept flow (not a
/// synthetic shortcut) so it carries exactly the shape the governed
/// lowering path expects.
fn restricted_action(s: &mut Service, id: &str) -> ActionRecord {
    let request = s
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse(format!("request-{id}")).unwrap(),
            title: text("Prepare synthetic launch"),
            details: text("Synthetic commitment details"),
            intended_owner: Some(owner()),
            response_due_at: Some(UtcTimestamp::from_unix_millis(500)),
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Restricted,
            context: ctx(&format!("create-{id}")),
        })
        .unwrap()
        .record;
    let request = s
        .submit_action_request(SubmitActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx(&format!("submit-{id}")),
        })
        .unwrap()
        .record;
    let prepared = s
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: ctx(&format!("accept-prepare-{id}")),
        })
        .unwrap();
    s.approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
        approval: approval(&prepared, &format!("accept-{id}")),
        context: ctx(&format!("accept-{id}")),
    })
    .unwrap()
    .action
}

#[test]
fn prepare_then_approve_atomically_lowers_classification_and_bumps_version() {
    let mut s = service();
    let action = restricted_action(&mut s, "alpha");

    let prepared = s
        .prepare_lower_action_classification(PrepareLowerActionClassification {
            action_id: action.id().clone(),
            expected_version: action.version(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-prepare"),
        })
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval: approval(&prepared, "lower-approve"),
                context: ctx("lower-approve"),
            },
        )
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.version(), action.version().next().unwrap());
    assert_eq!(
        s.action(action.id()).unwrap().classification(),
        DataClassification::Internal
    );
}

#[test]
fn prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let action = restricted_action(&mut s, "beta");

    let stale = s
        .prepare_lower_action_classification(PrepareLowerActionClassification {
            action_id: action.id().clone(),
            expected_version: AggregateVersion::initial().next().unwrap(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-stale"),
        })
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_action_classification(PrepareLowerActionClassification {
            action_id: action.id().clone(),
            expected_version: action.version(),
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
    let action = restricted_action(&mut s, "gamma");

    let intent = PrepareLowerActionClassification {
        action_id: action.id().clone(),
        expected_version: action.version(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale("Synthetic rationale."),
        context: ctx("lower-idem"),
    };
    let first = s
        .prepare_lower_action_classification(intent.clone())
        .unwrap_or_else(|error| panic!("first prepare: {error}"));
    let replay = s
        .prepare_lower_action_classification(intent)
        .unwrap_or_else(|error| panic!("replay prepare: {error}"));
    assert_eq!(first.id(), replay.id());
    assert_eq!(first.payload_digest(), replay.payload_digest());

    let command = ApproveAndExecuteLowerActionClassification {
        approval: approval(&first, "lower-exec-replay"),
        context: ctx("lower-exec-replay"),
    };
    let executed = s
        .approve_and_execute_lower_action_classification(command.clone())
        .unwrap_or_else(|error| panic!("first approve: {error}"));
    let replayed = s
        .approve_and_execute_lower_action_classification(command)
        .unwrap_or_else(|error| panic!("replay approve: {error}"));
    assert_eq!(executed, replayed);
}

#[test]
fn approve_rejects_a_mismatched_acknowledged_digest_without_effect() {
    let mut s = service();
    let action = restricted_action(&mut s, "delta");

    let prepared = s
        .prepare_lower_action_classification(PrepareLowerActionClassification {
            action_id: action.id().clone(),
            expected_version: action.version(),
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
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval: wrong_digest_approval,
                context: ctx("lower-digest"),
            },
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.action(action.id()).unwrap().classification(),
        DataClassification::Restricted
    );
}
