//! `diff_against_published` and `classify_projection_automation` -- the pure
//! H1-Auto/H2a eligibility predicate ADR 0007 requires before any publish routing decision. No filesystem, no
//! Ledger, no SQL: `published` is always caller-supplied data.

use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{AggregateVersion, ProductId};
use pmc_domain::projection_source::{LedgerProjectionSnapshot, ProductProjectionSource};
use pmc_domain::time::UtcTimestamp;
use pmc_knowledge::projections::{
    classify_projection_automation, diff_against_published, generate_projection_artifact_set,
    ProjectionArtifactChange, ProjectionAutomationEligibility, ProjectionChangeSet,
    ProjectionH2aReason, PROJECTION_H1_AUTO_MAX_DURATION_MILLIS, PROJECTION_H1_AUTO_MAX_FILES,
};

fn product(id: &str, revision: u64) -> ProductProjectionSource {
    ProductProjectionSource {
        id: ProductId::parse(id).unwrap(),
        classification: DataClassification::Internal,
        source_revision: AggregateVersion::new(revision).unwrap(),
    }
}

fn snapshot_with_products(products: Vec<ProductProjectionSource>) -> LedgerProjectionSnapshot {
    LedgerProjectionSnapshot {
        schema_version: 40,
        ledger_revision: 1,
        ledger_as_of_utc: UtcTimestamp::from_unix_millis(1_700_000_000_000),
        products,
        projects: Vec::new(),
        actions: Vec::new(),
        decisions: Vec::new(),
        risks: Vec::new(),
        kpis: Vec::new(),
    }
}

#[test]
fn diffing_against_an_empty_manifest_marks_every_artifact_created() {
    let snapshot = snapshot_with_products(vec![product("product-1", 1), product("product-2", 1)]);
    let fresh = generate_projection_artifact_set(&snapshot);

    let change_set = diff_against_published(&fresh, &[]);

    assert_eq!(change_set.len(), 2);
    assert!(change_set
        .entries
        .iter()
        .all(|entry| entry.change == ProjectionArtifactChange::Created));
}

#[test]
fn diffing_against_its_own_manifest_produces_no_changes() {
    let snapshot = snapshot_with_products(vec![product("product-1", 1)]);
    let fresh = generate_projection_artifact_set(&snapshot);
    let published = fresh.manifest_entries();

    let change_set = diff_against_published(&fresh, &published);

    assert!(change_set.is_empty());
}

#[test]
fn a_changed_revision_produces_exactly_one_updated_entry() {
    let published_snapshot = snapshot_with_products(vec![product("product-1", 1)]);
    let published = generate_projection_artifact_set(&published_snapshot).manifest_entries();

    let fresh_snapshot = snapshot_with_products(vec![product("product-1", 2)]);
    let fresh = generate_projection_artifact_set(&fresh_snapshot);

    let change_set = diff_against_published(&fresh, &published);

    assert_eq!(change_set.len(), 1);
    assert_eq!(
        change_set.entries[0].change,
        ProjectionArtifactChange::Updated
    );
    assert_eq!(change_set.entries[0].record_id, "product-1");
}

#[test]
fn a_record_missing_from_the_fresh_set_is_removed() {
    let published_snapshot =
        snapshot_with_products(vec![product("product-1", 1), product("product-2", 1)]);
    let published = generate_projection_artifact_set(&published_snapshot).manifest_entries();

    let fresh_snapshot = snapshot_with_products(vec![product("product-1", 1)]);
    let fresh = generate_projection_artifact_set(&fresh_snapshot);

    let change_set = diff_against_published(&fresh, &published);

    assert_eq!(change_set.len(), 1);
    assert_eq!(
        change_set.entries[0].change,
        ProjectionArtifactChange::Removed
    );
    assert_eq!(change_set.entries[0].record_id, "product-2");
}

#[test]
fn a_mixed_diff_reports_created_updated_and_removed_together() {
    let published_snapshot =
        snapshot_with_products(vec![product("product-1", 1), product("product-2", 1)]);
    let published = generate_projection_artifact_set(&published_snapshot).manifest_entries();

    let fresh_snapshot =
        snapshot_with_products(vec![product("product-1", 2), product("product-3", 1)]);
    let fresh = generate_projection_artifact_set(&fresh_snapshot);

    let change_set = diff_against_published(&fresh, &published);

    assert_eq!(change_set.len(), 3);
    let by_id = |id: &str| {
        change_set
            .entries
            .iter()
            .find(|entry| entry.record_id == id)
            .unwrap()
            .change
    };
    assert_eq!(by_id("product-1"), ProjectionArtifactChange::Updated);
    assert_eq!(by_id("product-2"), ProjectionArtifactChange::Removed);
    assert_eq!(by_id("product-3"), ProjectionArtifactChange::Created);
}

fn one_entry_change_set() -> ProjectionChangeSet {
    let snapshot = snapshot_with_products(vec![product("product-1", 1)]);
    let fresh = generate_projection_artifact_set(&snapshot);
    diff_against_published(&fresh, &[])
}

#[test]
fn an_empty_change_set_is_never_h1_auto_or_h2a() {
    let empty = ProjectionChangeSet {
        entries: Vec::new(),
    };
    assert_eq!(
        classify_projection_automation(&empty, false, 0, false),
        ProjectionAutomationEligibility::NoChanges
    );
}

#[test]
fn an_integrity_conflict_always_requires_h2a_even_when_small_and_fast() {
    let change_set = one_entry_change_set();
    assert_eq!(
        classify_projection_automation(&change_set, false, 100, true),
        ProjectionAutomationEligibility::H2aRequired(ProjectionH2aReason::IntegrityConflict)
    );
}

#[test]
fn an_initial_or_full_rebuild_always_requires_h2a_even_when_small_and_fast() {
    let change_set = one_entry_change_set();
    assert_eq!(
        classify_projection_automation(&change_set, true, 100, false),
        ProjectionAutomationEligibility::H2aRequired(ProjectionH2aReason::InitialOrFullRebuild)
    );
}

#[test]
fn exceeding_the_file_ceiling_requires_h2a() {
    let entries = one_entry_change_set().entries;
    let mut oversized = Vec::new();
    for index in 0..=PROJECTION_H1_AUTO_MAX_FILES {
        let mut entry = entries[0].clone();
        entry.relative_path = format!("{}-{index}", entry.relative_path);
        oversized.push(entry);
    }
    let change_set = ProjectionChangeSet { entries: oversized };

    assert_eq!(
        classify_projection_automation(&change_set, false, 100, false),
        ProjectionAutomationEligibility::H2aRequired(ProjectionH2aReason::ChangeSetTooLarge)
    );
}

#[test]
fn exceeding_the_duration_ceiling_requires_h2a() {
    let change_set = one_entry_change_set();
    assert_eq!(
        classify_projection_automation(
            &change_set,
            false,
            PROJECTION_H1_AUTO_MAX_DURATION_MILLIS + 1,
            false
        ),
        ProjectionAutomationEligibility::H2aRequired(ProjectionH2aReason::EstimatedDurationTooLong)
    );
}

#[test]
fn a_small_fast_incremental_conflict_free_change_set_is_h1_auto() {
    let change_set = one_entry_change_set();
    assert_eq!(
        classify_projection_automation(
            &change_set,
            false,
            PROJECTION_H1_AUTO_MAX_DURATION_MILLIS,
            false
        ),
        ProjectionAutomationEligibility::H1Auto
    );
}

#[test]
fn exactly_at_the_file_ceiling_is_still_h1_auto() {
    let entries = one_entry_change_set().entries;
    let mut at_ceiling = Vec::new();
    for index in 0..PROJECTION_H1_AUTO_MAX_FILES {
        let mut entry = entries[0].clone();
        entry.relative_path = format!("{}-{index}", entry.relative_path);
        at_ceiling.push(entry);
    }
    let change_set = ProjectionChangeSet {
        entries: at_ceiling,
    };

    assert_eq!(
        classify_projection_automation(&change_set, false, 0, false),
        ProjectionAutomationEligibility::H1Auto
    );
}

/// A removed record is gone from the Ledger, so the baseline manifest is
/// the only remaining source of its classification. The change entry must
/// carry that published classification rather than a default -- the H2a
/// preview covering the deletion has to bind an honest value.
#[test]
fn a_removed_entry_carries_the_classification_from_the_published_baseline() {
    let mut restricted = product("product-1", 1);
    restricted.classification = DataClassification::Restricted;
    let published = generate_projection_artifact_set(&snapshot_with_products(vec![restricted]))
        .manifest_entries();
    assert_eq!(published[0].classification, DataClassification::Restricted);

    let fresh = generate_projection_artifact_set(&snapshot_with_products(Vec::new()));
    let change_set = diff_against_published(&fresh, &published);

    assert_eq!(change_set.len(), 1);
    assert_eq!(
        change_set.entries[0].change,
        ProjectionArtifactChange::Removed
    );
    assert_eq!(
        change_set.entries[0].classification,
        DataClassification::Restricted
    );
}

/// A created/updated entry takes its classification from the freshly
/// generated artifact, not from the baseline.
#[test]
fn a_created_entry_carries_the_freshly_generated_classification() {
    let mut confidential = product("product-1", 1);
    confidential.classification = DataClassification::Confidential;
    let fresh = generate_projection_artifact_set(&snapshot_with_products(vec![confidential]));

    let change_set = diff_against_published(&fresh, &[]);

    assert_eq!(
        change_set.entries[0].change,
        ProjectionArtifactChange::Created
    );
    assert_eq!(
        change_set.entries[0].classification,
        DataClassification::Confidential
    );
}

/// The diff must not depend on the baseline arriving sorted: it is indexed
/// by path, so an out-of-order published slice produces the same result.
#[test]
fn an_unsorted_published_baseline_still_diffs_correctly() {
    let snapshot = snapshot_with_products(vec![product("product-1", 1), product("product-2", 1)]);
    let published_set = generate_projection_artifact_set(&snapshot);
    let mut published = published_set.manifest_entries();
    published.reverse();

    let change_set = diff_against_published(&published_set, &published);

    assert!(
        change_set.is_empty(),
        "self-diff must be empty regardless of baseline order"
    );
}
