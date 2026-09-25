#![allow(clippy::result_large_err)]

//! Cross-family H2 conformance at the public domain seams.
//!
//! This matrix intentionally compares the Action Complete H2a operation with
//! relationship removal H2b.  The fixtures contain synthetic data only.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use pmc_domain::actions::*;
use pmc_domain::audit::{AuditActor, AuditEffectScope, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::execution::*;
use pmc_domain::identity::*;
use pmc_domain::relationships::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    ActionState, ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceReferenceMetadata,
    EvidenceRole, EvidenceVerification, IntegrityDigest, WorkManagementApproval,
    WorkManagementOperation,
};
use pmc_domain::DomainValueError;

#[derive(Clone)]
struct ActionClock(Rc<Cell<i64>>);
impl Clock for ActionClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(self.0.get())
    }
}
struct ActionIds {
    next: u64,
}
impl ActionServiceIdSource for ActionIds {
    fn next_action_id(&mut self) -> Result<ActionId, DomainValueError> {
        self.next += 1;
        ActionId::parse(format!("action-{}", self.next))
    }
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.next += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.next))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.next += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.next))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.next += 1;
        AuditEventId::parse(format!("audit-{}", self.next))
    }
}
#[derive(Clone)]
struct ActionAuth;
impl ApprovalAuthorizationPort for ActionAuth {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
#[derive(Clone)]
struct ActionPolicy;
impl ActionExecutionPolicyPort for ActionPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}
#[derive(Clone, Default)]
struct ActionEvidence(Rc<RefCell<HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>>>);
impl ActionEvidenceAuthorityPort for ActionEvidence {
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
type ActionService =
    InMemoryActionService<ActionClock, ActionIds, ActionAuth, ActionPolicy, ActionEvidence>;

fn action_context(key: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(key).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{key}")).unwrap(),
    }
}
fn title(value: &str) -> pmc_domain::BoundedText<240> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn details(value: &str) -> pmc_domain::BoundedText<2_000> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}
fn action_fixture() -> (ActionService, ActionId, Rc<Cell<i64>>) {
    let now = Rc::new(Cell::new(1_000));
    let evidence = ActionEvidence::default();
    let mut service = InMemoryActionService::new(
        ActionClock(now.clone()),
        ActionIds { next: 0 },
        ActionAuth,
        ActionPolicy,
        evidence.clone(),
    );
    let request = service
        .create_action_request_draft(CreateActionRequestDraft {
            id: ActionRequestId::parse("request-conformance").unwrap(),
            title: title("Synthetic action"),
            details: details("Synthetic details"),
            intended_owner: Some(StakeholderId::parse("owner-synthetic").unwrap()),
            response_due_at: Some(UtcTimestamp::from_unix_millis(2_000)),
            intended_action_due_at: Some(UtcTimestamp::from_unix_millis(3_000)),
            classification: DataClassification::Internal,
            context: action_context("create-conformance"),
        })
        .unwrap()
        .record;
    let request = service
        .submit_action_request(SubmitActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: action_context("submit-conformance"),
        })
        .unwrap()
        .record;
    let prepared = service
        .prepare_accept_action_request(PrepareAcceptActionRequest {
            request_id: request.id().clone(),
            expected_version: request.version(),
            context: action_context("prepare-accept-conformance"),
        })
        .unwrap();
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("accept-conformance").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let accepted = service
        .approve_and_execute_accept_action_request(ApproveAndExecuteAcceptActionRequest {
            approval,
            context: action_context("accept-conformance"),
        })
        .unwrap();
    let started = service
        .start_action(StartAction {
            action_id: accepted.action.id().clone(),
            expected_version: accepted.action.version(),
            context: action_context("start-conformance"),
        })
        .unwrap()
        .record;
    let evidence_id = EvidenceReferenceId::parse("evidence-conformance").unwrap();
    evidence.0.borrow_mut().insert(
        evidence_id.clone(),
        EvidenceReferenceMetadata::new(
            evidence_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::ActionCompletion,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(900),
                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
            },
        ),
    );
    let linked = service
        .link_action_completion_evidence(LinkActionCompletionEvidence {
            action_id: started.id().clone(),
            expected_version: started.version(),
            evidence_id,
            context: action_context("link-conformance"),
        })
        .unwrap()
        .record;
    (service, linked.id().clone(), now)
}
fn action_approval(
    prepared: &pmc_domain::work_management::WorkManagementPreparedIntent,
    key: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(key).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

struct RelationshipAuditIds(u64);
impl AuditEventIdSource for RelationshipAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}
#[derive(Clone)]
struct RelationshipIds(u64);
impl ExecutionIdSource for RelationshipIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
}
#[derive(Clone, Copy)]
struct RelationshipPolicy;
impl RemovalPolicyPort for RelationshipPolicy {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}
#[derive(Clone, Copy)]
struct RelationshipAuth;
impl pmc_domain::execution::ApprovalAuthorizationPort for RelationshipAuth {
    fn authorize_relationship_removal(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
#[derive(Clone)]
struct Recovery(Option<RecoveryEvidence>);
impl RecoveryEvidencePort for Recovery {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        self.0.clone()
    }
}
type RelationshipService = InMemoryRelationshipService<
    RelationshipClock,
    InMemoryEndpointCatalog,
    RelationshipAuditIds,
    RelationshipIds,
    RelationshipPolicy,
    RelationshipAuth,
>;
#[derive(Clone)]
struct RelationshipClock(Arc<Mutex<i64>>);
impl Clock for RelationshipClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(*self.0.lock().unwrap())
    }
}
fn relationship_context(key: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(key).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{key}")).unwrap(),
    }
}
fn relationship_fixture() -> (RelationshipService, RelationshipId, Arc<Mutex<i64>>) {
    let now = Arc::new(Mutex::new(1_000));
    let mut service = InMemoryRelationshipService::with_execution_authorities(
        RelationshipClock(now.clone()),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-conformance").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-conformance").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        RelationshipAuditIds(0),
        RelationshipIds(10),
        RelationshipPolicy,
        RelationshipAuth,
    );
    let id = RelationshipId::parse("relationship-conformance").unwrap();
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-conformance").unwrap(),
            product_id: ProductId::parse("product-conformance").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: relationship_context("link-conformance"),
        })
        .unwrap();
    (service, id, now)
}
fn recovery(id: &RelationshipId) -> Recovery {
    Recovery(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-conformance").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            id.clone(),
            true,
        )
        .unwrap(),
    ))
}
fn relationship_approval(
    prepared: &PreparedIntent,
    key: &str,
) -> ApproveAndExecuteRemoveRelationship {
    ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: relationship_context(key),
    }
}

#[test]
fn common_h2_contract_binds_digest_expiry_and_discard_without_partial_effect() {
    let (mut actions, action_id, now) = action_fixture();
    let action = actions.action(&action_id).unwrap().clone();
    let prepared = actions
        .prepare_complete_action(PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: action.version(),
            judgment: None,
            context: action_context("prepare-complete"),
        })
        .unwrap();
    assert_eq!(prepared.preview().contract_version(), 1);
    assert!(prepared.preview().expires_at().unix_millis() > 1_000);
    assert!(matches!(
        prepared.operation(),
        WorkManagementOperation::CompleteAction { .. }
    ));
    let before = actions.action(&action_id).unwrap().clone();
    let audit_count = actions.audit_events().len();
    now.set(prepared.preview().expires_at().unix_millis());
    let expired = actions
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: action_approval(&prepared, "complete-expired"),
            context: action_context("complete-expired"),
        })
        .unwrap_err();
    assert_eq!(expired.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(actions.action(&action_id).unwrap(), &before);
    assert!(actions.audit_events().len() >= audit_count);
    assert_eq!(
        actions.audit_events().last().unwrap().effect_scope(),
        AuditEffectScope::None
    );

    let (mut actions, action_id, _) = action_fixture();
    let action = actions.action(&action_id).unwrap().clone();
    let prepared = actions
        .prepare_complete_action(PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: action.version(),
            judgment: None,
            context: action_context("prepare-discard"),
        })
        .unwrap();
    let before = actions.action(&action_id).unwrap().clone();
    assert!(actions.discard_prepared_intent(prepared.id()));
    assert!(actions
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: action_approval(&prepared, "complete-discarded"),
            context: action_context("complete-discarded")
        })
        .is_err());
    assert_eq!(actions.action(&action_id).unwrap(), &before);

    let (mut relationships, relationship_id, _) = relationship_fixture();
    let prepared = relationships
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("prepare-remove"),
            },
            &recovery(&relationship_id),
        )
        .unwrap();
    assert!(prepared.expires_at().unix_millis() > 1_000);
    let mut bad = relationship_approval(&prepared, "remove-bad-digest");
    bad.acknowledged_payload_digest = PayloadDigest::parse("0".repeat(64)).unwrap();
    assert!(relationships
        .approve_and_execute_remove_relationship(bad, &recovery(&relationship_id))
        .is_err());
    assert_eq!(relationships.relationship_count(), 1);
    relationships
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            relationship_context("remove-cancel"),
        )
        .unwrap();
    assert!(relationships
        .approve_and_execute_remove_relationship(
            relationship_approval(&prepared, "remove-cancelled"),
            &recovery(&relationship_id)
        )
        .is_err());
    assert_eq!(relationships.relationship_count(), 1);
}

#[test]
fn h2b_requires_named_confirmation_and_verified_recovery_beyond_h2a() {
    let (mut relationships, relationship_id, _) = relationship_fixture();
    let prepared = relationships
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("prepare-named"),
            },
            &recovery(&relationship_id),
        )
        .unwrap();
    let mut wrong = relationship_approval(&prepared, "wrong-confirmation");
    wrong.confirmation = "REMOVE another-relationship".into();
    assert!(relationships
        .approve_and_execute_remove_relationship(wrong, &recovery(&relationship_id))
        .is_err());
    assert!(relationships
        .approve_and_execute_remove_relationship(
            relationship_approval(&prepared, "missing-recovery"),
            &Recovery(None)
        )
        .is_err());
    assert_eq!(relationships.relationship_count(), 1);
}

#[test]
fn h2_commit_failure_and_restart_have_zero_authoritative_effect() {
    let (mut actions, action_id, _) = action_fixture();
    let action = actions.action(&action_id).unwrap().clone();
    let prepared = actions
        .prepare_complete_action(PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: action.version(),
            judgment: None,
            context: action_context("prepare-atomic"),
        })
        .unwrap();
    actions.inject_next_commit_failure();
    assert!(actions
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: action_approval(&prepared, "complete-atomic"),
            context: action_context("complete-atomic")
        })
        .is_err());
    assert_eq!(
        actions.action(&action_id).unwrap().state(),
        ActionState::InProgress
    );
    let (mut restarted, _, _) = action_fixture();
    assert!(restarted
        .approve_and_execute_complete_action(ApproveAndExecuteCompleteAction {
            approval: action_approval(&prepared, "complete-restart"),
            context: action_context("complete-restart")
        })
        .is_err());

    let (mut relationships, relationship_id, _) = relationship_fixture();
    let prepared = relationships
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("prepare-atomic-remove"),
            },
            &recovery(&relationship_id),
        )
        .unwrap();
    relationships.inject_next_commit_failure();
    assert!(relationships
        .approve_and_execute_remove_relationship(
            relationship_approval(&prepared, "remove-atomic"),
            &recovery(&relationship_id)
        )
        .is_err());
    assert_eq!(relationships.relationship_count(), 1);
    let (mut restarted, _, _) = relationship_fixture();
    assert!(restarted
        .approve_and_execute_remove_relationship(
            relationship_approval(&prepared, "remove-restart"),
            &recovery(&relationship_id)
        )
        .is_err());
    assert_eq!(restarted.relationship_count(), 1);
}
