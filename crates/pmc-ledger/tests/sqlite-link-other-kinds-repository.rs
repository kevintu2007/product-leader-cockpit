use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{
        AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, KpiId,
        PortfolioId, ProductId, ProjectId, RelationshipId, RoadmapId,
    },
    portfolio::{self, CreateKpiDefinition, CreatePortfolio, CreateProduct, CreateRoadmap},
    provenance::Provenance,
    relationships::{
        LinkInitiativeProject, LinkPortfolioInitiative, LinkProductKpi, LinkProductRoadmap,
        LinkProjectProduct, OperationContext as RelOperationContext,
    },
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::Connection;

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
            "pmc-synthetic-link-other-{label}-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn rel_context(idempotency_id: &str, correlation_id: &str) -> RelOperationContext {
    RelOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency_id).unwrap(),
        correlation_id: CorrelationId::parse(correlation_id).unwrap(),
    }
}

/// Initiative and Project have no real SQLite writer yet (Delivery/#54 is a
/// separate, not-yet-started slice). Link commands only ever read their
/// current version/classification from `aggregate_registry`, so a synthetic
/// direct-SQL seed row is a faithful, minimal stand-in -- matching the same
/// technique used for the Stakeholder cascade test in
/// `sqlite-stakeholder-repository.rs`.
fn seed_aggregate(ledger: &std::path::Path, id: &str, aggregate_type: &str, classification: &str) {
    let connection = Connection::open(ledger).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,?2,1,?3,500,500)",
            rusqlite::params![id, aggregate_type, classification],
        )
        .unwrap();
}

#[test]
fn link_portfolio_initiative_persists_and_survives_restart() {
    let ledger = SyntheticLedger::new("portfolio-initiative");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
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
    seed_aggregate(
        &ledger.0,
        "synthetic-initiative-1",
        "initiative",
        "internal",
    );

    let linked = writer
        .link_portfolio_initiative(
            LinkPortfolioInitiative {
                id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
                portfolio_id: PortfolioId::parse("synthetic-portfolio-1").unwrap(),
                initiative_id: InitiativeId::parse("synthetic-initiative-1").unwrap(),
                expected_portfolio_version: AggregateVersion::new(1).unwrap(),
                expected_initiative_version: AggregateVersion::new(1).unwrap(),
                context: rel_context("link-1", "link-correlation-1"),
            },
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    assert_eq!(linked.value().version().get(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn link_product_roadmap_persists_and_survives_restart() {
    let ledger = SyntheticLedger::new("product-roadmap");
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
                    idempotency_id: IdempotencyId::parse("product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("product-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    writer
        .create_roadmap(
            CreateRoadmap {
                id: RoadmapId::parse("synthetic-roadmap-1").unwrap(),
                name: portfolio::ShortText::parse("Synthetic Roadmap").unwrap(),
                details: portfolio::LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("roadmap-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("roadmap-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("roadmap-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();

    let linked = writer
        .link_product_roadmap(
            LinkProductRoadmap {
                id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
                product_id: ProductId::parse("synthetic-product-1").unwrap(),
                roadmap_id: RoadmapId::parse("synthetic-roadmap-1").unwrap(),
                expected_product_version: AggregateVersion::new(1).unwrap(),
                expected_roadmap_version: AggregateVersion::new(1).unwrap(),
                context: rel_context("link-1", "link-correlation-1"),
            },
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn link_product_kpi_persists_and_survives_restart() {
    let ledger = SyntheticLedger::new("product-kpi");
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
                    idempotency_id: IdempotencyId::parse("product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("product-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    writer
        .create_kpi_definition(
            CreateKpiDefinition {
                id: KpiId::parse("synthetic-kpi-1").unwrap(),
                name: portfolio::ShortText::parse("Synthetic KPI").unwrap(),
                definition: portfolio::LongText::parse("Synthetic only.").unwrap(),
                owner: portfolio::ShortText::parse("Owner").unwrap(),
                target: portfolio::ShortText::parse("Target").unwrap(),
                cadence: portfolio::ShortText::parse("weekly").unwrap(),
                source: portfolio::LongText::parse("Source").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: portfolio::OperationContext {
                    idempotency_id: IdempotencyId::parse("kpi-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("kpi-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("kpi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .unwrap();

    let linked = writer
        .link_product_kpi(
            LinkProductKpi {
                id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
                product_id: ProductId::parse("synthetic-product-1").unwrap(),
                kpi_id: KpiId::parse("synthetic-kpi-1").unwrap(),
                expected_product_version: AggregateVersion::new(1).unwrap(),
                expected_kpi_version: AggregateVersion::new(1).unwrap(),
                context: rel_context("link-1", "link-correlation-1"),
            },
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn link_initiative_project_persists_and_survives_restart() {
    let ledger = SyntheticLedger::new("initiative-project");
    let writer_open = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(writer_open);
    seed_aggregate(
        &ledger.0,
        "synthetic-initiative-1",
        "initiative",
        "internal",
    );
    seed_aggregate(&ledger.0, "synthetic-project-1", "project", "internal");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let linked = writer
        .link_initiative_project(
            LinkInitiativeProject {
                id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
                initiative_id: InitiativeId::parse("synthetic-initiative-1").unwrap(),
                project_id: ProjectId::parse("synthetic-project-1").unwrap(),
                expected_initiative_version: AggregateVersion::new(1).unwrap(),
                expected_project_version: AggregateVersion::new(1).unwrap(),
                context: rel_context("link-1", "link-correlation-1"),
            },
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}

#[test]
fn link_project_product_persists_and_survives_restart() {
    let ledger = SyntheticLedger::new("project-product");
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
                    idempotency_id: IdempotencyId::parse("product-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("product-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("product-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    drop(writer);
    seed_aggregate(&ledger.0, "synthetic-project-1", "project", "internal");
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let linked = writer
        .link_project_product(
            LinkProjectProduct {
                id: RelationshipId::parse("synthetic-relationship-1").unwrap(),
                project_id: ProjectId::parse("synthetic-project-1").unwrap(),
                product_id: ProductId::parse("synthetic-product-1").unwrap(),
                expected_project_version: AggregateVersion::new(1).unwrap(),
                expected_product_version: AggregateVersion::new(1).unwrap(),
                context: rel_context("link-1", "link-correlation-1"),
            },
            AuditEventId::parse("link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    assert_eq!(
        linked.value().classification(),
        DataClassification::Internal
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.revision().unwrap() > 0);
}
