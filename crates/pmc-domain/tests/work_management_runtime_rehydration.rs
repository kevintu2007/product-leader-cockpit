use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, IssueId,
        PreparedIntentId, RiskId,
    },
    issues::{
        DenyIssueEvidenceAuthority, IssueExecutionPolicy, IssueExecutionPolicyPort,
        IssueH1RuntimeSnapshot, IssueRecord, IssueServiceIdSource, RecordedIssueClassification,
    },
    risks::{
        AllowRiskEvidence, ApproveAndExecuteRecordRiskOccurrence, CreateRisk, InMemoryRiskService,
        PrepareRecordRiskOccurrence, RecordedRiskClassification, RiskExecutionPolicy,
        RiskExecutionPolicyPort, RiskH2aPersistenceDecodeInput, RiskOperationContext,
        RiskPersistenceSnapshot, RiskServiceIdSource,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
        WorkManagementOperation,
    },
    work_management_runtime::WorkManagementRuntimeComposition,
    BoundedText, DomainValueError,
};

#[derive(Clone, Copy)]
struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(100)
    }
}

#[derive(Clone)]
struct TestIds(u64);
impl RiskServiceIdSource for TestIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}
impl IssueServiceIdSource for TestIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct DenyPolicy;
impl RiskExecutionPolicyPort for DenyPolicy {
    fn current_policy(
        &self,
        _: &pmc_domain::work_management::WorkManagementOperation,
    ) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Denied
    }
}

#[derive(Clone, Copy)]
struct AllowAuthorization;
impl ApprovalAuthorizationPort for AllowAuthorization {
    fn authorize(&self, _: AuditActor) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct AllowPolicy;
impl RiskExecutionPolicyPort for AllowPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

impl IssueExecutionPolicyPort for AllowPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

fn risk_context(idempotency_id: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
        correlation_id: CorrelationId::parse(format!("{idempotency_id}-correlation")).unwrap(),
    }
}

fn occurred_risk_snapshot(issue_id: IssueId) -> pmc_domain::risks::RiskH2aRuntimeSnapshot {
    let mut risks = InMemoryRiskService::new(
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    let created = risks
        .create_risk(CreateRisk {
            id: RiskId::parse("synthetic-risk-for-composition").unwrap(),
            title: BoundedText::parse("Synthetic composition risk".to_owned()).unwrap(),
            details: BoundedText::parse("Public-safe occurrence fixture".to_owned()).unwrap(),
            classification: DataClassification::Internal,
            context: risk_context("synthetic-risk-create"),
        })
        .unwrap();
    let prepared = risks
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: created.record.id().clone(),
            expected_version: created.record.version(),
            issue_id,
            context: risk_context("synthetic-risk-prepare"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-risk-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    risks
        .approve_and_execute_record_risk_occurrence(ApproveAndExecuteRecordRiskOccurrence {
            approval,
            context: risk_context("synthetic-risk-execute"),
        })
        .unwrap();
    risks.persistence_snapshot_with_h2a().unwrap()
}
impl IssueExecutionPolicyPort for DenyPolicy {
    fn current_policy(
        &self,
        _: &pmc_domain::work_management::WorkManagementOperation,
    ) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Denied
    }
}

#[test]
fn rehydrates_independent_issue_h1_through_the_shared_runtime_authority() {
    let risk_snapshot = RiskH2aPersistenceDecodeInput::new(
        RiskPersistenceSnapshot::try_new(vec![], vec![], vec![]).unwrap(),
        vec![],
    )
    .decode()
    .unwrap();
    let issue = IssueRecord::from_persisted_created_open(
        IssueId::parse("synthetic-independent-issue").unwrap(),
        BoundedText::parse("Synthetic issue".to_owned()).unwrap(),
        BoundedText::parse("Public-safe restart fixture".to_owned()).unwrap(),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap();
    let runtime = WorkManagementRuntimeComposition::rehydrate_with_h2a_and_issue_h1(
        TestClock,
        TestIds(0),
        pmc_domain::work_management::DenyWorkManagementApproval,
        DenyPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
        TestClock,
        TestIds(0),
        pmc_domain::work_management::DenyWorkManagementApproval,
        DenyPolicy,
        DenyIssueEvidenceAuthority,
        RecordedIssueClassification,
        risk_snapshot,
        IssueH1RuntimeSnapshot::try_new(vec![issue.clone()]).unwrap(),
    )
    .unwrap();

    assert_eq!(runtime.issue(issue.id()), Some(issue));
}

#[test]
fn rehydrates_independent_and_risk_occurrence_issues_through_one_authority() {
    let independent = IssueRecord::from_persisted_created_open(
        IssueId::parse("synthetic-independent-h1").unwrap(),
        BoundedText::parse("Synthetic independent issue".to_owned()).unwrap(),
        BoundedText::parse("Public-safe H1 fixture".to_owned()).unwrap(),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap();
    let occurrence_id = IssueId::parse("synthetic-risk-occurrence").unwrap();
    let runtime = WorkManagementRuntimeComposition::rehydrate_with_h2a_and_issue_h1(
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        DenyIssueEvidenceAuthority,
        RecordedIssueClassification,
        occurred_risk_snapshot(occurrence_id.clone()),
        IssueH1RuntimeSnapshot::try_new(vec![independent.clone()]).unwrap(),
    )
    .unwrap();

    assert_eq!(runtime.issue(independent.id()), Some(independent));
    assert!(runtime.issue(&occurrence_id).is_some());
}

#[test]
fn rejects_independent_issue_identity_that_collides_with_risk_occurrence() {
    let occurrence_id = IssueId::parse("synthetic-colliding-issue").unwrap();
    let independent = IssueRecord::from_persisted_created_open(
        occurrence_id.clone(),
        BoundedText::parse("Synthetic colliding issue".to_owned()).unwrap(),
        BoundedText::parse("Public-safe collision fixture".to_owned()).unwrap(),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap();
    let result = WorkManagementRuntimeComposition::rehydrate_with_h2a_and_issue_h1(
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        DenyIssueEvidenceAuthority,
        RecordedIssueClassification,
        occurred_risk_snapshot(occurrence_id),
        IssueH1RuntimeSnapshot::try_new(vec![independent]).unwrap(),
    );

    assert!(matches!(
        result,
        Err(
            pmc_domain::work_management_runtime::WorkManagementRuntimeRehydrationError::InvalidRiskSnapshot
        )
    ));
}
