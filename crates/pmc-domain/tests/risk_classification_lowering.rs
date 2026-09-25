//! H2a "Lower Data Classification" for Risk -- the
//! third of the four remaining aggregate families (Action, Decision done;
//! Risk here, then Issue). `InMemoryRiskService` was the first H2a
//! service studied, and its own conventions (a `RiskFailure`
//! cause enum, a `Signature`/`Stored` idempotency pair, `map_failure`,
//! `record_prepare_failure_audit`/`record_execution_failure_audit`,
//! `is_durable_post_start_terminal`) were mirrored closely rather than
//! reusing Action's or Decision's shapes -- each H2a-native service has its
//! own established internal conventions, not one shared one.
//!
//! Notably safer by design than Action's `record_h2_failure` or Decision's
//! `record_h2_failure`: `prepared_risk_target`'s target-resolution match
//! already had a `_ => None` fallback (not `unreachable!()`), so this
//! operation could not have introduced a panic risk there even before an
//! explicit arm was added for audit-target correctness.

use std::cell::Cell;
use std::rc::Rc;

use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::*;
use pmc_domain::risks::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalConfirmation, WorkManagementApproval, WorkManagementOperation,
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
impl RiskServiceIdSource for TestIds {
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
impl pmc_domain::work_management::ApprovalAuthorizationPort for AllowHeadOfProducts {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
impl RiskExecutionPolicyPort for AllowHeadOfProducts {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

type Service = InMemoryRiskService<
    TestClock,
    TestIds,
    AllowHeadOfProducts,
    AllowHeadOfProducts,
    AllowRiskEvidence,
    RecordedRiskClassification,
>;

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn rationale(value: &str) -> WorkManagementRationale {
    WorkManagementRationale::parse(value).unwrap()
}
fn ctx(id: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn service() -> Service {
    InMemoryRiskService::new(
        TestClock(Rc::new(Cell::new(100))),
        TestIds(0),
        AllowHeadOfProducts,
        AllowHeadOfProducts,
        AllowRiskEvidence,
        RecordedRiskClassification,
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
fn restricted_risk(s: &mut Service, id: &str) -> RiskRecord {
    s.create_risk(CreateRisk {
        id: RiskId::parse(id).unwrap(),
        title: text("Synthetic risk"),
        details: text("Synthetic details"),
        classification: DataClassification::Restricted,
        context: ctx(&format!("create-{id}")),
    })
    .unwrap()
    .record
}

#[test]
fn prepare_then_approve_atomically_lowers_classification_and_bumps_version() {
    let mut s = service();
    let risk = restricted_risk(&mut s, "risk-alpha");

    let prepared = s
        .prepare_lower_risk_classification(PrepareLowerRiskClassification {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-prepare"),
        })
        .unwrap_or_else(|error| panic!("prepare: {error}"));

    let outcome = s
        .approve_and_execute_lower_risk_classification(ApproveAndExecuteLowerRiskClassification {
            approval: approval(&prepared, "lower-approve"),
            context: ctx("lower-approve"),
        })
        .unwrap_or_else(|error| panic!("approve: {error}"));

    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.version().get(), risk.version().get() + 1);
}

#[test]
fn prepare_rejects_a_stale_version_and_a_non_lowering_proposal() {
    let mut s = service();
    let risk = restricted_risk(&mut s, "risk-beta");

    let stale = s
        .prepare_lower_risk_classification(PrepareLowerRiskClassification {
            risk_id: risk.id().clone(),
            expected_version: AggregateVersion::initial().next().unwrap(),
            proposed_classification: DataClassification::Internal,
            rationale: rationale("Synthetic rationale."),
            context: ctx("lower-stale"),
        })
        .unwrap_err();
    assert_eq!(stale.code(), ErrorCode::DomainConflict);

    let unchanged = s
        .prepare_lower_risk_classification(PrepareLowerRiskClassification {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
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
    let risk = restricted_risk(&mut s, "risk-gamma");

    let intent = PrepareLowerRiskClassification {
        risk_id: risk.id().clone(),
        expected_version: risk.version(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale("Synthetic rationale."),
        context: ctx("lower-idem"),
    };
    let first = s
        .prepare_lower_risk_classification(intent.clone())
        .unwrap_or_else(|error| panic!("first prepare: {error}"));
    let replay = s
        .prepare_lower_risk_classification(intent)
        .unwrap_or_else(|error| panic!("replay prepare: {error}"));
    assert_eq!(first.id(), replay.id());

    let command = ApproveAndExecuteLowerRiskClassification {
        approval: approval(&first, "lower-exec-replay"),
        context: ctx("lower-exec-replay"),
    };
    let executed = s
        .approve_and_execute_lower_risk_classification(command.clone())
        .unwrap_or_else(|error| panic!("first approve: {error}"));
    let replayed = s
        .approve_and_execute_lower_risk_classification(command)
        .unwrap_or_else(|error| panic!("replay approve: {error}"));
    assert_eq!(executed, replayed);
}

#[test]
fn approve_rejects_a_mismatched_acknowledged_digest_without_effect() {
    let mut s = service();
    let risk = restricted_risk(&mut s, "risk-delta");

    let prepared = s
        .prepare_lower_risk_classification(PrepareLowerRiskClassification {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
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
        .approve_and_execute_lower_risk_classification(ApproveAndExecuteLowerRiskClassification {
            approval: wrong_digest_approval,
            context: ctx("lower-digest"),
        })
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(
        s.risk(risk.id()).unwrap().classification(),
        DataClassification::Restricted
    );
}
