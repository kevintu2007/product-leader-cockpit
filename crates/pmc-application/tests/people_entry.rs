//! Record entry for People (slice 6D) through the host's flows: a
//! Stakeholder named by its reservation, edited at the version read, and
//! related to a subject at both versions read; the relationship folds the
//! classification of both ends.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::people_entry::{
    attach_stakeholder_subject, enter_stakeholder, revise_stakeholder, StakeholderEntry,
};
use pmc_application::portfolio_entry::{enter_product, SimpleEntry};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{CorrelationId, IdempotencyId};
use pmc_domain::portfolio::{LongText, OperationContext as PortfolioContext, ShortText};
use pmc_domain::relationships::{
    OperationContext, StakeholderKind, StakeholderName, StakeholderRelationshipPurpose,
    StakeholderSubject,
};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-people-entry-{nonce}-{sequence}.sqlite3"))
}

fn context(request: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn stakeholder(name: &str, classification: DataClassification) -> StakeholderEntry {
    StakeholderEntry {
        name: StakeholderName::parse(name).unwrap_or_else(|_| panic!("text")),
        classification: Some(classification),
    }
}

#[test]
fn a_stakeholder_is_created_edited_and_related_to_a_subject_it_read() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(31);
    let product = enter_product(
        &mut ledger,
        SimpleEntry {
            name: ShortText::parse("Product A").unwrap_or_else(|_| panic!("text")),
            details: LongText::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
            classification: Some(DataClassification::Confidential),
        },
        PortfolioContext {
            idempotency_id: IdempotencyId::parse("d1").unwrap_or_else(|_| panic!("id")),
            correlation_id: CorrelationId::parse("corr-d1").unwrap_or_else(|_| panic!("id")),
        },
        &mut ids,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    let person = enter_stakeholder(
        &mut ledger,
        stakeholder("Synthetic Person", DataClassification::Internal),
        StakeholderKind::Person,
        context("s1"),
        &mut ids,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .into_value();
    assert!(person.id().as_str().starts_with("stakeholder-"));
    assert_eq!(person.kind(), StakeholderKind::Person);

    // A retry from another launch names the same Stakeholder.
    let again = enter_stakeholder(
        &mut ledger,
        stakeholder("Synthetic Person", DataClassification::Internal),
        StakeholderKind::Person,
        context("s1"),
        &mut OpaqueIdSource::with_nonce(99),
        at(3),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .into_value();
    assert_eq!(again.id(), person.id());

    // The relationship, at the versions read of both ends; its
    // classification is the fold of both.
    let linked = attach_stakeholder_subject(
        &mut ledger,
        (person.id().clone(), person.version()),
        (
            StakeholderSubject::Product(product.id.clone()),
            product.version,
        ),
        StakeholderRelationshipPurpose::Responsibility,
        context("r1"),
        &mut ids,
        at(4),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .into_value();
    assert_eq!(linked.classification(), DataClassification::Confidential);
    let composition = ledger
        .read_composition_snapshot(at(5))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(composition.stakeholders.len(), 1);
    assert_eq!(composition.stakeholder_relationships.len(), 1);

    // An edit at the version read; a stale one is refused.
    let renamed = revise_stakeholder(
        &mut ledger,
        person.id().clone(),
        person.version(),
        stakeholder("Synthetic Person, renamed", DataClassification::Internal),
        context("e1"),
        &mut ids,
        at(6),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .into_value();
    assert_eq!(renamed.version().get(), 2);
    assert_eq!(renamed.name().as_str(), "Synthetic Person, renamed");
    let stale = revise_stakeholder(
        &mut ledger,
        person.id().clone(),
        person.version(),
        stakeholder("Stale", DataClassification::Internal),
        context("e2"),
        &mut ids,
        at(7),
    );
    match stale {
        Err(error) => assert_eq!(error.code(), ErrorCode::DomainConflict),
        Ok(_) => panic!("a stale version must be refused"),
    }
    let read = ledger
        .read_stakeholder_entry(person.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the stakeholder exists"));
    assert_eq!(read.name, "Synthetic Person, renamed");
    assert_eq!(read.kind, StakeholderKind::Person);
    assert_eq!(read.version.get(), 2);
    assert_eq!(
        ledger
            .list_stakeholder_entries()
            .unwrap_or_else(|e| panic!("{e:?}"))
            .len(),
        1
    );
}
