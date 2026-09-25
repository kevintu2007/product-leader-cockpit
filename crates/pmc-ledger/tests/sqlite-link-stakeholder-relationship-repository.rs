use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{
        AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, MilestoneId, PortfolioId,
        RelationshipId, StakeholderId,
    },
    portfolio::{self, CreatePortfolio},
    provenance::Provenance,
    relationships::{
        CreateStakeholder, LinkStakeholderRelationship, OperationContext as RelOperationContext,
        StakeholderKind, StakeholderName, StakeholderRelationshipPurpose, StakeholderSubject,
    },
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-link-stakeholder-relationship-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn seed(writer: &mut SqliteProductLedger) {
    writer
        .create_stakeholder(
            CreateStakeholder {
                id: StakeholderId::parse("synthetic-stakeholder-1").unwrap(),
                name: StakeholderName::parse("Synthetic Stakeholder").unwrap(),
                kind: StakeholderKind::Person,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: RelOperationContext {
                    idempotency_id: IdempotencyId::parse("stakeholder-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("stakeholder-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("stakeholder-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
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
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();
}

fn link_command() -> LinkStakeholderRelationship {
    LinkStakeholderRelationship {
        id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
        stakeholder_id: StakeholderId::parse("synthetic-stakeholder-1").unwrap(),
        subject: StakeholderSubject::Portfolio(
            PortfolioId::parse("synthetic-portfolio-1").unwrap(),
        ),
        purpose: StakeholderRelationshipPurpose::Responsibility,
        expected_stakeholder_version: AggregateVersion::new(1).unwrap(),
        expected_subject_version: AggregateVersion::new(1).unwrap(),
        context: RelOperationContext {
            idempotency_id: IdempotencyId::parse("link-1").unwrap(),
            correlation_id: CorrelationId::parse("link-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_links_a_stakeholder_to_a_subject_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed(&mut writer);

    let linked = writer
        .link_stakeholder_relationship(
            link_command(),
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    assert_eq!(
        linked.value().purpose(),
        Some(StakeholderRelationshipPurpose::Responsibility)
    );
    assert_eq!(linked.outcome().audit_event_ids().len(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn link_stakeholder_relationship_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed(&mut writer);

    let first = writer
        .link_stakeholder_relationship(
            link_command(),
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let replayed = writer
        .link_stakeholder_relationship(
            link_command(),
            AuditEventId::parse("link-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(replayed.value(), first.value());
    assert_eq!(
        replayed.outcome().audit_event_ids(),
        first.outcome().audit_event_ids()
    );
}

#[test]
fn link_stakeholder_relationship_rejects_a_milestone_subject_as_unsupported() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed(&mut writer);

    let mut command = link_command();
    command.subject =
        StakeholderSubject::Milestone(MilestoneId::parse("synthetic-milestone-1").unwrap());
    let result = writer.link_stakeholder_relationship(
        command,
        AuditEventId::parse("link-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    // An unimplemented input is a domain refusal the caller can act
    // on, never a retryable storage failure.
    let Err(pmc_ledger::sqlite::LedgerTransactionError::Operation(error)) = result else {
        panic!("expected a domain refusal, got {result:?}");
    };
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::ValidationInvalidField
    );
    assert_eq!(
        error.message_key().as_str(),
        "relationship.milestone_subject_not_supported"
    );
    assert!(!error.retryable());
}
