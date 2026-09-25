#![allow(clippy::result_large_err)]

use pmc_domain::audit::{
    AuditActor, AuditApprovalOutcome, AuditEffectScope, AuditEvent, AuditEventIdSource,
    AuditExecutionOutcome, AuditPolicyOutcome, AuditTarget,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::execution::*;
use pmc_domain::identity::*;
use pmc_domain::relationships::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct TestClock(Arc<Mutex<i64>>);
impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(*self.0.lock().unwrap())
    }
}
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

fn assert_zero_effect_rejection(
    event: &AuditEvent,
    relationship_id: &RelationshipId,
    correlation: &str,
    policy: AuditPolicyOutcome,
    approval: AuditApprovalOutcome,
    execution: AuditExecutionOutcome,
) {
    assert_eq!(event.actor(), AuditActor::HeadOfProducts);
    assert_eq!(event.correlation_id().as_str(), correlation);
    assert_eq!(
        event.target(),
        &AuditTarget::Relationship(relationship_id.clone())
    );
    assert_eq!(event.policy_outcome(), policy);
    assert_eq!(event.approval_outcome(), approval);
    assert_eq!(event.execution_outcome(), execution);
    assert_eq!(event.effect_scope(), AuditEffectScope::None);
    assert!(event.actual_effects().is_empty());
}

fn assert_error_semantics(error: &pmc_domain::error::DomainError, code: ErrorCode, key: &str) {
    assert_eq!(error.code(), code);
    assert_eq!(error.message_key().as_str(), key);
}

fn assert_preview_changed(error: &pmc_domain::error::DomainError) {
    assert_error_semantics(
        error,
        ErrorCode::SecurityPreviewExpiredOrChanged,
        "relationship.removal.preview_expired_or_changed",
    );
}
#[derive(Clone)]
struct Ids(u64);
impl ExecutionIdSource for Ids {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("ephemeral-{}", self.0))
    }
}
#[derive(Clone, Copy)]
struct Allow;
impl RemovalPolicyPort for Allow {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}
#[derive(Clone)]
struct Authority(Arc<Mutex<bool>>);
impl ApprovalAuthorizationPort for Authority {
    fn authorize_relationship_removal(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts && *self.0.lock().unwrap()
    }
}
#[derive(Clone)]
struct Evidence(Option<RecoveryEvidence>);
impl RecoveryEvidencePort for Evidence {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        self.0.clone()
    }
}
#[derive(Clone)]
struct MutableCatalog {
    portfolio: Arc<Mutex<PortfolioSnapshot>>,
    product: Arc<Mutex<ProductSnapshot>>,
}
impl EndpointCatalog for MutableCatalog {
    fn portfolio(&self, id: &PortfolioId) -> Option<PortfolioSnapshot> {
        let value = self.portfolio.lock().unwrap().clone();
        (value.id() == id).then_some(value)
    }
    fn product(&self, id: &ProductId) -> Option<ProductSnapshot> {
        let value = self.product.lock().unwrap().clone();
        (value.id() == id).then_some(value)
    }
    fn initiative(&self, _: &InitiativeId) -> Option<InitiativeSnapshot> {
        None
    }
    fn roadmap(&self, _: &RoadmapId) -> Option<RoadmapSnapshot> {
        None
    }
    fn kpi(&self, _: &KpiId) -> Option<KpiSnapshot> {
        None
    }
    fn project(&self, _: &ProjectId) -> Option<ProjectSnapshot> {
        None
    }
    fn milestone(&self, _: &MilestoneId) -> Option<MilestoneSnapshot> {
        None
    }
}
#[derive(Clone)]
struct MutablePolicy(Arc<Mutex<bool>>);
impl RemovalPolicyPort for MutablePolicy {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        *self.0.lock().unwrap()
    }
}

type Service = InMemoryRelationshipService<
    TestClock,
    InMemoryEndpointCatalog,
    AuditIds,
    Ids,
    Allow,
    Authority,
>;
fn context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{id}")).unwrap(),
    }
}
fn recovery(id: &RelationshipId) -> Evidence {
    Evidence(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-alpha").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            id.clone(),
            true,
        )
        .unwrap(),
    ))
}
fn fixture() -> (Service, RelationshipId, Arc<Mutex<i64>>, Arc<Mutex<bool>>) {
    let now = Arc::new(Mutex::new(1_000));
    let authorized = Arc::new(Mutex::new(true));
    let mut svc = InMemoryRelationshipService::with_execution_authorities(
        TestClock(now.clone()),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-alpha").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-alpha").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds(0),
        Ids(10),
        Allow,
        Authority(authorized.clone()),
    );
    let id = RelationshipId::parse("relationship-alpha").unwrap();
    svc.link_portfolio_product(LinkPortfolioProduct {
        id: id.clone(),
        portfolio_id: PortfolioId::parse("portfolio-alpha").unwrap(),
        product_id: ProductId::parse("product-alpha").unwrap(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("link"),
    })
    .unwrap();
    (svc, id, now, authorized)
}
fn prepare(svc: &mut Service, id: &RelationshipId, key: &str) -> PreparedIntent {
    svc.prepare_remove_relationship(
        PrepareRemoveRelationship {
            relationship_id: id.clone(),
            context: context(key),
        },
        &recovery(id),
    )
    .unwrap()
}
fn execute(prepared: &PreparedIntent, key: &str) -> ApproveAndExecuteRemoveRelationship {
    ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().into(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: context(key),
    }
}

#[test]
fn preview_is_exact_typed_and_digest_is_stable() {
    let (mut svc, id, _, _) = fixture();
    let prepared = prepare(&mut svc, &id, "prepare");
    let preview = prepared.preview();
    assert_eq!(preview.intent_type(), REMOVE_RELATIONSHIP_INTENT_TYPE);
    assert_eq!(preview.intent_version(), 1);
    assert_eq!(preview.relationship_id(), &id);
    assert_eq!(preview.kind(), RelationshipKind::PortfolioProduct);
    assert_eq!(preview.purpose(), None);
    assert_eq!(preview.endpoints().len(), 2);
    assert_eq!(preview.classification(), DataClassification::Internal);
    assert_eq!(
        preview.effects(),
        &[
            RemovalEffect::RemoveRelationshipRecord,
            RemovalEffect::RemoveSemanticRelationshipIndex,
            RemovalEffect::CreateIdempotencyTombstone
        ]
    );
    assert_eq!(preview.policy_decision(), RemovalPolicyDecision::Allowed);
    assert_eq!(
        preview.cancellation_policy(),
        CancellationPolicy::NotCancellableAfterSubmit
    );
    assert_eq!(prepared.payload_digest().as_str().len(), 64);
    assert_eq!(
        svc.prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: id.clone(),
                context: context("prepare")
            },
            &recovery(&id)
        )
        .unwrap(),
        prepared
    );
}

#[test]
fn single_command_executes_atomically_and_exact_replay_returns_original() {
    let (mut svc, id, _, _) = fixture();
    let prepared = prepare(&mut svc, &id, "prepare");
    let request = execute(&prepared, "execute");
    svc.inject_next_commit_failure();
    assert!(svc
        .approve_and_execute_remove_relationship(request.clone(), &recovery(&id))
        .is_err());
    assert_eq!(svc.relationship_count(), 1);
    assert_eq!(svc.audit_events().len(), 1);
    let outcome = svc
        .approve_and_execute_remove_relationship(request.clone(), &recovery(&id))
        .unwrap();
    assert_eq!(svc.relationship_count(), 0);
    assert!(svc
        .link_portfolio_product(LinkPortfolioProduct {
            id: id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-alpha").unwrap(),
            product_id: ProductId::parse("product-alpha").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("link"),
        })
        .is_err());
    assert!(svc
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("relationship-new").unwrap(),
            portfolio_id: PortfolioId::parse("portfolio-alpha").unwrap(),
            product_id: ProductId::parse("product-alpha").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("execute"),
        })
        .is_err());
    assert_eq!(
        svc.approve_and_execute_remove_relationship(request, &recovery(&id))
            .unwrap(),
        outcome
    );
    let mut changed_confirmation = execute(&prepared, "execute");
    changed_confirmation.confirmation = "REMOVE changed-after-success".into();
    assert!(svc
        .approve_and_execute_remove_relationship(changed_confirmation, &recovery(&id))
        .is_err());
    let too_late = svc
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            context("cancel-after-success"),
        )
        .unwrap_err();
    assert_eq!(too_late.code(), ErrorCode::DomainConflict);
    assert_eq!(
        too_late.message_key().as_str(),
        "relationship.removal.too_late_to_cancel"
    );
}

#[test]
fn hash_mismatch_authority_revocation_and_expiry_have_zero_effect() {
    let (mut svc, id, now, authorized) = fixture();
    let prepared = prepare(&mut svc, &id, "prepare");
    let mut bad = execute(&prepared, "bad-hash");
    bad.acknowledged_payload_digest = PayloadDigest::parse("0".repeat(64)).unwrap();
    let error = svc
        .approve_and_execute_remove_relationship(bad, &recovery(&id))
        .unwrap_err();
    assert_preview_changed(&error);
    assert_eq!(svc.relationship_count(), 1);
    assert_eq!(svc.audit_events().len(), 2);
    assert_zero_effect_rejection(
        svc.audit_events().last().unwrap(),
        &id,
        "correlation-bad-hash",
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Rejected,
        AuditExecutionOutcome::NotAttempted,
    );
    let mut wrong_confirmation = execute(&prepared, "wrong-confirmation");
    wrong_confirmation.confirmation = "REMOVE something-else".into();
    let error = svc
        .approve_and_execute_remove_relationship(wrong_confirmation, &recovery(&id))
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.confirmation_mismatch",
    );
    assert_eq!(svc.audit_events().len(), 3);
    *authorized.lock().unwrap() = false;
    let error = svc
        .approve_and_execute_remove_relationship(execute(&prepared, "denied"), &recovery(&id))
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.authorization_denied",
    );
    assert_eq!(svc.audit_events().len(), 4);
    *authorized.lock().unwrap() = true;
    *now.lock().unwrap() = prepared.expires_at().unix_millis();
    let error = svc
        .approve_and_execute_remove_relationship(execute(&prepared, "expired"), &recovery(&id))
        .unwrap_err();
    assert_preview_changed(&error);
    assert_eq!(svc.relationship_count(), 1);
    assert_eq!(svc.audit_events().len(), 5);
}

#[test]
fn prepare_replay_is_clock_independent_and_returns_original_expiry() {
    let (mut svc, id, now, _) = fixture();
    let prepared = prepare(&mut svc, &id, "clock-stable-prepare");
    assert_eq!(prepared.expires_at().unix_millis(), 301_000);
    *now.lock().unwrap() = 250_000;
    let replay_inside_ttl = prepare(&mut svc, &id, "clock-stable-prepare");
    assert_eq!(replay_inside_ttl, prepared);
    *now.lock().unwrap() = 400_000;
    let replay_after_ttl = prepare(&mut svc, &id, "clock-stable-prepare");
    assert_eq!(replay_after_ttl, prepared);
    assert_eq!(replay_after_ttl.expires_at().unix_millis(), 301_000);
}

#[test]
fn evidence_is_revalidated_and_cancel_is_replay_safe() {
    let (mut svc, id, _, _) = fixture();
    let prepared = prepare(&mut svc, &id, "prepare");
    assert!(svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "missing-evidence"),
            &Evidence(None)
        )
        .is_err());
    let cancel = context("cancel");
    svc.cancel_remove_relationship(prepared.id(), AuditActor::HeadOfProducts, cancel.clone())
        .unwrap();
    svc.cancel_remove_relationship(prepared.id(), AuditActor::HeadOfProducts, cancel)
        .unwrap();
    assert_eq!(svc.relationship_count(), 1);
    assert!(svc
        .approve_and_execute_remove_relationship(execute(&prepared, "after-cancel"), &recovery(&id))
        .is_err());
}

#[test]
fn restart_and_cross_family_idempotency_fail_closed() {
    let (mut first, id, _, _) = fixture();
    let prepared = prepare(&mut first, &id, "prepare");
    let (mut restarted, _, _, _) = fixture();
    assert!(restarted
        .approve_and_execute_remove_relationship(execute(&prepared, "restart"), &recovery(&id))
        .is_err());
    assert!(first
        .approve_and_execute_remove_relationship(execute(&prepared, "link"), &recovery(&id))
        .is_err());
    assert_eq!(first.relationship_count(), 1);
}

#[test]
fn invalid_recovery_evidence_fails_preparation() {
    let (mut svc, id, _, _) = fixture();
    for evidence in [
        Evidence(None),
        Evidence(Some(
            RecoveryEvidence::new(
                RecoveryEvidenceId::parse("bad-scope").unwrap(),
                "synthetic",
                UtcTimestamp::from_unix_millis(900),
                RelationshipId::parse("other").unwrap(),
                true,
            )
            .unwrap(),
        )),
        Evidence(Some(
            RecoveryEvidence::new(
                RecoveryEvidenceId::parse("incompatible").unwrap(),
                "synthetic",
                UtcTimestamp::from_unix_millis(900),
                id.clone(),
                false,
            )
            .unwrap(),
        )),
    ] {
        assert!(svc
            .prepare_remove_relationship(
                PrepareRemoveRelationship {
                    relationship_id: id.clone(),
                    context: context(&format!("evidence-{}", svc.audit_events().len()))
                },
                &evidence
            )
            .is_err());
    }
    assert_eq!(svc.relationship_count(), 1);
}

#[test]
fn live_endpoint_version_classification_policy_and_evidence_are_revalidated() {
    let now = Arc::new(Mutex::new(1_000));
    let authorized = Arc::new(Mutex::new(true));
    let allowed = Arc::new(Mutex::new(true));
    let portfolio = Arc::new(Mutex::new(PortfolioSnapshot::new(
        PortfolioId::parse("portfolio-alpha").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Public,
    )));
    let product = Arc::new(Mutex::new(ProductSnapshot::new(
        ProductId::parse("product-alpha").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
    )));
    let mut svc = InMemoryRelationshipService::with_execution_authorities(
        TestClock(now),
        MutableCatalog {
            portfolio: portfolio.clone(),
            product: product.clone(),
        },
        AuditIds(0),
        Ids(100),
        MutablePolicy(allowed.clone()),
        Authority(authorized),
    );
    let id = RelationshipId::parse("relationship-mutable").unwrap();
    svc.link_portfolio_product(LinkPortfolioProduct {
        id: id.clone(),
        portfolio_id: PortfolioId::parse("portfolio-alpha").unwrap(),
        product_id: ProductId::parse("product-alpha").unwrap(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("mutable-link"),
    })
    .unwrap();
    let prepared = svc
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: id.clone(),
                context: context("mutable-prepare"),
            },
            &recovery(&id),
        )
        .unwrap();
    let mut expected_audits = 1;
    let mut assert_unchanged = |svc: &InMemoryRelationshipService<_, _, _, _, _, _>,
                                correlation: &str,
                                policy: AuditPolicyOutcome,
                                approval: AuditApprovalOutcome,
                                execution: AuditExecutionOutcome| {
        expected_audits += 1;
        assert_eq!(svc.relationship_count(), 1);
        assert_eq!(svc.audit_events().len(), expected_audits);
        assert_zero_effect_rejection(
            svc.audit_events().last().unwrap(),
            &id,
            correlation,
            policy,
            approval,
            execution,
        );
    };
    *portfolio.lock().unwrap() = PortfolioSnapshot::new(
        PortfolioId::parse("portfolio-alpha").unwrap(),
        AggregateVersion::new(2).unwrap(),
        DataClassification::Public,
    );
    let error = svc
        .approve_and_execute_remove_relationship(execute(&prepared, "stale-source"), &recovery(&id))
        .unwrap_err();
    assert_preview_changed(&error);
    assert_unchanged(
        &svc,
        "correlation-stale-source",
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Failed,
    );
    *portfolio.lock().unwrap() = PortfolioSnapshot::new(
        PortfolioId::parse("portfolio-alpha").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Public,
    );
    *product.lock().unwrap() = ProductSnapshot::new(
        ProductId::parse("product-alpha").unwrap(),
        AggregateVersion::new(2).unwrap(),
        DataClassification::Restricted,
    );
    let error = svc
        .approve_and_execute_remove_relationship(execute(&prepared, "stale-target"), &recovery(&id))
        .unwrap_err();
    assert_preview_changed(&error);
    assert_unchanged(
        &svc,
        "correlation-stale-target",
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Failed,
    );
    *product.lock().unwrap() = ProductSnapshot::new(
        ProductId::parse("product-alpha").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Restricted,
    );
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "classification-raised"),
            &recovery(&id),
        )
        .unwrap_err();
    assert_preview_changed(&error);
    assert_unchanged(
        &svc,
        "correlation-classification-raised",
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Failed,
    );
    *product.lock().unwrap() = ProductSnapshot::new(
        ProductId::parse("product-alpha").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Unclassified,
    );
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "classification-unknown"),
            &recovery(&id),
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_unchanged(
        &svc,
        "correlation-classification-unknown",
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );
    *product.lock().unwrap() = ProductSnapshot::new(
        ProductId::parse("product-alpha").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
    );
    *allowed.lock().unwrap() = false;
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "policy-revoked"),
            &recovery(&id),
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_unchanged(
        &svc,
        "correlation-policy-revoked",
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );
    *allowed.lock().unwrap() = true;
    let changed = Evidence(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-changed").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            id.clone(),
            true,
        )
        .unwrap(),
    ));
    let error = svc
        .approve_and_execute_remove_relationship(execute(&prepared, "evidence-changed"), &changed)
        .unwrap_err();
    assert_preview_changed(&error);
    assert_unchanged(
        &svc,
        "correlation-evidence-changed",
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Failed,
    );
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "evidence-missing"),
            &Evidence(None),
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_unchanged(
        &svc,
        "correlation-evidence-missing",
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );

    let incompatible = Evidence(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-incompatible").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            id.clone(),
            false,
        )
        .unwrap(),
    ));
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "evidence-incompatible"),
            &incompatible,
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_unchanged(
        &svc,
        "correlation-evidence-incompatible",
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );

    let wrong_relationship = Evidence(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-wrong-relationship").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            RelationshipId::parse("relationship-other").unwrap(),
            true,
        )
        .unwrap(),
    ));
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "evidence-wrong-relationship"),
            &wrong_relationship,
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_unchanged(
        &svc,
        "correlation-evidence-wrong-relationship",
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );

    let invalid_verification_time = Evidence(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-invalid-time").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(0),
            id.clone(),
            true,
        )
        .unwrap(),
    ));
    let error = svc
        .approve_and_execute_remove_relationship(
            execute(&prepared, "evidence-invalid-time"),
            &invalid_verification_time,
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_unchanged(
        &svc,
        "correlation-evidence-invalid-time",
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );
}
