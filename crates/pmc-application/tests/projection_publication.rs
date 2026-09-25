//! Managed-projection publication orchestration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::projection_publication::{
    attach_verified_manifest, change_kind_of, h2a_change_entries, manifest_entries_of,
    publish_change_set, record_kind_of, CancellationSignal, NeverCancelled,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{AggregateVersion, ProductId};
use pmc_domain::managed_projection_rebuild::{
    ManagedProjectionPublishItem, ManagedProjectionPublishItemState,
    ManagedProjectionPublishReport, ManagedProjectionPublishStatus,
    ManagedProjectionRebuildChangeKind, ManagedProjectionRebuildRecordKind,
};
use pmc_domain::projection_source::{LedgerProjectionSnapshot, ProductProjectionSource};
use pmc_domain::time::UtcTimestamp;
use pmc_knowledge::projections::{
    diff_against_published, generate_projection_artifact_set, ProjectionArtifactChange,
    ProjectionRecordType,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct ManagedRoot(PathBuf);

impl ManagedRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pmc-publish-{nonce}-{sequence}"));
        fs::create_dir_all(&root).expect("managed root must be creatable");
        Self(fs::canonicalize(&root).expect("managed root must canonicalize"))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ManagedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn product(id: &str, revision: u64) -> ProductProjectionSource {
    ProductProjectionSource {
        id: ProductId::parse(id).unwrap(),
        classification: DataClassification::Internal,
        source_revision: AggregateVersion::new(revision).unwrap(),
    }
}

fn snapshot(products: Vec<ProductProjectionSource>) -> LedgerProjectionSnapshot {
    LedgerProjectionSnapshot {
        schema_version: 42,
        ledger_revision: 7,
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
fn every_record_kind_and_change_kind_maps_across_the_two_vocabularies() {
    assert_eq!(
        record_kind_of(ProjectionRecordType::Product),
        ManagedProjectionRebuildRecordKind::Product
    );
    assert_eq!(
        record_kind_of(ProjectionRecordType::Kpi),
        ManagedProjectionRebuildRecordKind::Kpi
    );
    assert_eq!(
        change_kind_of(ProjectionArtifactChange::Removed),
        ManagedProjectionRebuildChangeKind::Removed
    );
}

#[test]
fn created_entries_carry_the_generated_hash_and_removed_entries_carry_none() {
    let published_set = generate_projection_artifact_set(&snapshot(vec![product("product-2", 1)]));
    let published = published_set.manifest_entries();
    let fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let change_set = diff_against_published(&fresh, &published);

    let entries = h2a_change_entries(&change_set, &fresh);

    assert_eq!(entries.len(), 2);
    let created = entries
        .iter()
        .find(|entry| entry.change == ManagedProjectionRebuildChangeKind::Created)
        .unwrap();
    let removed = entries
        .iter()
        .find(|entry| entry.change == ManagedProjectionRebuildChangeKind::Removed)
        .unwrap();
    assert_eq!(
        created.expected_final_file_sha256.as_deref(),
        Some(
            fresh
                .artifacts
                .iter()
                .find(|artifact| artifact.relative_path == created.relative_path)
                .unwrap()
                .final_file_sha256
                .as_str()
        )
    );
    assert_eq!(removed.expected_final_file_sha256, None);
}

#[test]
fn publishing_writes_every_planned_file_and_reports_them_committed() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![
        product("product-1", 1),
        product("product-2", 1),
    ]));
    let change_set = diff_against_published(&fresh, &[]);
    let entries = h2a_change_entries(&change_set, &fresh);

    let report = publish_change_set(root.path(), &fresh, &entries, "op-1", &NeverCancelled);

    assert_eq!(report.status, ManagedProjectionPublishStatus::Succeeded);
    assert_eq!(report.items.len(), 2);
    assert!(report
        .items
        .iter()
        .all(|item| item.state == ManagedProjectionPublishItemState::Committed));
    for artifact in &fresh.artifacts {
        let written = fs::read(root.path().join(&artifact.relative_path)).unwrap();
        assert_eq!(
            written, artifact.content,
            "the published bytes must be exactly what was generated and hashed"
        );
    }
    assert!(
        report.manifest.is_none(),
        "the publisher never mints a manifest; only a verified caller may"
    );
}

#[test]
fn a_removal_deletes_the_published_file() {
    let root = ManagedRoot::new();
    let first = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let entries = h2a_change_entries(&diff_against_published(&first, &[]), &first);
    let seeded = publish_change_set(root.path(), &first, &entries, "op-1", &NeverCancelled);
    assert_eq!(seeded.status, ManagedProjectionPublishStatus::Succeeded);
    let path = root.path().join(&first.artifacts[0].relative_path);
    assert!(path.exists());

    let empty = generate_projection_artifact_set(&snapshot(Vec::new()));
    let removal = h2a_change_entries(
        &diff_against_published(&empty, &first.manifest_entries()),
        &empty,
    );
    let report = publish_change_set(root.path(), &empty, &removal, "op-2", &NeverCancelled);

    assert_eq!(report.status, ManagedProjectionPublishStatus::Succeeded);
    assert!(!path.exists());
}

/// ADR 0007 permits stopping only at enumerated safe boundaries, and the
/// boundary is between exact-path publications. A failure must therefore
/// leave every later path Untouched rather than pressing on, and the
/// report must distinguish committed, failed and untouched so a repair
/// knows exactly where it stands.
#[test]
fn a_failure_stops_at_the_safe_boundary_and_reports_all_three_states() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![
        product("product-1", 1),
        product("product-2", 1),
        product("product-3", 1),
    ]));
    let mut entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);
    // Make the middle entry unpublishable by naming a file the generator
    // never produced: the publisher must fail it rather than skip it.
    entries[1].relative_path = "Products/not-generated--zz.md".to_string();

    let report = publish_change_set(root.path(), &fresh, &entries, "op-1", &NeverCancelled);

    assert_eq!(report.status, ManagedProjectionPublishStatus::Failed);
    assert_eq!(
        report.items[0].state,
        ManagedProjectionPublishItemState::Committed
    );
    assert_eq!(
        report.items[1].state,
        ManagedProjectionPublishItemState::Failed
    );
    assert_eq!(
        report.items[2].state,
        ManagedProjectionPublishItemState::Untouched,
        "publication must stop at the boundary, not press on past a failure"
    );
    assert!(
        report.manifest.is_none(),
        "a partial publication must not offer a manifest generation"
    );
}

#[test]
fn manifest_entries_carry_the_generated_hashes_and_classification() {
    let fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 3)]));

    let entries = manifest_entries_of(&fresh);

    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].record_kind,
        ManagedProjectionRebuildRecordKind::Product
    );
    assert_eq!(entries[0].record_id, "product-1");
    assert_eq!(entries[0].source_revision, 3);
    assert_eq!(entries[0].classification, DataClassification::Internal);
    assert_eq!(
        entries[0].final_file_sha256,
        fresh.artifacts[0].final_file_sha256
    );
}

/// A signal that asks to stop once the given number of boundaries have been
/// reached, so a test can cancel partway rather than before anything runs.
struct CancelAfter {
    boundaries: std::cell::Cell<usize>,
    cancel_at: usize,
}

impl CancelAfter {
    fn new(cancel_at: usize) -> Self {
        Self {
            boundaries: std::cell::Cell::new(0),
            cancel_at,
        }
    }
}

impl CancellationSignal for CancelAfter {
    fn is_cancellation_requested(&self) -> bool {
        let seen = self.boundaries.get();
        self.boundaries.set(seen + 1);
        seen >= self.cancel_at
    }
}

/// ADR 0006 keeps status and effect scope orthogonal: a cancelled operation
/// still enumerates what committed, and what was never attempted.
#[test]
fn cancelling_partway_stops_at_the_boundary_and_reports_cancelled_not_failed() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![
        product("product-1", 1),
        product("product-2", 1),
        product("product-3", 1),
    ]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);

    let report = publish_change_set(root.path(), &fresh, &entries, "op-1", &CancelAfter::new(1));

    assert_eq!(
        report.status,
        ManagedProjectionPublishStatus::Cancelled,
        "a cancelled publication is not a failed one"
    );
    assert_eq!(
        report.items[0].state,
        ManagedProjectionPublishItemState::Committed
    );
    assert_eq!(
        report.items[1].state,
        ManagedProjectionPublishItemState::Untouched
    );
    assert_eq!(
        report.items[2].state,
        ManagedProjectionPublishItemState::Untouched
    );
    assert!(report.manifest.is_none());
}

/// A file is never abandoned half-written: the question is only ever asked
/// between whole publications, so whatever committed before the cancellation
/// is complete and byte-exact.
#[test]
fn a_cancellation_never_leaves_a_partially_written_file() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![
        product("product-1", 1),
        product("product-2", 1),
    ]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);

    let report = publish_change_set(root.path(), &fresh, &entries, "op-1", &CancelAfter::new(1));

    let committed = report
        .items
        .iter()
        .filter(|item| item.state == ManagedProjectionPublishItemState::Committed)
        .count();
    assert_eq!(committed, 1);
    let artifact = fresh
        .artifacts
        .iter()
        .find(|artifact| artifact.relative_path == report.items[0].relative_path)
        .unwrap();
    let written = fs::read(root.path().join(&artifact.relative_path)).unwrap();
    assert_eq!(written, artifact.content);
}

/// Cancelling after the last file still committed everything, so the honest
/// terminal status is success, not cancellation.
#[test]
fn a_cancellation_arriving_after_the_last_file_is_still_a_success() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);

    let report = publish_change_set(root.path(), &fresh, &entries, "op-1", &CancelAfter::new(5));

    assert_eq!(report.status, ManagedProjectionPublishStatus::Succeeded);
}

#[test]
fn tampered_artifact_content_is_refused_and_nothing_is_written() {
    // `ProjectionArtifact::content` is public, so without a re-derivation
    // gate the field allowlist would constrain the generator while the
    // publisher wrote whatever the caller put in the struct.
    let root = ManagedRoot::new();
    let mut fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let change_set = diff_against_published(&fresh, &[]);
    let entries = h2a_change_entries(&change_set, &fresh);
    fresh.artifacts[0]
        .content
        .extend_from_slice(br"leaked-secret: C:\Users\someone\secrets.txt");

    let report = publish_change_set(root.path(), &fresh, &entries, "op-tamper", &NeverCancelled);

    assert_eq!(report.status, ManagedProjectionPublishStatus::Failed);
    assert!(
        report
            .items
            .iter()
            .all(|item| item.state == ManagedProjectionPublishItemState::Untouched),
        "a refusal before publication starts must report every path untouched"
    );
    assert!(
        !root.path().join(&entries[0].relative_path).exists(),
        "no tampered byte may reach the managed subtree"
    );
}

#[test]
fn content_that_does_not_match_the_approved_hash_is_not_published() {
    // The approval binds an exact final-file hash. Publishing a different
    // artifact at the approved path would make approval advisory.
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let change_set = diff_against_published(&fresh, &[]);
    let mut entries = h2a_change_entries(&change_set, &fresh);
    entries[0].expected_final_file_sha256 = Some("0".repeat(64));

    let report = publish_change_set(root.path(), &fresh, &entries, "op-swap", &NeverCancelled);

    assert_eq!(report.status, ManagedProjectionPublishStatus::Failed);
    assert_eq!(
        report.items[0].state,
        ManagedProjectionPublishItemState::Failed
    );
    assert!(
        !root.path().join(&entries[0].relative_path).exists(),
        "unapproved bytes must not land even though the set is self-consistent"
    );
}

#[test]
fn a_fully_verified_publication_mints_the_manifest_that_means_synchronized() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![
        product("product-1", 1),
        product("product-2", 1),
    ]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);

    let report = publish_change_set(root.path(), &fresh, &entries, "op-sync", &NeverCancelled);
    let report = attach_verified_manifest(root.path(), &fresh, report, "manifest-1".to_owned());

    let manifest = report
        .manifest
        .expect("a fully committed and disk-verified publication must mint a manifest");
    assert_eq!(manifest.manifest_id, "manifest-1");
    assert_eq!(manifest.ledger_revision, fresh.ledger_revision);
    assert_eq!(manifest.manifest_digest, fresh.aggregate_manifest_digest());
    assert_eq!(manifest.entries.len(), fresh.artifacts.len());
}

#[test]
fn a_file_edited_after_a_successful_publication_does_not_verify() {
    // The point of verifying against disk rather than against the attempt:
    // every write succeeded, so the attempt says Succeeded, but the subtree
    // no longer holds what was published. That is a manual conflict, and it
    // must read as out of sync rather than synchronized.
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);
    let report = publish_change_set(root.path(), &fresh, &entries, "op-edit", &NeverCancelled);
    assert_eq!(report.status, ManagedProjectionPublishStatus::Succeeded);

    fs::write(
        root.path().join(&fresh.artifacts[0].relative_path),
        b"a human edited this file",
    )
    .expect("the managed file must be writable for the fixture");
    let report = attach_verified_manifest(root.path(), &fresh, report, "manifest-2".to_owned());

    assert!(
        report.manifest.is_none(),
        "a subtree that no longer matches must never mint a manifest"
    );
}

#[test]
fn a_missing_file_that_was_never_part_of_the_change_set_blocks_verification() {
    // Synchronized is a statement about the whole projection, not about the
    // paths this operation happened to touch.
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![
        product("product-1", 1),
        product("product-2", 1),
    ]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);
    let report = publish_change_set(root.path(), &fresh, &entries, "op-gap", &NeverCancelled);

    fs::remove_file(root.path().join(&fresh.artifacts[1].relative_path))
        .expect("the managed file must be removable for the fixture");
    let report = attach_verified_manifest(root.path(), &fresh, report, "manifest-3".to_owned());

    assert!(report.manifest.is_none());
}

#[test]
fn a_partial_publication_cannot_be_talked_into_a_manifest() {
    let root = ManagedRoot::new();
    let fresh = generate_projection_artifact_set(&snapshot(vec![product("product-1", 1)]));
    let entries = h2a_change_entries(&diff_against_published(&fresh, &[]), &fresh);
    let failed = ManagedProjectionPublishReport {
        status: ManagedProjectionPublishStatus::Failed,
        items: vec![ManagedProjectionPublishItem {
            relative_path: entries[0].relative_path.clone(),
            state: ManagedProjectionPublishItemState::Failed,
        }],
        manifest: None,
    };

    let report = attach_verified_manifest(root.path(), &fresh, failed, "manifest-4".to_owned());

    assert!(report.manifest.is_none());
    assert_eq!(report.status, ManagedProjectionPublishStatus::Failed);
}
