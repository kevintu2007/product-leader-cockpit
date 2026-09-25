//! Record entry for the Delivery family (slice 6C) through the host's
//! flows: each create names the record its reservation holds, a Milestone
//! belongs to the Project it was entered under, an edit needs the version
//! the sheet read, raising a Project's classification raises its Milestones,
//! and the two links bind records the sheet read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::delivery_entry::{
    attach_initiative_project, attach_project_product, enter_initiative, enter_milestone,
    enter_project, revise_initiative, revise_milestone, revise_project, InitiativeEntry,
    MilestoneEntry, ProjectEntry,
};
use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::portfolio_entry::{enter_product, SimpleEntry};
use pmc_application::record_entry::RecordEntryError;
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
use pmc_domain::delivery::{DefinedOutcome, OperationContext, RecordName, VerificationCriteria};
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{AggregateVersion, CorrelationId, IdempotencyId};
use pmc_domain::portfolio::{LongText, OperationContext as PortfolioContext, ShortText};
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
    std::env::temp_dir().join(format!("pmc-delivery-entry-{nonce}-{sequence}.sqlite3"))
}

fn correlation(request: &str) -> CorrelationId {
    CorrelationId::parse(format!("corr-{request}")).unwrap_or_else(|_| panic!("id"))
}

fn context(request: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: correlation(request),
    }
}

fn link_context(request: &str) -> LinkContext {
    LinkContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: correlation(request),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn name(value: &str) -> RecordName {
    RecordName::parse(value, &correlation("text")).unwrap_or_else(|e| panic!("{e:?}"))
}

fn initiative_entry(classification: DataClassification) -> InitiativeEntry {
    InitiativeEntry {
        name: name("Synthetic Initiative"),
        defined_outcome: DefinedOutcome::parse("An outcome.", &correlation("text"))
            .unwrap_or_else(|e| panic!("{e:?}")),
        classification: Some(classification),
    }
}

fn project_entry(classification: DataClassification) -> ProjectEntry {
    ProjectEntry {
        name: name("Synthetic Project"),
        start_at: at(1_700_000_000_000),
        end_at: at(1_700_500_000_000),
        classification: Some(classification),
    }
}

fn milestone_entry(classification: Option<DataClassification>) -> MilestoneEntry {
    MilestoneEntry {
        name: name("Synthetic Milestone"),
        verification_criteria: VerificationCriteria::parse(
            "Done when shipped.",
            &correlation("text"),
        )
        .unwrap_or_else(|e| panic!("{e:?}")),
        due_at: at(1_700_400_000_000),
        classification,
    }
}

#[test]
fn the_delivery_family_is_created_edited_linked_and_read_back() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(21);
    let product = enter_product(
        &mut ledger,
        SimpleEntry {
            name: ShortText::parse("Product A").unwrap_or_else(|_| panic!("text")),
            details: LongText::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
            classification: Some(DataClassification::Internal),
        },
        PortfolioContext {
            idempotency_id: IdempotencyId::parse("d1").unwrap_or_else(|_| panic!("id")),
            correlation_id: correlation("d1"),
        },
        &mut ids,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    let initiative = enter_initiative(
        &mut ledger,
        initiative_entry(DataClassification::Internal),
        context("i1"),
        &mut ids,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    let project = enter_project(
        &mut ledger,
        project_entry(DataClassification::Internal),
        context("p1"),
        &mut ids,
        at(3),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    // A Milestone under a Project the sheet read at another version is a
    // conflict, not an attachment to something the person did not see.
    let stale_parent = enter_milestone(
        &mut ledger,
        (
            project.id().clone(),
            AggregateVersion::initial()
                .next()
                .unwrap_or_else(|| panic!("v")),
        ),
        milestone_entry(Some(DataClassification::Internal)),
        context("m0"),
        &mut ids,
        at(4),
    );
    match stale_parent {
        Err(error) => assert_eq!(error.code(), ErrorCode::DomainConflict),
        Ok(_) => panic!("a Project past the version read must be refused"),
    }
    let milestone = enter_milestone(
        &mut ledger,
        (project.id().clone(), project.version()),
        milestone_entry(Some(DataClassification::Internal)),
        context("m1"),
        &mut ids,
        at(4),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert!(initiative.id().as_str().starts_with("initiative-"));
    assert!(project.id().as_str().starts_with("project-"));
    assert!(milestone.id().as_str().starts_with("milestone-"));
    assert_eq!(milestone.project_id(), project.id());

    // The links, at the versions just read; a retry answers the same link.
    let linked = attach_project_product(
        &mut ledger,
        (project.id().clone(), project.version()),
        (product.id.clone(), product.version),
        link_context("l1"),
        &mut ids,
        at(5),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let again = attach_project_product(
        &mut ledger,
        (project.id().clone(), project.version()),
        (product.id.clone(), product.version),
        link_context("l1"),
        &mut ids,
        at(6),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.value().id(), linked.value().id());
    attach_initiative_project(
        &mut ledger,
        (initiative.id().clone(), initiative.version()),
        (project.id().clone(), project.version()),
        link_context("l2"),
        &mut ids,
        at(7),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let composition = ledger
        .read_composition_snapshot(at(8))
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(composition.portfolio_relationships.len(), 2);
    assert_eq!(composition.milestones.len(), 1);

    // The reads give a sheet what it edits.
    let read = ledger
        .read_project_entry(project.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the project exists"));
    assert_eq!(read.name, "Synthetic Project");
    assert_eq!(read.start_at, at(1_700_000_000_000));
    assert_eq!(read.end_at, at(1_700_500_000_000));
    let read = ledger
        .read_milestone_entry(milestone.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the milestone exists"));
    assert_eq!(read.project_id, project.id().as_str());
    assert_eq!(read.due_at, at(1_700_400_000_000));
    assert_eq!(
        ledger
            .list_initiative_entries()
            .unwrap_or_else(|e| panic!("{e:?}"))
            .len(),
        1
    );
    assert_eq!(
        ledger
            .list_project_entries()
            .unwrap_or_else(|e| panic!("{e:?}"))
            .len(),
        1
    );

    // Edits at the version read: a stale one is a conflict.
    let stale = revise_initiative(
        &mut ledger,
        initiative.id().clone(),
        initiative.version(),
        initiative_named("Renamed"),
        context("e0"),
        &mut ids,
        at(9),
    );
    let renamed = stale.unwrap_or_else(|e| panic!("{e:?}")).record;
    assert_eq!(renamed.version().get(), 2);
    let refused = revise_initiative(
        &mut ledger,
        initiative.id().clone(),
        initiative.version(),
        initiative_named("Stale"),
        context("e1"),
        &mut ids,
        at(10),
    );
    match refused {
        Err(error) => assert_eq!(error.code(), ErrorCode::DomainConflict),
        Ok(_) => panic!("a stale version must be refused"),
    }
    let milestone2 = revise_milestone(
        &mut ledger,
        milestone.id().clone(),
        milestone.version(),
        milestone_entry(Some(DataClassification::Internal)),
        context("e2"),
        &mut ids,
        at(11),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(milestone2.version().get(), 2);

    // Raising the Project's classification raises its Milestone too, each
    // with its own audit.
    let raised = revise_project(
        &mut ledger,
        project.id().clone(),
        project.version(),
        project_entry(DataClassification::Restricted),
        context("e3"),
        &mut ids,
        at(12),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        raised.record.classification(),
        DataClassification::Restricted
    );
    assert_eq!(raised.cascaded_milestones.len(), 1);
    assert_eq!(raised.audit_events.len(), 2);
    let read = ledger
        .read_milestone_entry(milestone.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("the milestone exists"));
    assert_eq!(read.classification, DataClassification::Restricted);
    assert!(ledger
        .read_initiative_entry(initiative.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_some());
}

fn initiative_named(value: &str) -> InitiativeEntry {
    InitiativeEntry {
        name: name(value),
        ..initiative_entry(DataClassification::Internal)
    }
}

#[test]
fn a_create_retried_from_another_launch_names_the_same_record() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(1);
    let first = enter_project(
        &mut ledger,
        project_entry(DataClassification::Public),
        context("sheet"),
        &mut ids,
        at(1),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut later = OpaqueIdSource::with_nonce(2);
    let again = enter_project(
        &mut ledger,
        project_entry(DataClassification::Public),
        context("sheet"),
        &mut later,
        at(2),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .record;
    assert_eq!(again, first);
    // The same request for another kind is refused outright.
    let reused = enter_initiative(
        &mut ledger,
        initiative_entry(DataClassification::Public),
        context("sheet"),
        &mut later,
        at(3),
    );
    assert!(matches!(reused, Err(RecordEntryError::RequestReused)));
}
