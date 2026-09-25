//! H2a governance for Projection rebuild publish.

use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{CorrelationId, IdempotencyId, PreparedIntentId};
use pmc_domain::managed_projection_rebuild::{
    execute_rebuild_managed_projections, prepare_rebuild_managed_projections,
    ManagedProjectionRebuildApproval, ManagedProjectionRebuildChangeEntry,
    ManagedProjectionRebuildChangeKind, ManagedProjectionRebuildError,
    ManagedProjectionRebuildPayloadDigest, ManagedProjectionRebuildPreparedIntent,
    ManagedProjectionRebuildRecordKind, OperationContext, PrepareRebuildManagedProjections,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::WorkManagementRationale;

fn change_entry(path: &str, id: &str) -> ManagedProjectionRebuildChangeEntry {
    ManagedProjectionRebuildChangeEntry {
        relative_path: path.to_string(),
        record_kind: ManagedProjectionRebuildRecordKind::Product,
        record_id: id.to_string(),
        change: ManagedProjectionRebuildChangeKind::Created,
        classification: DataClassification::Internal,
        expected_final_file_sha256: Some("a".repeat(64)),
    }
}

fn context(idempotency: &str, correlation: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn intent(changes: Vec<ManagedProjectionRebuildChangeEntry>) -> PrepareRebuildManagedProjections {
    PrepareRebuildManagedProjections {
        ledger_schema_version: 40,
        ledger_revision: 100,
        published_baseline_digest: "b".repeat(64),
        changes,
        estimated_duration_millis: 5_000,
        rationale: WorkManagementRationale::parse("Weekly incremental Projection refresh").unwrap(),
        context: context("prepare-1", "correlation-1"),
    }
}

#[test]
fn prepare_rejects_an_empty_change_set() {
    let result = prepare_rebuild_managed_projections(
        &intent(Vec::new()),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    );
    assert_eq!(result, Err(ManagedProjectionRebuildError::EmptyChangeSet));
}

#[test]
fn prepare_rejects_an_unsorted_change_set() {
    let changes = vec![
        change_entry("Products/product-2--x.md", "product-2"),
        change_entry("Products/product-1--x.md", "product-1"),
    ];
    let result = prepare_rebuild_managed_projections(
        &intent(changes),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    );
    assert_eq!(
        result,
        Err(ManagedProjectionRebuildError::ChangesNotCanonical)
    );
}

#[test]
fn prepare_rejects_a_duplicate_path() {
    let changes = vec![
        change_entry("Products/product-1--x.md", "product-1"),
        change_entry("Products/product-1--x.md", "product-1"),
    ];
    let result = prepare_rebuild_managed_projections(
        &intent(changes),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    );
    assert_eq!(
        result,
        Err(ManagedProjectionRebuildError::ChangesNotCanonical)
    );
}

#[test]
fn prepare_succeeds_and_binds_the_expiry_ttl() {
    let changes = vec![change_entry("Products/product-1--x.md", "product-1")];
    let prepared = prepare_rebuild_managed_projections(
        &intent(changes),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    assert_eq!(prepared.preview().expires_at.unix_millis(), 1_000 + 300_000);
    assert_eq!(prepared.created_at().unix_millis(), 1_000);
}

#[test]
fn two_prepares_over_identical_inputs_produce_identical_digests() {
    let changes = vec![change_entry("Products/product-1--x.md", "product-1")];
    let first = prepare_rebuild_managed_projections(
        &intent(changes.clone()),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    let second = prepare_rebuild_managed_projections(
        &intent(changes),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    assert_eq!(
        first.payload_digest().as_str(),
        second.payload_digest().as_str()
    );
}

#[test]
fn a_different_ledger_revision_changes_the_digest() {
    let changes = vec![change_entry("Products/product-1--x.md", "product-1")];
    let base = prepare_rebuild_managed_projections(
        &intent(changes.clone()),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    let mut different = intent(changes);
    different.ledger_revision = 101;
    let other = prepare_rebuild_managed_projections(
        &different,
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    assert_ne!(
        base.payload_digest().as_str(),
        other.payload_digest().as_str()
    );
}

#[test]
fn approval_rejects_a_non_head_of_products_actor() {
    let result = ManagedProjectionRebuildApproval::new(
        PreparedIntentId::parse("prepared-1").unwrap(),
        AuditActor::PolicyAuthorizedSystem,
        ManagedProjectionRebuildPayloadDigest::from_persisted("c".repeat(64)).unwrap(),
        IdempotencyId::parse("execute-1").unwrap(),
        true,
    );
    assert_eq!(
        result,
        Err(ManagedProjectionRebuildError::UnauthorizedActor)
    );
}

#[test]
fn approval_rejects_missing_confirmation() {
    let result = ManagedProjectionRebuildApproval::new(
        PreparedIntentId::parse("prepared-1").unwrap(),
        AuditActor::HeadOfProducts,
        ManagedProjectionRebuildPayloadDigest::from_persisted("c".repeat(64)).unwrap(),
        IdempotencyId::parse("execute-1").unwrap(),
        false,
    );
    assert_eq!(
        result,
        Err(ManagedProjectionRebuildError::MissingConfirmation)
    );
}

fn prepared_and_approval() -> (
    ManagedProjectionRebuildPreparedIntent,
    ManagedProjectionRebuildApproval,
) {
    let changes = vec![change_entry("Products/product-1--x.md", "product-1")];
    let prepared = prepare_rebuild_managed_projections(
        &intent(changes),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    let approval = ManagedProjectionRebuildApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("execute-1").unwrap(),
        true,
    )
    .unwrap();
    (prepared, approval)
}

#[test]
fn execute_succeeds_when_current_state_matches_the_preview_exactly() {
    let (prepared, approval) = prepared_and_approval();
    let current_changes = prepared.preview().changes.clone();

    let effects = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        100,
        &"b".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_100),
    )
    .unwrap();

    assert_eq!(effects.changes, prepared.preview().changes);
}

#[test]
fn execute_rejects_a_mismatched_prepared_intent() {
    let (prepared, mut approval) = prepared_and_approval();
    approval = ManagedProjectionRebuildApproval::new(
        PreparedIntentId::parse("some-other-prepared-id").unwrap(),
        approval.actor(),
        approval.acknowledged_payload_digest().clone(),
        approval.idempotency_id().clone(),
        true,
    )
    .unwrap();
    let current_changes = prepared.preview().changes.clone();

    let result = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        100,
        &"b".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_100),
    );

    assert_eq!(
        result,
        Err(ManagedProjectionRebuildError::PreparedIntentMismatch)
    );
}

#[test]
fn execute_rejects_a_mismatched_acknowledged_digest() {
    let (prepared, mut approval) = prepared_and_approval();
    approval = ManagedProjectionRebuildApproval::new(
        approval.prepared_id().clone(),
        approval.actor(),
        ManagedProjectionRebuildPayloadDigest::from_persisted("d".repeat(64)).unwrap(),
        approval.idempotency_id().clone(),
        true,
    )
    .unwrap();
    let current_changes = prepared.preview().changes.clone();

    let result = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        100,
        &"b".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_100),
    );

    assert_eq!(result, Err(ManagedProjectionRebuildError::DigestMismatch));
}

#[test]
fn execute_rejects_after_expiry() {
    let (prepared, approval) = prepared_and_approval();
    let current_changes = prepared.preview().changes.clone();

    let result = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        100,
        &"b".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_000 + 300_000),
    );

    assert_eq!(result, Err(ManagedProjectionRebuildError::Expired));
}

#[test]
fn execute_rejects_a_drifted_ledger_revision() {
    let (prepared, approval) = prepared_and_approval();
    let current_changes = prepared.preview().changes.clone();

    let result = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        101,
        &"b".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_100),
    );

    assert_eq!(result, Err(ManagedProjectionRebuildError::PreviewChanged));
}

#[test]
fn execute_rejects_a_drifted_published_baseline() {
    let (prepared, approval) = prepared_and_approval();
    let current_changes = prepared.preview().changes.clone();

    let result = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        100,
        &"e".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_100),
    );

    assert_eq!(result, Err(ManagedProjectionRebuildError::PreviewChanged));
}

#[test]
fn execute_rejects_a_drifted_change_set() {
    let (prepared, approval) = prepared_and_approval();
    let mut current_changes = prepared.preview().changes.clone();
    current_changes.push(change_entry("Products/product-2--y.md", "product-2"));

    let result = execute_rebuild_managed_projections(
        &approval,
        &prepared,
        40,
        100,
        &"b".repeat(64),
        &current_changes,
        UtcTimestamp::from_unix_millis(1_100),
    );

    assert_eq!(result, Err(ManagedProjectionRebuildError::PreviewChanged));
}

/// The operation's effective classification is derived, never supplied:
/// it is the most restrictive value across every file the rebuild touches,
/// so one Restricted file cannot be published under an Internal label.
#[test]
fn the_preview_classification_is_the_most_restrictive_across_all_changes() {
    let mut public = change_entry("Products/product-1--x.md", "product-1");
    public.classification = DataClassification::Public;
    let mut restricted = change_entry("Products/product-2--x.md", "product-2");
    restricted.classification = DataClassification::Restricted;

    let prepared = prepare_rebuild_managed_projections(
        &intent(vec![public, restricted]),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    assert_eq!(
        prepared.preview().classification,
        DataClassification::Restricted
    );
}

/// `combine` treats Unclassified as most restrictive, so an unclassified
/// file must force the whole rebuild to Unclassified rather than letting it
/// publish under a weaker label.
#[test]
fn an_unclassified_change_forces_the_whole_rebuild_to_unclassified() {
    let mut confidential = change_entry("Products/product-1--x.md", "product-1");
    confidential.classification = DataClassification::Confidential;
    let mut unclassified = change_entry("Products/product-2--x.md", "product-2");
    unclassified.classification = DataClassification::Unclassified;

    let prepared = prepare_rebuild_managed_projections(
        &intent(vec![confidential, unclassified]),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    assert_eq!(
        prepared.preview().classification,
        DataClassification::Unclassified
    );
}

#[test]
fn a_changed_entry_classification_changes_the_digest() {
    let base = prepare_rebuild_managed_projections(
        &intent(vec![change_entry("Products/product-1--x.md", "product-1")]),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    let mut reclassified = change_entry("Products/product-1--x.md", "product-1");
    reclassified.classification = DataClassification::Confidential;
    let other = prepare_rebuild_managed_projections(
        &intent(vec![reclassified]),
        PreparedIntentId::parse("prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    assert_ne!(
        base.payload_digest().as_str(),
        other.payload_digest().as_str()
    );
}
