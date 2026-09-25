//! Managed-projection rebuild H2a persistence.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{ApprovalReceiptId, CorrelationId, IdempotencyId, PreparedIntentId},
    managed_projection_rebuild::{
        ApproveAndExecuteRebuildManagedProjections, ManagedProjectionEffectScope,
        ManagedProjectionH1AutoAuthorization, ManagedProjectionManifestEntry,
        ManagedProjectionManifestGeneration, ManagedProjectionPublishItem,
        ManagedProjectionPublishItemState, ManagedProjectionPublishReport,
        ManagedProjectionPublishStatus, ManagedProjectionRebuildApproval,
        ManagedProjectionRebuildChangeEntry, ManagedProjectionRebuildChangeKind,
        ManagedProjectionRebuildPayloadDigest, ManagedProjectionRebuildPreparedIntent,
        ManagedProjectionRebuildRecordKind, OperationContext, PrepareRebuildManagedProjections,
    },
    time::UtcTimestamp,
    work_management::WorkManagementRationale,
};
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-projection-rebuild-{nonce}-{sequence}.sqlite3"
        )))
    }
}

impl Drop for SyntheticLedger {
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            PathBuf::from(format!("{}-wal", self.0.display())),
            PathBuf::from(format!("{}-shm", self.0.display())),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn change(
    path: &str,
    id: &str,
    classification: DataClassification,
) -> ManagedProjectionRebuildChangeEntry {
    ManagedProjectionRebuildChangeEntry {
        relative_path: path.to_string(),
        record_kind: ManagedProjectionRebuildRecordKind::Product,
        record_id: id.to_string(),
        change: ManagedProjectionRebuildChangeKind::Created,
        classification,
        expected_final_file_sha256: Some("a".repeat(64)),
    }
}

fn command(
    idempotency: &str,
    changes: Vec<ManagedProjectionRebuildChangeEntry>,
) -> PrepareRebuildManagedProjections {
    PrepareRebuildManagedProjections {
        ledger_schema_version: 41,
        ledger_revision: 3,
        published_baseline_digest: "b".repeat(64),
        changes,
        estimated_duration_millis: 4_000,
        rationale: WorkManagementRationale::parse("Incremental projection refresh").unwrap(),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse("projection-correlation-1").unwrap(),
        },
    }
}

fn one_change() -> Vec<ManagedProjectionRebuildChangeEntry> {
    vec![change(
        "Products/product-1--aa.md",
        "product-1",
        DataClassification::Internal,
    )]
}

#[test]
fn prepare_persists_the_intent_payload_and_change_set() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let prepared = writer
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", one_change()),
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .expect("a valid rebuild must prepare");

    assert_eq!(prepared.id().as_str(), "projection-prepared-1");
    assert_eq!(prepared.preview().ledger_revision, 3);
    assert_eq!(prepared.preview().changes.len(), 1);
    assert_eq!(
        prepared.preview().classification,
        DataClassification::Internal
    );
}

/// The whole reason this mechanism does not advance `ledger_revision`: the
/// preview binds the revision it was computed from and execute rejects any
/// change, so a prepare that bumped it would invalidate its own preview.
#[test]
fn prepare_does_not_advance_the_ledger_revision() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let before = writer.revision().unwrap();

    writer
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", one_change()),
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    assert_eq!(
        writer.revision().unwrap(),
        before,
        "a projection rebuild mutates no aggregate and must not advance the revision"
    );
}

#[test]
fn prepare_survives_restart_and_replays_the_original_outcome() {
    let ledger = SyntheticLedger::new();
    let original = {
        let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
        writer
            .prepare_rebuild_managed_projections(
                command("projection-prepare-1", one_change()),
                PreparedIntentId::parse("projection-prepared-1").unwrap(),
                UtcTimestamp::from_unix_millis(1_000),
            )
            .unwrap()
    };

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replayed = reopened
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", one_change()),
            PreparedIntentId::parse("projection-prepared-does-not-matter").unwrap(),
            UtcTimestamp::from_unix_millis(9_999),
        )
        .expect("an identical replay must return the original outcome");

    assert_eq!(replayed.id().as_str(), original.id().as_str());
    assert_eq!(
        replayed.payload_digest().as_str(),
        original.payload_digest().as_str()
    );
    assert_eq!(replayed.preview(), original.preview());
    assert_eq!(
        replayed.created_at().unix_millis(),
        original.created_at().unix_millis(),
        "a replay must not restamp the original preparation time"
    );
}

#[test]
fn replaying_an_idempotency_id_with_different_scalars_is_a_conflict() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", one_change()),
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let mut different = command("projection-prepare-1", one_change());
    different.ledger_revision = 4;

    assert!(
        writer
            .prepare_rebuild_managed_projections(
                different,
                PreparedIntentId::parse("projection-prepared-2").unwrap(),
                UtcTimestamp::from_unix_millis(2_000),
            )
            .is_err(),
        "the same idempotency id with a different revision must not silently succeed"
    );
}

/// Two rebuilds can share a Ledger revision, baseline and duration yet
/// touch entirely different files. Comparing only the scalars would return
/// a prepared intent for a publish nobody previewed.
#[test]
fn replaying_an_idempotency_id_with_a_different_change_set_is_a_conflict() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", one_change()),
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    let other_files = vec![change(
        "Products/product-9--zz.md",
        "product-9",
        DataClassification::Internal,
    )];

    assert!(
        writer
            .prepare_rebuild_managed_projections(
                command("projection-prepare-1", other_files),
                PreparedIntentId::parse("projection-prepared-2").unwrap(),
                UtcTimestamp::from_unix_millis(2_000),
            )
            .is_err(),
        "identical scalars over a different file set must be a conflict"
    );
}

#[test]
fn prepare_rejects_a_non_canonical_change_set() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let unsorted = vec![
        change(
            "Products/product-2--bb.md",
            "product-2",
            DataClassification::Internal,
        ),
        change(
            "Products/product-1--aa.md",
            "product-1",
            DataClassification::Internal,
        ),
    ];

    assert!(writer
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", unsorted),
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .is_err());
}

/// The persisted effective classification is the folded, most-restrictive
/// value across the whole change set -- it is what the shared
/// `prepared_intents` row carries, and it must not be the first entry's.
#[test]
fn the_persisted_classification_is_the_most_restrictive_across_the_change_set() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let mixed = vec![
        change(
            "Products/product-1--aa.md",
            "product-1",
            DataClassification::Public,
        ),
        change(
            "Products/product-2--bb.md",
            "product-2",
            DataClassification::Restricted,
        ),
    ];

    let prepared = writer
        .prepare_rebuild_managed_projections(
            command("projection-prepare-1", mixed),
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();

    assert_eq!(
        prepared.preview().classification,
        DataClassification::Restricted
    );

    // Prove the folded value was actually persisted rather than only
    // computed in memory: reopen and replay, which reconstructs the
    // preview purely from stored rows.
    drop(writer);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replayed = reopened
        .prepare_rebuild_managed_projections(
            command(
                "projection-prepare-1",
                vec![
                    change(
                        "Products/product-1--aa.md",
                        "product-1",
                        DataClassification::Public,
                    ),
                    change(
                        "Products/product-2--bb.md",
                        "product-2",
                        DataClassification::Restricted,
                    ),
                ],
            ),
            PreparedIntentId::parse("projection-prepared-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .expect("an identical replay must return the original outcome");

    assert_eq!(
        replayed.preview().classification,
        DataClassification::Restricted
    );
}

fn approval_for(
    prepared: &ManagedProjectionRebuildPreparedIntent,
    idempotency: &str,
) -> ApproveAndExecuteRebuildManagedProjections {
    ApproveAndExecuteRebuildManagedProjections {
        approval: ManagedProjectionRebuildApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse(idempotency).unwrap(),
            true,
        )
        .unwrap(),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse("projection-correlation-2").unwrap(),
        },
    }
}

/// The preview binds the Ledger revision it was computed from, so a fixture
/// must prepare against the ledger's real revision or execute would
/// legitimately reject it as drift.
fn prepared_ledger(
    changes: Vec<ManagedProjectionRebuildChangeEntry>,
) -> (
    SyntheticLedger,
    SqliteProductLedger,
    ManagedProjectionRebuildPreparedIntent,
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    // Read both from the ledger rather than hardcoding: execute revalidates
    // the preview against the live schema version and revision, so a
    // literal here would silently rot on the next schema bump.
    let revision = writer.revision().unwrap();
    let mut cmd = command("projection-prepare-1", changes);
    cmd.ledger_schema_version = writer.schema_version();
    cmd.ledger_revision = revision;
    let prepared = writer
        .prepare_rebuild_managed_projections(
            cmd,
            PreparedIntentId::parse("projection-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    (ledger, writer, prepared)
}

fn open_publication(
    writer: &mut SqliteProductLedger,
    prepared: &ManagedProjectionRebuildPreparedIntent,
) {
    writer
        .approve_and_execute_rebuild_managed_projections(
            approval_for(prepared, "projection-execute-1"),
            ApprovalReceiptId::parse("projection-receipt-1").unwrap(),
            "b".repeat(64),
            prepared.preview().changes.clone(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .expect("an unchanged preview must execute");
}

fn execute_context() -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse("projection-execute-1").unwrap(),
        correlation_id: CorrelationId::parse("projection-correlation-2").unwrap(),
    }
}

#[test]
fn execute_returns_exactly_the_previewed_change_set() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());

    let effects = writer
        .approve_and_execute_rebuild_managed_projections(
            approval_for(&prepared, "projection-execute-1"),
            ApprovalReceiptId::parse("projection-receipt-1").unwrap(),
            "b".repeat(64),
            prepared.preview().changes.clone(),
            UtcTimestamp::from_unix_millis(1_100),
        )
        .expect("an unchanged preview must execute");

    assert_eq!(effects.changes, prepared.preview().changes);
}

/// Execute writes no files, so a crash before completion must leave a
/// durable record of what was intended rather than an invisible half-state.
#[test]
fn execute_leaves_the_operation_publishing_until_completed() {
    let (ledger, mut writer, prepared) = prepared_ledger(one_change());
    open_publication(&mut writer, &prepared);
    drop(writer);

    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let status: String = raw
        .query_row(
            "SELECT status FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id='projection-execute-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "publishing");
    let untouched: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM rebuild_managed_projections_operation_items WHERE idempotency_id='projection-execute-1' AND state='untouched'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(untouched, 1);
}

/// The receipt is minted already consumed: the durable publication begins
/// in the same transaction, so a still-spendable receipt would be a lie.
#[test]
fn execute_mints_an_already_consumed_receipt_and_consumes_the_intent() {
    let (ledger, mut writer, prepared) = prepared_ledger(one_change());
    open_publication(&mut writer, &prepared);
    drop(writer);

    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let receipt_consumed: i64 = raw
        .query_row(
            "SELECT consumed_at IS NOT NULL FROM approval_receipts WHERE id='projection-receipt-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(receipt_consumed, 1);
    let intent_consumed: i64 = raw
        .query_row(
            "SELECT consumed_at IS NOT NULL FROM prepared_intents WHERE id='projection-prepared-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(intent_consumed, 1);
}

#[test]
fn execute_rejects_a_drifted_change_set() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    let mut drifted = prepared.preview().changes.clone();
    drifted.push(change(
        "Products/product-9--zz.md",
        "product-9",
        DataClassification::Internal,
    ));

    assert!(writer
        .approve_and_execute_rebuild_managed_projections(
            approval_for(&prepared, "projection-execute-1"),
            ApprovalReceiptId::parse("projection-receipt-1").unwrap(),
            "b".repeat(64),
            drifted,
            UtcTimestamp::from_unix_millis(1_100),
        )
        .is_err());
}

#[test]
fn a_fully_verified_publication_switches_the_head_to_verified() {
    let (ledger, mut writer, prepared) = prepared_ledger(one_change());
    let path = prepared.preview().changes[0].relative_path.clone();
    open_publication(&mut writer, &prepared);

    let scope = writer
        .complete_rebuild_managed_projections(
            execute_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Succeeded,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: path.clone(),
                    state: ManagedProjectionPublishItemState::Committed,
                }],
                manifest: Some(ManagedProjectionManifestGeneration {
                    manifest_id: "manifest-1".to_string(),
                    ledger_schema_version: 41,
                    ledger_revision: 1,
                    manifest_digest: "c".repeat(64),
                    entries: vec![ManagedProjectionManifestEntry {
                        relative_path: path,
                        record_kind: ManagedProjectionRebuildRecordKind::Product,
                        record_id: "product-1".to_string(),
                        source_revision: 1,
                        classification: DataClassification::Internal,
                        managed_payload_sha256: "d".repeat(64),
                        final_file_sha256: "a".repeat(64),
                    }],
                }),
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .expect("a fully committed publication must complete");

    assert_eq!(scope, ManagedProjectionEffectScope::Complete);
    drop(writer);
    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let head: (String, String) = raw
        .query_row(
            "SELECT manifest_id,integrity_state FROM projection_manifest_head WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(head, ("manifest-1".to_string(), "verified".to_string()));
}

/// ADR 0007: Synchronized is an integrity result, not a publish-attempt
/// result. A partial publication must stay out-of-sync and must record no
/// manifest generation, so the previous verified one remains the baseline.
#[test]
fn a_partial_publication_stays_out_of_sync_and_records_no_manifest() {
    let two = vec![
        change(
            "Products/product-1--aa.md",
            "product-1",
            DataClassification::Internal,
        ),
        change(
            "Products/product-2--bb.md",
            "product-2",
            DataClassification::Internal,
        ),
    ];
    let (ledger, mut writer, prepared) = prepared_ledger(two.clone());
    open_publication(&mut writer, &prepared);

    let scope = writer
        .complete_rebuild_managed_projections(
            execute_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Failed,
                items: vec![
                    ManagedProjectionPublishItem {
                        relative_path: two[0].relative_path.clone(),
                        state: ManagedProjectionPublishItemState::Committed,
                    },
                    ManagedProjectionPublishItem {
                        relative_path: two[1].relative_path.clone(),
                        state: ManagedProjectionPublishItemState::Failed,
                    },
                ],
                manifest: None,
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();

    assert_eq!(scope, ManagedProjectionEffectScope::Partial);
    drop(writer);
    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let state: String = raw
        .query_row(
            "SELECT integrity_state FROM projection_manifest_head WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "out_of_sync");
    let generations: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM projection_manifest_generations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        generations, 0,
        "a partial publication must not record a verified generation"
    );
}

/// Reporting a path nobody planned would misreport what the approved
/// operation actually touched.
#[test]
fn completing_with_an_unplanned_path_is_rejected() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    open_publication(&mut writer, &prepared);

    assert!(writer
        .complete_rebuild_managed_projections(
            execute_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Succeeded,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: "Products/never-planned--xx.md".to_string(),
                    state: ManagedProjectionPublishItemState::Committed,
                }],
                manifest: None,
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .is_err());
}

#[test]
fn an_already_terminal_operation_cannot_be_completed_twice() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    let path = prepared.preview().changes[0].relative_path.clone();
    open_publication(&mut writer, &prepared);

    let report = || ManagedProjectionPublishReport {
        status: ManagedProjectionPublishStatus::Failed,
        items: vec![ManagedProjectionPublishItem {
            relative_path: path.clone(),
            state: ManagedProjectionPublishItemState::Failed,
        }],
        manifest: None,
    };
    writer
        .complete_rebuild_managed_projections(
            execute_context(),
            report(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();

    assert!(writer
        .complete_rebuild_managed_projections(
            execute_context(),
            report(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .is_err());
}

fn h1_auto_context() -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse("projection-h1-1").unwrap(),
        correlation_id: CorrelationId::parse("projection-correlation-3").unwrap(),
    }
}

/// The H1-Auto route (DG0's RebuildManagedProjectionsIncremental) is
/// pre-authorized, so it records a publication with neither a Prepared
/// Intent nor an Approval Receipt -- the shape V42 exists to admit.
#[test]
fn an_h1_auto_publication_records_without_an_intent_or_receipt() {
    // Automation runs from a synchronized baseline, so the fixture reaches
    // one first through the real H2a path.
    let (ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();

    let effects = writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes.clone(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .expect("an in-bounds incremental change set must be admitted");

    assert_eq!(effects.changes, changes);
    drop(writer);
    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let row: (String, String, Option<String>, Option<String>) = raw
        .query_row(
            "SELECT authorization,status,prepared_intent_id,approval_receipt_id FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id='projection-h1-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row.0, "h1_auto");
    assert_eq!(row.1, "publishing");
    assert_eq!(row.2, None);
    assert_eq!(row.3, None);
}

/// The authorization is a witness, not a flag: work beyond ADR 0007's
/// ceiling cannot be constructed as automatable at all.
#[test]
fn work_beyond_the_ceiling_cannot_be_authorized_as_h1_auto() {
    assert!(ManagedProjectionH1AutoAuthorization::authorize(501, 1_000, false, false).is_err());
    assert!(ManagedProjectionH1AutoAuthorization::authorize(1, 30_001, false, false).is_err());
    assert!(
        ManagedProjectionH1AutoAuthorization::authorize(1, 1_000, true, false).is_err(),
        "an initial or full rebuild must escalate to H2a"
    );
    assert!(
        ManagedProjectionH1AutoAuthorization::authorize(1, 1_000, false, true).is_err(),
        "an integrity conflict must escalate to H2a"
    );
    assert!(ManagedProjectionH1AutoAuthorization::authorize(500, 30_000, false, false).is_ok());
}

/// A witness for a small change set must not admit a larger one.
#[test]
fn an_authorization_cannot_be_reused_for_a_bigger_change_set() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(1, 1_000, false, false).unwrap();

    let two = vec![
        change(
            "Products/product-1--aa.md",
            "product-1",
            DataClassification::Internal,
        ),
        change(
            "Products/product-2--bb.md",
            "product-2",
            DataClassification::Internal,
        ),
    ];

    assert!(writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            two,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .is_err());
}

/// ADR 0006 shows Cancelling until a terminal outcome, so requesting
/// cancellation moves the operation to `cancelling` without inventing a
/// terminal state, and the publisher can observe it.
#[test]
fn requesting_cancellation_marks_the_operation_and_is_observable() {
    let (_ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    assert!(!writer.rebuild_managed_projections_cancellation_requested("projection-h1-1"));

    writer
        .request_rebuild_managed_projections_cancellation(h1_auto_context())
        .expect("a publishing operation must accept a cancellation request");

    assert!(writer.rebuild_managed_projections_cancellation_requested("projection-h1-1"));
    // Idempotent: asking twice is not an error.
    assert!(writer
        .request_rebuild_managed_projections_cancellation(h1_auto_context())
        .is_ok());
}

/// A cancelled publication still completes: it records what landed and
/// leaves the head out-of-sync, exactly like any other partial outcome.
#[test]
fn a_cancelled_operation_can_still_be_completed_and_stays_out_of_sync() {
    let (ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let path = changes[0].relative_path.clone();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    writer
        .request_rebuild_managed_projections_cancellation(h1_auto_context())
        .unwrap();

    let scope = writer
        .complete_rebuild_managed_projections(
            h1_auto_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Cancelled,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: path,
                    state: ManagedProjectionPublishItemState::Untouched,
                }],
                manifest: None,
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .expect("a cancelling operation must be completable");

    assert_eq!(scope, ManagedProjectionEffectScope::None);
    drop(writer);
    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let status: String = raw
        .query_row(
            "SELECT status FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id='projection-h1-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "cancelled");
    let state: String = raw
        .query_row(
            "SELECT integrity_state FROM projection_manifest_head WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "out_of_sync");
}

/// Cancellation is only meaningful while work is in flight.
#[test]
fn cancelling_a_terminal_operation_is_refused() {
    let (_ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let path = changes[0].relative_path.clone();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
    writer
        .complete_rebuild_managed_projections(
            h1_auto_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Succeeded,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: path,
                    state: ManagedProjectionPublishItemState::Committed,
                }],
                manifest: None,
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();

    assert!(writer
        .request_rebuild_managed_projections_cancellation(h1_auto_context())
        .is_err());
}

/// Without exact coverage the effect scope describes the report rather than
/// the operation: mention only the path that committed, out of two, and the
/// operation reads `complete` while the other row silently stays untouched.
#[test]
fn a_report_that_omits_a_planned_path_is_refused() {
    let two = vec![
        change(
            "Products/product-1--aa.md",
            "product-1",
            DataClassification::Internal,
        ),
        change(
            "Products/product-2--bb.md",
            "product-2",
            DataClassification::Internal,
        ),
    ];
    let (ledger, mut writer, prepared) = prepared_ledger(two.clone());
    open_publication(&mut writer, &prepared);

    let outcome = writer.complete_rebuild_managed_projections(
        execute_context(),
        ManagedProjectionPublishReport {
            status: ManagedProjectionPublishStatus::Succeeded,
            items: vec![ManagedProjectionPublishItem {
                relative_path: two[0].relative_path.clone(),
                state: ManagedProjectionPublishItemState::Committed,
            }],
            manifest: None,
        },
        UtcTimestamp::from_unix_millis(1_200),
    );

    assert!(
        outcome.is_err(),
        "a report covering one of two planned paths must be refused"
    );
    drop(writer);
    let raw = rusqlite::Connection::open(&ledger.0).unwrap();
    let status: String = raw
        .query_row(
            "SELECT status FROM rebuild_managed_projections_execute_replay_operations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "publishing",
        "a refused report must leave the operation in flight, not half-terminal"
    );
}

/// Naming one path twice would cover the planned count while leaving
/// another row unreported.
#[test]
fn a_report_that_names_one_path_twice_is_refused() {
    let two = vec![
        change(
            "Products/product-1--aa.md",
            "product-1",
            DataClassification::Internal,
        ),
        change(
            "Products/product-2--bb.md",
            "product-2",
            DataClassification::Internal,
        ),
    ];
    let (_ledger, mut writer, prepared) = prepared_ledger(two.clone());
    open_publication(&mut writer, &prepared);

    let outcome = writer.complete_rebuild_managed_projections(
        execute_context(),
        ManagedProjectionPublishReport {
            status: ManagedProjectionPublishStatus::Succeeded,
            items: vec![
                ManagedProjectionPublishItem {
                    relative_path: two[0].relative_path.clone(),
                    state: ManagedProjectionPublishItemState::Committed,
                },
                ManagedProjectionPublishItem {
                    relative_path: two[0].relative_path.clone(),
                    state: ManagedProjectionPublishItemState::Committed,
                },
            ],
            manifest: None,
        },
        UtcTimestamp::from_unix_millis(1_200),
    );

    assert!(outcome.is_err());
}

/// Drives a real H2a publication to a verified head, so a test can start
/// from the synchronized baseline H1-Auto now requires. Seeded through the
/// real path rather than by raw insert: the schema only admits a verified
/// head that points at an actual manifest generation.
fn ledger_with_verified_head() -> (SyntheticLedger, SqliteProductLedger) {
    let (ledger, mut writer, prepared) = prepared_ledger(one_change());
    let path = prepared.preview().changes[0].relative_path.clone();
    open_publication(&mut writer, &prepared);
    writer
        .complete_rebuild_managed_projections(
            execute_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Succeeded,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: path.clone(),
                    state: ManagedProjectionPublishItemState::Committed,
                }],
                manifest: Some(ManagedProjectionManifestGeneration {
                    manifest_id: "baseline-manifest".to_string(),
                    ledger_schema_version: 41,
                    ledger_revision: 1,
                    manifest_digest: "c".repeat(64),
                    entries: vec![ManagedProjectionManifestEntry {
                        relative_path: path,
                        record_kind: ManagedProjectionRebuildRecordKind::Product,
                        record_id: "product-1".to_string(),
                        source_revision: 1,
                        classification: DataClassification::Internal,
                        managed_payload_sha256: "d".repeat(64),
                        final_file_sha256: "a".repeat(64),
                    }],
                }),
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .expect("the baseline publication must complete");
    (ledger, writer)
}

/// ADR 0007 forbids batching around approval. A per-operation ceiling alone
/// cannot enforce that, so automation may only run from a synchronized
/// baseline -- and a half-published diff never reaches one.
#[test]
fn h1_auto_is_refused_when_the_projection_is_out_of_sync() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    let path = prepared.preview().changes[0].relative_path.clone();
    open_publication(&mut writer, &prepared);
    writer
        .complete_rebuild_managed_projections(
            execute_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Failed,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: path,
                    state: ManagedProjectionPublishItemState::Failed,
                }],
                manifest: None,
            },
            UtcTimestamp::from_unix_millis(1_200),
        )
        .expect("a failed publication must still complete");

    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();

    assert!(
        writer
            .begin_h1_auto_rebuild_managed_projections(
                h1_auto_context(),
                authorization,
                changes,
                UtcTimestamp::from_unix_millis(1_300),
            )
            .is_err(),
        "an out-of-sync projection is an integrity conflict and must escalate to H2a"
    );
}

/// The first rebuild of all is an initial rebuild, which ADR 0007 sends to
/// H2a. With no head at all there is no synchronized baseline to start from.
#[test]
fn h1_auto_is_refused_when_nothing_has_ever_been_published() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();

    assert!(writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .is_err());
}

/// An idempotency key must bind everything the caller asserted.
///
/// Comparing only the prepared identifier let a replay that agreed on the
/// intent but disagreed on the acknowledged digest be accepted as the
/// original -- a different request wearing the same key.
#[test]
fn replaying_an_execute_with_a_different_acknowledged_digest_is_a_conflict() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    open_publication(&mut writer, &prepared);

    let mut tampered = approval_for(&prepared, "projection-execute-1");
    tampered.approval = ManagedProjectionRebuildApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        // A digest the original approval never acknowledged.
        ManagedProjectionRebuildPayloadDigest::from_persisted("f".repeat(64)).unwrap(),
        IdempotencyId::parse("projection-execute-1").unwrap(),
        true,
    )
    .unwrap();

    let outcome = writer.approve_and_execute_rebuild_managed_projections(
        tampered,
        ApprovalReceiptId::parse("projection-receipt-conflict").unwrap(),
        "b".repeat(64),
        prepared.preview().changes.clone(),
        UtcTimestamp::from_unix_millis(1_200),
    );

    assert!(
        outcome.is_err(),
        "a replay acknowledging a different digest must be an idempotency conflict"
    );
}

/// The same key with a different correlation is also a different request.
#[test]
fn replaying_an_execute_under_a_different_correlation_is_a_conflict() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    open_publication(&mut writer, &prepared);

    let mut different = approval_for(&prepared, "projection-execute-1");
    different.context.correlation_id =
        CorrelationId::parse("projection-correlation-different").unwrap();

    let outcome = writer.approve_and_execute_rebuild_managed_projections(
        different,
        ApprovalReceiptId::parse("projection-receipt-conflict-2").unwrap(),
        "b".repeat(64),
        prepared.preview().changes.clone(),
        UtcTimestamp::from_unix_millis(1_200),
    );

    assert!(outcome.is_err());
}

/// An identical replay must still return the original outcome rather than
/// conflicting -- tightening the binding must not break the retry it exists
/// to serve.
#[test]
fn an_identical_execute_replay_still_returns_the_original_effects() {
    let (_ledger, mut writer, prepared) = prepared_ledger(one_change());
    open_publication(&mut writer, &prepared);

    let effects = writer
        .approve_and_execute_rebuild_managed_projections(
            approval_for(&prepared, "projection-execute-1"),
            ApprovalReceiptId::parse("projection-receipt-1").unwrap(),
            "b".repeat(64),
            prepared.preview().changes.clone(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .expect("an identical replay must replay, not conflict");

    assert_eq!(effects.changes, prepared.preview().changes);
}

// ---------------------------------------------------------------------------
// The head is out of sync from the moment a publication opens, and a
// second publication cannot open while one is in flight.
// ---------------------------------------------------------------------------

fn head_state(path: &std::path::Path) -> String {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT integrity_state FROM projection_manifest_head WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn in_flight_count(path: &std::path::Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM rebuild_managed_projections_execute_replay_operations WHERE status IN ('publishing','cancelling')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn opening_a_publication_marks_the_head_out_of_sync_before_any_file_effect() {
    // ADR 0007 writes files outside the Ledger transaction, so from the
    // moment an operation opens the disk may no longer match the verified
    // manifest. The head must say so at that moment, not only after a
    // completed failure -- otherwise a crash in between leaves "verified"
    // pointing at a partly rewritten tree.
    let (ledger, mut writer) = ledger_with_verified_head();
    assert_eq!(head_state(&ledger.0), "verified");
    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();

    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .expect("an in-bounds change set opens");
    drop(writer);

    assert_eq!(head_state(&ledger.0), "out_of_sync");
}

#[test]
fn a_fully_verified_completion_restores_the_head_after_an_open_marked_it() {
    // Marking at open must not make a successful publication look failed.
    let (ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let path = changes[0].relative_path.clone();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    writer
        .complete_rebuild_managed_projections(
            h1_auto_context(),
            ManagedProjectionPublishReport {
                status: ManagedProjectionPublishStatus::Succeeded,
                items: vec![ManagedProjectionPublishItem {
                    relative_path: path.clone(),
                    state: ManagedProjectionPublishItemState::Committed,
                }],
                manifest: Some(ManagedProjectionManifestGeneration {
                    manifest_id: "manifest-after-open".to_string(),
                    ledger_schema_version: 41,
                    ledger_revision: 1,
                    manifest_digest: "e".repeat(64),
                    entries: vec![ManagedProjectionManifestEntry {
                        relative_path: path,
                        record_kind: ManagedProjectionRebuildRecordKind::Product,
                        record_id: "product-1".to_string(),
                        source_revision: 1,
                        classification: DataClassification::Internal,
                        managed_payload_sha256: "d".repeat(64),
                        final_file_sha256: "a".repeat(64),
                    }],
                }),
            },
            UtcTimestamp::from_unix_millis(2_500),
        )
        .expect("a fully committed publication completes");
    drop(writer);

    assert_eq!(head_state(&ledger.0), "verified");
}

#[test]
fn a_second_publication_cannot_open_while_one_is_in_flight() {
    // Two publications writing the same tree outside any transaction would
    // race on disk with no record of which bytes won. The H2a route does
    // not gate on the head, so this exercises the in-flight refusal itself.
    let (ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes.clone(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();

    let mut cmd = command("projection-prepare-2", changes.clone());
    cmd.ledger_schema_version = writer.schema_version();
    cmd.ledger_revision = writer.revision().unwrap();
    let prepared = writer
        .prepare_rebuild_managed_projections(
            cmd,
            PreparedIntentId::parse("projection-prepared-2").unwrap(),
            UtcTimestamp::from_unix_millis(2_100),
        )
        .expect("preparing is a read of current state and may proceed");

    let refused = writer.approve_and_execute_rebuild_managed_projections(
        approval_for(&prepared, "projection-execute-2"),
        ApprovalReceiptId::parse("projection-receipt-2").unwrap(),
        "b".repeat(64),
        prepared.preview().changes.clone(),
        UtcTimestamp::from_unix_millis(2_200),
    );

    assert!(
        refused.is_err(),
        "a second publication opened beside one in flight"
    );
    drop(writer);
    assert_eq!(in_flight_count(&ledger.0), 1);
}

#[test]
fn a_crashed_publication_leaves_the_head_out_of_sync_after_reopen() {
    // The crash is simulated by dropping the handle with the operation open.
    // On reopen the head must not read as verified, and automation must not
    // be permitted to start on top of the half-written tree.
    let (ledger, mut writer) = ledger_with_verified_head();
    let changes = one_change();
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    writer
        .begin_h1_auto_rebuild_managed_projections(
            h1_auto_context(),
            authorization,
            changes.clone(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    drop(writer);

    let mut reopened = SqliteProductLedger::open(&ledger.0).expect("the Ledger reopens");
    assert_eq!(head_state(&ledger.0), "out_of_sync");
    assert_eq!(in_flight_count(&ledger.0), 1);
    let authorization =
        ManagedProjectionH1AutoAuthorization::authorize(changes.len(), 1_000, false, false)
            .unwrap();
    let refused = reopened.begin_h1_auto_rebuild_managed_projections(
        OperationContext {
            idempotency_id: IdempotencyId::parse("projection-h1-2").unwrap(),
            correlation_id: CorrelationId::parse("projection-correlation-4").unwrap(),
        },
        authorization,
        changes,
        UtcTimestamp::from_unix_millis(3_000),
    );
    assert!(
        refused.is_err(),
        "automation started on top of a crashed publication"
    );
}
