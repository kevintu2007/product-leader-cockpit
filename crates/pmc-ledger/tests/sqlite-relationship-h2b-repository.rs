use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    execution::{
        ApprovalAuthorizationPort, ApproveAndExecuteRemoveRelationship, PayloadDigest,
        PrepareRemoveRelationship, RecoveryEvidence, RecoveryEvidencePort, RemovalPolicyPort,
    },
    identity::{
        AuditEventId, CorrelationId, IdempotencyId, PortfolioId, PreparedIntentId, ProductId,
        RecoveryEvidenceId, RelationshipId,
    },
    portfolio::{self, CreatePortfolio, CreateProduct},
    provenance::Provenance,
    relationships::{LinkPortfolioProduct, OperationContext as RelOperationContext},
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-relationship-h2b-{label}-{nonce}-{sequence}.sqlite3"
        )))
    }
}

#[derive(Clone, Copy)]
struct AllowRemoval;
impl RemovalPolicyPort for AllowRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct DenyRemoval;
impl RemovalPolicyPort for DenyRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        false
    }
}

#[derive(Clone)]
struct FixedRecoveryEvidence(RelationshipId);
impl RecoveryEvidencePort for FixedRecoveryEvidence {
    fn recovery_evidence(&self, relationship_id: &RelationshipId) -> Option<RecoveryEvidence> {
        if relationship_id != &self.0 {
            return None;
        }
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("synthetic-recovery-1").unwrap(),
            "Synthetic recovery",
            UtcTimestamp::from_unix_millis(500),
            relationship_id.clone(),
            true,
        )
        .ok()
    }
}

#[derive(Clone, Copy)]
struct AllowApproval;
impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize_relationship_removal(&self, _: AuditActor) -> bool {
        true
    }
}

fn seed_linked_relationship(writer: &mut SqliteProductLedger) {
    writer
        .create_portfolio(
            CreatePortfolio {
                id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
                name: portfolio::ShortText::parse("Synthetic Portfolio").unwrap(),
                details: portfolio::LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("portfolio-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("portfolio-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("portfolio-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    writer
        .create_product(
            CreateProduct {
                id: ProductId::parse("synthetic-product-1").unwrap(),
                name: portfolio::ShortText::parse("Synthetic Product").unwrap(),
                details: portfolio::LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("product-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();
    writer
        .link_portfolio_product(
            LinkPortfolioProduct {
                id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
                portfolio_id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
                product_id: ProductId::parse("synthetic-product-1").unwrap(),
                expected_portfolio_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
                expected_product_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
                context: RelOperationContext {
                    idempotency_id: IdempotencyId::parse("link-1").unwrap(),
                    correlation_id: CorrelationId::parse("link-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
}

fn prepare_command() -> PrepareRemoveRelationship {
    PrepareRemoveRelationship {
        relationship_id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
        context: RelOperationContext {
            idempotency_id: IdempotencyId::parse("prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("prepare-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_prepares_a_removal_and_survives_restart() {
    let ledger = SyntheticLedger::new("prepare");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);

    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(prepared.id().as_str(), "synthetic-prepared-1");
    assert_eq!(
        prepared.relationship_id().as_str(),
        "synthetic-relationship-1"
    );
    assert_eq!(
        prepared.confirmation_challenge(),
        "REMOVE synthetic-prepared-1"
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn prepare_remove_relationship_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new("prepare-replay");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);

    let first = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let replayed = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(replayed.payload_digest(), first.payload_digest());
    assert_eq!(replayed.id(), first.id());
}

#[test]
fn prepare_remove_relationship_rejects_when_policy_denies() {
    let ledger = SyntheticLedger::new("prepare-denied");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);

    let result = writer.prepare_remove_relationship(
        prepare_command(),
        &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
        &DenyRemoval,
        PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn prepare_remove_relationship_rejects_a_second_outstanding_prepare_for_the_same_relationship() {
    let ledger = SyntheticLedger::new("prepare-outstanding");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let mut second = prepare_command();
    second.context.idempotency_id = IdempotencyId::parse("prepare-2").unwrap();
    let result = writer.prepare_remove_relationship(
        second,
        &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
        &AllowRemoval,
        PreparedIntentId::parse("synthetic-prepared-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_100),
    );
    assert!(result.is_err());
}

#[test]
fn writer_cancels_a_prepared_removal_and_survives_restart() {
    let ledger = SyntheticLedger::new("cancel");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    writer
        .cancel_remove_relationship(
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            AuditActor::HeadOfProducts,
            RelOperationContext {
                idempotency_id: IdempotencyId::parse("cancel-1").unwrap(),
                correlation_id: CorrelationId::parse("cancel-correlation-1").unwrap(),
            },
            &AllowApproval,
            AuditEventId::parse("cancel-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn cancel_remove_relationship_allows_a_new_prepare_for_the_same_relationship_afterward() {
    let ledger = SyntheticLedger::new("cancel-then-prepare");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    writer
        .cancel_remove_relationship(
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            AuditActor::HeadOfProducts,
            RelOperationContext {
                idempotency_id: IdempotencyId::parse("cancel-1").unwrap(),
                correlation_id: CorrelationId::parse("cancel-correlation-1").unwrap(),
            },
            &AllowApproval,
            AuditEventId::parse("cancel-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();

    let mut second = prepare_command();
    second.context.idempotency_id = IdempotencyId::parse("prepare-2").unwrap();
    let prepared = writer
        .prepare_remove_relationship(
            second,
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-2").unwrap(),
            UtcTimestamp::from_unix_millis(4_000),
        )
        .unwrap();
    assert_eq!(prepared.id().as_str(), "synthetic-prepared-2");
}

#[test]
fn cancel_remove_relationship_rejects_cancelling_an_unknown_prepared_intent() {
    let ledger = SyntheticLedger::new("cancel-unknown");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);

    let result = writer.cancel_remove_relationship(
        PreparedIntentId::parse("synthetic-prepared-missing").unwrap(),
        AuditActor::HeadOfProducts,
        RelOperationContext {
            idempotency_id: IdempotencyId::parse("cancel-1").unwrap(),
            correlation_id: CorrelationId::parse("cancel-correlation-1").unwrap(),
        },
        &AllowApproval,
        AuditEventId::parse("cancel-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}

#[test]
fn writer_executes_a_prepared_removal_and_survives_restart() {
    let ledger = SyntheticLedger::new("execute");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let outcome = writer
        .approve_and_execute_remove_relationship(
            ApproveAndExecuteRemoveRelationship {
                prepared_id: PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
                actor: AuditActor::HeadOfProducts,
                confirmation: prepared.confirmation_challenge().to_owned(),
                acknowledged_payload_digest: PayloadDigest::parse(
                    prepared.payload_digest().as_str(),
                )
                .unwrap(),
                context: RelOperationContext {
                    idempotency_id: IdempotencyId::parse("execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("execute-correlation-1").unwrap(),
                },
            },
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            &AllowApproval,
            AuditEventId::parse("execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(outcome.relationship_id.as_str(), "synthetic-relationship-1");
    assert_eq!(outcome.audit_event_ids.len(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM relationships WHERE id='synthetic-relationship-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "removed relationship must not remain in current authority"
    );
}

#[test]
fn approve_and_execute_remove_relationship_replay_with_the_same_payload_returns_the_original_outcome(
) {
    let ledger = SyntheticLedger::new("execute-replay");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let execute_command = ApproveAndExecuteRemoveRelationship {
        prepared_id: PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().to_owned(),
        acknowledged_payload_digest: PayloadDigest::parse(prepared.payload_digest().as_str())
            .unwrap(),
        context: RelOperationContext {
            idempotency_id: IdempotencyId::parse("execute-1").unwrap(),
            correlation_id: CorrelationId::parse("execute-correlation-1").unwrap(),
        },
    };

    let first = writer
        .approve_and_execute_remove_relationship(
            execute_command.clone(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            &AllowApproval,
            AuditEventId::parse("execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    let replayed = writer
        .approve_and_execute_remove_relationship(
            execute_command,
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            &AllowApproval,
            AuditEventId::parse("execute-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(4_000),
        )
        .unwrap();
    assert_eq!(replayed.relationship_id, first.relationship_id);
    assert_eq!(replayed.audit_event_ids, first.audit_event_ids);
}

#[test]
fn approve_and_execute_remove_relationship_rejects_a_wrong_confirmation() {
    let ledger = SyntheticLedger::new("execute-wrong-confirmation");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let result = writer.approve_and_execute_remove_relationship(
        ApproveAndExecuteRemoveRelationship {
            prepared_id: PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            actor: AuditActor::HeadOfProducts,
            confirmation: "WRONG CONFIRMATION".to_owned(),
            acknowledged_payload_digest: PayloadDigest::parse(prepared.payload_digest().as_str())
                .unwrap(),
            context: RelOperationContext {
                idempotency_id: IdempotencyId::parse("execute-1").unwrap(),
                correlation_id: CorrelationId::parse("execute-correlation-1").unwrap(),
            },
        },
        &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
        &AllowRemoval,
        &AllowApproval,
        AuditEventId::parse("execute-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}

#[test]
fn approve_and_execute_remove_relationship_rejects_when_the_prepared_intent_was_cancelled() {
    let ledger = SyntheticLedger::new("execute-cancelled");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    writer
        .cancel_remove_relationship(
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            AuditActor::HeadOfProducts,
            RelOperationContext {
                idempotency_id: IdempotencyId::parse("cancel-1").unwrap(),
                correlation_id: CorrelationId::parse("cancel-correlation-1").unwrap(),
            },
            &AllowApproval,
            AuditEventId::parse("cancel-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_500),
        )
        .unwrap();

    let result = writer.approve_and_execute_remove_relationship(
        ApproveAndExecuteRemoveRelationship {
            prepared_id: PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            actor: AuditActor::HeadOfProducts,
            confirmation: prepared.confirmation_challenge().to_owned(),
            acknowledged_payload_digest: PayloadDigest::parse(prepared.payload_digest().as_str())
                .unwrap(),
            context: RelOperationContext {
                idempotency_id: IdempotencyId::parse("execute-1").unwrap(),
                correlation_id: CorrelationId::parse("execute-correlation-1").unwrap(),
            },
        },
        &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
        &AllowRemoval,
        &AllowApproval,
        AuditEventId::parse("execute-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}

#[test]
fn a_stale_link_replay_after_removal_is_rejected_not_resurrected() {
    let ledger = SyntheticLedger::new("execute-then-stale-link-replay");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    writer
        .approve_and_execute_remove_relationship(
            ApproveAndExecuteRemoveRelationship {
                prepared_id: PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
                actor: AuditActor::HeadOfProducts,
                confirmation: prepared.confirmation_challenge().to_owned(),
                acknowledged_payload_digest: PayloadDigest::parse(
                    prepared.payload_digest().as_str(),
                )
                .unwrap(),
                context: RelOperationContext {
                    idempotency_id: IdempotencyId::parse("execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("execute-correlation-1").unwrap(),
                },
            },
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            &AllowApproval,
            AuditEventId::parse("execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();

    // Replaying the *original* link command (same idempotency id as the one
    // that first created this now-removed relationship) must fail closed,
    // not resurrect it via a trusted replay.
    let result = writer.link_portfolio_product(
        LinkPortfolioProduct {
            id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
            portfolio_id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
            product_id: ProductId::parse("synthetic-product-1").unwrap(),
            expected_portfolio_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
            expected_product_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
            context: RelOperationContext {
                idempotency_id: IdempotencyId::parse("link-1").unwrap(),
                correlation_id: CorrelationId::parse("link-correlation-1").unwrap(),
            },
        },
        AuditEventId::parse("link-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(4_000),
    );
    assert!(
        result.is_err(),
        "a tombstoned idempotency id must not silently replay a removed relationship back into existence"
    );
}

#[test]
fn a_removed_relationship_id_cannot_be_reused_by_a_brand_new_link() {
    let ledger = SyntheticLedger::new("execute-then-id-reuse");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_linked_relationship(&mut writer);
    let prepared = writer
        .prepare_remove_relationship(
            prepare_command(),
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    writer
        .approve_and_execute_remove_relationship(
            ApproveAndExecuteRemoveRelationship {
                prepared_id: PreparedIntentId::parse("synthetic-prepared-1").unwrap(),
                actor: AuditActor::HeadOfProducts,
                confirmation: prepared.confirmation_challenge().to_owned(),
                acknowledged_payload_digest: PayloadDigest::parse(
                    prepared.payload_digest().as_str(),
                )
                .unwrap(),
                context: RelOperationContext {
                    idempotency_id: IdempotencyId::parse("execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("execute-correlation-1").unwrap(),
                },
            },
            &FixedRecoveryEvidence(RelationshipId::parse("synthetic-relationship-1").unwrap()),
            &AllowRemoval,
            &AllowApproval,
            AuditEventId::parse("execute-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();

    // A brand-new idempotency id, but reusing the same (now-removed)
    // relationship id: relationship ids are authority identities, not
    // recyclable row keys, so this must be rejected even though nothing
    // about this specific idempotency id was ever tombstoned.
    let result = writer.link_portfolio_product(
        LinkPortfolioProduct {
            id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
            portfolio_id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
            product_id: ProductId::parse("synthetic-product-1").unwrap(),
            expected_portfolio_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
            expected_product_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
            context: RelOperationContext {
                idempotency_id: IdempotencyId::parse("link-2").unwrap(),
                correlation_id: CorrelationId::parse("link-correlation-2").unwrap(),
            },
        },
        AuditEventId::parse("link-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(4_000),
    );
    assert!(
        result.is_err(),
        "a removed relationship id must never be reusable for a brand-new link"
    );
}
