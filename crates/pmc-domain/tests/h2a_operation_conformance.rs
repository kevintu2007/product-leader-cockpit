#![allow(clippy::result_large_err)]

//! Public H2a contract matrix for the four work-management service families.
//!
//! Lifecycle suites exercise the concrete services.  This test closes the
//! cross-family gap at the shared public prepared-intent seam: every named
//! operation has an exact operation kind, versioned target topology, effect
//! topology, expiry, classification binding, and approval digest.

use pmc_domain::actions::*;
use pmc_domain::audit::{AuditActor, AuditEffectScope};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::*;
use pmc_domain::issues::*;
use pmc_domain::time::Clock;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::*;
use pmc_domain::BoundedText;
use pmc_domain::DomainValueError;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

fn text<const N: usize>(value: &str) -> BoundedText<N> {
    BoundedText::parse(value.to_owned()).unwrap()
}

fn digest() -> IntegrityDigest {
    IntegrityDigest::parse("a".repeat(64)).unwrap()
}

fn evidence(role: EvidenceRole) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse(format!("evidence-{}", role_name(role))).unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        role,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(900),
            integrity_digest: digest(),
        },
    )
}

fn role_name(role: EvidenceRole) -> &'static str {
    match role {
        EvidenceRole::ActionCompletion => "action-completion",
        EvidenceRole::DecisionResolution => "decision-resolution",
        EvidenceRole::IssueResolution => "issue-resolution",
        EvidenceRole::IssueClosureVerification => "issue-closure-verification",
        EvidenceRole::IssueFailedVerification => "issue-failed-verification",
    }
}

fn support(role: EvidenceRole) -> SupportWitness {
    EvidenceOrJudgment::new(vec![evidence(role)], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap()
}

fn decision_support() -> SupportWitness {
    EvidenceOrJudgment::new(vec![evidence(EvidenceRole::DecisionResolution)], vec![])
        .unwrap()
        .evaluate_evidence_or_judgment()
        .unwrap()
}

#[allow(clippy::type_complexity)]
fn ids() -> (
    ActionRequestId,
    ActionId,
    DecisionRequestId,
    DecisionId,
    RiskId,
    IssueId,
    PortfolioId,
    ProductId,
    RoadmapId,
    KpiId,
    KpiObservationId,
) {
    (
        ActionRequestId::parse("request-h2a").unwrap(),
        ActionId::parse("action-h2a").unwrap(),
        DecisionRequestId::parse("decision-request-h2a").unwrap(),
        DecisionId::parse("decision-h2a").unwrap(),
        RiskId::parse("risk-h2a").unwrap(),
        IssueId::parse("issue-h2a").unwrap(),
        PortfolioId::parse("portfolio-h2a").unwrap(),
        ProductId::parse("product-h2a").unwrap(),
        RoadmapId::parse("roadmap-h2a").unwrap(),
        KpiId::parse("kpi-h2a").unwrap(),
        KpiObservationId::parse("observation-h2a").unwrap(),
    )
}

fn operations() -> Vec<(
    WorkManagementH2aIntentKind,
    WorkManagementOperation,
    Option<SupportWitness>,
)> {
    let (
        request,
        action,
        decision_request,
        decision,
        risk,
        issue,
        portfolio,
        product,
        roadmap,
        kpi,
        observation,
    ) = ids();
    let owner = StakeholderId::parse("owner-h2a").unwrap();
    let at = UtcTimestamp::from_unix_millis(2_000);
    vec![
        (
            WorkManagementH2aIntentKind::AcceptActionRequest,
            WorkManagementOperation::AcceptActionRequest {
                request_id: request,
                request_version: AggregateVersion::initial(),
                action_id: action.clone(),
                action_classification: DataClassification::Internal,
                action_subject: text("Synthetic action"),
                commitment_details: text("Synthetic commitment details"),
                intended_owner: owner.clone(),
                intended_due_at: at,
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::ResolveDecisionRequest,
            WorkManagementOperation::ResolveDecisionRequest {
                request_id: decision_request,
                request_version: AggregateVersion::initial(),
                decision_id: decision.clone(),
                decision_classification: DataClassification::Internal,
                statement: text("Synthetic decision statement"),
                rationale: text("Synthetic rationale"),
                impact: text("Synthetic impact"),
                decision_owner: owner.clone(),
                decided_at: at,
                resulting_action_requests: vec![],
            },
            Some(decision_support()),
        ),
        (
            WorkManagementH2aIntentKind::CompleteAction,
            WorkManagementOperation::CompleteAction {
                action_id: action.clone(),
                action_version: AggregateVersion::initial(),
            },
            Some(support(EvidenceRole::ActionCompletion)),
        ),
        (
            WorkManagementH2aIntentKind::CancelAction,
            WorkManagementOperation::CancelAction {
                action_id: action.clone(),
                action_version: AggregateVersion::initial(),
                reason: text("Synthetic cancellation rationale"),
                evidence_classifications: vec![],
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::ReopenAction,
            WorkManagementOperation::ReopenAction {
                action_id: action.clone(),
                action_version: AggregateVersion::initial(),
                mode: ActionReopenMode::RestartCancelled,
                reason: text("Synthetic reopen rationale"),
                evidence_classifications: vec![],
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::SupersedeDecision,
            WorkManagementOperation::SupersedeDecision {
                decision_id: decision.clone(),
                decision_version: AggregateVersion::initial(),
                replacement_decision_id: DecisionId::parse("replacement-h2a").unwrap(),
                replacement_decision_version: AggregateVersion::initial(),
                replacement_decision_classification: DataClassification::Internal,
                replacement_statement: text("Synthetic replacement statement"),
                replacement_rationale: text("Synthetic replacement rationale"),
                replacement_impact: text("Synthetic replacement impact"),
                replacement_owner: owner.clone(),
                replacement_decided_at: at,
                resulting_action_requests: vec![],
                incomplete_downstream: vec![],
            },
            Some(decision_support()),
        ),
        (
            WorkManagementH2aIntentKind::RecordRiskOccurrence,
            WorkManagementOperation::RecordRiskOccurrence {
                risk_id: risk.clone(),
                risk_version: AggregateVersion::initial(),
                issue_id: issue.clone(),
                issue_classification: DataClassification::Internal,
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::CloseRisk,
            WorkManagementOperation::CloseRisk {
                risk_id: risk.clone(),
                risk_version: AggregateVersion::initial(),
                rationale: text("Synthetic risk closure rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::ResolveIssue,
            WorkManagementOperation::ResolveIssue {
                issue_id: issue.clone(),
                issue_version: AggregateVersion::initial(),
                resolution_type: IssueResolutionType::Resolved,
                rationale: text("Synthetic issue resolution rationale"),
            },
            Some(support(EvidenceRole::IssueResolution)),
        ),
        (
            WorkManagementH2aIntentKind::CloseIssue,
            WorkManagementOperation::CloseIssue {
                issue_id: issue.clone(),
                issue_version: AggregateVersion::initial(),
            },
            Some(support(EvidenceRole::IssueClosureVerification)),
        ),
        (
            WorkManagementH2aIntentKind::ReopenIssue,
            WorkManagementOperation::ReopenIssue {
                issue_id: issue.clone(),
                issue_version: AggregateVersion::initial(),
                rationale: text("Synthetic issue reopen rationale"),
            },
            Some(support(EvidenceRole::IssueFailedVerification)),
        ),
        (
            WorkManagementH2aIntentKind::LowerPortfolioClassification,
            WorkManagementOperation::LowerPortfolioClassification {
                portfolio_id: portfolio,
                portfolio_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerProductClassification,
            WorkManagementOperation::LowerProductClassification {
                product_id: product,
                product_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerRoadmapClassification,
            WorkManagementOperation::LowerRoadmapClassification {
                roadmap_id: roadmap,
                roadmap_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerKpiClassification,
            WorkManagementOperation::LowerKpiClassification {
                kpi_id: kpi,
                kpi_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerKpiObservationClassification,
            WorkManagementOperation::LowerKpiObservationClassification {
                observation_id: observation,
                observation_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerActionClassification,
            WorkManagementOperation::LowerActionClassification {
                action_id: action,
                action_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerRiskClassification,
            WorkManagementOperation::LowerRiskClassification {
                risk_id: risk,
                risk_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerDecisionClassification,
            WorkManagementOperation::LowerDecisionClassification {
                decision_id: decision,
                decision_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerIssueClassification,
            WorkManagementOperation::LowerIssueClassification {
                issue_id: issue,
                issue_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerInitiativeClassification,
            WorkManagementOperation::LowerInitiativeClassification {
                initiative_id: InitiativeId::parse("initiative-h2a").unwrap(),
                initiative_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerProjectClassification,
            WorkManagementOperation::LowerProjectClassification {
                project_id: ProjectId::parse("project-h2a").unwrap(),
                project_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
        (
            WorkManagementH2aIntentKind::LowerMilestoneClassification,
            WorkManagementOperation::LowerMilestoneClassification {
                milestone_id: MilestoneId::parse("milestone-h2a").unwrap(),
                milestone_version: AggregateVersion::initial(),
                current_classification: DataClassification::Restricted,
                proposed_classification: DataClassification::Internal,
                rationale: text("Synthetic classification lowering rationale"),
            },
            None,
        ),
    ]
}

fn operation_kind(operation: &WorkManagementOperation) -> WorkManagementH2aIntentKind {
    match operation {
        WorkManagementOperation::AcceptActionRequest { .. } => {
            WorkManagementH2aIntentKind::AcceptActionRequest
        }
        WorkManagementOperation::ResolveDecisionRequest { .. } => {
            WorkManagementH2aIntentKind::ResolveDecisionRequest
        }
        WorkManagementOperation::CompleteAction { .. } => {
            WorkManagementH2aIntentKind::CompleteAction
        }
        WorkManagementOperation::CancelAction { .. } => WorkManagementH2aIntentKind::CancelAction,
        WorkManagementOperation::ReopenAction { .. } => WorkManagementH2aIntentKind::ReopenAction,
        WorkManagementOperation::SupersedeDecision { .. } => {
            WorkManagementH2aIntentKind::SupersedeDecision
        }
        WorkManagementOperation::RecordRiskOccurrence { .. } => {
            WorkManagementH2aIntentKind::RecordRiskOccurrence
        }
        WorkManagementOperation::CloseRisk { .. } => WorkManagementH2aIntentKind::CloseRisk,
        WorkManagementOperation::ResolveIssue { .. } => WorkManagementH2aIntentKind::ResolveIssue,
        WorkManagementOperation::CloseIssue { .. } => WorkManagementH2aIntentKind::CloseIssue,
        WorkManagementOperation::ReopenIssue { .. } => WorkManagementH2aIntentKind::ReopenIssue,
        WorkManagementOperation::LowerPortfolioClassification { .. } => {
            WorkManagementH2aIntentKind::LowerPortfolioClassification
        }
        WorkManagementOperation::LowerProductClassification { .. } => {
            WorkManagementH2aIntentKind::LowerProductClassification
        }
        WorkManagementOperation::LowerRoadmapClassification { .. } => {
            WorkManagementH2aIntentKind::LowerRoadmapClassification
        }
        WorkManagementOperation::LowerKpiClassification { .. } => {
            WorkManagementH2aIntentKind::LowerKpiClassification
        }
        WorkManagementOperation::LowerKpiObservationClassification { .. } => {
            WorkManagementH2aIntentKind::LowerKpiObservationClassification
        }
        WorkManagementOperation::LowerActionClassification { .. } => {
            WorkManagementH2aIntentKind::LowerActionClassification
        }
        WorkManagementOperation::LowerDecisionClassification { .. } => {
            WorkManagementH2aIntentKind::LowerDecisionClassification
        }
        WorkManagementOperation::LowerRiskClassification { .. } => {
            WorkManagementH2aIntentKind::LowerRiskClassification
        }
        WorkManagementOperation::LowerIssueClassification { .. } => {
            WorkManagementH2aIntentKind::LowerIssueClassification
        }
        WorkManagementOperation::LowerInitiativeClassification { .. } => {
            WorkManagementH2aIntentKind::LowerInitiativeClassification
        }
        WorkManagementOperation::LowerProjectClassification { .. } => {
            WorkManagementH2aIntentKind::LowerProjectClassification
        }
        WorkManagementOperation::LowerMilestoneClassification { .. } => {
            WorkManagementH2aIntentKind::LowerMilestoneClassification
        }
    }
}

#[test]
fn every_named_h2a_operation_prepares_a_versioned_exact_public_preview() {
    let cases = operations();
    assert_eq!(cases.len(), WorkManagementH2aIntentKind::ALL.len());
    for (index, (kind, operation, witness)) in cases.into_iter().enumerate() {
        let prepared = WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse(format!("prepared-h2a-{index}")).unwrap(),
            operation,
            DataClassification::Internal,
            witness,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
        assert_eq!(operation_kind(prepared.operation()), kind);
        assert_eq!(prepared.preview().contract_version(), 1);
        assert_eq!(
            prepared.authority(),
            WorkManagementAuthority::HeadOfProducts
        );
        assert_eq!(
            prepared.cancellation_policy(),
            WorkManagementCancellationPolicy::NotCancellableAfterSubmit
        );
        assert_eq!(
            prepared.preview().expires_at(),
            UtcTimestamp::from_unix_millis(1_000 + WORK_MANAGEMENT_H2A_TTL_MILLIS)
        );
        assert_eq!(prepared.classification(), DataClassification::Internal);
        assert_eq!(
            prepared.payload_digest(),
            &prepared.preview().payload_digest()
        );
        assert!(!prepared.payload_digest().as_str().is_empty());
    }
}

#[test]
fn operation_inventory_round_trips_without_collapsing_family_boundaries() {
    let persisted: Vec<_> = WorkManagementH2aIntentKind::ALL
        .iter()
        .map(|kind| kind.as_persisted())
        .collect();
    assert_eq!(
        persisted,
        vec![
            "accept_action_request",
            "resolve_decision_request",
            "complete_action",
            "cancel_action",
            "reopen_action",
            "supersede_decision",
            "record_risk_occurrence",
            "close_risk",
            "resolve_issue",
            "close_issue",
            "reopen_issue",
            "lower_portfolio_classification",
            "lower_product_classification",
            "lower_roadmap_classification",
            "lower_kpi_classification",
            "lower_kpi_observation_classification",
            "lower_action_classification",
            "lower_decision_classification",
            "lower_risk_classification",
            "lower_issue_classification",
            "lower_initiative_classification",
            "lower_project_classification",
            "lower_milestone_classification",
        ]
    );
    for kind in WorkManagementH2aIntentKind::ALL {
        assert_eq!(
            WorkManagementH2aIntentKind::from_persisted(kind.as_persisted()),
            Ok(kind)
        );
    }

    let families = operations()
        .into_iter()
        .map(|(_, operation, _)| match operation {
            WorkManagementOperation::AcceptActionRequest { .. }
            | WorkManagementOperation::CompleteAction { .. }
            | WorkManagementOperation::CancelAction { .. }
            | WorkManagementOperation::ReopenAction { .. } => "action",
            WorkManagementOperation::ResolveDecisionRequest { .. }
            | WorkManagementOperation::SupersedeDecision { .. } => "decision",
            WorkManagementOperation::RecordRiskOccurrence { .. }
            | WorkManagementOperation::CloseRisk { .. } => "risk",
            WorkManagementOperation::ResolveIssue { .. }
            | WorkManagementOperation::CloseIssue { .. }
            | WorkManagementOperation::ReopenIssue { .. } => "issue",
            WorkManagementOperation::LowerPortfolioClassification { .. }
            | WorkManagementOperation::LowerProductClassification { .. }
            | WorkManagementOperation::LowerRoadmapClassification { .. }
            | WorkManagementOperation::LowerKpiClassification { .. }
            | WorkManagementOperation::LowerKpiObservationClassification { .. } => "portfolio",
            WorkManagementOperation::LowerActionClassification { .. } => "action",
            WorkManagementOperation::LowerRiskClassification { .. } => "risk",
            WorkManagementOperation::LowerDecisionClassification { .. } => "decision",
            WorkManagementOperation::LowerIssueClassification { .. } => "issue",
            WorkManagementOperation::LowerInitiativeClassification { .. }
            | WorkManagementOperation::LowerProjectClassification { .. }
            | WorkManagementOperation::LowerMilestoneClassification { .. } => "delivery",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        families,
        vec![
            "action",
            "decision",
            "action",
            "action",
            "action",
            "decision",
            "risk",
            "risk",
            "issue",
            "issue",
            "issue",
            "portfolio",
            "portfolio",
            "portfolio",
            "portfolio",
            "portfolio",
            "action",
            "risk",
            "decision",
            "issue",
            "delivery",
            "delivery",
            "delivery"
        ]
    );
}

#[test]
fn public_preparation_rejects_unsafe_topology_and_unclassified_bindings_without_effect() {
    let (_, _, _, decision, _, issue, _, _, _, _, _) = ids();
    let duplicate = ActionRequestId::parse("duplicate-h2a").unwrap();
    let request = DecisionResultingActionRequest {
        id: duplicate.clone(),
        subject: text("Synthetic resulting request"),
        details: text("Synthetic resulting details"),
        intended_owner: StakeholderId::parse("owner-h2a").unwrap(),
        due_at: UtcTimestamp::from_unix_millis(2_000),
        classification: DataClassification::Internal,
    };
    let invalid = WorkManagementOperation::ResolveDecisionRequest {
        request_id: DecisionRequestId::parse("request-invalid").unwrap(),
        request_version: AggregateVersion::initial(),
        decision_id: decision,
        decision_classification: DataClassification::Internal,
        statement: text("Synthetic statement"),
        rationale: text("Synthetic rationale"),
        impact: text("Synthetic impact"),
        decision_owner: StakeholderId::parse("owner-h2a").unwrap(),
        decided_at: UtcTimestamp::from_unix_millis(2_000),
        resulting_action_requests: vec![request.clone(), request],
    };
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("prepared-invalid-topology").unwrap(),
            invalid,
            DataClassification::Internal,
            Some(decision_support()),
            UtcTimestamp::from_unix_millis(1_000),
        ),
        Err(PreparedIntentError::InvalidTopology)
    );

    let unclassified = WorkManagementOperation::CloseIssue {
        issue_id: issue,
        issue_version: AggregateVersion::initial(),
    };
    assert_eq!(
        WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("prepared-unclassified").unwrap(),
            unclassified,
            DataClassification::Unclassified,
            Some(support(EvidenceRole::IssueClosureVerification)),
            UtcTimestamp::from_unix_millis(1_000),
        ),
        Err(PreparedIntentError::UnclassifiedBinding)
    );
}

#[test]
fn approval_constructor_requires_head_of_products_confirmation() {
    let approval = WorkManagementApproval::new(
        PreparedIntentId::parse("prepared-approval").unwrap(),
        pmc_domain::audit::AuditActor::HeadOfProducts,
        WorkManagementPayloadDigest::from_public_test_digest(),
        IdempotencyId::parse("idem-approval").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    assert_eq!(
        approval.actor(),
        pmc_domain::audit::AuditActor::HeadOfProducts
    );
    assert_eq!(approval.prepared_id().as_str(), "prepared-approval");
}

#[derive(Clone)]
struct ActionTestClock(Rc<Cell<i64>>);
impl Clock for ActionTestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

struct ActionTestIds(u64);
impl ActionServiceIdSource for ActionTestIds {
    fn next_action_id(&mut self) -> Result<ActionId, DomainValueError> {
        self.0 += 1;
        ActionId::parse(format!("action-test-{}", self.0))
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-action-test-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-action-test-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-action-test-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct ActionTestAuth;
impl ApprovalAuthorizationPort for ActionTestAuth {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

#[derive(Clone, Copy)]
struct ActionTestPolicy;
impl ActionExecutionPolicyPort for ActionTestPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}

#[derive(Clone, Default)]
struct ActionTestEvidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl ActionEvidenceAuthorityPort for ActionTestEvidence {
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

type ActionTestService = InMemoryActionService<
    ActionTestClock,
    ActionTestIds,
    ActionTestAuth,
    ActionTestPolicy,
    ActionTestEvidence,
>;

fn action_context(key: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(key).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{key}")).unwrap(),
    }
}

fn action_fixture() -> (ActionTestService, ActionId) {
    let now = Rc::new(Cell::new(1_000));
    let evidence = ActionTestEvidence::default();
    let mut service = InMemoryActionService::new(
        ActionTestClock(now),
        ActionTestIds(0),
        ActionTestAuth,
        ActionTestPolicy,
        evidence.clone(),
    );
    let request = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-action-test").unwrap(),
            title: text("Synthetic action request"),
            details: text("Synthetic action details"),
            intended_owner: Some(StakeholderId::parse("owner-action-test").unwrap()),
            response_due_at: Some(UtcTimestamp::from_unix_millis(2_000)),
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(3_000)),
            classification: DataClassification::Internal,
            context: action_context("action-create"),
        })
        .unwrap()
        .record;
    let request = service
        .submit_action_request(SubmitActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: action_context("action-submit"),
        })
        .unwrap()
        .record;
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: action_context("action-accept-prepare"),
        })
        .unwrap();
    let accepted = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval: action_approval(&prepared, "action-accept"),
            context: action_context("action-accept"),
        })
        .unwrap();
    let started = service
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: action_context("action-start"),
        })
        .unwrap()
        .record;
    let evidence_id = EvidenceReferenceId::parse("evidence-action-service").unwrap();
    evidence.0.borrow_mut().insert(
        evidence_id.clone(),
        EvidenceReferenceMetadata::new(
            evidence_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::ActionCompletion,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(900),
                integrity_digest: digest(),
            },
        ),
    );
    service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id,
            context: action_context("action-evidence"),
        })
        .unwrap();
    (service, started.id().clone())
}

fn action_approval(prepared: &WorkManagementPreparedIntent, key: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

#[test]
fn action_complete_mismatched_acknowledged_digest_is_safe_and_effect_free() {
    let (mut service, action_id) = action_fixture();
    let action = service.action(&action_id).unwrap().clone();
    let prepared = service
        .prepare_complete_action(PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: action.version(),
            judgment: None,
            context: action_context("action-complete-prepare"),
        })
        .unwrap();
    let wrong_digest = operations()
        .into_iter()
        .next()
        .map(|(_, operation, witness)| {
            WorkManagementPreparedIntent::prepare(
                PreparedIntentId::parse("wrong-action-digest").unwrap(),
                operation,
                DataClassification::Internal,
                witness,
                UtcTimestamp::from_unix_millis(1_000),
            )
            .unwrap()
            .payload_digest()
            .clone()
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        wrong_digest,
        IdempotencyId::parse("action-complete-wrong-digest").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let error = service
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval,
            context: action_context("action-complete-wrong-digest"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(service.action(&action_id).unwrap(), &action);
    assert_eq!(
        service.audit_events().last().unwrap().effect_scope(),
        AuditEffectScope::None
    );
}

#[derive(Clone)]
struct IssueTestClock(Rc<Cell<i64>>);
impl Clock for IssueTestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}

struct IssueTestIds(u64);
impl IssueServiceIdSource for IssueTestIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-issue-test-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-issue-test-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-issue-test-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct IssueTestGate;
impl ApprovalAuthorizationPort for IssueTestGate {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
impl IssueExecutionPolicyPort for IssueTestGate {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

#[derive(Clone, Default)]
struct IssueTestEvidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl IssueEvidenceAuthorityPort for IssueTestEvidence {
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
struct IssueTestClassification;
impl IssueClassificationAuthorityPort for IssueTestClassification {
    fn current_classification(
        &self,
        _: &IssueId,
        classification: DataClassification,
    ) -> Result<DataClassification, IssueEvidenceAuthorityError> {
        Ok(classification)
    }
}

type IssueTestService = InMemoryIssueService<
    IssueTestClock,
    IssueTestIds,
    IssueTestGate,
    IssueTestGate,
    IssueTestEvidence,
    IssueTestClassification,
>;

struct IssueFixture {
    service: IssueTestService,
    evidence: IssueTestEvidence,
    now: Rc<Cell<i64>>,
}

fn issue_context(key: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(key).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{key}")).unwrap(),
    }
}

fn issue_fixture() -> IssueFixture {
    let now = Rc::new(Cell::new(1_000));
    let evidence = IssueTestEvidence::default();
    IssueFixture {
        service: InMemoryIssueService::new(
            IssueTestClock(now.clone()),
            IssueTestIds(0),
            IssueTestGate,
            IssueTestGate,
            evidence.clone(),
            IssueTestClassification,
        ),
        evidence,
        now,
    }
}

fn issue_add_evidence(fixture: &IssueFixture, id: &str, role: EvidenceRole) {
    let evidence_id = EvidenceReferenceId::parse(id).unwrap();
    fixture.evidence.0.borrow_mut().insert(
        evidence_id.clone(),
        EvidenceReferenceMetadata::new(
            evidence_id,
            AggregateVersion::initial(),
            DataClassification::Internal,
            role,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(900),
                integrity_digest: digest(),
            },
        ),
    );
}

fn issue_approval(prepared: &WorkManagementPreparedIntent, key: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

fn create_test_issue(fixture: &mut IssueFixture) -> IssueRecord {
    fixture
        .service
        .create_issue(CreateIssue {
            id: IssueId::parse("issue-service-test").unwrap(),
            title: text("Synthetic issue"),
            details: text("Synthetic observed condition"),
            classification: DataClassification::Internal,
            recurrence_of: None,
            context: issue_context("issue-create"),
        })
        .unwrap()
        .record
}

fn prepare_issue_resolve(
    fixture: &mut IssueFixture,
    issue: &IssueRecord,
    prepare_key: &str,
) -> WorkManagementPreparedIntent {
    fixture
        .service
        .prepare_resolve_issue(PrepareResolveIssue {
            issue_id: issue.id().clone(),
            expected_version: issue.version(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: text("Synthetic resolution rationale"),
            evidence_ids: vec![EvidenceReferenceId::parse("evidence-issue-service").unwrap()],
            judgment: None,
            context: issue_context(prepare_key),
        })
        .unwrap()
}

#[test]
fn issue_resolve_mismatched_digest_is_effect_free_and_expiry_is_exact() {
    let mut fixture = issue_fixture();
    issue_add_evidence(
        &fixture,
        "evidence-issue-service",
        EvidenceRole::IssueResolution,
    );
    let issue = create_test_issue(&mut fixture);
    let prepared = prepare_issue_resolve(&mut fixture, &issue, "issue-resolve-prepare");
    let wrong_digest = operations()
        .into_iter()
        .next()
        .map(|(_, operation, witness)| {
            WorkManagementPreparedIntent::prepare(
                PreparedIntentId::parse("wrong-issue-digest").unwrap(),
                operation,
                DataClassification::Internal,
                witness,
                UtcTimestamp::from_unix_millis(1_000),
            )
            .unwrap()
            .payload_digest()
            .clone()
        })
        .unwrap();
    let mismatch = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        wrong_digest,
        IdempotencyId::parse("issue-wrong-digest").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let error = fixture
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: mismatch,
            context: issue_context("issue-wrong-digest"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(fixture.service.issue(issue.id()).unwrap(), issue);
    assert_eq!(
        fixture
            .service
            .audit_events()
            .last()
            .unwrap()
            .effect_scope(),
        AuditEffectScope::None
    );

    let prepared = prepare_issue_resolve(&mut fixture, &issue, "issue-expiry-prepare");
    let before = fixture.service.issue(issue.id()).unwrap().clone();
    fixture
        .now
        .set(prepared.preview().expires_at().unix_millis());
    let error = fixture
        .service
        .approve_and_execute(ApproveAndExecuteIssueTransition {
            approval: issue_approval(&prepared, "issue-expired"),
            context: issue_context("issue-expired"),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::SecurityPreviewExpiredOrChanged
    );
    assert_eq!(fixture.service.issue(issue.id()).unwrap(), before);
    assert_eq!(
        fixture
            .service
            .audit_events()
            .last()
            .unwrap()
            .effect_scope(),
        AuditEffectScope::None
    );
}

#[test]
fn issue_resolve_exact_replay_returns_original_without_duplicate_authority() {
    let mut fixture = issue_fixture();
    issue_add_evidence(
        &fixture,
        "evidence-issue-service",
        EvidenceRole::IssueResolution,
    );
    let issue = create_test_issue(&mut fixture);
    let prepared = prepare_issue_resolve(&mut fixture, &issue, "issue-replay-prepare");
    let command = ApproveAndExecuteIssueTransition {
        approval: issue_approval(&prepared, "issue-replay"),
        context: issue_context("issue-replay"),
    };
    let outcome = fixture
        .service
        .approve_and_execute(command.clone())
        .unwrap();
    let audit_count = fixture.service.audit_events().len();
    let replay = fixture.service.approve_and_execute(command).unwrap();
    assert_eq!(replay, outcome);
    assert_eq!(fixture.service.audit_events().len(), audit_count);
    assert_eq!(fixture.service.issue(issue.id()).unwrap(), outcome.record);
    assert_eq!(
        fixture.service.issue(issue.id()).unwrap().version(),
        outcome.record.version()
    );
    assert_eq!(
        fixture.service.issue(issue.id()).unwrap().state(),
        IssueState::Resolved
    );
}

// This helper deliberately stays in the test boundary: the production digest
// type has no arbitrary constructor, and approval tests only need a synthetic
// acknowledged value to exercise actor/confirmation validation.
trait PublicTestDigest {
    fn from_public_test_digest() -> Self;
}

impl PublicTestDigest for WorkManagementPayloadDigest {
    fn from_public_test_digest() -> Self {
        // A prepared preview supplies the canonical public digest; use its
        // value here instead of adding a production-only test constructor.
        let (_, operation, witness) = operations().into_iter().next().unwrap();
        WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("digest-source").unwrap(),
            operation,
            DataClassification::Internal,
            witness,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap()
        .payload_digest()
        .clone()
    }
}
