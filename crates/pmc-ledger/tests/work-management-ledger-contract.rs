use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use pmc_domain::{
    actions::*,
    audit::AuditActor,
    classification::DataClassification,
    decisions::*,
    identity::*,
    issues::*,
    risks::*,
    time::{Clock, UtcTimestamp},
    work_management::*,
    BoundedText, DomainValueError,
};
use pmc_ledger::InMemoryWorkManagementLedger;

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}
#[derive(Clone)]
struct Ids {
    n: u64,
    family: &'static str,
}
impl Ids {
    fn next(&mut self, kind: &str) -> String {
        self.n += 1;
        format!("{}-{kind}-{}", self.family, self.n)
    }
}
impl ActionServiceIdSource for Ids {
    fn next_action_id(&mut self) -> Result<ActionId, DomainValueError> {
        ActionId::parse(self.next("action"))
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.next("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.next("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.next("audit"))
    }
}
impl DecisionServiceIdSource for Ids {
    fn next_decision_id(&mut self) -> Result<DecisionId, DomainValueError> {
        DecisionId::parse(self.next("decision"))
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.next("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.next("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.next("audit"))
    }
}
impl RiskServiceIdSource for Ids {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.next("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.next("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.next("audit"))
    }
}
impl IssueServiceIdSource for Ids {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(self.next("prepared"))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        ApprovalReceiptId::parse(self.next("receipt"))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        AuditEventId::parse(self.next("audit"))
    }
}
#[derive(Clone, Copy)]
struct Gate;
impl ApprovalAuthorizationPort for Gate {
    fn authorize(&self, a: AuditActor) -> bool {
        a == AuditActor::HeadOfProducts
    }
}
impl ActionExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}
impl DecisionExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        DecisionExecutionPolicy::Allowed
    }
}
impl RiskExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}
impl IssueExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}
#[derive(Clone, Default)]
struct Evidence {
    available: Rc<Cell<bool>>,
    records: Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>,
}
impl ActionEvidenceAuthorityPort for Evidence {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        if !self.available.get() {
            return Err(ActionEvidenceAuthorityError::Unavailable);
        }
        self.records
            .borrow()
            .get(id)
            .cloned()
            .ok_or(ActionEvidenceAuthorityError::NotFound)
    }
}
impl DecisionEvidenceAuthorityPort for Evidence {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        if !self.available.get() {
            return Err(DecisionEvidenceAuthorityError::Unavailable);
        }
        self.records
            .borrow()
            .get(id)
            .cloned()
            .ok_or(DecisionEvidenceAuthorityError::NotFound)
    }
}
impl RiskEvidenceAuthorityPort for Evidence {}
impl RiskClassificationAuthorityPort for Evidence {}
impl IssueEvidenceAuthorityPort for Evidence {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        if !self.available.get() {
            return Err(IssueEvidenceAuthorityError::Unavailable);
        }
        self.records
            .borrow()
            .get(id)
            .cloned()
            .ok_or(IssueEvidenceAuthorityError::NotFound)
    }
}
impl IssueClassificationAuthorityPort for Evidence {
    fn current_classification(
        &self,
        _: &IssueId,
        c: DataClassification,
    ) -> Result<DataClassification, IssueEvidenceAuthorityError> {
        Ok(c)
    }
}

type Ledger = InMemoryWorkManagementLedger<
    TestClock,
    Ids,
    Ids,
    Ids,
    Ids,
    Gate,
    Gate,
    Gate,
    Gate,
    Gate,
    Evidence,
    Evidence,
    Evidence,
    Evidence,
    Evidence,
    Evidence,
>;
fn ledger_with_evidence() -> (Ledger, Evidence) {
    let evidence = Evidence::default();
    evidence.available.set(true);
    (
        InMemoryWorkManagementLedger::new(
            TestClock(Rc::new(Cell::new(100))),
            Ids {
                n: 0,
                family: "action",
            },
            Ids {
                n: 0,
                family: "decision",
            },
            Ids {
                n: 0,
                family: "risk",
            },
            Ids {
                n: 0,
                family: "issue",
            },
            Gate,
            Gate,
            Gate,
            Gate,
            Gate,
            evidence.clone(),
            evidence.clone(),
            evidence.clone(),
            evidence.clone(),
            evidence.clone(),
            evidence.clone(),
        ),
        evidence,
    )
}
fn ledger() -> Ledger {
    ledger_with_evidence().0
}
fn collision_ledger() -> Ledger {
    let evidence = Evidence::default();
    evidence.available.set(true);
    InMemoryWorkManagementLedger::new(
        TestClock(Rc::new(Cell::new(100))),
        Ids {
            n: 0,
            family: "same",
        },
        Ids {
            n: 0,
            family: "decision",
        },
        Ids {
            n: 0,
            family: "same",
        },
        Ids {
            n: 0,
            family: "issue",
        },
        Gate,
        Gate,
        Gate,
        Gate,
        Gate,
        evidence.clone(),
        evidence.clone(),
        evidence.clone(),
        evidence.clone(),
        evidence.clone(),
        evidence,
    )
}
fn text<const N: usize>(v: &str) -> BoundedText<N> {
    BoundedText::parse(v.to_owned()).unwrap()
}
fn action_ctx(id: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn risk_ctx(id: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn decision_ctx(id: &str) -> DecisionOperationContext {
    DecisionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn issue_ctx(id: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn approval(p: &WorkManagementPreparedIntent, id: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        p.id().clone(),
        AuditActor::HeadOfProducts,
        p.payload_digest().clone(),
        IdempotencyId::parse(id).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

#[test]
fn outer_failure_rolls_back_record_audit_and_idempotency_namespace() {
    let mut l = ledger();
    l.inject_next_work_management_commit_failure();
    let command = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-1").unwrap(),
        title: text("Synthetic request"),
        details: text("Public-safe details"),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        context: action_ctx("same-id"),
    };
    assert!(l.create_action_request_draft(command.clone()).is_err());
    assert!(l.action_request(&command.id).is_none());
    assert!(l.work_management_audit_events().is_empty());
    assert!(l.create_action_request_draft(command).is_ok());
}

#[test]
fn terminal_replay_does_not_consume_pending_outer_failure() {
    let mut l = ledger();
    let command = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-replay").unwrap(),
        title: text("Synthetic request"),
        details: text("Public-safe details"),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        context: action_ctx("replay-id"),
    };
    let first = l.create_action_request_draft(command.clone()).unwrap();
    let before_audits = l.work_management_audit_events();
    l.inject_next_work_management_commit_failure();
    let replay = l.create_action_request_draft(command.clone()).unwrap();
    assert_eq!(replay, first);
    assert_eq!(l.action_request(&command.id), Some(first.record.clone()));
    assert_eq!(l.work_management_audit_events(), before_audits);
    let next = CreateActionRequestDraft {
        id: ActionRequestId::parse("request-after-replay").unwrap(),
        title: text("Synthetic next request"),
        details: text("Public-safe details"),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        context: action_ctx("after-replay-id"),
    };
    assert!(l.create_action_request_draft(next.clone()).is_err());
    assert!(l.action_request(&next.id).is_none());
    assert_eq!(l.work_management_audit_events(), before_audits);
    assert!(l.create_action_request_draft(next.clone()).is_ok());
    assert!(l.action_request(&next.id).is_some());
}

#[test]
fn idempotency_ids_are_unique_across_work_families() {
    let mut l = ledger();
    l.create_action_request_draft(CreateActionRequestDraft {
        id: ActionRequestId::parse("request-1").unwrap(),
        title: text("Synthetic request"),
        details: text("Public-safe details"),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        context: action_ctx("shared"),
    })
    .unwrap();
    let error = l
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-1").unwrap(),
            title: text("Synthetic risk"),
            details: text("Public-safe details"),
            classification: DataClassification::Internal,
            context: risk_ctx("shared"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    assert!(l.risk(&RiskId::parse("risk-1").unwrap()).is_none());
}

#[test]
fn idempotency_ids_reject_a_different_operation_in_the_same_family() {
    let mut l = ledger();
    let draft = l
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-same-family-key").unwrap(),
            title: text("Synthetic request"),
            details: text("Public-safe details"),
            intended_owner: None,
            response_due_at: None,
            intended_action_due_at: None,
            classification: DataClassification::Internal,
            context: action_ctx("same-family-operation-key"),
        })
        .unwrap()
        .record;
    let audits = l.work_management_audit_events();
    let error = l
        .submit_action_request(SubmitActionRequest {
            request_id: draft.id().clone(),
            expected_version: draft.version(),
            context: action_ctx("same-family-operation-key"),
        })
        .expect_err("same key cannot name another Action operation");
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(l.action_request(draft.id()), Some(draft));
    assert_eq!(l.work_management_audit_events(), audits);
}

#[test]
fn central_audits_preserve_commit_order_and_reject_cross_family_id_collision() {
    let mut l = ledger();
    l.create_risk(CreateRisk {
        id: RiskId::parse("risk-order").unwrap(),
        title: text("Synthetic risk"),
        details: text("Public-safe details"),
        classification: DataClassification::Internal,
        context: risk_ctx("risk-order"),
    })
    .unwrap();
    l.create_action_request_draft(CreateActionRequestDraft {
        id: ActionRequestId::parse("request-order").unwrap(),
        title: text("Synthetic request"),
        details: text("Public-safe details"),
        intended_owner: None,
        response_due_at: None,
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        context: action_ctx("action-order"),
    })
    .unwrap();
    let ids = l
        .work_management_audit_events()
        .into_iter()
        .map(|event| event.id().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            AuditEventId::parse("risk-audit-1").unwrap(),
            AuditEventId::parse("action-audit-1").unwrap()
        ]
    );

    let mut collision = collision_ledger();
    collision
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-collision").unwrap(),
            title: text("Synthetic request"),
            details: text("Public-safe details"),
            intended_owner: None,
            response_due_at: None,
            intended_action_due_at: None,
            classification: DataClassification::Internal,
            context: action_ctx("action-collision"),
        })
        .unwrap();
    assert!(collision
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-collision").unwrap(),
            title: text("Synthetic risk"),
            details: text("Public-safe details"),
            classification: DataClassification::Internal,
            context: risk_ctx("risk-collision")
        })
        .is_err());
    assert!(collision
        .risk(&RiskId::parse("risk-collision").unwrap())
        .is_none());
    assert_eq!(collision.work_management_audit_events().len(), 1);
}

#[test]
fn named_discard_and_link_queries_do_not_expose_generic_mutation() {
    let mut l = ledger();
    let draft = l
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-discard").unwrap(),
            title: text("Synthetic request"),
            details: text("Public-safe details"),
            intended_owner: Some(StakeholderId::parse("owner-product").unwrap()),
            response_due_at: None,
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(900)),
            classification: DataClassification::Internal,
            context: action_ctx("discard-create"),
        })
        .unwrap()
        .record;
    let open = l
        .submit_action_request(SubmitActionRequest {
            request_id: draft.id().clone(),
            expected_version: draft.version(),
            context: action_ctx("discard-submit"),
        })
        .unwrap()
        .record;
    let prepare = PrepareAcceptActionRequest {
        request_id: open.id().clone(),
        expected_version: open.version(),
        context: action_ctx("discard-prepare"),
    };
    let prepared = l.prepare_accept_action_request(prepare.clone()).unwrap();
    assert!(l.discard_action_prepared_intent(prepared.id()));
    assert!(!l.discard_action_prepared_intent(prepared.id()));
    let replacement = l.prepare_accept_action_request(prepare).unwrap();
    assert_ne!(replacement.id(), prepared.id());
    assert!(l
        .risk_issue_link(
            &RiskId::parse("missing-risk").unwrap(),
            &IssueId::parse("missing-issue").unwrap()
        )
        .is_none());
}

#[test]
fn a_new_ledger_is_empty_and_attention_inputs_are_read_only_queries() {
    let l = ledger();
    assert!(l
        .action_request(&ActionRequestId::parse("request-1").unwrap())
        .is_none());
    assert!(l
        .decision(&DecisionId::parse("decision-1").unwrap())
        .is_none());
    assert!(l.risk(&RiskId::parse("risk-1").unwrap()).is_none());
    assert!(l.issue(&IssueId::parse("issue-1").unwrap()).is_none());
    assert!(l.work_management_audit_events().is_empty());
    let inputs = pmc_domain::attention::AttentionInputs::default();
    let untouched = inputs.clone();
    assert_eq!(
        l.derive_attention(&inputs),
        pmc_domain::attention::derive_attention(&inputs)
    );
    assert_eq!(inputs, untouched);
}

#[test]
fn decision_resulting_request_is_rolled_back_by_outer_failure_then_commits_once() {
    let mut l = ledger();
    let draft = l
        .create_decision_request_draft(CreateDecisionRequestDraft {
            id: DecisionRequestId::parse("decision-request-ledger").unwrap(),
            subject: text("Choose synthetic direction"),
            details: text("Public-safe decision details"),
            intended_owner: Some(StakeholderId::parse("owner-product").unwrap()),
            classification: DataClassification::Internal,
            context: decision_ctx("decision-create"),
        })
        .unwrap()
        .record;
    let open = l
        .submit_decision_request(SubmitDecisionRequest {
            request_id: draft.id().clone(),
            expected_version: draft.version(),
            context: decision_ctx("decision-submit"),
        })
        .unwrap()
        .record;
    let resulting_id = ActionRequestId::parse("decision-resulting-request").unwrap();
    let prepared = l
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: open.id().clone(),
            expected_version: open.version(),
            statement: text("Proceed with synthetic option A"),
            rationale: text("Best public-safe tradeoff"),
            impact: text("Improves synthetic delivery"),
            evidence_ids: vec![],
            judgments: vec![HumanJudgment::new(
                HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                "Synthetic owner judgment",
                DataClassification::Internal,
            )
            .unwrap()],
            resulting_action_requests: vec![DecisionResultingActionRequest {
                id: resulting_id.clone(),
                subject: text("Follow up"),
                details: text("Execute synthetic follow-up"),
                intended_owner: StakeholderId::parse("owner-product").unwrap(),
                due_at: UtcTimestamp::from_unix_millis(900),
                classification: DataClassification::Internal,
            }],
            context: decision_ctx("decision-prepare"),
        })
        .unwrap();
    let execute = ApproveAndExecuteResolveDecisionRequest {
        approval: approval(&prepared, "decision-execute"),
        context: decision_ctx("decision-execute"),
    };
    let before_audits = l.work_management_audit_events();
    l.inject_next_work_management_commit_failure();
    assert!(l
        .approve_and_execute_resolve_decision_request(execute.clone())
        .is_err());
    assert!(l.action_request(&resulting_id).is_none());
    assert_eq!(l.work_management_audit_events(), before_audits);
    let resolved = l
        .approve_and_execute_resolve_decision_request(execute)
        .unwrap();
    assert_eq!(
        l.action_request(&resulting_id)
            .unwrap()
            .source_decision_id(),
        Some(resolved.decision.id())
    );
    assert_eq!(
        l.action_requests_for_decision(resolved.decision.id()).len(),
        1
    );
    assert!(l.actions_for_decision(resolved.decision.id()).is_empty());
}

#[test]
fn risk_occurrence_issue_authority_is_deeply_isolated_by_outer_transaction() {
    let mut l = ledger();
    let risk = l
        .create_risk(CreateRisk {
            id: RiskId::parse("risk-occurrence").unwrap(),
            title: text("Synthetic risk"),
            details: text("Public-safe occurred condition"),
            classification: DataClassification::Internal,
            context: risk_ctx("risk-create"),
        })
        .unwrap()
        .record;
    let prepared = l
        .prepare_record_risk_occurrence(PrepareRecordRiskOccurrence {
            risk_id: risk.id().clone(),
            expected_version: risk.version(),
            issue_id: IssueId::parse("issue-occurrence").unwrap(),
            context: risk_ctx("risk-prepare"),
        })
        .unwrap();
    let execute = ApproveAndExecuteRecordRiskOccurrence {
        approval: approval(&prepared, "risk-execute"),
        context: risk_ctx("risk-execute"),
    };
    l.inject_next_work_management_commit_failure();
    assert!(l
        .approve_and_execute_record_risk_occurrence(execute.clone())
        .is_err());
    assert!(l
        .issue(&IssueId::parse("issue-occurrence").unwrap())
        .is_none());
    assert!(l.risk_issue_links().is_empty());
    let occurred = l
        .approve_and_execute_record_risk_occurrence(execute)
        .unwrap();
    assert_eq!(l.issue(occurred.issue.id()), Some(occurred.issue.clone()));
    assert_eq!(l.risk_issue_links().len(), 1);
    assert!(l
        .risk_issue_link(occurred.risk.id(), occurred.issue.id())
        .is_some());
}

#[test]
fn audited_retry_after_dependency_recovery_still_obeys_outer_failure_atomically() {
    let (mut l, evidence) = ledger_with_evidence();
    let issue = l
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-retry").unwrap(),
            title: text("Synthetic issue"),
            details: text("Public-safe observed condition"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: issue_ctx("issue-create"),
        })
        .unwrap()
        .record;
    let evidence_id = EvidenceReferenceId::parse("issue-resolution-evidence").unwrap();
    evidence.records.borrow_mut().insert(
        evidence_id.clone(),
        EvidenceReferenceMetadata::new(
            evidence_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::IssueResolution,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(50),
                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
            },
        ),
    );
    let prepared = l
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Resolved with synthetic evidence"),
            evidence_ids: vec![evidence_id],
            judgment: None,
            context: issue_ctx("issue-prepare"),
        })
        .unwrap();
    let execute = ApproveAndExecuteIssueTransition {
        approval: approval(&prepared, "issue-execute"),
        context: issue_ctx("issue-execute"),
    };
    let other_issue = l
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-retry-other").unwrap(),
            title: text("Other synthetic issue"),
            details: text("Public-safe observed condition"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: issue_ctx("issue-create-other"),
        })
        .unwrap()
        .record;
    let other_prepared = l
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: other_issue.id().clone(),
            expected_version: other_issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Other resolution with synthetic evidence"),
            evidence_ids: vec![EvidenceReferenceId::parse("issue-resolution-evidence").unwrap()],
            judgment: None,
            context: issue_ctx("issue-prepare-other"),
        })
        .unwrap();
    evidence.available.set(false);
    let denial = l
        .approve_and_execute_resolve_issue(execute.clone())
        .expect_err("unavailable evidence is denied");
    let before_record = l.issue(issue.id()).unwrap();
    let before_domain_audits = l.issue_audit_events();
    let before_central_audits = l.work_management_audit_events();
    let different_identity = ApproveAndExecuteIssueTransition {
        approval: approval(&other_prepared, "other-approval"),
        context: issue_ctx("issue-execute"),
    };
    assert_eq!(
        l.approve_and_execute_resolve_issue(different_identity)
            .expect_err("same key cannot execute a different prepared intent")
            .code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    assert_eq!(l.issue(other_issue.id()), Some(other_issue));
    assert_eq!(l.issue_audit_events(), before_domain_audits);
    assert_eq!(l.work_management_audit_events(), before_central_audits);
    let mut changed_correlation = execute.clone();
    changed_correlation.context.correlation_id =
        CorrelationId::parse("corr-issue-execute-replay").unwrap();
    assert_eq!(
        l.approve_and_execute_resolve_issue(changed_correlation)
            .expect_err("exact denial replay"),
        denial
    );
    assert_eq!(l.issue_audit_events(), before_domain_audits);
    assert_eq!(l.work_management_audit_events(), before_central_audits);
    let cross_family = l.create_risk(CreateRisk {
        id: RiskId::parse("risk-after-issue-denial").unwrap(),
        title: text("Synthetic risk"),
        details: text("Public-safe details"),
        classification: DataClassification::Internal,
        context: risk_ctx("issue-execute"),
    });
    assert_eq!(
        cross_family
            .expect_err("denial owns the global idempotency key")
            .code(),
        pmc_domain::error::ErrorCode::DomainIdempotencyConflict
    );
    evidence.available.set(true);
    l.inject_next_work_management_commit_failure();
    assert!(l
        .approve_and_execute_resolve_issue(execute.clone())
        .is_err());
    assert_eq!(l.issue(issue.id()), Some(before_record));
    assert_eq!(l.issue_audit_events(), before_domain_audits);
    assert_eq!(l.work_management_audit_events(), before_central_audits);
    let resolved = l.approve_and_execute_resolve_issue(execute).unwrap();
    assert_eq!(l.issue(issue.id()), Some(resolved.record));
    assert_eq!(l.issue_audit_events().len(), before_domain_audits.len() + 1);
    assert_eq!(
        l.work_management_audit_events().len(),
        before_central_audits.len() + 1
    );
}
