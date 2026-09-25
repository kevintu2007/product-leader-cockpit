//! Domain-level proof that the Issue H2a execute path can rehydrate purely
//! from durable parts (current Issue record + outstanding prepared intent),
//! independent of the SQLite persistence adapter.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, IssueId, PreparedIntentId,
    },
    issues::{
        CreateIssue, InMemoryIssueService, IssueEvidenceAuthorityError, IssueEvidenceAuthorityPort,
        IssueExecutionPolicy, IssueExecutionPolicyPort, IssueH2aRuntimeSnapshot,
        IssueH2aRuntimeSnapshotError, IssueOperationContext, IssueServiceIdSource,
        PrepareResolveIssue, RecordedIssueClassification,
    },
    risks::{
        AllowRiskEvidence, RecordedRiskClassification, RiskExecutionPolicy, RiskExecutionPolicyPort,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceReferenceMetadata, EvidenceRole,
        EvidenceVerification, IntegrityDigest, IssueResolutionType, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent,
    },
    work_management_runtime::WorkManagementRuntimeComposition,
    BoundedText, DomainValueError,
};

#[derive(Clone, Default)]
struct Evidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl Evidence {
    fn insert(&self, id: &str, role: EvidenceRole) {
        let parsed = EvidenceReferenceId::parse(id).unwrap();
        self.0.borrow_mut().insert(
            parsed.clone(),
            EvidenceReferenceMetadata::new(
                parsed,
                AggregateVersion::initial(),
                DataClassification::Internal,
                role,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(50),
                    integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                },
            ),
        );
    }
}
impl IssueEvidenceAuthorityPort for Evidence {
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

#[derive(Clone, Copy)]
struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(100)
    }
}

#[derive(Clone)]
struct TestIds(u64);
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
impl pmc_domain::risks::RiskServiceIdSource for TestIds {
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
struct AllowAuthorization;
impl ApprovalAuthorizationPort for AllowAuthorization {
    fn authorize(&self, _: AuditActor) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct AllowPolicy;
impl IssueExecutionPolicyPort for AllowPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}
impl RiskExecutionPolicyPort for AllowPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

fn context(id: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("{id}-correlation")).unwrap(),
    }
}

fn text<const N: usize>(value: &str) -> BoundedText<N> {
    BoundedText::parse(value.to_owned()).unwrap()
}

#[test]
fn rehydrated_composition_executes_a_resolve_from_only_the_issue_and_its_outstanding_prepared_intent(
) {
    let evidence = Evidence::default();
    evidence.insert("ev-h2a-rehydrate-resolution", EvidenceRole::IssueResolution);
    let mut original = InMemoryIssueService::new(
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        evidence.clone(),
        RecordedIssueClassification,
    );
    let created = original
        .create_issue(CreateIssue {
            id: IssueId::parse("synthetic-issue-h2a-rehydrate").unwrap(),
            title: text("Synthetic H2a rehydration issue"),
            details: text("Public-safe H2a rehydration fixture"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: context("synthetic-issue-create"),
        })
        .unwrap();
    let prepared = original
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: created.record.id().clone(),
            expected_version: created.record.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Synthetic resolution rationale"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-h2a-rehydrate-resolution").unwrap()],
            judgment: None,
            context: context("synthetic-issue-prepare"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-issue-execute").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();

    let issue_snapshot =
        IssueH2aRuntimeSnapshot::try_new(vec![created.record.clone()], vec![prepared]).unwrap();
    let mut runtime = WorkManagementRuntimeComposition::rehydrate_with_issue_h2a(
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
        evidence,
        RecordedIssueClassification,
        issue_snapshot,
    )
    .unwrap();

    let outcome = runtime
        .approve_and_execute_resolve_issue(pmc_domain::issues::ApproveAndExecuteIssueTransition {
            approval,
            context: context("synthetic-issue-execute"),
        })
        .unwrap();

    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::IssueState::Resolved
    );
    assert_eq!(outcome.record.version(), AggregateVersion::new(2).unwrap());
    assert_eq!(outcome.audit_events.len(), 1);
    assert!(outcome.approval_receipt_id.is_some());
    assert_eq!(runtime.issue(created.record.id()), Some(outcome.record));
}

/// A snapshot answers about the Issues it carries. A preview whose target is
/// not among them would let the runtime speak for an Issue it cannot see --
/// which no caller could produce while each passed exactly one record and its
/// own preview, and which a whole-Ledger decode makes reachable.
#[test]
fn a_preview_whose_issue_is_absent_is_refused_and_so_is_one_from_another_namespace() {
    let evidence = Evidence::default();
    evidence.insert("ev-absent-target-resolution", EvidenceRole::IssueResolution);
    let mut service = InMemoryIssueService::new(
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        evidence.clone(),
        RecordedIssueClassification,
    );
    let created = service
        .create_issue(CreateIssue {
            id: IssueId::parse("synthetic-issue-absent-target").unwrap(),
            title: text("Synthetic absent-target issue"),
            details: text("Public-safe absent-target fixture"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: context("synthetic-absent-create"),
        })
        .unwrap();
    let prepared = service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: created.record.id().clone(),
            expected_version: created.record.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Synthetic resolution rationale"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-absent-target-resolution").unwrap()],
            judgment: None,
            context: context("synthetic-absent-prepare"),
        })
        .unwrap();

    // Its own Issue present: admitted.
    assert!(
        IssueH2aRuntimeSnapshot::try_new(vec![created.record.clone()], vec![prepared.clone()])
            .is_ok()
    );
    // The same preview with no record behind it: refused.
    assert_eq!(
        IssueH2aRuntimeSnapshot::try_new(vec![], vec![prepared]),
        Err(IssueH2aRuntimeSnapshotError::InvalidPreparedIntent)
    );
    // A preview that is not an Issue operation at all: refused, even with the
    // Issue present.
    let foreign = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-foreign-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: pmc_domain::identity::RiskId::parse("synthetic-foreign-risk").unwrap(),
            risk_version: created.record.version(),
            rationale: text("Synthetic foreign rationale"),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    assert_eq!(
        IssueH2aRuntimeSnapshot::try_new(vec![created.record], vec![foreign]),
        Err(IssueH2aRuntimeSnapshotError::InvalidPreparedIntent)
    );
}
