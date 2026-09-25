#![allow(clippy::result_large_err)]

//! RED contract for durable relationship-removal H2b state.
//!
//! This file deliberately names the smallest typed persistence seam still
//! missing from the domain module.  It is a contract test only: the test must
//! fail to compile until H2b state has an explicit, validated snapshot rather
//! than being silently omitted from the ordinary relationship snapshot.

use std::sync::{Arc, Mutex};

use pmc_domain::audit::{AuditActor, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::execution::*;
use pmc_domain::identity::*;
use pmc_domain::provenance::Provenance;
use pmc_domain::relationships::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;

#[derive(Clone)]
struct TestClock(Arc<Mutex<i64>>);

impl Clock for TestClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(*self.0.lock().expect("synthetic clock"))
    }
}

#[test]
fn relationship_rows_require_a_validated_domain_typed_rehydration_boundary() {
    let (service, relationship_id, _, _) = fixture();
    let snapshot = service
        .persistence_snapshot_with_h2b()
        .expect("synthetic relationship snapshot");
    let original = snapshot
        .relationships()
        .iter()
        .find(|record| record.id() == &relationship_id)
        .expect("synthetic relationship");
    let reconstructed = RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
        id: original.id().clone(),
        kind: original.kind(),
        endpoints: original.endpoints().to_vec(),
        purpose: original.purpose(),
        classification: original.classification(),
        version: original.version(),
        created_at: original.created_at(),
        updated_at: original.updated_at(),
    })
    .expect("validated typed relationship row");
    assert_eq!(&reconstructed, original);

    assert_eq!(
        RelationshipRecord::rehydrate(RelationshipPersistenceRecord {
            id: original.id().clone(),
            kind: original.kind(),
            endpoints: original.endpoints().to_vec(),
            purpose: original.purpose(),
            classification: DataClassification::Public,
            version: original.version(),
            created_at: original.created_at(),
            updated_at: original.updated_at(),
        }),
        Err(RelationshipPersistenceError::EndpointMismatch),
        "a row cannot understate inherited endpoint classification"
    );
}

#[test]
fn stakeholder_rows_reject_invalid_timestamp_order_before_snapshot_validation() {
    let (mut service, _, _, _) = fixture();
    let stakeholder_id = StakeholderId::parse("stakeholder-persistence-row").expect("synthetic id");
    service
        .create_stakeholder(CreateStakeholder {
            id: stakeholder_id.clone(),
            name: StakeholderName::parse("Synthetic stakeholder").expect("synthetic name"),
            kind: StakeholderKind::Person,
            classification: Some(DataClassification::Internal),
            provenance: Provenance::UserEntered,
            context: context("create-stakeholder-row"),
        })
        .expect("synthetic stakeholder");
    let snapshot = service
        .persistence_snapshot_with_h2b()
        .expect("synthetic snapshot");
    let original = snapshot
        .stakeholders()
        .iter()
        .find(|record| record.id() == &stakeholder_id)
        .expect("synthetic stakeholder row");
    let reconstructed = StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
        id: original.id().clone(),
        name: original.name().clone(),
        kind: original.kind(),
        classification: original.classification(),
        provenance: original.provenance().clone(),
        version: original.version(),
        created_at: original.created_at(),
        updated_at: original.updated_at(),
    })
    .expect("valid typed row");
    assert_eq!(&reconstructed, original);
    assert_eq!(
        StakeholderRecord::rehydrate(StakeholderPersistenceRecord {
            id: original.id().clone(),
            name: original.name().clone(),
            kind: original.kind(),
            classification: original.classification(),
            provenance: original.provenance().clone(),
            version: original.version(),
            created_at: UtcTimestamp::from_unix_millis(2_000),
            updated_at: UtcTimestamp::from_unix_millis(1_000),
        }),
        Err(RelationshipPersistenceError::InvalidTimestampOrder)
    );
}

#[derive(Clone, Default)]
struct AuditIds(u64);

impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("h2b-audit-{}", self.0))
    }
}

#[derive(Clone)]
struct ExecutionIds(u64);

impl ExecutionIdSource for ExecutionIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("h2b-prepared-{}", self.0))
    }

    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("h2b-receipt-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct AllowRemoval;

impl RemovalPolicyPort for AllowRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}

#[derive(Clone)]
struct Authorization(Arc<Mutex<bool>>);

impl ApprovalAuthorizationPort for Authorization {
    fn authorize_relationship_removal(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts && *self.0.lock().expect("synthetic authority")
    }
}

#[derive(Clone)]
struct Recovery(Option<RecoveryEvidence>);

impl RecoveryEvidencePort for Recovery {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        self.0.clone()
    }
}

type Service = InMemoryRelationshipService<
    TestClock,
    InMemoryEndpointCatalog,
    AuditIds,
    ExecutionIds,
    AllowRemoval,
    Authorization,
>;

fn context(key: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(format!("h2b-{key}")).expect("synthetic id"),
        correlation_id: CorrelationId::parse(format!("h2b-correlation-{key}"))
            .expect("synthetic correlation"),
    }
}

fn fixture() -> (Service, RelationshipId, Arc<Mutex<i64>>, Arc<Mutex<bool>>) {
    let now = Arc::new(Mutex::new(1_000));
    let authority = Arc::new(Mutex::new(true));
    let mut service = InMemoryRelationshipService::with_execution_authorities(
        TestClock(now.clone()),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-h2b").expect("synthetic id"),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-h2b").expect("synthetic id"),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecutionIds(100),
        AllowRemoval,
        Authorization(authority.clone()),
    );
    let relationship_id = RelationshipId::parse("relationship-h2b").expect("synthetic id");
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-h2b").expect("synthetic id"),
            product_id: ProductId::parse("product-h2b").expect("synthetic id"),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("link"),
        })
        .expect("synthetic relationship");
    (service, relationship_id, now, authority)
}

fn recovery(id: &RelationshipId) -> Recovery {
    Recovery(Some(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-h2b").expect("synthetic id"),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            id.clone(),
            true,
        )
        .expect("synthetic evidence"),
    ))
}

fn execute(prepared: &PreparedIntent, key: &str) -> ApproveAndExecuteRemoveRelationship {
    ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: context(key),
    }
}

fn snapshot(service: &Service) -> RelationshipH2bPersistenceSnapshot {
    service
        .persistence_snapshot_with_h2b()
        .expect("H2b snapshot is a typed seam")
}

fn validate(snapshot: RelationshipH2bPersistenceSnapshot) -> RelationshipH2bPersistenceSnapshot {
    snapshot
        .validate()
        .expect("synthetic H2b snapshot validates")
}

fn restore(snapshot: RelationshipH2bPersistenceSnapshot, authority: Arc<Mutex<bool>>) -> Service {
    InMemoryRelationshipService::rehydrate_with_h2b(
        TestClock(Arc::new(Mutex::new(1_000))),
        InMemoryEndpointCatalog::new([
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
                PortfolioId::parse("portfolio-h2b").expect("synthetic id"),
                AggregateVersion::initial(),
                DataClassification::Public,
            )),
            EndpointSnapshot::Product(ProductSnapshot::new(
                ProductId::parse("product-h2b").expect("synthetic id"),
                AggregateVersion::initial(),
                DataClassification::Internal,
            )),
        ]),
        AuditIds::default(),
        ExecutionIds(200),
        AllowRemoval,
        Authorization(authority),
        validate(snapshot),
    )
    .expect("synthetic H2b rehydration")
}

#[test]
fn pending_prepare_survives_without_auto_execution_and_replays_exactly() {
    let (mut original, relationship_id, _, authority) = fixture();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-pending"),
            },
            &recovery(&relationship_id),
        )
        .expect("synthetic H2b prepare");
    // A later ordinary no-effect command shares the global operation timeline;
    // persistence must preserve the interleaving without inventing an audit.
    original
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("relationship-h2b-semantic-replay").expect("synthetic id"),
            portfolio_id: PortfolioId::parse("portfolio-h2b").expect("synthetic id"),
            product_id: ProductId::parse("product-h2b").expect("synthetic id"),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("ordinary-after-prepare"),
        })
        .expect("semantic duplicate remains no-effect");
    let audit_count = original.audit_events().len();
    let restored = restore(snapshot(&original), authority);

    assert_eq!(restored.relationship_count(), 1);
    assert_eq!(restored.audit_events().len(), audit_count);
    let mut restored = restored;
    let replay = restored
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id,
                context: context("prepare-pending"),
            },
            &recovery(prepared.relationship_id()),
        )
        .expect("exact prepare replay");
    assert_eq!(replay, prepared);
    assert_eq!(restored.audit_events().len(), audit_count);
}

#[test]
fn corrupted_prepared_digest_audit_order_and_terminal_markers_fail_closed() {
    let (mut pending_service, relationship_id, _, _) = fixture();
    let prepared = pending_service
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-corrupt"),
            },
            &recovery(&relationship_id),
        )
        .expect("synthetic prepare");
    let pending_snapshot = snapshot(&pending_service);
    let bad_prepared = PreparedIntent::from_persistence(
        prepared.preview().clone(),
        PayloadDigest::parse("0".repeat(64)).expect("synthetic digest"),
    );
    let bad_h2b = pending_snapshot
        .h2b_replay()
        .iter()
        .map(|capsule| {
            RelationshipH2bReplayCapsule::new(
                capsule.idempotency_id().clone(),
                capsule.correlation_id().clone(),
                capsule.operation_ordinal(),
                capsule.command().clone(),
                match capsule.result() {
                    RelationshipH2bPersistenceResult::Prepared(_) => {
                        RelationshipH2bPersistenceResult::Prepared(bad_prepared.clone())
                    }
                    other => other.clone(),
                },
                capsule.audit_event_ids().to_vec(),
            )
        })
        .collect();
    assert_eq!(
        RelationshipH2bPersistenceSnapshot::from_persistence(
            pending_snapshot.stakeholders().to_vec(),
            pending_snapshot.relationships().to_vec(),
            pending_snapshot.ordinary_history_relationships().to_vec(),
            pending_snapshot.ordinary_replay().to_vec(),
            bad_h2b,
            pending_snapshot.audits().to_vec(),
            vec![bad_prepared],
            pending_snapshot.completed().to_vec(),
            pending_snapshot.tombstoned_ordinary().to_vec(),
            pending_snapshot.tombstoned_h2b().to_vec(),
        )
        .expect_err("digest corruption must fail closed"),
        RelationshipPersistenceError::InvalidRemovalState
    );

    let forged_preview = RemoveRelationshipPreview::from_persistence(
        prepared.id().clone(),
        prepared.relationship_id().clone(),
        prepared.preview().relationship_version(),
        prepared.preview().endpoints().to_vec(),
        prepared.preview().kind(),
        prepared.preview().purpose(),
        prepared.preview().effects().to_vec(),
        prepared.preview().classification(),
        prepared.preview().policy_decision(),
        prepared.preview().evidence().clone(),
        prepared.preview().expires_at(),
        prepared.preview().cancellation_policy(),
        "REMOVE forged-prepared-id".to_owned(),
    );
    let forged_prepared = PreparedIntent::from_persistence_preview(forged_preview);
    let forged_h2b = pending_snapshot
        .h2b_replay()
        .iter()
        .map(|capsule| {
            RelationshipH2bReplayCapsule::new(
                capsule.idempotency_id().clone(),
                capsule.correlation_id().clone(),
                capsule.operation_ordinal(),
                capsule.command().clone(),
                match capsule.result() {
                    RelationshipH2bPersistenceResult::Prepared(_) => {
                        RelationshipH2bPersistenceResult::Prepared(forged_prepared.clone())
                    }
                    other => other.clone(),
                },
                capsule.audit_event_ids().to_vec(),
            )
        })
        .collect();
    assert_eq!(
        RelationshipH2bPersistenceSnapshot::from_persistence(
            pending_snapshot.stakeholders().to_vec(),
            pending_snapshot.relationships().to_vec(),
            pending_snapshot.ordinary_history_relationships().to_vec(),
            pending_snapshot.ordinary_replay().to_vec(),
            forged_h2b,
            pending_snapshot.audits().to_vec(),
            vec![forged_prepared],
            pending_snapshot.completed().to_vec(),
            pending_snapshot.tombstoned_ordinary().to_vec(),
            pending_snapshot.tombstoned_h2b().to_vec(),
        )
        .expect_err("self-consistent but noncanonical preview must fail closed"),
        RelationshipPersistenceError::InvalidRemovalState
    );

    let (mut terminal_service, terminal_id, _, _) = fixture();
    let terminal_prepared = terminal_service
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: terminal_id.clone(),
                context: context("prepare-corrupt-terminal"),
            },
            &recovery(&terminal_id),
        )
        .expect("synthetic terminal prepare");
    terminal_service
        .approve_and_execute_remove_relationship(
            execute(&terminal_prepared, "execute-corrupt-terminal"),
            &recovery(&terminal_id),
        )
        .expect("synthetic terminal removal");
    let terminal = snapshot(&terminal_service);
    assert!(RelationshipH2bPersistenceSnapshot::from_persistence(
        terminal.stakeholders().to_vec(),
        terminal.relationships().to_vec(),
        terminal.ordinary_history_relationships().to_vec(),
        terminal.ordinary_replay().to_vec(),
        terminal.h2b_replay().to_vec(),
        terminal.audits().iter().cloned().rev().collect(),
        terminal.pending().to_vec(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .is_err());
}

#[test]
fn pre_submit_cancellation_survives_and_exact_cancel_replay_adds_no_audit() {
    let (mut original, relationship_id, _, authority) = fixture();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-cancel"),
            },
            &recovery(&relationship_id),
        )
        .expect("synthetic H2b prepare");
    original
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            context("cancel-before-submit"),
        )
        .expect("synthetic cancellation");
    let audit_count = original.audit_events().len();
    let mut restored = restore(snapshot(&original), authority);
    restored
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            context("cancel-before-submit"),
        )
        .expect("exact cancellation replay");
    assert_eq!(restored.relationship_count(), 1);
    assert_eq!(restored.audit_events().len(), audit_count);

    // Cancellation removes only the pending authority. A fresh idempotency ID
    // may prepare a new, distinct short-lived intent for the still-existing
    // relationship; rehydration must not turn cancellation into a permanent
    // tombstone or resurrect the cancelled intent.
    let reprepare = restored
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id,
                context: context("prepare-cancel-retry"),
            },
            &recovery(prepared.relationship_id()),
        )
        .expect("fresh prepare after cancellation");
    assert_ne!(reprepare.id(), prepared.id());
}

#[test]
fn audited_nonretryable_rejection_replays_original_safe_error_without_duplicate_audit() {
    let (mut original, relationship_id, _, authority) = fixture();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-reject"),
            },
            &recovery(&relationship_id),
        )
        .expect("synthetic H2b prepare");
    let mut rejected = execute(&prepared, "execute-reject");
    rejected.confirmation = "REMOVE wrong-challenge".to_owned();
    let error = original
        .approve_and_execute_remove_relationship(rejected.clone(), &recovery(&relationship_id))
        .expect_err("synthetic nonretryable rejection");
    assert_eq!(error.code(), ErrorCode::SecurityPolicyDenied);
    assert!(!error.retryable());
    let audit_count = original.audit_events().len();
    let mut restored = restore(snapshot(&original), authority);
    let replay = restored
        .approve_and_execute_remove_relationship(rejected, &recovery(&relationship_id))
        .expect_err("exact rejection replay");
    assert_eq!(replay, error);
    assert_eq!(restored.audit_events().len(), audit_count);

    let mut changed = execute(&prepared, "execute-reject");
    changed.confirmation = "REMOVE another-challenge".to_owned();
    assert_ne!(
        restored
            .approve_and_execute_remove_relationship(changed, &recovery(&relationship_id))
            .expect_err("changed command conflict"),
        error
    );
}

#[test]
fn successful_terminal_removal_rehydrates_without_resurrection() {
    let (mut original, relationship_id, _, authority) = fixture();
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: context("prepare-terminal"),
            },
            &recovery(&relationship_id),
        )
        .expect("synthetic H2b prepare");
    let outcome = original
        .approve_and_execute_remove_relationship(
            execute(&prepared, "execute-terminal"),
            &recovery(&relationship_id),
        )
        .expect("synthetic terminal removal");
    let audit_count = original.audit_events().len();
    let mut restored = restore(snapshot(&original), authority);

    assert_eq!(restored.relationship_count(), 0);

    let reused_id = restored.link_portfolio_product(LinkPortfolioProduct {
        id: relationship_id.clone(),
        portfolio_id: PortfolioId::parse("portfolio-h2b").expect("synthetic id"),
        product_id: ProductId::parse("product-h2b").expect("synthetic id"),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("reuse-removed-relationship-id"),
    });
    assert_eq!(
        reused_id
            .expect_err("removed authority identity cannot be recycled")
            .code(),
        ErrorCode::DomainConflict
    );
    assert_eq!(restored.audit_events().len(), audit_count);
    assert_eq!(
        restored
            .approve_and_execute_remove_relationship(
                execute(&prepared, "execute-terminal"),
                &recovery(&relationship_id)
            )
            .expect("exact terminal replay"),
        outcome
    );
    assert_eq!(restored.audit_events().len(), audit_count);
    let too_late = restored
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            context("cancel-terminal"),
        )
        .expect_err("completed removal cannot be cancelled");
    assert_eq!(too_late.code(), ErrorCode::DomainConflict);
    assert_eq!(restored.relationship_count(), 0);
    // The original link and prepare idempotency keys are tombstoned: neither
    // exact link replay nor exact prepare replay can resurrect authority.
    let link_replay = restored.link_portfolio_product(LinkPortfolioProduct {
        id: relationship_id.clone(),
        portfolio_id: PortfolioId::parse("portfolio-h2b").expect("synthetic id"),
        product_id: ProductId::parse("product-h2b").expect("synthetic id"),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("link"),
    });
    assert_eq!(
        link_replay
            .expect_err("removed relationship cannot replay into existence")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
    let prepare_replay = restored.prepare_remove_relationship(
        PrepareRemoveRelationship {
            relationship_id,
            context: context("prepare-terminal"),
        },
        &Recovery(None),
    );
    assert_eq!(
        prepare_replay
            .expect_err("removed prepare is tombstoned")
            .code(),
        ErrorCode::DomainIdempotencyConflict
    );
}
