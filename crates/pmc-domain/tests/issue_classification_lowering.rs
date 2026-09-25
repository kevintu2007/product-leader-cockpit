//! H2a "Lower Data Classification" for Issue -- the
//! last of the four remaining aggregate families (Action, Decision, Risk
//! done; Issue here). `InMemoryIssueService` unifies Resolve/Close/Reopen
//! through ONE shared private `prepare`/`execute` pair, both of which
//! hardcode a required Evidence role via `required_role(&op)` -- itself
//! ending in `unreachable!()` for any other operation -- and a
//! state-transition-specific `next_state`/`required` prior-state check.
//! Reusing either would have made a failed lowering panic instead of
//! returning an error, exactly the same class of risk found in Action and
//! Decision. Built as fully independent prepare/execute methods instead,
//! reusing only the generic seams (`replay`, `make_audit`, `failure_audit`,
//! `map_failure`, `self.authority`) that don't assume a specific operation
//! shape.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::*;
use pmc_domain::issues::*;
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
impl IssueServiceIdSource for TestIds {
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
impl IssueExecutionPolicyPort for AllowHeadOfProducts {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

#[derive(Clone, Default)]
struct EmptyEvidenceAuthority(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl IssueEvidenceAuthorityPort for EmptyEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        self.0
            .borrow()
            .get(id)
            .cloned()
            .ok_or(IssueEvidenceAuthorityError::NotFound)
    }
}

type Service = InMemoryIssueService<
    TestClock,
    TestIds,
    AllowHeadOfProducts,
    AllowHeadOfProducts,
    EmptyEvidenceAuthority,
    RecordedIssueClassification,
>;

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn rationale(value: &str) -> WorkManagementRationale {
    WorkManagementRationale::parse(value).unwrap()
}
fn ctx(id: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn service() -> Service {
    InMemoryIssueService::new(
        TestClock(Rc::new(Cell::new(100))),
        TestIds(0),
        AllowHeadOfProducts,
        AllowHeadOfProducts,
        EmptyEvidenceAuthority::default(),
        RecordedIssueClassification,
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
fn restricted_issue(s: &mut Service, id: &str) -> IssueRecord {
    s.create_issue(CreateIssue {
        id: IssueId::parse(id).unwrap(),
        title: text("Synthetic issue"),
        details: text("Synthetic observed condition"),
        classification: DataClassification::Restricted,
        recurrence_of: None,
        context: ctx(&format!("create-{id}")),
    })
    .unwrap()
    .record
}

#[test]
fn prepare_then_approve_atomically_lowers_classification_and_bumps_version() {
    let mut s = service();
    let issue = restricted_issue(&mut s, "issue-alpha");

    let prepared = s
        .prepare_lower_issue_classification(PrepareLowerIssueClassification {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-prepare"),
        })
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_issue_classification(ApproveAndExecuteLowerIssueClassification {
            approval: approval(&prepared, "lower-approve"),
            context: ctx("lower-approve"),
        })
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.version().get(), issue.version().get() + 1);
}

#[test]
fn prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let issue = restricted_issue(&mut s, "issue-beta");

    let stale = s
        .prepare_lower_issue_classification(PrepareLowerIssueClassification {
            issue_id: issue.id().clone(),
            expected_version: AggregateVersion::initial().next().unwrap(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-stale"),
        })
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_issue_classification(PrepareLowerIssueClassification {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
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
    let issue = restricted_issue(&mut s, "issue-gamma");

    let intent = PrepareLowerIssueClassification {
        issue_id: issue.id().clone(),
        expected_version: issue.version(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale("Synthetic rationale."),
        context: ctx("lower-idem"),
    };
    let first = s
        .prepare_lower_issue_classification(intent.clone())
        .unwrap_or_else(|error| panic!("first prepare: {error}"));
    let replay = s
        .prepare_lower_issue_classification(intent)
        .unwrap_or_else(|error| panic!("replay prepare: {error}"));
    assert_eq!(first.id(), replay.id());

    let command = ApproveAndExecuteLowerIssueClassification {
        approval: approval(&first, "lower-exec-replay"),
        context: ctx("lower-exec-replay"),
    };
    let executed = s
        .approve_and_execute_lower_issue_classification(command.clone())
        .unwrap_or_else(|error| panic!("first approve: {error}"));
    let replayed = s
        .approve_and_execute_lower_issue_classification(command)
        .unwrap_or_else(|error| panic!("replay approve: {error}"));
    assert_eq!(executed, replayed);
}

/// Regression test proving the panic risk this slice deliberately avoided
/// (see the module doc comment) does not exist: an execute-time failure
/// (a mismatched acknowledged digest) must return a clean `Err`, never
/// panic, and must leave the Issue untouched.
#[test]
fn approve_failure_is_recorded_without_panicking_on_a_mismatched_digest() {
    let mut s = service();
    let issue = restricted_issue(&mut s, "issue-delta");

    let prepared = s
        .prepare_lower_issue_classification(PrepareLowerIssueClassification {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
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
        .approve_and_execute_lower_issue_classification(ApproveAndExecuteLowerIssueClassification {
            approval: wrong_digest_approval,
            context: ctx("lower-digest"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.issue(issue.id()).unwrap().classification(),
        DataClassification::Restricted
    );
}
