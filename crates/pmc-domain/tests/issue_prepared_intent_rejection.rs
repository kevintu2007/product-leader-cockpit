//! v46: the recorded refusal of a pending Issue resolve/close/reopen
//! preview -- live behaviour, restart through the typed snapshot, and the
//! corruption the snapshot must refuse.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use pmc_domain::{
    audit::{AuditActor, AuditApprovalOutcome, AuditModule, AuditPolicyOutcome, AuditTarget},
    classification::DataClassification,
    error::ErrorCode,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, IssueId, PreparedIntentId,
    },
    issues::{
        ApproveAndExecuteIssueTransition, CreateIssue, InMemoryIssueService,
        IssueEvidenceAuthorityError, IssueEvidenceAuthorityPort, IssueExecutionPolicy,
        IssueExecutionPolicyPort, IssueH2aRejectionReplay, IssueH2aRuntimeSnapshot,
        IssueH2aRuntimeSnapshotError, IssueOperationContext, IssueServiceIdSource,
        PrepareResolveIssue, RecordedIssueClassification, RejectIssuePreparedIntent,
    },
    risks::{
        AllowRiskEvidence, RecordedRiskClassification, RiskExecutionPolicy, RiskExecutionPolicyPort,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceReferenceMetadata, EvidenceRole,
        EvidenceVerification, IntegrityDigest, IssueResolutionType, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, ISSUE_PREPARED_REJECTED_AUDIT_CODE,
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

type Service = InMemoryIssueService<
    TestClock,
    TestIds,
    AllowAuthorization,
    AllowPolicy,
    Evidence,
    RecordedIssueClassification,
>;

/// One Open Issue with a prepared resolution. Returns the service, the
/// created record, and the prepared intent.
fn prepared_resolve() -> (
    Service,
    pmc_domain::issues::IssueRecord,
    WorkManagementPreparedIntent,
) {
    let evidence = Evidence::default();
    evidence.insert("ev-reject-resolution", EvidenceRole::IssueResolution);
    let mut service = InMemoryIssueService::new(
        TestClock,
        TestIds(0),
        AllowAuthorization,
        AllowPolicy,
        evidence,
        RecordedIssueClassification,
    );
    let created = service
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-reject").unwrap(),
            title: text("Synthetic rejection issue"),
            details: text("Public-safe rejection fixture"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: context("issue-create"),
        })
        .unwrap();
    let prepared = service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: created.record.id().clone(),
            expected_version: created.record.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Synthetic resolution rationale"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-reject-resolution").unwrap()],
            judgment: None,
            context: context("issue-prepare"),
        })
        .unwrap();
    (service, created.record, prepared)
}

fn reject(prepared_id: &PreparedIntentId, id: &str) -> RejectIssuePreparedIntent {
    RejectIssuePreparedIntent {
        prepared_id: prepared_id.clone(),
        actor: AuditActor::HeadOfProducts,
        context: context(id),
    }
}

#[test]
fn rejecting_consumes_the_preview_records_one_zero_effect_audit_and_changes_no_issue() {
    let (mut service, record, prepared) = prepared_resolve();
    let audits_before = service.audit_events().len();

    let outcome = service
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();

    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(100));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.actor(), AuditActor::HeadOfProducts);
    assert_eq!(audit.module(), AuditModule::WorkManagement);
    assert_eq!(audit.code().as_str(), ISSUE_PREPARED_REJECTED_AUDIT_CODE);
    assert_eq!(audit.target(), &AuditTarget::Issue(record.id().clone()));
    assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Allowed);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(service.audit_events().len(), audits_before + 1);
    assert_eq!(service.issue(record.id()), Some(record.clone()));

    // Consumed: the same preview can no longer be executed, and the Issue
    // is untouched.
    let error = service
        .approve_and_execute_resolve(ApproveAndExecuteIssueTransition {
            approval: WorkManagementApproval::new(
                prepared.id().clone(),
                AuditActor::HeadOfProducts,
                prepared.payload_digest().clone(),
                IdempotencyId::parse("issue-execute-after-reject").unwrap(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .unwrap(),
            context: context("issue-execute-after-reject"),
        })
        .unwrap_err();
    assert_ne!(error.code(), ErrorCode::PlatformInternal);
    assert_eq!(service.issue(record.id()), Some(record));
}

#[test]
fn the_same_rejection_replays_and_a_second_or_unknown_one_is_refused() {
    let (mut service, _, prepared) = prepared_resolve();
    let first = service
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();
    let audits_after_first = service.audit_events().len();
    let replayed = service
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();
    assert_eq!(replayed, first);
    // A replay answers from the record; it records nothing new.
    assert_eq!(service.audit_events().len(), audits_after_first);

    assert_eq!(
        service
            .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-2"))
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
    assert_eq!(
        service
            .reject_issue_prepared_intent(reject(
                &PreparedIntentId::parse("prepared-unknown").unwrap(),
                "issue-reject-unknown"
            ))
            .unwrap_err()
            .code(),
        ErrorCode::DomainNotFound
    );
}

/// Restart: the rejection rides the typed snapshot, the rehydrated
/// composition replays it without a second audit, and the preview is gone.
#[test]
fn a_rejection_survives_restart_through_the_snapshot_and_replays_without_effect() {
    let (mut original, record, prepared) = prepared_resolve();
    let outcome = original
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();

    let snapshot = IssueH2aRuntimeSnapshot::try_new_with_rejections(
        vec![record.clone()],
        vec![],
        vec![IssueH2aRejectionReplay::new(
            prepared.clone(),
            context("issue-reject-1"),
            outcome.clone(),
        )],
    )
    .unwrap();
    let evidence = Evidence::default();
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
        snapshot,
    )
    .unwrap();

    let replayed = runtime
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();
    assert_eq!(replayed, outcome);
    assert_eq!(runtime.issue_audit_events().len(), 1);
    assert_eq!(
        runtime
            .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-2"))
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
}

/// The corruption the snapshot exists to refuse: the same preview handed in
/// as both rejected and outstanding, or a rejection filed under a context
/// its audit does not match.
#[test]
fn a_rejected_preview_that_is_also_outstanding_or_misfiled_is_refused() {
    let (mut original, record, prepared) = prepared_resolve();
    let outcome = original
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();

    assert_eq!(
        IssueH2aRuntimeSnapshot::try_new_with_rejections(
            vec![record.clone()],
            vec![prepared.clone()],
            vec![IssueH2aRejectionReplay::new(
                prepared.clone(),
                context("issue-reject-1"),
                outcome.clone(),
            )],
        )
        .unwrap_err(),
        IssueH2aRuntimeSnapshotError::InvalidRejection
    );
    assert_eq!(
        IssueH2aRuntimeSnapshot::try_new_with_rejections(
            vec![record],
            vec![],
            vec![IssueH2aRejectionReplay::new(
                prepared,
                context("issue-reject-other"),
                outcome,
            )],
        )
        .unwrap_err(),
        IssueH2aRuntimeSnapshotError::InvalidRejection
    );
}

/// A rejection whose audit carries some other code is not a rejection the
/// service produced, and the snapshot must say so -- even when every other
/// field is right.
#[test]
fn a_rejection_whose_audit_carries_the_wrong_code_is_refused() {
    let (mut original, record, prepared) = prepared_resolve();
    let outcome = original
        .reject_issue_prepared_intent(reject(prepared.id(), "issue-reject-1"))
        .unwrap();
    let wrong_code = pmc_domain::work_management::prepared_intent_rejection_audit(
        outcome.audit_event().id().clone(),
        outcome.rejected_at(),
        "issue.resolved",
        AuditTarget::Issue(record.id().clone()),
        CorrelationId::parse("issue-reject-1-correlation").unwrap(),
    )
    .unwrap();
    let forged = pmc_domain::work_management::RejectedPreparedIntentOutcome::new(
        prepared.id().clone(),
        outcome.rejected_at(),
        outcome.expired_at_rejection(),
        wrong_code,
    );
    assert_eq!(
        IssueH2aRuntimeSnapshot::try_new_with_rejections(
            vec![record],
            vec![],
            vec![IssueH2aRejectionReplay::new(
                prepared,
                context("issue-reject-1"),
                forged
            )],
        )
        .unwrap_err(),
        IssueH2aRuntimeSnapshotError::InvalidRejection
    );
}
