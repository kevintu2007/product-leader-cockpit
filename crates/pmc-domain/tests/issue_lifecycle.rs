use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::*;
use pmc_domain::issues::*;
use pmc_domain::risks::{
    AllowRiskEvidence, ApproveAndExecuteRecordRiskOccurrence, CreateRisk,
    PrepareRecordRiskOccurrence, RecordedRiskClassification, RiskExecutionPolicy,
    RiskExecutionPolicyPort, RiskOperationContext, RiskServiceIdSource,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::*;
use pmc_domain::work_management_runtime::WorkManagementRuntimeComposition;
use pmc_domain::{BoundedText, DomainValueError};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}
struct Ids(u64);
impl IssueServiceIdSource for Ids {
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
impl RiskServiceIdSource for Ids {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        IssueServiceIdSource::next_prepared_intent_id(self)
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        IssueServiceIdSource::next_approval_receipt_id(self)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        IssueServiceIdSource::next_audit_event_id(self)
    }
}
#[derive(Clone)]
struct Gate(Rc<Cell<bool>>);
impl ApprovalAuthorizationPort for Gate {
    fn authorize(&self, a: AuditActor) -> bool {
        self.0.get() && a == AuditActor::HeadOfProducts
    }
}
impl IssueExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        if self.0.get() {
            IssueExecutionPolicy::Allowed
        } else {
            IssueExecutionPolicy::Denied
        }
    }
}
impl RiskExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        if self.0.get() {
            RiskExecutionPolicy::Allowed
        } else {
            RiskExecutionPolicy::Denied
        }
    }
}
#[derive(Clone)]
struct Evidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
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
#[derive(Clone)]
struct FlakyEvidence {
    available: Rc<Cell<bool>>,
    metadata: EvidenceReferenceMetadata,
}
impl IssueEvidenceAuthorityPort for FlakyEvidence {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        if !self.available.get() {
            return Err(IssueEvidenceAuthorityError::Unavailable);
        }
        if self.metadata.id() != id {
            return Err(IssueEvidenceAuthorityError::NotFound);
        }
        Ok(self.metadata.clone())
    }
}
#[derive(Clone, Copy)]
struct Class;
impl IssueClassificationAuthorityPort for Class {
    fn current_classification(
        &self,
        _: &IssueId,
        c: DataClassification,
    ) -> Result<DataClassification, IssueEvidenceAuthorityError> {
        Ok(c)
    }
}
#[derive(Clone)]
struct DynamicClass(Rc<Cell<DataClassification>>);
impl IssueClassificationAuthorityPort for DynamicClass {
    fn current_classification(
        &self,
        _: &IssueId,
        _: DataClassification,
    ) -> Result<DataClassification, IssueEvidenceAuthorityError> {
        Ok(self.0.get())
    }
}
type Service = InMemoryIssueService<TestClock, Ids, Gate, Gate, Evidence, Class>;
struct Harness {
    service: Service,
    evidence: Evidence,
}
fn text<const N: usize>(v: &str) -> BoundedText<N> {
    BoundedText::parse(v.to_owned()).unwrap()
}
fn ctx(v: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(v).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{v}")).unwrap(),
    }
}
fn harness() -> Harness {
    let gate = Gate(Rc::new(Cell::new(true)));
    let evidence = Evidence(Rc::new(RefCell::new(HashMap::new())));
    Harness {
        service: InMemoryIssueService::new(
            TestClock(Rc::new(Cell::new(100))),
            Ids(0),
            gate.clone(),
            gate,
            evidence.clone(),
            Class,
        ),
        evidence,
    }
}
fn add_evidence(h: &Harness, id: &str, role: EvidenceRole) {
    let eid = EvidenceReferenceId::parse(id).unwrap();
    h.evidence.0.borrow_mut().insert(
        eid.clone(),
        EvidenceReferenceMetadata::new(
            eid,
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
fn create(h: &mut Harness, id: &str, key: &str) -> pmc_domain::issues::IssueRecord {
    h.service
        .create_issue(CreateIssue {
            id: IssueId::parse(id).unwrap(),
            title: text("Synthetic issue"),
            details: text("Synthetic observed condition"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: ctx(key),
        })
        .unwrap()
        .record
}
fn approval(p: &WorkManagementPreparedIntent, key: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        p.id().clone(),
        AuditActor::HeadOfProducts,
        p.payload_digest().clone(),
        IdempotencyId::parse(key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}
fn resolve(
    h: &mut Harness,
    issue: &pmc_domain::issues::IssueRecord,
    prefix: &str,
) -> pmc_domain::issues::IssueRecord {
    let eid = format!("ev-r-{prefix}");
    add_evidence(h, &eid, EvidenceRole::IssueResolution);
    let p = h
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolved with evidence"),
            evidence_ids: vec![EvidenceReferenceId::parse(eid).unwrap()],
            judgment: None,
            context: ctx(&format!("p-r-{prefix}")),
        })
        .unwrap();
    h.service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, &format!("e-r-{prefix}")),
            context: ctx(&format!("e-r-{prefix}")),
        })
        .unwrap()
        .record
}

#[test]
fn independent_issue_starts_open_and_replays_exactly() {
    let mut h = harness();
    let cmd = CreateIssue {
        id: IssueId::parse("issue-a").unwrap(),
        title: text("Synthetic issue"),
        details: text("Occurred"),
        classification: DataClassification::Internal,
        recurrence_of: None,
        context: ctx("create"),
    };
    let a = h.service.create_issue(cmd.clone()).unwrap();
    let count = h.service.audit_events().len();
    assert_eq!(a.record.state(), IssueState::Open);
    assert_eq!(h.service.create_issue(cmd).unwrap(), a);
    assert_eq!(h.service.audit_events().len(), count);
}

#[test]
fn resolve_prepare_idempotency_rejects_resolution_attribute_substitution() {
    let mut h = harness();
    let issue = create(&mut h, "issue-substitute", "create-substitute");
    add_evidence(&h, "ev-substitute", EvidenceRole::IssueResolution);
    let base = PrepareResolveIssue {
        issue_id: issue.id().clone(),
        expected_version: issue.version(),
        resolution_type: IssueResolutionType::Resolved,
        rationale: text("Original rationale"),
        evidence_ids: vec![EvidenceReferenceId::parse("ev-substitute").unwrap()],
        judgment: None,
        context: ctx("prepare-substitute"),
    };
    h.service.prepare_resolve_issue(base.clone()).unwrap();
    let mut changed_type = base.clone();
    changed_type.resolution_type = IssueResolutionType::Workaround;
    assert_eq!(
        h.service
            .prepare_resolve_issue(changed_type)
            .unwrap_err()
            .code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    let mut changed_rationale = base;
    changed_rationale.rationale = text("Changed rationale");
    assert_eq!(
        h.service
            .prepare_resolve_issue(changed_rationale)
            .unwrap_err()
            .code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
}

#[test]
fn resolve_close_retains_distinct_resolution_and_verification_evidence() {
    let mut h = harness();
    let issue = create(&mut h, "issue-a", "create");
    add_evidence(&h, "ev-resolution", EvidenceRole::IssueResolution);
    let p = h
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Workaround,
            rationale: text("Temporary safe path"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-resolution").unwrap()],
            judgment: None,
            context: ctx("prepare-resolve"),
        })
        .unwrap();
    let resolved = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "execute-resolve"),
            context: ctx("execute-resolve"),
        })
        .unwrap()
        .record;
    assert_eq!(resolved.state(), IssueState::Resolved);
    assert_eq!(
        resolved.resolution_type(),
        Some(IssueResolutionType::Workaround)
    );
    add_evidence(&h, "ev-verify", EvidenceRole::IssueClosureVerification);
    let p = h
        .service
        .prepare_close_issue(PrepareCloseIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-verify").unwrap()],
            judgment: None,
            context: ctx("prepare-close"),
        })
        .unwrap();
    let closed = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "execute-close"),
            context: ctx("execute-close"),
        })
        .unwrap()
        .record;
    assert_eq!(closed.state(), IssueState::Closed);
    assert_eq!(closed.resolution_evidence()[0].as_str(), "ev-resolution");
    assert_eq!(
        closed.closure_verification_evidence()[0].as_str(),
        "ev-verify"
    );
}

#[test]
fn failed_verification_reopens_resolved_issue_and_retains_history() {
    let mut h = harness();
    let issue = create(&mut h, "issue-a", "create");
    add_evidence(&h, "ev-resolution", EvidenceRole::IssueResolution);
    let p = h
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Fixed"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-resolution").unwrap()],
            judgment: None,
            context: ctx("pr"),
        })
        .unwrap();
    let resolved = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "er"),
            context: ctx("er"),
        })
        .unwrap()
        .record;
    add_evidence(&h, "ev-failed", EvidenceRole::IssueFailedVerification);
    let p = h
        .service
        .prepare_reopen_issue(PrepareReopenIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            rationale: text("Verification failed"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-failed").unwrap()],
            judgment: None,
            context: ctx("po"),
        })
        .unwrap();
    let open = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "eo"),
            context: ctx("eo"),
        })
        .unwrap()
        .record;
    assert_eq!(open.state(), IssueState::Open);
    assert_eq!(open.failed_verification_evidence()[0].as_str(), "ev-failed");
    assert_eq!(open.resolution_evidence()[0].as_str(), "ev-resolution");
}

#[test]
fn recurrence_creates_new_issue_and_closed_original_is_immutable() {
    let mut h = harness();
    let issue = create(&mut h, "issue-old", "create");
    add_evidence(&h, "ev-r", EvidenceRole::IssueResolution);
    let p = h
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::AcceptedImpact,
            rationale: text("Accepted"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-r").unwrap()],
            judgment: None,
            context: ctx("pr"),
        })
        .unwrap();
    let resolved = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "er"),
            context: ctx("er"),
        })
        .unwrap()
        .record;
    add_evidence(&h, "ev-c", EvidenceRole::IssueClosureVerification);
    let p = h
        .service
        .prepare_close_issue(PrepareCloseIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-c").unwrap()],
            judgment: None,
            context: ctx("pc"),
        })
        .unwrap();
    let closed = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "ec"),
            context: ctx("ec"),
        })
        .unwrap()
        .record;
    let recurrence = h
        .service
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-new").unwrap(),
            title: text("Recurring condition"),
            details: text("Occurred again"),
            classification: DataClassification::Public,
            recurrence_of: Some(closed.id().clone()),
            context: ctx("recurrence"),
        })
        .unwrap()
        .record;
    assert_eq!(recurrence.recurrence_of(), Some(closed.id()));
    assert_eq!(recurrence.classification(), DataClassification::Internal);
    assert_eq!(h.service.issue(closed.id()).unwrap(), closed);
}

#[test]
fn live_evidence_change_invalidates_prepared_transition_without_effect() {
    let mut h = harness();
    let issue = create(&mut h, "issue-a", "create");
    add_evidence(&h, "ev-live", EvidenceRole::IssueResolution);
    let p = h
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Fixed"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-live").unwrap()],
            judgment: None,
            context: ctx("prepare"),
        })
        .unwrap();
    h.evidence
        .0
        .borrow_mut()
        .remove(&EvidenceReferenceId::parse("ev-live").unwrap());
    let error = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "execute"),
            context: ctx("execute"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPolicyDenied
    );
    assert_eq!(
        h.service.issue(issue.id()).unwrap().state(),
        IssueState::Open
    );
}

#[test]
fn transactional_failure_rolls_back_and_same_approval_can_retry() {
    let mut h = harness();
    let issue = create(&mut h, "issue-a", "create");
    add_evidence(&h, "ev-rollback", EvidenceRole::IssueResolution);
    let p = h
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Fixed"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-rollback").unwrap()],
            judgment: None,
            context: ctx("prepare"),
        })
        .unwrap();
    let command = ApproveAndExecuteIssueTransition {
        approval: approval(&p, "execute"),
        context: ctx("execute"),
    };
    h.service.inject_next_commit_failure();
    assert!(h.service.approve_and_execute(command.clone()).is_err());
    assert_eq!(
        h.service.issue(issue.id()).unwrap().state(),
        IssueState::Open
    );
    assert_eq!(
        h.service
            .approve_and_execute(command)
            .unwrap()
            .record
            .state(),
        IssueState::Resolved
    );
}

#[test]
fn h1_create_failure_rolls_back_record_audit_and_idempotency_then_retries() {
    let mut h = harness();
    let command = CreateIssue {
        id: IssueId::parse("issue-h1").unwrap(),
        title: text("Synthetic issue"),
        details: text("Observed"),
        classification: DataClassification::Internal,
        recurrence_of: None,
        context: ctx("h1-create"),
    };
    h.service.inject_next_commit_failure();
    let before = h.service.audit_events().len();
    assert!(h
        .service
        .create_issue(command.clone())
        .unwrap_err()
        .retryable());
    assert!(h.service.issue(&command.id).is_none());
    assert_eq!(h.service.audit_events().len(), before);
    let created = h.service.create_issue(command.clone()).unwrap();
    assert_eq!(created.record.state(), IssueState::Open);
    assert_eq!(h.service.create_issue(command).unwrap(), created);
}

#[test]
fn unavailable_evidence_authority_is_retryable_and_audited_without_effect() {
    let gate = Gate(Rc::new(Cell::new(true)));
    let mut service = InMemoryIssueService::new(
        TestClock(Rc::new(Cell::new(100))),
        Ids(0),
        gate.clone(),
        gate,
        DenyIssueEvidenceAuthority,
        Class,
    );
    let issue = service
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-unavailable").unwrap(),
            title: text("Synthetic issue"),
            details: text("Observed"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: ctx("create-unavailable"),
        })
        .unwrap()
        .record;
    let error = service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolution"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-unavailable").unwrap()],
            judgment: None,
            context: ctx("prepare-unavailable"),
        })
        .unwrap_err();
    assert_eq!(error.code(), pmc_domain::error::ErrorCode::PlatformInternal);
    assert!(error.retryable());
    assert_eq!(service.issue(issue.id()).unwrap().state(), IssueState::Open);
    let audit = service.audit_events().last().unwrap();
    assert_eq!(
        audit.effect_scope(),
        pmc_domain::audit::AuditEffectScope::None
    );
    assert_eq!(
        audit.execution_outcome(),
        pmc_domain::audit::AuditExecutionOutcome::NotAttempted
    );
}

#[test]
fn execute_evidence_outage_preserves_prepared_authority_and_same_approval_retries() {
    let gate = Gate(Rc::new(Cell::new(true)));
    let available = Rc::new(Cell::new(true));
    let evidence_id = EvidenceReferenceId::parse("ev-execute-outage").unwrap();
    let evidence = FlakyEvidence {
        available: available.clone(),
        metadata: EvidenceReferenceMetadata::new(
            evidence_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::IssueResolution,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(50),
                integrity_digest: IntegrityDigest::parse("e".repeat(64)).unwrap(),
            },
        ),
    };
    let mut service = InMemoryIssueService::new(
        TestClock(Rc::new(Cell::new(100))),
        Ids(0),
        gate.clone(),
        gate,
        evidence,
        Class,
    );
    let issue = service
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-execute-outage").unwrap(),
            title: text("Synthetic issue"),
            details: text("Observed"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: ctx("outage-create"),
        })
        .unwrap()
        .record;
    let prepared = service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolved"),
            evidence_ids: vec![evidence_id],
            judgment: None,
            context: ctx("outage-prepare"),
        })
        .unwrap();
    let command = ApproveAndExecuteIssueTransition {
        approval: approval(&prepared, "outage-execute"),
        context: ctx("outage-execute"),
    };
    available.set(false);
    let error = service.approve_and_execute(command.clone()).unwrap_err();
    assert_eq!(error.code(), pmc_domain::error::ErrorCode::PlatformInternal);
    assert!(error.retryable());
    assert_eq!(service.issue(issue.id()).unwrap(), issue);
    let audit = service.audit_events().last().unwrap();
    assert_eq!(
        audit.policy_outcome(),
        pmc_domain::audit::AuditPolicyOutcome::Allowed
    );
    assert_eq!(
        audit.approval_outcome(),
        pmc_domain::audit::AuditApprovalOutcome::Approved
    );
    assert_eq!(
        audit.execution_outcome(),
        pmc_domain::audit::AuditExecutionOutcome::Failed
    );
    assert_eq!(
        audit.effect_scope(),
        pmc_domain::audit::AuditEffectScope::None
    );
    available.set(true);
    let resolved = service.approve_and_execute(command).unwrap().record;
    assert_eq!(resolved.state(), IssueState::Resolved);
}

#[test]
fn each_transition_commits_the_most_restrictive_fresh_classification() {
    let gate = Gate(Rc::new(Cell::new(true)));
    let evidence = Evidence(Rc::new(RefCell::new(HashMap::new())));
    let classes = DynamicClass(Rc::new(Cell::new(DataClassification::Internal)));
    let mut service = InMemoryIssueService::new(
        TestClock(Rc::new(Cell::new(100))),
        Ids(0),
        gate.clone(),
        gate,
        evidence.clone(),
        classes.clone(),
    );
    let issue = service
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-class").unwrap(),
            title: text("Synthetic issue"),
            details: text("Observed"),
            classification: DataClassification::Public,
            recurrence_of: None,
            context: ctx("class-create"),
        })
        .unwrap()
        .record;
    let put = |id: &str, role: EvidenceRole, class: DataClassification| {
        let eid = EvidenceReferenceId::parse(id).unwrap();
        evidence.0.borrow_mut().insert(
            eid.clone(),
            EvidenceReferenceMetadata::new(
                eid,
                AggregateVersion::initial(),
                class,
                role,
                EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(50),
                    integrity_digest: IntegrityDigest::parse("d".repeat(64)).unwrap(),
                },
            ),
        );
    };
    put(
        "ev-class-r",
        EvidenceRole::IssueResolution,
        DataClassification::Confidential,
    );
    let p = service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolved"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-class-r").unwrap()],
            judgment: None,
            context: ctx("class-pr"),
        })
        .unwrap();
    let resolved = service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "class-er"),
            context: ctx("class-er"),
        })
        .unwrap()
        .record;
    assert_eq!(resolved.classification(), DataClassification::Confidential);
    classes.0.set(DataClassification::Restricted);
    put(
        "ev-class-f",
        EvidenceRole::IssueFailedVerification,
        DataClassification::Public,
    );
    let p = service
        .prepare_reopen_issue(PrepareReopenIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            rationale: text("Failed verification"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-class-f").unwrap()],
            judgment: None,
            context: ctx("class-po"),
        })
        .unwrap();
    let open = service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "class-eo"),
            context: ctx("class-eo"),
        })
        .unwrap()
        .record;
    assert_eq!(open.classification(), DataClassification::Restricted);
    classes.0.set(DataClassification::Public);
    put(
        "ev-class-r2",
        EvidenceRole::IssueResolution,
        DataClassification::Public,
    );
    let p = service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: open.id().clone(),
            expected_version: open.version(),
            resolution_type: IssueResolutionType::Workaround,
            rationale: text("Resolved again"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-class-r2").unwrap()],
            judgment: None,
            context: ctx("class-pr2"),
        })
        .unwrap();
    let resolved = service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "class-er2"),
            context: ctx("class-er2"),
        })
        .unwrap()
        .record;
    put(
        "ev-class-c",
        EvidenceRole::IssueClosureVerification,
        DataClassification::Public,
    );
    let p = service
        .prepare_close_issue(PrepareCloseIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-class-c").unwrap()],
            judgment: None,
            context: ctx("class-pc"),
        })
        .unwrap();
    let closed = service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "class-ec"),
            context: ctx("class-ec"),
        })
        .unwrap()
        .record;
    assert_eq!(closed.classification(), DataClassification::Restricted);
}

#[test]
fn stale_versions_and_closed_terminal_are_rejected() {
    let mut h = harness();
    let issue = create(&mut h, "issue-a", "create");
    let stale = h
        .service
        .prepare_close_issue(PrepareCloseIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            evidence_ids: vec![],
            judgment: None,
            context: ctx("stale"),
        })
        .unwrap_err();
    assert_eq!(stale.code(), pmc_domain::error::ErrorCode::DomainConflict);
    assert!(
        matches!(stale.extensions(), [pmc_domain::error::SafeErrorExtension::CurrentVersion(v)] if *v == issue.version())
    );
    assert!(stale.params().iter().any(|p| p.key() == "current_state"));
    assert!(stale
        .params()
        .iter()
        .any(|p| p.key() == "allowed_next_intents"));
    assert!(stale.params().iter().any(|p| p.key() == "remediation"));
    let resolved = resolve(&mut h, &issue, "terminal");
    add_evidence(
        &h,
        "ev-close-terminal",
        EvidenceRole::IssueClosureVerification,
    );
    let p = h
        .service
        .prepare_close_issue(PrepareCloseIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-close-terminal").unwrap()],
            judgment: None,
            context: ctx("pc"),
        })
        .unwrap();
    let closed = h
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "ec"),
            context: ctx("ec"),
        })
        .unwrap()
        .record;
    let error = h
        .service
        .prepare_reopen_issue(PrepareReopenIssue {
            issue_id: closed.id().clone(),
            expected_version: closed.version(),
            rationale: text("not allowed"),
            evidence_ids: vec![],
            judgment: None,
            context: ctx("terminal"),
        })
        .unwrap_err();
    assert_eq!(error.code(), pmc_domain::error::ErrorCode::DomainConflict);
}

#[test]
fn reconstructed_service_has_no_pending_prepared_authority() {
    let mut original = harness();
    let issue = create(&mut original, "issue-a", "create");
    add_evidence(&original, "ev-restart", EvidenceRole::IssueResolution);
    let p = original
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Fixed"),
            evidence_ids: vec![EvidenceReferenceId::parse("ev-restart").unwrap()],
            judgment: None,
            context: ctx("prepare"),
        })
        .unwrap();
    let mut reconstructed = harness();
    let error = reconstructed
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "execute"),
            context: ctx("execute"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
}

#[test]
fn risk_occurrence_and_issue_lifecycle_share_one_atomic_authority() {
    let gate = Gate(Rc::new(Cell::new(true)));
    let evidence = Evidence(Rc::new(RefCell::new(HashMap::new())));
    let clock = TestClock(Rc::new(Cell::new(100)));
    let mut runtime = WorkManagementRuntimeComposition::new(
        clock.clone(),
        Ids(0),
        gate.clone(),
        gate.clone(),
        AllowRiskEvidence,
        RecordedRiskClassification,
        clock,
        Ids(100),
        gate.clone(),
        gate,
        evidence.clone(),
        Class,
    );
    let risk = runtime
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-shared").unwrap(),
            title: text("Synthetic risk"),
            details: text("Synthetic risk occurred"),
            classification: DataClassification::Internal,
            context: RiskOperationContext {
                idempotency_id: IdempotencyId::parse("risk-create").unwrap(),
                correlation_id: CorrelationId::parse("corr-risk-create").unwrap(),
            },
        })
        .unwrap()
        .record;
    let prepared = runtime
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse("issue-shared").unwrap(),
            context: RiskOperationContext {
                idempotency_id: IdempotencyId::parse("risk-prepare").unwrap(),
                correlation_id: CorrelationId::parse("corr-risk-prepare").unwrap(),
            },
        })
        .unwrap();
    let execute = ApproveAndExecuteRecordRiskOccurrence {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("risk-execute").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: RiskOperationContext {
            idempotency_id: IdempotencyId::parse("risk-execute").unwrap(),
            correlation_id: CorrelationId::parse("corr-risk-execute").unwrap(),
        },
    };
    runtime.inject_next_risk_commit_failure();
    assert!(runtime
        .approve_and_execute_record_risk_occurrence(execute.clone())
        .is_err());
    assert!(runtime
        .issue(&IssueId::parse("issue-shared").unwrap())
        .is_none());
    let occurred = runtime
        .approve_and_execute_record_risk_occurrence(execute)
        .unwrap();
    let shared_issue = runtime.issue(occurred.issue.id()).unwrap();
    let resolution_id = EvidenceReferenceId::parse("ev-shared-resolution").unwrap();
    evidence.0.borrow_mut().insert(
        resolution_id.clone(),
        EvidenceReferenceMetadata::new(
            resolution_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::IssueResolution,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(50),
                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
            },
        ),
    );
    let p = runtime
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: shared_issue.id().clone(),
            expected_version: shared_issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolved shared issue"),
            evidence_ids: vec![resolution_id],
            judgment: None,
            context: ctx("shared-prepare-resolve"),
        })
        .unwrap();
    let resolved = runtime
        .approve_and_execute_resolve_issue(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "shared-execute-resolve"),
            context: ctx("shared-execute-resolve"),
        })
        .unwrap()
        .record;
    let closure_id = EvidenceReferenceId::parse("ev-shared-closure").unwrap();
    evidence.0.borrow_mut().insert(
        closure_id.clone(),
        EvidenceReferenceMetadata::new(
            closure_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::IssueClosureVerification,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(60),
                integrity_digest: IntegrityDigest::parse("b".repeat(64)).unwrap(),
            },
        ),
    );
    let p = runtime
        .prepare_close_issue(PrepareCloseIssue {
            issue_id: resolved.id().clone(),
            expected_version: resolved.version(),
            evidence_ids: vec![closure_id],
            judgment: None,
            context: ctx("shared-prepare-close"),
        })
        .unwrap();
    let closed = runtime
        .approve_and_execute_close_issue(ApproveAndExecuteIssueTransition {
            approval: approval(&p, "shared-execute-close"),
            context: ctx("shared-execute-close"),
        })
        .unwrap()
        .record;
    assert_eq!(closed.state(), IssueState::Closed);
    assert_eq!(runtime.issue(closed.id()).unwrap(), closed);
}

#[test]
fn persisted_independent_issue_decode_accepts_only_canonical_initial_open_shape() {
    let id = IssueId::parse("issue-persisted-created").unwrap();
    let title = text("Synthetic persisted issue");
    let details = text("Synthetic only; no organizational data.");
    let decoded = IssueRecord::from_persisted_created_open(
        id.clone(),
        title.clone(),
        details.clone(),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap();
    assert_eq!(decoded.id(), &id);
    assert_eq!(decoded.title(), &title);
    assert_eq!(decoded.details(), &details);
    assert_eq!(decoded.state(), IssueState::Open);
    assert!(decoded.source_risk_id().is_none());
    assert!(decoded.recurrence_of().is_none());
    assert!(IssueRecord::from_persisted_created_open(
        id,
        title,
        details,
        DataClassification::Unclassified,
        AggregateVersion::initial(),
    )
    .is_err());
}

// --- Judgment on Issue resolve / close / reopen (2026-09-18) ---

#[derive(Clone, Copy, Debug)]
enum Transition {
    Resolve,
    Close,
    Reopen,
}

impl Transition {
    const ALL: [Self; 3] = [Self::Resolve, Self::Close, Self::Reopen];
    const fn role(self) -> EvidenceRole {
        match self {
            Self::Resolve => EvidenceRole::IssueResolution,
            Self::Close => EvidenceRole::IssueClosureVerification,
            Self::Reopen => EvidenceRole::IssueFailedVerification,
        }
    }
    const fn wrong_role(self) -> EvidenceRole {
        match self {
            Self::Resolve => EvidenceRole::IssueClosureVerification,
            Self::Close | Self::Reopen => EvidenceRole::IssueResolution,
        }
    }
}

fn digest() -> IntegrityDigest {
    IntegrityDigest::parse("b".repeat(64)).unwrap()
}

fn verified() -> EvidenceVerification {
    EvidenceVerification::Verified {
        verified_at: UtcTimestamp::from_unix_millis(50),
        integrity_digest: digest(),
    }
}
fn observed_unpinned() -> EvidenceVerification {
    EvidenceVerification::ObservedUnpinned {
        observed_at: UtcTimestamp::from_unix_millis(50),
        integrity_digest: digest(),
    }
}
fn degraded() -> EvidenceVerification {
    EvidenceVerification::DegradedLastVerified {
        last_verified_at: UtcTimestamp::from_unix_millis(50),
        integrity_digest: digest(),
    }
}

fn add_evidence_with(h: &Harness, id: &str, role: EvidenceRole, v: EvidenceVerification) {
    let eid = EvidenceReferenceId::parse(id).unwrap();
    h.evidence.0.borrow_mut().insert(
        eid.clone(),
        EvidenceReferenceMetadata::new(
            eid,
            AggregateVersion::initial(),
            DataClassification::Internal,
            role,
            v,
        ),
    );
}

fn judgment(rationale: &str, classification: DataClassification) -> HumanJudgment {
    HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        rationale,
        classification,
    )
    .unwrap()
}

/// An Issue at the state the transition starts from: Open for resolve,
/// Resolved (through fully verified Evidence) for close and reopen.
fn issue_ready_for(h: &mut Harness, t: Transition) -> pmc_domain::issues::IssueRecord {
    let issue = create(h, "issue-j", "create-j");
    match t {
        Transition::Resolve => issue,
        Transition::Close | Transition::Reopen => resolve(h, &issue, "j"),
    }
}

#[allow(clippy::result_large_err)]
fn prepare_transition(
    h: &mut Harness,
    t: Transition,
    issue: &pmc_domain::issues::IssueRecord,
    evidence_ids: Vec<EvidenceReferenceId>,
    judgment: Option<HumanJudgment>,
    key: &str,
) -> Result<WorkManagementPreparedIntent, pmc_domain::error::DomainError> {
    match t {
        Transition::Resolve => h.service.prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolved"),
            evidence_ids,
            judgment,
            context: ctx(key),
        }),
        Transition::Close => h.service.prepare_close_issue(PrepareCloseIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            evidence_ids,
            judgment,
            context: ctx(key),
        }),
        Transition::Reopen => h.service.prepare_reopen_issue(PrepareReopenIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            rationale: text("Verification failed"),
            evidence_ids,
            judgment,
            context: ctx(key),
        }),
    }
}

/// The whole support matrix, for each of resolve, close and reopen: a
/// Judgment moves partly verified Evidence forward as VerificationPending,
/// and nothing moves unverified, mismatched, wrongly-roled or absent
/// Evidence.
#[test]
fn issue_transitions_accept_a_judgment_only_for_partly_verified_evidence() {
    enum Expect {
        Ok(SupportDisposition),
        Refused,
    }
    let cases: Vec<(&str, Option<EvidenceVerification>, bool, bool, Expect)> = vec![
        // (label, verification (None = no Evidence), wrong role, with Judgment, expect)
        (
            "verified",
            Some(verified()),
            false,
            false,
            Expect::Ok(SupportDisposition::EvidenceSatisfied),
        ),
        (
            "unpinned-no-judgment",
            Some(observed_unpinned()),
            false,
            false,
            Expect::Refused,
        ),
        (
            "unpinned-judgment",
            Some(observed_unpinned()),
            false,
            true,
            Expect::Ok(SupportDisposition::VerificationPending),
        ),
        (
            "degraded-no-judgment",
            Some(degraded()),
            false,
            false,
            Expect::Refused,
        ),
        (
            "degraded-judgment",
            Some(degraded()),
            false,
            true,
            Expect::Ok(SupportDisposition::VerificationPending),
        ),
        (
            "unverified-judgment",
            Some(EvidenceVerification::Unverified),
            false,
            true,
            Expect::Refused,
        ),
        (
            "mismatch-judgment",
            Some(EvidenceVerification::IntegrityMismatch),
            false,
            true,
            Expect::Refused,
        ),
        (
            "wrong-role-judgment",
            Some(observed_unpinned()),
            true,
            true,
            Expect::Refused,
        ),
        ("no-evidence-judgment", None, false, true, Expect::Refused),
    ];
    for t in Transition::ALL {
        for (label, verification, wrong_role, with_judgment, expect) in &cases {
            let mut h = harness();
            let issue = issue_ready_for(&mut h, t);
            let evidence_ids = match verification {
                Some(v) => {
                    let role = if *wrong_role {
                        t.wrong_role()
                    } else {
                        t.role()
                    };
                    add_evidence_with(&h, "ev-j", role, v.clone());
                    vec![EvidenceReferenceId::parse("ev-j").unwrap()]
                }
                None => vec![],
            };
            let j = with_judgment
                .then(|| judgment("Owner accepts the limitation", DataClassification::Internal));
            let result = prepare_transition(&mut h, t, &issue, evidence_ids, j.clone(), "prep-j");
            match expect {
                Expect::Ok(disposition) => {
                    let p = result.unwrap_or_else(|e| panic!("{t:?}/{label}: {e:?}"));
                    let support = p.preview().support().unwrap();
                    assert_eq!(support.disposition(), *disposition, "{t:?}/{label}");
                    assert_eq!(support.judgments(), j.as_slice(), "{t:?}/{label}");
                    let after = h
                        .service
                        .approve_and_execute(ApproveAndExecuteIssueTransition {
                            approval: approval(&p, "exec-j"),
                            context: ctx("exec-j"),
                        })
                        .unwrap_or_else(|e| panic!("{t:?}/{label} execute: {e:?}"))
                        .record;
                    let expected_state = match t {
                        Transition::Resolve => IssueState::Resolved,
                        Transition::Close => IssueState::Closed,
                        Transition::Reopen => IssueState::Open,
                    };
                    assert_eq!(after.state(), expected_state, "{t:?}/{label}");
                }
                Expect::Refused => {
                    assert!(result.is_err(), "{t:?}/{label} must be refused");
                    assert_eq!(
                        h.service.issue(issue.id()).unwrap(),
                        issue,
                        "{t:?}/{label} must leave the Issue untouched"
                    );
                }
            }
        }
    }
}

/// The Judgment's rationale and classification are part of the exact
/// preview: changing either changes the digest, and a sensitive Judgment
/// raises the preview's classification.
#[test]
fn issue_judgment_rationale_and_classification_are_bound_by_the_digest() {
    for t in Transition::ALL {
        let digest_for = |j: HumanJudgment| {
            let mut h = harness();
            let issue = issue_ready_for(&mut h, t);
            add_evidence_with(&h, "ev-d", t.role(), observed_unpinned());
            prepare_transition(
                &mut h,
                t,
                &issue,
                vec![EvidenceReferenceId::parse("ev-d").unwrap()],
                Some(j),
                "prep-d",
            )
            .unwrap()
        };
        let base = digest_for(judgment("Reason A", DataClassification::Internal));
        let other_rationale = digest_for(judgment("Reason B", DataClassification::Internal));
        let other_class = digest_for(judgment("Reason A", DataClassification::Confidential));
        assert_ne!(
            base.payload_digest(),
            other_rationale.payload_digest(),
            "{t:?}"
        );
        assert_ne!(base.payload_digest(), other_class.payload_digest(), "{t:?}");
        assert_eq!(base.classification(), DataClassification::Internal, "{t:?}");
        assert_eq!(
            other_class.classification(),
            DataClassification::Confidential,
            "{t:?}: the Judgment's classification is folded in"
        );
    }
}

/// Same client request, different Judgment: an idempotency conflict, never
/// the first preview. The exact retry returns the original preview.
#[test]
fn issue_prepare_retry_with_a_different_judgment_is_an_idempotency_conflict() {
    for t in Transition::ALL {
        let mut h = harness();
        let issue = issue_ready_for(&mut h, t);
        add_evidence_with(&h, "ev-i", t.role(), degraded());
        let ids = vec![EvidenceReferenceId::parse("ev-i").unwrap()];
        let first = prepare_transition(
            &mut h,
            t,
            &issue,
            ids.clone(),
            Some(judgment("Reason A", DataClassification::Internal)),
            "prep-i",
        )
        .unwrap();
        let again = prepare_transition(
            &mut h,
            t,
            &issue,
            ids.clone(),
            Some(judgment("Reason A", DataClassification::Internal)),
            "prep-i",
        )
        .unwrap();
        assert_eq!(first, again, "{t:?}");
        for changed in [
            Some(judgment("Reason B", DataClassification::Internal)),
            Some(judgment("Reason A", DataClassification::Confidential)),
            None,
        ] {
            assert_eq!(
                prepare_transition(&mut h, t, &issue, ids.clone(), changed, "prep-i")
                    .unwrap_err()
                    .code(),
                pmc_domain::error::ErrorCode::DomainIdempotencyConflict,
                "{t:?}"
            );
        }
    }
}

/// An approval carrying the digest of a preview with a different Judgment
/// does not execute.
#[test]
fn issue_approval_with_the_digest_of_a_different_judgment_is_refused() {
    for t in Transition::ALL {
        let mut h = harness();
        let issue = issue_ready_for(&mut h, t);
        add_evidence_with(&h, "ev-a", t.role(), observed_unpinned());
        let ids = vec![EvidenceReferenceId::parse("ev-a").unwrap()];
        let a = prepare_transition(
            &mut h,
            t,
            &issue,
            ids.clone(),
            Some(judgment("Reason A", DataClassification::Internal)),
            "prep-a",
        )
        .unwrap();
        let b = prepare_transition(
            &mut h,
            t,
            &issue,
            ids,
            Some(judgment("Reason B", DataClassification::Internal)),
            "prep-b",
        )
        .unwrap();
        let forged = WorkManagementApproval::new(
            a.id().clone(),
            AuditActor::HeadOfProducts,
            b.payload_digest().clone(),
            IdempotencyId::parse("exec-a").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap();
        assert!(
            h.service
                .approve_and_execute(ApproveAndExecuteIssueTransition {
                    approval: forged,
                    context: ctx("exec-a"),
                })
                .is_err(),
            "{t:?}"
        );
        assert_eq!(h.service.issue(issue.id()).unwrap(), issue, "{t:?}");
    }
}
