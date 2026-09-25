#![allow(clippy::result_large_err)]

use pmc_domain::audit::{
    AuditActor, AuditApprovalOutcome, AuditEffectScope, AuditEvent, AuditEventIdSource,
    AuditExecutionOutcome, AuditModule, AuditPolicyOutcome, AuditTarget,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::{DomainError, ErrorCode, SafeErrorExtension, SafeParamValue};
use pmc_domain::execution::*;
use pmc_domain::identity::*;
use pmc_domain::provenance::Provenance;
use pmc_domain::relationships::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ClockState(Arc<Mutex<i64>>);
impl Clock for ClockState {
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

struct FailAfterFirstAuditId(u64);
impl AuditEventIdSource for FailAfterFirstAuditId {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        if self.0 >= 1 {
            return AuditEventId::parse("");
        }
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

#[derive(Clone)]
struct ExecutionIds(u64);
impl ExecutionIdSource for ExecutionIds {
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
struct AllowPolicy;
impl RemovalPolicyPort for AllowPolicy {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}

#[derive(Clone)]
struct TogglePolicy(Arc<Mutex<bool>>);
impl RemovalPolicyPort for TogglePolicy {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        *self.0.lock().unwrap()
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

type Service = InMemoryRelationshipService<
    ClockState,
    InMemoryEndpointCatalog,
    AuditIds,
    ExecutionIds,
    AllowPolicy,
    Authority,
>;

fn context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(id).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{id}")).unwrap(),
    }
}

fn evidence(id: &RelationshipId) -> Evidence {
    Evidence(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-synthetic").unwrap(),
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
    let mut service = InMemoryRelationshipService::with_execution_authorities(
        ClockState(now.clone()),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-synthetic").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-synthetic").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds(0),
        ExecutionIds(10),
        AllowPolicy,
        Authority(authorized.clone()),
    );
    let relationship_id = RelationshipId::parse("relationship-synthetic").unwrap();
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-synthetic").unwrap(),
            product_id: ProductId::parse("product-synthetic").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("link-synthetic"),
        })
        .unwrap();
    (service, relationship_id, now, authorized)
}

fn prepare(service: &mut Service, id: &RelationshipId, key: &str) -> PreparedIntent {
    service
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: id.clone(),
                context: context(key),
            },
            &evidence(id),
        )
        .unwrap()
}

fn approve(prepared: &PreparedIntent, key: &str) -> ApproveAndExecuteRemoveRelationship {
    ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: context(key),
    }
}

fn assert_safe(error: &DomainError, correlation: &str, forbidden: &[&str]) {
    assert_eq!(error.correlation_id().as_str(), correlation);
    assert!(error.message_key().as_str().contains('.'));
    assert!(error.private_detail_ref().is_none());
    assert!(error.params().is_empty());
    assert!(error.extensions().is_empty());
    for param in error.params() {
        assert!(
            param.key().contains('-') || param.key().contains('_') || param.key().contains('.')
        );
        if let SafeParamValue::Identifier(value) | SafeParamValue::FieldKey(value) = param.value() {
            for secret in forbidden {
                assert!(!value.contains(secret), "unsafe parameter leaked: {secret}");
            }
        }
    }
    for extension in error.extensions() {
        match extension {
            SafeErrorExtension::CurrentVersion(version) => assert!(version.get() > 0),
            SafeErrorExtension::FieldErrors(fields) => {
                for field in fields {
                    assert!(!field.field_key().contains("relationship-synthetic"));
                    assert!(!field.reason_key().contains("synthetic-recovery"));
                }
            }
        }
    }
    let rendered = error.to_string();
    let debugged = format!("{error:?}");
    for secret in forbidden {
        assert!(!rendered.contains(secret));
        assert!(!debugged.contains(secret));
    }
}

fn assert_latest_rejection_audit(
    events: &[AuditEvent],
    id: &RelationshipId,
    correlation: &str,
    actor: AuditActor,
    policy: AuditPolicyOutcome,
    approval: AuditApprovalOutcome,
    execution: AuditExecutionOutcome,
) {
    let event = events.last().unwrap();
    let expected_code = if policy == AuditPolicyOutcome::Denied {
        "relationship.removal.policy_denied"
    } else if execution == AuditExecutionOutcome::Failed {
        "relationship.removal.execution_rejected"
    } else {
        "relationship.removal.approval_rejected"
    };
    assert_eq!(event.actor(), actor);
    assert_eq!(event.module(), AuditModule::Execution);
    assert_eq!(event.code().as_str(), expected_code);
    assert_eq!(event.correlation_id().as_str(), correlation);
    assert_eq!(event.policy_outcome(), policy);
    assert_eq!(event.approval_outcome(), approval);
    assert_eq!(event.execution_outcome(), execution);
    assert_eq!(event.effect_scope(), AuditEffectScope::None);
    assert!(event.actual_effects().is_empty());
    assert_eq!(event.target(), &AuditTarget::Relationship(id.clone()));
}

fn assert_error_semantics(error: &DomainError, code: ErrorCode, message_key: &str) {
    assert_eq!(error.code(), code);
    assert_eq!(error.message_key().as_str(), message_key);
}

#[test]
fn every_h2b_denial_is_safe_and_has_zero_business_effect() {
    let forbidden = [
        "relationship-synthetic",
        "synthetic-recovery",
        "portfolio-synthetic",
    ];
    let (mut service, id, now, authorized) = fixture();
    let prepared = prepare(&mut service, &id, "prepare-safe");
    let initial_audits = service.audit_events().len();

    let mut wrong_hash = approve(&prepared, "wrong-hash-safe");
    wrong_hash.acknowledged_payload_digest = PayloadDigest::parse("0".repeat(64)).unwrap();
    let error = service
        .approve_and_execute_remove_relationship(wrong_hash.clone(), &evidence(&id))
        .unwrap_err();
    assert_safe(&error, "correlation-wrong-hash-safe", &forbidden);
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPreviewExpiredOrChanged,
        "relationship.removal.preview_expired_or_changed",
    );
    assert_latest_rejection_audit(
        service.audit_events(),
        &id,
        "correlation-wrong-hash-safe",
        AuditActor::HeadOfProducts,
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Rejected,
        AuditExecutionOutcome::NotAttempted,
    );
    let audits_after_first_rejection = service.audit_events().len();
    assert_eq!(
        service
            .approve_and_execute_remove_relationship(wrong_hash, &evidence(&id))
            .unwrap_err(),
        error
    );
    assert_eq!(service.audit_events().len(), audits_after_first_rejection);

    let mut wrong_confirmation = approve(&prepared, "wrong-confirmation-safe");
    wrong_confirmation.confirmation = "REMOVE secret-relationship-synthetic".into();
    let error = service
        .approve_and_execute_remove_relationship(wrong_confirmation, &evidence(&id))
        .unwrap_err();
    assert_safe(&error, "correlation-wrong-confirmation-safe", &forbidden);
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.confirmation_mismatch",
    );
    assert_latest_rejection_audit(
        service.audit_events(),
        &id,
        "correlation-wrong-confirmation-safe",
        AuditActor::HeadOfProducts,
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Rejected,
        AuditExecutionOutcome::NotAttempted,
    );

    wrong_authority(&mut service, &prepared, &id, &forbidden);
    *authorized.lock().unwrap() = true;
    *now.lock().unwrap() = prepared.expires_at().unix_millis();
    let error = service
        .approve_and_execute_remove_relationship(approve(&prepared, "expired-safe"), &evidence(&id))
        .unwrap_err();
    assert_safe(&error, "correlation-expired-safe", &forbidden);
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPreviewExpiredOrChanged,
        "relationship.removal.preview_expired_or_changed",
    );
    assert_latest_rejection_audit(
        service.audit_events(),
        &id,
        "correlation-expired-safe",
        AuditActor::HeadOfProducts,
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Rejected,
        AuditExecutionOutcome::NotAttempted,
    );
    assert_eq!(service.relationship_count(), 1);
    assert_eq!(service.audit_events().len(), initial_audits + 4);
}

fn wrong_authority(
    service: &mut Service,
    prepared: &PreparedIntent,
    id: &RelationshipId,
    forbidden: &[&str],
) {
    let error = service
        .approve_and_execute_remove_relationship(
            ApproveAndExecuteRemoveRelationship {
                actor: AuditActor::PolicyAuthorizedSystem,
                ..approve(prepared, "wrong-actor-safe")
            },
            &evidence(id),
        )
        .unwrap_err();
    assert_safe(&error, "correlation-wrong-actor-safe", forbidden);
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.authorization_denied",
    );
    assert_latest_rejection_audit(
        service.audit_events(),
        id,
        "correlation-wrong-actor-safe",
        AuditActor::PolicyAuthorizedSystem,
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Rejected,
        AuditExecutionOutcome::NotAttempted,
    );
}

#[test]
fn cancellation_is_pre_submit_only_and_restart_does_not_replay_approval() {
    let forbidden = ["relationship-synthetic", "synthetic-recovery"];
    let (mut service, id, _, _) = fixture();
    let prepared = prepare(&mut service, &id, "prepare-cancel-safe");
    service
        .discard_prepared_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            context("discard-safe"),
        )
        .unwrap();
    let error = service
        .approve_and_execute_remove_relationship(
            approve(&prepared, "after-discard-safe"),
            &evidence(&id),
        )
        .unwrap_err();
    assert_safe(&error, "correlation-after-discard-safe", &forbidden);
    assert_eq!(service.relationship_count(), 1);

    let (mut service, id, _, _) = fixture();
    let prepared = prepare(&mut service, &id, "prepare-restart-safe");
    let (mut restarted, _, _, _) = fixture();
    let error = restarted
        .approve_and_execute_remove_relationship(approve(&prepared, "restart-safe"), &evidence(&id))
        .unwrap_err();
    assert_safe(&error, "correlation-restart-safe", &forbidden);
    assert_eq!(restarted.relationship_count(), 1);
}

#[test]
fn commit_failure_rolls_back_removal_and_retry_is_exactly_once() {
    let (mut service, id, _, _) = fixture();
    let prepared = prepare(&mut service, &id, "prepare-rollback-safe");
    service.inject_next_commit_failure();
    let error = service
        .approve_and_execute_remove_relationship(
            approve(&prepared, "commit-failure-safe"),
            &evidence(&id),
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::PlatformInternal);
    assert_safe(
        &error,
        "correlation-commit-failure-safe",
        &["relationship-synthetic"],
    );
    assert_eq!(service.relationship_count(), 1);
    assert_eq!(service.audit_events().len(), 1);

    let outcome = service
        .approve_and_execute_remove_relationship(
            approve(&prepared, "commit-failure-safe"),
            &evidence(&id),
        )
        .unwrap();
    assert_eq!(outcome.relationship_id, id);
    assert_eq!(service.relationship_count(), 0);
    assert_eq!(service.audit_events().len(), 2);
    assert_eq!(
        service
            .approve_and_execute_remove_relationship(
                approve(&prepared, "commit-failure-safe"),
                &evidence(&id)
            )
            .unwrap(),
        outcome
    );
    assert_eq!(service.audit_events().len(), 2);
}

#[test]
fn stakeholder_reclassification_expires_prepared_preview_without_removal() {
    let (mut service, _, _, _) = fixture();
    let stakeholder_id = StakeholderId::parse("stakeholder-preview-source").unwrap();
    let created = service
        .create_stakeholder(CreateStakeholder {
            id: stakeholder_id.clone(),
            name: StakeholderName::parse("Synthetic Preview Stakeholder").unwrap(),
            kind: StakeholderKind::Organization,
            classification: Some(DataClassification::Public),
            provenance: Provenance::UserEntered,
            context: context("create-preview-stakeholder"),
        })
        .unwrap();
    let relationship_id = RelationshipId::parse("relationship-stakeholder-preview").unwrap();
    service
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: relationship_id.clone(),
            stakeholder_id: stakeholder_id.clone(),
            subject: StakeholderSubject::Product(ProductId::parse("product-synthetic").unwrap()),
            purpose: StakeholderRelationshipPurpose::Responsibility,
            expected_stakeholder_version: created.value().version(),
            expected_subject_version: AggregateVersion::initial(),
            context: context("link-preview-stakeholder"),
        })
        .unwrap();
    let prepared = prepare(&mut service, &relationship_id, "prepare-stakeholder-change");
    service
        .update_stakeholder(UpdateStakeholderDetails {
            id: stakeholder_id,
            expected_version: created.value().version(),
            name: StakeholderName::parse("Synthetic Preview Stakeholder Updated").unwrap(),
            classification: Some(DataClassification::Restricted),
            context: context("reclassify-preview-stakeholder"),
        })
        .unwrap();

    let error = service
        .approve_and_execute_remove_relationship(
            approve(&prepared, "execute-after-stakeholder-change"),
            &evidence(&relationship_id),
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPreviewExpiredOrChanged,
        "relationship.removal.preview_expired_or_changed",
    );
    assert_safe(
        &error,
        "correlation-execute-after-stakeholder-change",
        &[
            "relationship-stakeholder-preview",
            "Synthetic Preview Stakeholder",
        ],
    );
    assert_eq!(service.relationship_count(), 2);
    assert_latest_rejection_audit(
        service.audit_events(),
        &relationship_id,
        "correlation-execute-after-stakeholder-change",
        AuditActor::HeadOfProducts,
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Failed,
    );
}

#[test]
fn endpoint_source_change_expires_preview_even_when_resolved_max_is_unchanged() {
    let (mut service, relationship_id, _, _) = fixture();
    let prepared = prepare(&mut service, &relationship_id, "prepare-source-change");
    service.replace_endpoint_catalog(InMemoryEndpointCatalog::new([
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-synthetic").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )),
        EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-synthetic").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )),
    ]));

    let error = service
        .approve_and_execute_remove_relationship(
            approve(&prepared, "execute-after-source-change"),
            &evidence(&relationship_id),
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPreviewExpiredOrChanged,
        "relationship.removal.preview_expired_or_changed",
    );
    assert_safe(
        &error,
        "correlation-execute-after-source-change",
        &["relationship-synthetic", "portfolio-synthetic"],
    );
    assert_eq!(service.relationship_count(), 1);
    assert_latest_rejection_audit(
        service.audit_events(),
        &relationship_id,
        "correlation-execute-after-source-change",
        AuditActor::HeadOfProducts,
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Failed,
    );
}

#[test]
fn rejection_audit_id_failure_fails_closed_without_business_mutation() {
    let now = Arc::new(Mutex::new(1_000));
    let authorized = Arc::new(Mutex::new(true));
    let mut service = InMemoryRelationshipService::with_execution_authorities(
        ClockState(now),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-synthetic").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-synthetic").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        FailAfterFirstAuditId(0),
        ExecutionIds(10),
        AllowPolicy,
        Authority(authorized),
    );
    let relationship_id = RelationshipId::parse("relationship-audit-failure").unwrap();
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-synthetic").unwrap(),
            product_id: ProductId::parse("product-synthetic").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("link-audit-failure"),
        })
        .unwrap();
    let prepared = service
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-audit-failure"),
            },
            &evidence(&relationship_id),
        )
        .unwrap();
    let mut request = approve(&prepared, "reject-audit-failure");
    request.confirmation = "INVALID".to_owned();

    let error = service
        .approve_and_execute_remove_relationship(request, &evidence(&relationship_id))
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::PlatformInternal);
    assert_safe(
        &error,
        "correlation-reject-audit-failure",
        &["relationship-audit-failure"],
    );
    assert_eq!(service.relationship_count(), 1);
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn live_policy_denial_is_audited_without_consuming_prepared_authority() {
    let allowed = Arc::new(Mutex::new(true));
    let authorized = Arc::new(Mutex::new(true));
    let mut service = InMemoryRelationshipService::with_execution_authorities(
        ClockState(Arc::new(Mutex::new(1_000))),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-synthetic").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-synthetic").unwrap(),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds(0),
        ExecutionIds(10),
        TogglePolicy(allowed.clone()),
        Authority(authorized),
    );
    let relationship_id = RelationshipId::parse("relationship-policy-change").unwrap();
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-synthetic").unwrap(),
            product_id: ProductId::parse("product-synthetic").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("link-policy-change"),
        })
        .unwrap();
    let prepared = service
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-policy-change"),
            },
            &evidence(&relationship_id),
        )
        .unwrap();
    *allowed.lock().unwrap() = false;

    let error = service
        .approve_and_execute_remove_relationship(
            approve(&prepared, "execute-policy-denied"),
            &evidence(&relationship_id),
        )
        .unwrap_err();
    assert_error_semantics(
        &error,
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
    );
    assert_eq!(service.relationship_count(), 1);
    assert_latest_rejection_audit(
        service.audit_events(),
        &relationship_id,
        "correlation-execute-policy-denied",
        AuditActor::HeadOfProducts,
        AuditPolicyOutcome::Denied,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::NotAttempted,
    );
}
