//! Record entry for the Portfolio family (slice 6B) through the host's
//! flows: each create names the record its reservation holds, an edit needs
//! the version the sheet read, a link binds two records it read, and the
//! edit-detail reads return what a sheet will edit.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::portfolio_entry::{
    attach_portfolio_product, attach_product_kpi, attach_product_roadmap, enter_kpi_definition,
    enter_kpi_observation, enter_portfolio, enter_product, enter_roadmap, revise_kpi_definition,
    revise_kpi_observation, revise_portfolio, revise_product, KpiDefinitionEntry,
    KpiObservationEntry, SimpleEntry,
};
use pmc_application::record_entry::RecordEntryError;
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{AggregateVersion, CorrelationId, IdempotencyId};
use pmc_domain::portfolio::{LongText, OperationContext, ShortText};
use pmc_domain::relationships::OperationContext as LinkContext;
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-portfolio-entry-{nonce}-{sequence}.sqlite3"))
}

fn context(request: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn link_context(request: &str) -> LinkContext {
    LinkContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn simple(name: &str) -> SimpleEntry {
    SimpleEntry {
        name: ShortText::parse(name).unwrap_or_else(|_| panic!("text")),
        details: LongText::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
        classification: Some(DataClassification::Internal),
    }
}

fn kpi() -> KpiDefinitionEntry {
    KpiDefinitionEntry {
        name: ShortText::parse("Synthetic KPI").unwrap_or_else(|_| panic!("text")),
        definition: LongText::parse("What it measures.").unwrap_or_else(|_| panic!("text")),
        owner: ShortText::parse("Owner").unwrap_or_else(|_| panic!("text")),
        target: ShortText::parse("10").unwrap_or_else(|_| panic!("text")),
        cadence: ShortText::parse("monthly").unwrap_or_else(|_| panic!("text")),
        source: LongText::parse("Where it comes from.").unwrap_or_else(|_| panic!("text")),
        classification: Some(DataClassification::Internal),
    }
}

#[test]
fn the_whole_family_is_created_edited_linked_and_read_back() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(11);
    let portfolio = enter_portfolio(
        &mut ledger,
        simple("Portfolio A"),
        context("p1"),
        &mut ids,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    let product = enter_product(
        &mut ledger,
        simple("Product A"),
        context("d1"),
        &mut ids,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    let roadmap = enter_roadmap(
        &mut ledger,
        simple("Roadmap A"),
        context("r1"),
        &mut ids,
        at(3),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    let definition = enter_kpi_definition(&mut ledger, kpi(), context("k1"), &mut ids, at(4))
        .unwrap_or_else(|e| panic!("{e:?}"))
        .record;
    let observation = enter_kpi_observation(
        &mut ledger,
        (definition.id.clone(), definition.version),
        KpiObservationEntry {
            value: ShortText::parse("7").unwrap_or_else(|_| panic!("text")),
            observed_at: at(1_700_000_000_000),
            source: LongText::parse("Counted by hand.").unwrap_or_else(|_| panic!("text")),
            classification: None,
        },
        context("o1"),
        &mut ids,
        at(5),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    // An observation with no chosen classification inherits its definition's.
    assert_eq!(observation.classification, DataClassification::Internal);
    for (id, prefix) in [
        (portfolio.id.as_str(), "portfolio-"),
        (product.id.as_str(), "product-"),
        (roadmap.id.as_str(), "roadmap-"),
        (definition.id.as_str(), "kpi-"),
        (observation.id.as_str(), "observation-"),
    ] {
        assert!(id.starts_with(prefix), "{id}");
    }

    // The links, at the versions just read.
    attach_portfolio_product(
        &mut ledger,
        (portfolio.id.clone(), portfolio.version),
        (product.id.clone(), product.version),
        link_context("l1"),
        &mut ids,
        at(6),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    attach_product_roadmap(
        &mut ledger,
        (product.id.clone(), product.version),
        (roadmap.id.clone(), roadmap.version),
        link_context("l2"),
        &mut ids,
        at(7),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let kpi_link = attach_product_kpi(
        &mut ledger,
        (product.id.clone(), product.version),
        (definition.id.clone(), definition.version),
        link_context("l3"),
        &mut ids,
        at(8),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(kpi_link.value().id().as_str().starts_with("relationship-"));
    // A retried link returns the same relationship, not a second one.
    let again = attach_product_kpi(
        &mut ledger,
        (product.id.clone(), product.version),
        (definition.id.clone(), definition.version),
        link_context("l3"),
        &mut ids,
        at(9),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.value().id(), kpi_link.value().id());

    // The composition the inspector reads sees the structure.
    let composition = ledger
        .read_composition_snapshot(at(10))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(composition.portfolio_relationships.len(), 3);
    assert_eq!(composition.products.len(), 1);
    assert_eq!(composition.roadmaps.len(), 1);
    assert_eq!(composition.kpi_definitions.len(), 1);
    assert_eq!(composition.kpi_observations.len(), 1);

    // Edits at the version read; the reads give the sheet the new fields.
    let edited = revise_product(
        &mut ledger,
        product.id.clone(),
        product.version,
        SimpleEntry {
            name: ShortText::parse("Product A, renamed").unwrap_or_else(|_| panic!("text")),
            details: LongText::parse("Edited.").unwrap_or_else(|_| panic!("text")),
            classification: Some(DataClassification::Confidential),
        },
        context("e1"),
        &mut ids,
        at(11),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(edited.version.get(), 2);
    let read = ledger
        .read_product_entry(&product.id)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the product exists"));
    assert_eq!(read.name, "Product A, renamed");
    assert_eq!(read.classification, DataClassification::Confidential);
    assert_eq!(read.version.get(), 2);
    // A stale edit is a conflict, and changes nothing.
    let stale = revise_product(
        &mut ledger,
        product.id.clone(),
        product.version,
        simple("Stale"),
        context("e2"),
        &mut ids,
        at(12),
    );
    match stale {
        Err(error) => assert_eq!(error.code(), ErrorCode::DomainConflict),
        Ok(_) => panic!("a stale version must be refused"),
    }
    revise_portfolio(
        &mut ledger,
        portfolio.id.clone(),
        portfolio.version,
        simple("Portfolio A, renamed"),
        context("e3"),
        &mut ids,
        at(13),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let portfolios = ledger
        .list_portfolio_entries()
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(portfolios.len(), 1);
    assert_eq!(portfolios[0].name, "Portfolio A, renamed");

    // Raising the definition's classification raises the observation too.
    let mut raised = kpi();
    raised.classification = Some(DataClassification::Restricted);
    let definition = revise_kpi_definition(
        &mut ledger,
        definition.id.clone(),
        definition.version,
        raised,
        context("e4"),
        &mut ids,
        at(14),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(definition.classification, DataClassification::Restricted);
    let observed = ledger
        .read_kpi_observation_entry(&observation.id)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the observation exists"));
    assert_eq!(observed.classification, DataClassification::Restricted);
    assert_eq!(observed.observed_at, at(1_700_000_000_000));
    let observation_version = observed.version;
    let re_observed = revise_kpi_observation(
        &mut ledger,
        observation.id.clone(),
        observation_version,
        KpiObservationEntry {
            value: ShortText::parse("8").unwrap_or_else(|_| panic!("text")),
            observed_at: at(1_700_000_001_000),
            source: LongText::parse("Counted again.").unwrap_or_else(|_| panic!("text")),
            classification: None,
        },
        context("e5"),
        &mut ids,
        at(15),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(re_observed.value.as_str(), "8");
    let read_definition = ledger
        .read_kpi_definition_entry(&definition.id)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the definition exists"));
    assert_eq!(read_definition.cadence, "monthly");
    assert!(ledger
        .read_roadmap_entry(&roadmap.id)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_some());
    assert!(ledger
        .read_portfolio_entry(&portfolio.id)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_some());
}

#[test]
fn a_create_retried_from_another_launch_names_the_same_record() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(1);
    let first = enter_product(
        &mut ledger,
        simple("Once"),
        context("sheet"),
        &mut ids,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut later = OpaqueIdSource::with_nonce(2);
    let again = enter_product(
        &mut ledger,
        simple("Once"),
        context("sheet"),
        &mut later,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(again, first);
    // The same request for another kind is refused outright.
    let reused = enter_roadmap(
        &mut ledger,
        simple("Once"),
        context("sheet"),
        &mut later,
        at(3),
    );
    assert!(matches!(reused, Err(RecordEntryError::RequestReused)));
    assert_eq!(
        ledger
            .read_composition_snapshot(at(4))
            .unwrap_or_else(|e| panic!("{e:?}"))
            .products
            .len(),
        1
    );
    let _ = AggregateVersion::initial();
}
