//! v46: the recorded refusal of a pending Risk occurrence/close preview.
//!
//! Live behaviour, restart behaviour through the typed decode boundary, and
//! the corruption that boundary must refuse -- a rejected preview handed back
//! as outstanding, or a rejection whose audit does not say what the service
//! would have said.

use std::cell::Cell;
use std::rc::Rc;

use pmc_domain::audit::{
    AuditActor, AuditApprovalOutcome, AuditExecutionOutcome, AuditModule, AuditPolicyOutcome,
    AuditTarget,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, IssueId, PreparedIntentId,
    RiskId,
};
use pmc_domain::risks::{
    AllowRiskEvidence, ApproveAndExecuteRecordRiskOccurrence, CreateRisk, InMemoryRiskService,
    PrepareRecordRiskOccurrence, RecordedRiskClassification, RejectRiskPreparedIntent,
    RiskExecutionPolicy, RiskExecutionPolicyPort, RiskH2aPersistenceDecodeInput,
    RiskH2aRejectionReplay, RiskOperationContext, RiskPersistenceSnapshot, RiskServiceIdSource,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ApprovalAuthorizationPort, ApprovalConfirmation, RejectedPreparedIntentOutcome,
    WorkManagementApproval, WorkManagementOperation, RISK_PREPARED_REJECTED_AUDIT_CODE,
};

/// A clock the test can move, so an expired-but-unconsumed preview is a
/// reachable case rather than a hypothetical one.
#[derive(Clone)]
struct CellClock(Rc<Cell<i64>>);

impl Clock for CellClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

#[derive(Clone)]
struct TestIds(Rc<Cell<u64>>);

impl RiskServiceIdSource for TestIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        let id = self.0.get() + 1;
        self.0.set(id);
        PreparedIntentId::parse(format!("prepared-risk-reject-{id}"))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        let id = self.0.get() + 1;
        self.0.set(id);
        ApprovalReceiptId::parse(format!("receipt-risk-reject-{id}"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        let id = self.0.get() + 1;
        self.0.set(id);
        AuditEventId::parse(format!("audit-risk-reject-{id}"))
    }
}

#[derive(Clone, Copy)]
struct AllowPolicy;

impl RiskExecutionPolicyPort for AllowPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

#[derive(Clone, Copy)]
struct AllowApproval;

impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize(&self, _: AuditActor) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct DenyApproval;

impl ApprovalAuthorizationPort for DenyApproval {
    fn authorize(&self, _: AuditActor) -> bool {
        false
    }
}

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}

fn context(id: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("{id}-correlation")).unwrap(),
    }
}

type Service<Z> = InMemoryRiskService<
    CellClock,
    TestIds,
    Z,
    AllowPolicy,
    AllowRiskEvidence,
    RecordedRiskClassification,
>;

fn new_service<Z: ApprovalAuthorizationPort>(
    clock: Rc<Cell<i64>>,
    ids: Rc<Cell<u64>>,
    authorization: Z,
) -> Service<Z> {
    InMemoryRiskService::new(
        CellClock(clock),
        TestIds(ids),
        authorization,
        AllowPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
    )
}

/// Creates one Open Risk and prepares its occurrence; returns the prepared
/// intent so a test can act on it.
fn prepared_occurrence<Z: ApprovalAuthorizationPort>(
    service: &mut Service<Z>,
) -> pmc_domain::work_management::WorkManagementPreparedIntent {
    let created = service
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-reject").unwrap(),
            title: text("Synthetic rejection subject"),
            details: text("Synthetic rejection details"),
            classification: DataClassification::Internal,
            context: context("risk-create"),
        })
        .unwrap();
    service
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: created.record.id().clone(),
            expected_version: created.record.version(),
            issue_id: IssueId::parse("issue-from-risk-reject").unwrap(),
            context: context("risk-prepare"),
        })
        .unwrap()
}

fn reject(prepared_id: &PreparedIntentId, id: &str) -> RejectRiskPreparedIntent {
    RejectRiskPreparedIntent {
        prepared_id: prepared_id.clone(),
        actor: AuditActor::HeadOfProducts,
        context: context(id),
    }
}

fn assert_zero_effect_audit(outcome: &RejectedPreparedIntentOutcome, correlation: &str) {
    let audit = outcome.audit_event();
    assert_eq!(audit.actor(), AuditActor::HeadOfProducts);
    assert_eq!(audit.module(), AuditModule::WorkManagement);
    assert_eq!(audit.code().as_str(), RISK_PREPARED_REJECTED_AUDIT_CODE);
    assert_eq!(
        audit.target(),
        &AuditTarget::Risk(RiskId::parse("risk-reject").unwrap())
    );
    assert_eq!(
        audit.correlation_id(),
        &CorrelationId::parse(correlation).unwrap()
    );
    assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Allowed);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
}

#[test]
fn rejecting_consumes_the_preview_records_one_zero_effect_audit_and_changes_no_risk() {
    let clock = Rc::new(Cell::new(100));
    let mut service = new_service(clock, Rc::new(Cell::new(0)), AllowApproval);
    let prepared = prepared_occurrence(&mut service);
    let risk_before = service
        .risk(&RiskId::parse("risk-reject").unwrap())
        .cloned()
        .unwrap();
    let audits_before = service.audit_events().len();

    let outcome = service
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();

    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(100));
    assert!(!outcome.expired_at_rejection());
    assert_zero_effect_audit(&outcome, "risk-reject-1-correlation");
    assert_eq!(service.audit_events().len(), audits_before + 1);
    assert_eq!(
        service.risk(&RiskId::parse("risk-reject").unwrap()),
        Some(&risk_before)
    );

    // Consumed: the same preview can no longer be executed.
    let error = service
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval: WorkManagementApproval::new(
                prepared.id().clone(),
                AuditActor::HeadOfProducts,
                prepared.payload_digest().clone(),
                IdempotencyId::parse("risk-execute-after-reject").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("risk-execute-after-reject"),
        })
        .unwrap_err();
    assert_ne!(error.code(), ErrorCode::PlatformInternal);
    assert_eq!(
        service.risk(&RiskId::parse("risk-reject").unwrap()),
        Some(&risk_before)
    );
}

#[test]
fn the_same_rejection_replays_its_outcome_and_a_second_rejection_is_a_conflict() {
    let mut service = new_service(
        Rc::new(Cell::new(100)),
        Rc::new(Cell::new(0)),
        AllowApproval,
    );
    let prepared = prepared_occurrence(&mut service);
    let first = service
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    let audits_after_first = service.audit_events().len();

    let replayed = service
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    assert_eq!(replayed, first);
    // A replay answers from the record; it records nothing new.
    assert_eq!(service.audit_events().len(), audits_after_first);

    let again = service
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-2"))
        .unwrap_err();
    assert_eq!(again.code(), ErrorCode::DomainConflict);

    // Same idempotency id, different command: a conflict, not a replay.
    let mut differing = reject(prepared.id(), "risk-reject-1");
    differing.actor = AuditActor::PolicyAuthorizedSystem;
    assert_eq!(
        service
            .reject_risk_prepared_intent(differing)
            .unwrap_err()
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
}

#[test]
fn an_unknown_preview_is_not_found_and_an_unauthorized_actor_is_denied() {
    let mut service = new_service(
        Rc::new(Cell::new(100)),
        Rc::new(Cell::new(0)),
        AllowApproval,
    );
    let _ = prepared_occurrence(&mut service);
    let unknown = PreparedIntentId::parse("prepared-risk-unknown").unwrap();
    assert_eq!(
        service
            .reject_risk_prepared_intent(reject(&unknown, "risk-reject-unknown"))
            .unwrap_err()
            .code(),
        ErrorCode::DomainNotFound
    );

    let mut denied = new_service(Rc::new(Cell::new(100)), Rc::new(Cell::new(0)), DenyApproval);
    let prepared = prepared_occurrence(&mut denied);
    let audits_before_denial = denied.audit_events().len();
    assert_eq!(
        denied
            .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-denied"))
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPolicyDenied
    );
    // A denied rejection is refused before anything is recorded.
    assert_eq!(denied.audit_events().len(), audits_before_denial);
}

/// Expiry limits approval, not refusal: an expired-but-unconsumed preview is
/// rejected normally and the outcome says it had expired.
#[test]
fn an_expired_preview_can_still_be_rejected_and_the_outcome_says_so() {
    let clock = Rc::new(Cell::new(100));
    let mut service = new_service(clock.clone(), Rc::new(Cell::new(0)), AllowApproval);
    let prepared = prepared_occurrence(&mut service);
    clock.set(prepared.preview().expires_at().unix_millis() + 1);

    let outcome = service
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-late"))
        .unwrap();
    assert!(outcome.expired_at_rejection());
    assert_eq!(
        outcome.rejected_at(),
        UtcTimestamp::from_unix_millis(prepared.preview().expires_at().unix_millis() + 1)
    );
}

/// The restart half. A rejection is handed back through the typed decode
/// boundary; the rehydrated service replays it without a second audit, and
/// the preview is not outstanding.
#[test]
fn a_rejection_survives_restart_through_the_decode_boundary_and_replays_without_effect() {
    let mut original = new_service(
        Rc::new(Cell::new(100)),
        Rc::new(Cell::new(0)),
        AllowApproval,
    );
    let prepared = prepared_occurrence(&mut original);
    let outcome = original
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    let base = RiskPersistenceSnapshot::try_new(
        vec![original
            .risk(&RiskId::parse("risk-reject").unwrap())
            .cloned()
            .unwrap()],
        vec![],
        vec![],
    )
    .unwrap();

    let snapshot = RiskH2aPersistenceDecodeInput::new(base, vec![])
        .with_rejections(vec![RiskH2aRejectionReplay::new(
            prepared.clone(),
            context("risk-reject-1"),
            outcome.clone(),
        )])
        .decode()
        .unwrap();
    let mut rehydrated = InMemoryRiskService::rehydrate_with_h2a(
        CellClock(Rc::new(Cell::new(200))),
        TestIds(Rc::new(Cell::new(50))),
        AllowApproval,
        AllowPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
        snapshot,
    )
    .unwrap();

    let replayed = rehydrated
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    assert_eq!(replayed, outcome);
    assert_eq!(rehydrated.audit_events().len(), 1);
    assert_eq!(
        rehydrated
            .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-2"))
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
}

/// The corruption the boundary exists to refuse: the same preview handed in
/// as both rejected and outstanding. Neither story is trusted.
#[test]
fn a_rejected_preview_that_is_also_outstanding_is_refused_as_corrupt() {
    let mut original = new_service(
        Rc::new(Cell::new(100)),
        Rc::new(Cell::new(0)),
        AllowApproval,
    );
    let prepared = prepared_occurrence(&mut original);
    let outcome = original
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    let base = RiskPersistenceSnapshot::try_new(
        vec![original
            .risk(&RiskId::parse("risk-reject").unwrap())
            .cloned()
            .unwrap()],
        vec![],
        vec![],
    )
    .unwrap();

    let decoded = RiskH2aPersistenceDecodeInput::new(base, vec![])
        .with_prepared(vec![prepared.clone()])
        .with_rejections(vec![RiskH2aRejectionReplay::new(
            prepared,
            context("risk-reject-1"),
            outcome,
        )])
        .decode();
    assert!(decoded.is_err());
}

/// A rejection whose audit does not say what the service says is refused.
#[test]
fn a_rejection_with_the_wrong_audit_is_refused_as_corrupt() {
    let mut original = new_service(
        Rc::new(Cell::new(100)),
        Rc::new(Cell::new(0)),
        AllowApproval,
    );
    let prepared = prepared_occurrence(&mut original);
    let outcome = original
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    let base = RiskPersistenceSnapshot::try_new(
        vec![original
            .risk(&RiskId::parse("risk-reject").unwrap())
            .cloned()
            .unwrap()],
        vec![],
        vec![],
    )
    .unwrap();

    // Same outcome, but claimed under a different correlation: the audit no
    // longer matches the context it is filed under.
    let decoded = RiskH2aPersistenceDecodeInput::new(base, vec![])
        .with_rejections(vec![RiskH2aRejectionReplay::new(
            prepared,
            context("risk-reject-other"),
            outcome,
        )])
        .decode();
    assert!(decoded.is_err());
}

/// A rejection whose audit carries some other code is not a rejection the
/// service produced, and the boundary must say so -- even when every other
/// field is right.
#[test]
fn a_rejection_whose_audit_carries_the_wrong_code_is_refused_as_corrupt() {
    let mut original = new_service(
        Rc::new(Cell::new(100)),
        Rc::new(Cell::new(0)),
        AllowApproval,
    );
    let prepared = prepared_occurrence(&mut original);
    let outcome = original
        .reject_risk_prepared_intent(reject(prepared.id(), "risk-reject-1"))
        .unwrap();
    let base = RiskPersistenceSnapshot::try_new(
        vec![original
            .risk(&RiskId::parse("risk-reject").unwrap())
            .cloned()
            .unwrap()],
        vec![],
        vec![],
    )
    .unwrap();

    let wrong_code = pmc_domain::work_management::prepared_intent_rejection_audit(
        outcome.audit_event().id().clone(),
        outcome.rejected_at(),
        "risk.closed",
        AuditTarget::Risk(RiskId::parse("risk-reject").unwrap()),
        CorrelationId::parse("risk-reject-1-correlation").unwrap(),
    )
    .unwrap();
    let forged = RejectedPreparedIntentOutcome::new(
        prepared.id().clone(),
        outcome.rejected_at(),
        outcome.expired_at_rejection(),
        wrong_code,
    );

    let decoded = RiskH2aPersistenceDecodeInput::new(base, vec![])
        .with_rejections(vec![RiskH2aRejectionReplay::new(
            prepared,
            context("risk-reject-1"),
            forged,
        )])
        .decode();
    assert!(decoded.is_err());
}
