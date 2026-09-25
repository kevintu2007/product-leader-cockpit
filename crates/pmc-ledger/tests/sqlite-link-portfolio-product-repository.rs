use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{
        AuditEventId, CorrelationId, IdempotencyId, PortfolioId, ProductId, RelationshipId,
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
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-link-portfolio-product-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn seed_portfolio_and_product(writer: &mut SqliteProductLedger) {
    writer
        .create_portfolio(
            CreatePortfolio {
                id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
                name: portfolio::ShortText::parse("Synthetic Portfolio").unwrap(),
                details: portfolio::LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-portfolio-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-portfolio-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-portfolio-audit-1").unwrap(),
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
                    idempotency_id: IdempotencyId::parse("synthetic-product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-product-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();
}

fn link_command(relationship_id: &str, idempotency_suffix: &str) -> LinkPortfolioProduct {
    LinkPortfolioProduct {
        id: RelationshipId::parse(relationship_id).unwrap(),
        portfolio_id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
        product_id: ProductId::parse("synthetic-product-1").unwrap(),
        expected_portfolio_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
        expected_product_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
        context: RelOperationContext {
            idempotency_id: IdempotencyId::parse(format!("synthetic-link-{idempotency_suffix}"))
                .unwrap(),
            correlation_id: CorrelationId::parse(format!(
                "synthetic-link-correlation-{idempotency_suffix}"
            ))
            .unwrap(),
        },
    }
}

#[test]
fn writer_links_a_portfolio_and_product_and_survives_restart() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_portfolio_and_product(&mut writer);

    let linked = writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    assert_eq!(linked.value().version().get(), 1);
    assert_eq!(linked.outcome().audit_event_ids().len(), 1);
    assert_eq!(writer.revision().unwrap(), 3);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 3);
}

#[test]
fn link_portfolio_product_replay_with_the_same_payload_returns_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_portfolio_and_product(&mut writer);

    let first = writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let replayed = writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "1"),
            AuditEventId::parse("synthetic-link-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(replayed.value(), first.value());
    assert_eq!(
        replayed.outcome().audit_event_ids(),
        first.outcome().audit_event_ids()
    );
    assert_eq!(writer.revision().unwrap(), 3);
}

#[test]
fn link_portfolio_product_rejects_a_same_id_replay_with_a_different_payload() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_portfolio_and_product(&mut writer);
    writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let mut drifted = link_command("synthetic-relationship-1", "1");
    drifted.id = RelationshipId::parse("synthetic-relationship-2").unwrap();
    let result = writer.link_portfolio_product(
        drifted,
        AuditEventId::parse("synthetic-link-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(result.is_err());
}

#[test]
fn relinking_the_same_semantic_pair_under_a_new_idempotency_id_returns_the_existing_relationship_as_a_no_op(
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_portfolio_and_product(&mut writer);

    let first = writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let revision_after_first = writer.revision().unwrap();

    let relinked = writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "2"),
            AuditEventId::parse("synthetic-link-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(relinked.value(), first.value());
    assert!(relinked.outcome().audit_event_ids().is_empty());
    assert_eq!(writer.revision().unwrap(), revision_after_first + 1);
}

#[test]
fn relinking_the_same_semantic_pair_reflects_an_endpoints_classification_raised_after_the_original_link(
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_portfolio_and_product(&mut writer);

    writer
        .link_portfolio_product(
            link_command("synthetic-relationship-1", "1"),
            AuditEventId::parse("synthetic-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    // Raise the Portfolio's classification after the link -- nothing in the
    // Link write path updates relationship_endpoints for non-Stakeholder
    // endpoints, so a relink must re-resolve this live rather than reuse the
    // stale snapshot captured at link time.
    writer
        .update_portfolio_details(
            portfolio::UpdatePortfolioDetails {
                id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
                name: portfolio::ShortText::parse("Synthetic Portfolio").unwrap(),
                details: portfolio::LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Confidential),
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-portfolio-update-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-portfolio-correlation-2")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-portfolio-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_500),
        )
        .unwrap();

    let mut relink = link_command("synthetic-relationship-1", "2");
    relink.expected_portfolio_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let relinked = writer
        .link_portfolio_product(
            relink,
            AuditEventId::parse("synthetic-link-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
        )
        .unwrap();
    assert_eq!(
        relinked.value().classification(),
        DataClassification::Confidential
    );
}

#[test]
fn link_portfolio_product_rejects_a_stale_expected_version() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    seed_portfolio_and_product(&mut writer);

    let mut stale = link_command("synthetic-relationship-1", "1");
    stale.expected_portfolio_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let result = writer.link_portfolio_product(
        stale,
        AuditEventId::parse("synthetic-link-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}

#[test]
fn link_portfolio_product_rejects_a_missing_portfolio() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_product(
            CreateProduct {
                id: ProductId::parse("synthetic-product-1").unwrap(),
                name: portfolio::ShortText::parse("Synthetic Product").unwrap(),
                details: portfolio::LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-product-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();

    let result = writer.link_portfolio_product(
        link_command("synthetic-relationship-1", "1"),
        AuditEventId::parse("synthetic-link-audit-1").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );
    assert!(result.is_err());
}
