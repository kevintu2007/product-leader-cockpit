use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pmc_domain::actions::*;
use pmc_domain::audit::{
    AuditActor, AuditApprovalOutcome, AuditEffectScope, AuditExecutionOutcome, AuditPolicyOutcome,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::decisions::*;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::*;

#[derive(Clone)]
struct TestClock(Rc<Cell<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}
struct DecisionIds {
    counter: u64,
    control: Rc<Cell<i64>>,
}
impl DecisionServiceIdSource for DecisionIds {
    fn next_decision_id(&mut self) -> Result<DecisionId, pmc_domain::DomainValueError> {
        self.counter += 1;
        DecisionId::parse(format!("decision-{}", self.counter))
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.counter += 1;
        PreparedIntentId::parse(format!("decision-prepared-{}", self.counter))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.counter += 1;
        ApprovalReceiptId::parse(format!("decision-receipt-{}", self.counter))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        if self.control.get() == -3 {
            return AuditEventId::parse("");
        }
        self.counter += 1;
        AuditEventId::parse(format!("decision-audit-{}", self.counter))
    }
}
struct ActionIds(u64);
impl ActionServiceIdSource for ActionIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ActionId::parse(format!("action-{}", self.0))
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("action-prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("action-receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("action-audit-{}", self.0))
    }
}
#[derive(Clone)]
struct Gate(Rc<Cell<i64>>);
impl ApprovalAuthorizationPort for Gate {
    fn authorize(&self, a: AuditActor) -> bool {
        a == AuditActor::HeadOfProducts && self.0.get() != -2
    }
}
impl DecisionExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        if self.0.get() == -1 {
            DecisionExecutionPolicy::Denied
        } else {
            DecisionExecutionPolicy::Allowed
        }
    }
}
impl ActionExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}
#[derive(Clone, Default)]
struct Evidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl DecisionEvidenceAuthorityPort for Evidence {
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
impl ActionEvidenceAuthorityPort for Evidence {
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
impl Evidence {
    fn put(&self, x: EvidenceReferenceMetadata) {
        self.0.borrow_mut().insert(x.id().clone(), x);
    }
}
type Decisions = InMemoryDecisionService<TestClock, DecisionIds, Gate, Gate, Evidence>;
type Actions = InMemoryActionService<TestClock, ActionIds, Gate, Gate, Evidence>;
fn services() -> (Decisions, Actions, Rc<Cell<i64>>, Evidence) {
    let c = Rc::new(Cell::new(100));
    let e = Evidence::default();
    (
        InMemoryDecisionService::new(
            TestClock(c.clone()),
            DecisionIds {
                counter: 0,
                control: c.clone(),
            },
            Gate(c.clone()),
            Gate(c.clone()),
            e.clone(),
        ),
        InMemoryActionService::new(
            TestClock(c.clone()),
            ActionIds(0),
            Gate(c.clone()),
            Gate(c.clone()),
            e.clone(),
        ),
        c,
        e,
    )
}
fn txt<const N: usize>(s: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(s.to_owned()).unwrap()
}
fn ctx(id: &str) -> DecisionOperationContext {
    DecisionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("corr-{id}")).unwrap(),
    }
}
fn ctx_corr(id: &str, correlation: &str) -> DecisionOperationContext {
    DecisionOperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}
fn owner() -> StakeholderId {
    StakeholderId::parse("owner-product").unwrap()
}
fn judgment() -> HumanJudgment {
    HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Synthetic owner judgment",
        DataClassification::Internal,
    )
    .unwrap()
}
fn resulting(id: &str) -> DecisionResultingActionRequest {
    DecisionResultingActionRequest {
        id: ActionRequestId::parse(id).unwrap(),
        subject: txt("Follow up"),
        details: txt("Execute synthetic follow-up"),
        intended_owner: owner(),
        due_at: UtcTimestamp::from_unix_millis(900),
        classification: DataClassification::Internal,
    }
}
fn open(s: &mut Decisions, id: &str) -> DecisionRequestRecord {
    let d = s
        .create_decision_request_draft(CreateDecisionRequestDraft {
            id: DecisionRequestId::parse(id).unwrap(),
            subject: txt("Choose direction"),
            details: txt("Synthetic decision details"),
            intended_owner: Some(owner()),
            classification: DataClassification::Internal,
            context: ctx(&format!("create-{id}")),
        })
        .unwrap()
        .record;
    s.submit_decision_request(SubmitDecisionRequest {
        request_id: d.id().clone(),
        expected_version: d.version(),
        context: ctx(&format!("submit-{id}")),
    })
    .unwrap()
    .record
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
fn resolve(s: &mut Decisions, a: &mut Actions, id: &str) -> ResolvedDecisionOutcome {
    let r = open(s, id);
    let p = s
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: r.id().clone(),
            expected_version: r.version(),
            statement: txt("Proceed with option A"),
            rationale: txt("Best synthetic tradeoff"),
            impact: txt("Improves synthetic delivery"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![resulting(&format!("action-request-{id}"))],
            context: ctx(&format!("prepare-{id}")),
        })
        .unwrap();
    s.approve_and_execute_resolve_decision_request(
        ApproveAndExecuteResolveDecisionRequest {
            approval: approval(&p, &format!("execute-{id}")),
            context: ctx(&format!("execute-{id}")),
        },
        a,
    )
    .unwrap()
}

#[test]
fn request_lifecycle_has_only_draft_open_resolved_or_withdrawn() {
    let (mut s, _, _, _) = services();
    let d = s
        .create_decision_request_draft(CreateDecisionRequestDraft {
            id: DecisionRequestId::parse("request-withdraw").unwrap(),
            subject: txt("Withdraw me"),
            details: txt("Synthetic"),
            intended_owner: Some(owner()),
            classification: DataClassification::Internal,
            context: ctx("create-withdraw"),
        })
        .unwrap()
        .record;
    assert_eq!(d.state(), DecisionRequestState::Draft);
    let o = s
        .submit_decision_request(SubmitDecisionRequest {
            request_id: d.id().clone(),
            expected_version: d.version(),
            context: ctx("submit-withdraw"),
        })
        .unwrap()
        .record;
    let w = s
        .withdraw_decision_request(WithdrawDecisionRequest {
            request_id: o.id().clone(),
            expected_version: o.version(),
            rationale: txt("No longer needed"),
            context: ctx("withdraw"),
        })
        .unwrap()
        .record;
    assert_eq!(w.state(), DecisionRequestState::Withdrawn);
    assert!(w.withdrawal_rationale().is_some());
    assert_eq!(
        s.submit_decision_request(SubmitDecisionRequest {
            request_id: w.id().clone(),
            expected_version: w.version(),
            context: ctx("illegal")
        })
        .unwrap_err()
        .code(),
        ErrorCode::DomainConflict
    );
}

#[test]
fn resolve_is_exact_atomic_and_creates_open_requests_but_no_actions() {
    let (mut s, mut a, _, _) = services();
    let out = resolve(&mut s, &mut a, "request-resolve");
    assert_eq!(out.request.state(), DecisionRequestState::Resolved);
    assert_eq!(out.decision.state(), DecisionState::Effective);
    assert_eq!(out.decision.statement().as_str(), "Proceed with option A");
    let id = &out.resulting_action_request_ids[0];
    let request = a.request(id).unwrap();
    assert_eq!(request.state(), ActionRequestState::Open);
    assert_eq!(request.source_decision_id(), Some(out.decision.id()));
    assert!(a.actions_for_decision(out.decision.id()).is_empty());
    assert_eq!(
        s.request(out.request.id()).unwrap().linked_decision_id(),
        Some(out.decision.id())
    );
}

#[test]
fn evidence_mutation_and_action_commit_failure_leave_both_services_unchanged() {
    let (mut s, mut a, _, e) = services();
    let r = open(&mut s, "request-evidence");
    let eid = EvidenceReferenceId::parse("decision-evidence").unwrap();
    let restrictive = EvidenceReferenceId::parse("decision-evidence-restricted").unwrap();
    e.put(EvidenceReferenceMetadata::new(
        eid.clone(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::DecisionResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(90),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    ));
    e.put(EvidenceReferenceMetadata::new(
        restrictive.clone(),
        AggregateVersion::initial(),
        DataClassification::Restricted,
        EvidenceRole::DecisionResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(90),
            integrity_digest: IntegrityDigest::parse("b".repeat(64)).unwrap(),
        },
    ));
    let p = s
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: r.id().clone(),
            expected_version: r.version(),
            statement: txt("Evidence decision"),
            rationale: txt("Verified"),
            impact: txt("Synthetic"),
            evidence_ids: vec![eid.clone(), restrictive],
            judgments: vec![],
            resulting_action_requests: vec![resulting("action-request-evidence")],
            context: ctx("prepare-evidence"),
        })
        .unwrap();
    e.put(EvidenceReferenceMetadata::new(
        eid,
        AggregateVersion::initial(),
        DataClassification::Confidential,
        EvidenceRole::DecisionResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(90),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    ));
    assert_eq!(
        s.approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: approval(&p, "execute-evidence"),
                context: ctx("execute-evidence")
            },
            &mut a
        )
        .unwrap_err()
        .code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(
        s.request(r.id()).unwrap().state(),
        DecisionRequestState::Open
    );
    assert!(a
        .request(&ActionRequestId::parse("action-request-evidence").unwrap())
        .is_none());
}

#[test]
fn supersede_replaces_immutable_decision_and_flags_only_incomplete_downstream() {
    let (mut s, mut a, _, _) = services();
    let first = resolve(&mut s, &mut a, "request-old");
    let p = s
        .prepare_supersede_decision(
            PrepareSupersedeDecision {
                decision_id: first.decision.id().clone(),
                expected_version: first.decision.version(),
                replacement_statement: txt("Proceed with option B"),
                replacement_rationale: txt("Premise changed"),
                replacement_impact: txt("Revised synthetic impact"),
                replacement_owner: owner(),
                evidence_ids: vec![],
                judgments: vec![judgment()],
                resulting_action_requests: vec![resulting("replacement-action-request")],
                context: ctx("prepare-supersede"),
            },
            &a,
        )
        .unwrap();
    let out = s
        .approve_and_execute_supersede_decision(
            ApproveAndExecuteSupersedeDecision {
                approval: approval(&p, "execute-supersede"),
                context: ctx("execute-supersede"),
            },
            &mut a,
        )
        .unwrap();
    assert_eq!(out.superseded.state(), DecisionState::Superseded);
    assert_eq!(out.replacement.state(), DecisionState::Effective);
    assert_eq!(
        out.replacement.supersedes_decision_id(),
        Some(out.superseded.id())
    );
    let old = a.request(&first.resulting_action_request_ids[0]).unwrap();
    assert_eq!(old.state(), ActionRequestState::Open);
    assert!(old.has_superseded_premise());
    assert_eq!(
        a.request(&ActionRequestId::parse("replacement-action-request").unwrap())
            .unwrap()
            .state(),
        ActionRequestState::Open
    );
    assert_eq!(
        out.audit_events
            .iter()
            .map(|event| event.code().as_str())
            .collect::<Vec<_>>(),
        vec![
            "decision.superseded",
            "decision.replacement_created",
            "decision.replacement_linked"
        ]
    );
    let child_codes = a
        .audit_events()
        .iter()
        .rev()
        .take(2)
        .map(|event| event.code().as_str())
        .collect::<Vec<_>>();
    assert!(child_codes.contains(&"action_request.created_from_decision"));
    assert!(child_codes.contains(&"action_request.superseded_premise_marked"));
    assert!(a
        .audit_events()
        .iter()
        .rev()
        .take(2)
        .all(|event| event.approval_outcome() == AuditApprovalOutcome::Approved));
}

#[test]
fn missing_owner_support_policy_and_expiry_fail_without_effect() {
    let (mut service, mut actions, control, _) = services();
    let draft = service
        .create_decision_request_draft(CreateDecisionRequestDraft {
            id: DecisionRequestId::parse("request-gates").unwrap(),
            subject: txt("Gate checks"),
            details: txt("Synthetic"),
            intended_owner: None,
            classification: DataClassification::Internal,
            context: ctx("create-gates"),
        })
        .unwrap()
        .record;
    let ownerless_open = service
        .submit_decision_request(SubmitDecisionRequest {
            request_id: draft.id().clone(),
            expected_version: draft.version(),
            context: ctx("submit-gates"),
        })
        .unwrap()
        .record;
    let command = |context| PrepareResolveDecisionRequest {
        request_id: ownerless_open.id().clone(),
        expected_version: ownerless_open.version(),
        statement: txt("Synthetic statement"),
        rationale: txt("Synthetic rationale"),
        impact: txt("Synthetic impact"),
        evidence_ids: vec![],
        judgments: vec![],
        resulting_action_requests: vec![],
        context,
    };
    assert_eq!(
        service
            .prepare_resolve_decision_request(command(ctx("missing-owner")))
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
    let mut service_and_actions = services();
    let request = open(&mut service_and_actions.0, "request-support");
    let unsupported = PrepareResolveDecisionRequest {
        request_id: request.id().clone(),
        expected_version: request.version(),
        statement: txt("Synthetic statement"),
        rationale: txt("Synthetic rationale"),
        impact: txt("Synthetic impact"),
        evidence_ids: vec![],
        judgments: vec![],
        resulting_action_requests: vec![],
        context: ctx("missing-support"),
    };
    assert_eq!(
        service_and_actions
            .0
            .prepare_resolve_decision_request(unsupported)
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPolicyDenied
    );
    let support_request = open(&mut service, "request-unclassified-support");
    assert_eq!(
        service
            .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
                request_id: support_request.id().clone(),
                expected_version: support_request.version(),
                statement: txt("Support class"),
                rationale: txt("Synthetic"),
                impact: txt("Synthetic"),
                evidence_ids: vec![],
                judgments: vec![HumanJudgment::new(
                    HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                    "Unclassified support",
                    DataClassification::Unclassified
                )
                .unwrap()],
                resulting_action_requests: vec![],
                context: ctx("prepare-unclassified-support")
            })
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPolicyDenied
    );
    let downstream_request = open(&mut service, "request-unclassified-result");
    let mut unclassified_result = resulting("unclassified-result");
    unclassified_result.classification = DataClassification::Unclassified;
    assert_eq!(
        service
            .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
                request_id: downstream_request.id().clone(),
                expected_version: downstream_request.version(),
                statement: txt("Result class"),
                rationale: txt("Synthetic"),
                impact: txt("Synthetic"),
                evidence_ids: vec![],
                judgments: vec![judgment()],
                resulting_action_requests: vec![unclassified_result],
                context: ctx("prepare-unclassified-result")
            })
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPolicyDenied
    );
    let denial = service_and_actions.0.audit_events().last().unwrap();
    assert_eq!(denial.policy_outcome(), AuditPolicyOutcome::Denied);
    assert_eq!(denial.approval_outcome(), AuditApprovalOutcome::NotRequired);
    assert_eq!(
        denial.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(denial.effect_scope(), AuditEffectScope::None);
    let policy_request = open(&mut service, "request-policy");
    control.set(-1);
    assert_eq!(
        service
            .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
                request_id: policy_request.id().clone(),
                expected_version: policy_request.version(),
                statement: txt("Synthetic statement"),
                rationale: txt("Synthetic rationale"),
                impact: txt("Synthetic impact"),
                evidence_ids: vec![],
                judgments: vec![judgment()],
                resulting_action_requests: vec![],
                context: ctx("policy-denied"),
            })
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPolicyDenied
    );
    assert!(actions
        .actions_for_decision(&DecisionId::parse("decision-none").unwrap())
        .is_empty());
    control.set(100);
    let request = open(&mut service, "request-expiry");
    let prepared = service
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            statement: txt("Synthetic statement"),
            rationale: txt("Synthetic rationale"),
            impact: txt("Synthetic impact"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![],
            context: ctx("prepare-expiry"),
        })
        .unwrap();
    control.set(prepared.preview().expires_at().unix_millis());
    assert_eq!(
        service
            .approve_and_execute_resolve_decision_request(
                ApproveAndExecuteResolveDecisionRequest {
                    approval: approval(&prepared, "execute-expiry"),
                    context: ctx("execute-expiry"),
                },
                &mut actions,
            )
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(
        service.request(request.id()).unwrap().state(),
        DecisionRequestState::Open
    );
}

#[test]
fn action_or_decision_commit_failure_rolls_back_both_services() {
    for fail_action in [true, false] {
        let (mut service, mut actions, _, _) = services();
        let request = open(
            &mut service,
            if fail_action {
                "request-action-fail"
            } else {
                "request-decision-fail"
            },
        );
        let resulting_id = if fail_action {
            "result-action-fail"
        } else {
            "result-decision-fail"
        };
        let prepared = service
            .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
                request_id: request.id().clone(),
                expected_version: request.version(),
                statement: txt("Atomic statement"),
                rationale: txt("Atomic rationale"),
                impact: txt("Atomic impact"),
                evidence_ids: vec![],
                judgments: vec![judgment()],
                resulting_action_requests: vec![resulting(resulting_id)],
                context: ctx(if fail_action {
                    "prepare-action-fail"
                } else {
                    "prepare-decision-fail"
                }),
            })
            .unwrap();
        if fail_action {
            actions.inject_next_commit_failure();
        } else {
            service.inject_next_commit_failure();
        }
        let execute_id = if fail_action {
            "execute-action-fail"
        } else {
            "execute-decision-fail"
        };
        assert_eq!(
            service
                .approve_and_execute_resolve_decision_request(
                    ApproveAndExecuteResolveDecisionRequest {
                        approval: approval(&prepared, execute_id),
                        context: ctx(execute_id)
                    },
                    &mut actions
                )
                .unwrap_err()
                .code(),
            ErrorCode::PlatformInternal
        );
        let failure_audit = service.audit_events().last().unwrap();
        assert_eq!(failure_audit.code().as_str(), "decision.execution_failed");
        assert_eq!(failure_audit.policy_outcome(), AuditPolicyOutcome::Allowed);
        assert_eq!(
            failure_audit.approval_outcome(),
            AuditApprovalOutcome::Approved
        );
        assert_eq!(
            failure_audit.execution_outcome(),
            AuditExecutionOutcome::Failed
        );
        assert_eq!(failure_audit.effect_scope(), AuditEffectScope::None);
        assert_eq!(
            service.request(request.id()).unwrap().state(),
            DecisionRequestState::Open
        );
        assert!(actions
            .request(&ActionRequestId::parse(resulting_id).unwrap())
            .is_none());
    }
}

#[test]
fn execute_replay_uses_business_identity_and_max_length_caller_id_is_safe() {
    let (mut service, mut actions, _, _) = services();
    let request = open(&mut service, "request-replay");
    let prepared = service
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            statement: txt("Replay statement"),
            rationale: txt("Replay rationale"),
            impact: txt("Replay impact"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![resulting("replay-result")],
            context: ctx("prepare-replay"),
        })
        .unwrap();
    let execute_id = "x".repeat(128);
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(execute_id.clone()).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let first = service
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: approval.clone(),
                context: ctx_corr(&execute_id, "corr-replay-first"),
            },
            &mut actions,
        )
        .unwrap();
    let decision_audits = service.audit_events().len();
    let action_audits = actions.audit_events().len();
    let replay = service
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: approval.clone(),
                context: ctx_corr(&execute_id, "corr-replay-second"),
            },
            &mut actions,
        )
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(service.audit_events().len(), decision_audits);
    assert_eq!(actions.audit_events().len(), action_audits);
    let other_request = open(&mut service, "request-other-digest");
    let other = service
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: other_request.id().clone(),
            expected_version: other_request.version(),
            statement: txt("Other"),
            rationale: txt("Other rationale"),
            impact: txt("Other impact"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![],
            context: ctx("prepare-other-digest"),
        })
        .unwrap();
    let changed = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        other.payload_digest().clone(),
        IdempotencyId::parse(execute_id.clone()).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    assert_eq!(
        service
            .approve_and_execute_resolve_decision_request(
                ApproveAndExecuteResolveDecisionRequest {
                    approval: changed,
                    context: ctx_corr(&execute_id, "corr-replay-changed")
                },
                &mut actions
            )
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
    assert_eq!(
        service
            .create_decision_request_draft(CreateDecisionRequestDraft {
                id: DecisionRequestId::parse("cross-family").unwrap(),
                subject: txt("Cross"),
                details: txt("Cross family"),
                intended_owner: Some(owner()),
                classification: DataClassification::Internal,
                context: ctx_corr(&execute_id, "corr-cross-family")
            })
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );
}

#[test]
fn post_prepare_request_and_decision_drift_are_preview_changed() {
    let (mut service, mut actions, _, _) = services();
    let request = open(&mut service, "request-drift");
    let prepared = service
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            statement: txt("Drift"),
            rationale: txt("Drift rationale"),
            impact: txt("Drift impact"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![],
            context: ctx("prepare-request-drift"),
        })
        .unwrap();
    service
        .withdraw_decision_request(WithdrawDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            rationale: txt("Changed after preview"),
            context: ctx("withdraw-after-preview"),
        })
        .unwrap();
    assert_eq!(
        service
            .approve_and_execute_resolve_decision_request(
                ApproveAndExecuteResolveDecisionRequest {
                    approval: approval(&prepared, "execute-request-drift"),
                    context: ctx("execute-request-drift")
                },
                &mut actions
            )
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    let first = resolve(&mut service, &mut actions, "request-decision-drift");
    let command = |id| PrepareSupersedeDecision {
        decision_id: first.decision.id().clone(),
        expected_version: first.decision.version(),
        replacement_statement: txt("Replacement"),
        replacement_rationale: txt("Replacement rationale"),
        replacement_impact: txt("Replacement impact"),
        replacement_owner: owner(),
        evidence_ids: vec![],
        judgments: vec![judgment()],
        resulting_action_requests: vec![],
        context: ctx(id),
    };
    let older = service
        .prepare_supersede_decision(command("prepare-drift-old"), &actions)
        .unwrap();
    let winner = service
        .prepare_supersede_decision(command("prepare-drift-winner"), &actions)
        .unwrap();
    service
        .approve_and_execute_supersede_decision(
            ApproveAndExecuteSupersedeDecision {
                approval: approval(&winner, "execute-drift-winner"),
                context: ctx("execute-drift-winner"),
            },
            &mut actions,
        )
        .unwrap();
    assert_eq!(
        service
            .approve_and_execute_supersede_decision(
                ApproveAndExecuteSupersedeDecision {
                    approval: approval(&older, "execute-drift-old"),
                    context: ctx("execute-drift-old")
                },
                &mut actions
            )
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
}

#[test]
fn unclassified_sources_fail_closed_and_restart_cannot_replay_pending_approval() {
    let (mut service, mut actions, _, _) = services();
    let draft = service
        .create_decision_request_draft(CreateDecisionRequestDraft {
            id: DecisionRequestId::parse("request-unclassified").unwrap(),
            subject: txt("Unclassified"),
            details: txt("Synthetic"),
            intended_owner: Some(owner()),
            classification: DataClassification::Unclassified,
            context: ctx("create-unclassified"),
        })
        .unwrap()
        .record;
    let opened = service
        .submit_decision_request(SubmitDecisionRequest {
            request_id: draft.id().clone(),
            expected_version: draft.version(),
            context: ctx("submit-unclassified"),
        })
        .unwrap()
        .record;
    assert_eq!(
        service
            .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
                request_id: opened.id().clone(),
                expected_version: opened.version(),
                statement: txt("Unclassified"),
                rationale: txt("Synthetic"),
                impact: txt("Synthetic"),
                evidence_ids: vec![],
                judgments: vec![judgment()],
                resulting_action_requests: vec![],
                context: ctx("prepare-unclassified")
            })
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPolicyDenied
    );
    let valid = open(&mut service, "request-restart");
    let prepared = service
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: valid.id().clone(),
            expected_version: valid.version(),
            statement: txt("Restart"),
            rationale: txt("Restart rationale"),
            impact: txt("Restart impact"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![],
            context: ctx("prepare-restart"),
        })
        .unwrap();
    let (mut reconstructed, _, _, _) = services();
    assert_eq!(
        reconstructed
            .approve_and_execute_resolve_decision_request(
                ApproveAndExecuteResolveDecisionRequest {
                    approval: approval(&prepared, "execute-restart"),
                    context: ctx("execute-restart")
                },
                &mut actions
            )
            .unwrap_err()
            .code(),
        ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert!(reconstructed.audit_events().is_empty());
}

#[test]
fn semantic_audits_and_audit_id_failure_are_atomic() {
    let (mut service, mut actions, control, _) = services();
    let before_action = actions.audit_events().len();
    let resolved = resolve(&mut service, &mut actions, "request-audit-inventory");
    let decision_codes = resolved
        .audit_events
        .iter()
        .map(|x| x.code().as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        decision_codes,
        vec![
            "decision_request.resolved",
            "decision.created",
            "decision_request.decision_linked"
        ]
    );
    assert_eq!(actions.audit_events().len(), before_action + 1);
    assert_eq!(
        actions.audit_events().last().unwrap().code().as_str(),
        "action_request.created_from_decision"
    );
    assert!(resolved
        .audit_events
        .iter()
        .all(|event| event.approval_outcome() == AuditApprovalOutcome::Approved));
    assert_eq!(
        actions.audit_events().last().unwrap().approval_outcome(),
        AuditApprovalOutcome::Approved
    );
    let request = open(&mut service, "request-audit-failure");
    let prepared = service
        .prepare_resolve_decision_request(PrepareResolveDecisionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            statement: txt("Audit fail"),
            rationale: txt("Audit fail rationale"),
            impact: txt("Audit fail impact"),
            evidence_ids: vec![],
            judgments: vec![judgment()],
            resulting_action_requests: vec![resulting("audit-fail-result")],
            context: ctx("prepare-audit-fail"),
        })
        .unwrap();
    let decision_count = service.audit_events().len();
    let action_count = actions.audit_events().len();
    control.set(-3);
    assert_eq!(
        service
            .approve_and_execute_resolve_decision_request(
                ApproveAndExecuteResolveDecisionRequest {
                    approval: approval(&prepared, "execute-audit-fail"),
                    context: ctx("execute-audit-fail")
                },
                &mut actions
            )
            .unwrap_err()
            .code(),
        ErrorCode::PlatformInternal
    );
    assert_eq!(
        service.request(request.id()).unwrap().state(),
        DecisionRequestState::Open
    );
    assert_eq!(service.audit_events().len(), decision_count);
    assert_eq!(actions.audit_events().len(), action_count);
    assert!(actions
        .request(&ActionRequestId::parse("audit-fail-result").unwrap())
        .is_none());
}

#[test]
fn supersede_mixed_class_graph_uses_source_formula_and_propagates_aggregate() {
    let (mut service, mut actions, _, _) = services();
    let first = resolve(&mut service, &mut actions, "request-mixed-class");
    assert_eq!(
        first.decision.classification(),
        DataClassification::Internal
    );
    let mut higher_result = resulting("mixed-class-replacement-request");
    higher_result.classification = DataClassification::Restricted;
    let prepared = service
        .prepare_supersede_decision(
            PrepareSupersedeDecision {
                decision_id: first.decision.id().clone(),
                expected_version: first.decision.version(),
                replacement_statement: txt("Higher classified replacement"),
                replacement_rationale: txt("Synthetic restricted result"),
                replacement_impact: txt("Mixed classification graph"),
                replacement_owner: owner(),
                evidence_ids: vec![],
                judgments: vec![judgment()],
                resulting_action_requests: vec![higher_result],
                context: ctx("prepare-mixed-class"),
            },
            &actions,
        )
        .unwrap();
    assert_eq!(prepared.classification(), DataClassification::Restricted);
    let outcome = service
        .approve_and_execute_supersede_decision(
            ApproveAndExecuteSupersedeDecision {
                approval: approval(&prepared, "execute-mixed-class"),
                context: ctx("execute-mixed-class"),
            },
            &mut actions,
        )
        .unwrap();
    assert_eq!(
        outcome.replacement.classification(),
        DataClassification::Restricted
    );
    assert_eq!(
        actions
            .request(&ActionRequestId::parse("mixed-class-replacement-request").unwrap())
            .unwrap()
            .classification(),
        DataClassification::Restricted
    );
    assert_eq!(
        actions
            .request(&first.resulting_action_request_ids[0])
            .unwrap()
            .classification(),
        DataClassification::Restricted
    );
}

fn prepared_resolve(s: &mut Decisions, id: &str) -> WorkManagementPreparedIntent {
    let r = open(s, id);
    s.prepare_resolve_decision_request(PrepareResolveDecisionRequest {
        request_id: r.id().clone(),
        expected_version: r.version(),
        statement: txt("Proceed with option A"),
        rationale: txt("Best synthetic tradeoff"),
        impact: txt("Improves synthetic delivery"),
        evidence_ids: vec![],
        judgments: vec![judgment()],
        resulting_action_requests: vec![resulting(&format!("action-request-{id}"))],
        context: ctx(&format!("prepare-{id}")),
    })
    .unwrap()
}

fn reject_resolve(p: &WorkManagementPreparedIntent, id: &str) -> RejectDecisionPreparedIntent {
    RejectDecisionPreparedIntent {
        prepared_id: p.id().clone(),
        actor: AuditActor::HeadOfProducts,
        context: ctx(id),
    }
}

/// v45: a rejection is a successful command with no effect.
#[test]
fn rejecting_a_pending_resolve_preview_consumes_it_without_effects_and_replays() {
    let (mut s, mut a, now, _) = services();
    let p = prepared_resolve(&mut s, "request-reject");
    now.set(150);
    let outcome = s
        .reject_decision_prepared_intent(reject_resolve(&p, "reject-1"))
        .unwrap();
    assert_eq!(outcome.prepared_intent_id(), p.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(150));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.code().as_str(), "decision.prepared_rejected");
    assert_eq!(audit.policy_outcome(), AuditPolicyOutcome::Allowed);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());

    assert_eq!(
        s.reject_decision_prepared_intent(reject_resolve(&p, "reject-1"))
            .unwrap(),
        outcome
    );
    let again = s
        .reject_decision_prepared_intent(reject_resolve(&p, "reject-2"))
        .unwrap_err();
    assert_eq!(again.code(), ErrorCode::DomainConflict);
    let executed = s
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: approval(&p, "execute-after-reject"),
                context: ctx("execute-after-reject"),
            },
            &mut a,
        )
        .unwrap_err();
    assert_eq!(executed.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    let unknown = s
        .reject_decision_prepared_intent(RejectDecisionPreparedIntent {
            prepared_id: PreparedIntentId::parse("decision-prepared-missing").unwrap(),
            actor: AuditActor::HeadOfProducts,
            context: ctx("reject-missing"),
        })
        .unwrap_err();
    assert_eq!(unknown.code(), ErrorCode::DomainNotFound);
}

#[test]
fn rejecting_after_a_resolve_executed_conflicts_and_an_expired_preview_is_still_rejectable() {
    let (mut s, mut a, now, _) = services();
    let p = prepared_resolve(&mut s, "request-executed");
    s.approve_and_execute_resolve_decision_request(
        ApproveAndExecuteResolveDecisionRequest {
            approval: approval(&p, "execute-executed"),
            context: ctx("execute-executed"),
        },
        &mut a,
    )
    .unwrap();
    let late = s
        .reject_decision_prepared_intent(reject_resolve(&p, "reject-late"))
        .unwrap_err();
    assert_eq!(late.code(), ErrorCode::DomainConflict);

    let p = prepared_resolve(&mut s, "request-expired");
    now.set(p.preview().expires_at().unix_millis() + 1);
    let outcome = s
        .reject_decision_prepared_intent(reject_resolve(&p, "reject-expired"))
        .unwrap();
    assert!(outcome.expired_at_rejection());
}
