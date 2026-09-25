//! Regression coverage for a bug found and fixed 2026-09-02 (same day
//! `LinkActionCompletionEvidence` shipped): `prepare_cancel_action`/
//! `prepare_reopen_action` and their EXECUTE counterparts used to rehydrate
//! `InMemoryActionService` with `DenyActionEvidenceAuthority`, reasoned safe
//! only because `action_completion_evidence` durably held zero rows before
//! `LinkActionCompletionEvidence`'s own SQLite writer existed --
//! `prepare_cancel_action_cause`/`prepare_reopen_action_cause` and the
//! shared `execute_h2` both unconditionally call
//! `authoritative_action_snapshot`, which resolves every linked evidence id
//! regardless of Cancel/Reopen's own business logic. The moment an Action
//! could have linked completion evidence, all four call sites hard-failed
//! with a generic `ledger.persistence_failed` instead of succeeding or
//! cleanly denying. Fixed via a real, SQLite-backed
//! `PersistedActionEvidenceAuthority`.
//!
//! Mirrors `sqlite-action-reopen.rs`'s setup (through Accept), inserting
//! `StartAction` + `LinkActionCompletionEvidence` before Cancel, matching
//! `sqlite-action-link-completion-evidence.rs`'s own evidence-seeding
//! pattern -- but with COMPLETE verification fields
//! (`verification='verified'` needs `last_verified_at`/`integrity_digest` to
//! be a truthful row; the real evidence authority this file exercises
//! parses that shape strictly, unlike `LinkActionCompletionEvidence`'s own
//! writer, which never reads it at all).

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionTitle, ApproveAndExecuteAcceptActionRequest, ApproveAndExecuteCancelAction,
        ApproveAndExecuteReopenAction, CreateActionRequestDraft, LinkActionCompletionEvidence,
        PrepareAcceptActionRequest, PrepareCancelAction, PrepareReopenAction, StartAction,
        SubmitActionRequest,
    },
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, EvidenceReferenceId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ActionReopenMode, ActionState, ApprovalAuthorizationPort, ApprovalConfirmation,
        WorkManagementApproval, WorkManagementOperation, WorkManagementPreparedIntent,
    },
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::Connection;

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
            "pmc-synthetic-action-cancel-reopen-evidence-{nonce}-{sequence}.sqlite3"
        )))
    }
}

#[derive(Clone, Copy)]
struct H2aPolicy(ActionExecutionPolicy);

impl ApprovalAuthorizationPort for H2aPolicy {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

impl ActionExecutionPolicyPort for H2aPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        self.0
    }
}

fn create_command(idempotency: &str, correlation: &str, request: &str) -> CreateActionRequestDraft {
    CreateActionRequestDraft {
        id: ActionRequestId::parse(request).unwrap(),
        title: ActionTitle::parse("Synthetic Cancel/Reopen-with-evidence request").unwrap(),
        details: ActionDetails::parse("Persist an Action through the durable writer.").unwrap(),
        intended_owner: Some(StakeholderId::parse("stakeholder-owner-1").unwrap()),
        response_due_at: Some(UtcTimestamp::from_unix_millis(200_000)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(300_000)),
        classification: DataClassification::Internal,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn submit_command(idempotency: &str, correlation: &str, request: &str) -> SubmitActionRequest {
    SubmitActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::initial(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn prepare_accept_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
) -> PrepareAcceptActionRequest {
    PrepareAcceptActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::initial().next().unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn prepared_accept_fixture(
    request: &str,
    prepared_id: &str,
    action_id: &str,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::AcceptActionRequest {
            request_id: ActionRequestId::parse(request).unwrap(),
            request_version: AggregateVersion::initial().next().unwrap(),
            action_id: ActionId::parse(action_id).unwrap(),
            action_classification: DataClassification::Internal,
            action_subject: ActionTitle::parse("Synthetic Cancel/Reopen-with-evidence request")
                .unwrap(),
            commitment_details: ActionDetails::parse(
                "Persist an Action through the durable writer.",
            )
            .unwrap(),
            intended_owner: StakeholderId::parse("stakeholder-owner-1").unwrap(),
            intended_due_at: UtcTimestamp::from_unix_millis(300_000),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

fn start_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
) -> StartAction {
    StartAction {
        action_id: action_id.clone(),
        expected_version,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn link_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
    evidence_id: &str,
) -> LinkActionCompletionEvidence {
    LinkActionCompletionEvidence {
        action_id: action_id.clone(),
        expected_version,
        evidence_id: EvidenceReferenceId::parse(evidence_id).unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn cancel_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
) -> PrepareCancelAction {
    PrepareCancelAction {
        action_id: action_id.clone(),
        expected_version,
        reason: ActionDetails::parse("Synthetic cancel reason").unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn reopen_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
) -> PrepareReopenAction {
    PrepareReopenAction {
        action_id: action_id.clone(),
        expected_version,
        mode: ActionReopenMode::RestartCancelled,
        reason: ActionDetails::parse("Synthetic reopen reason").unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn approval_for(
    prepared_id: &str,
    payload_digest: &pmc_domain::work_management::WorkManagementPayloadDigest,
    idempotency_id: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        PreparedIntentId::parse(prepared_id).unwrap(),
        AuditActor::HeadOfProducts,
        payload_digest.clone(),
        IdempotencyId::parse(idempotency_id).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

/// Seeds one raw, truthful Evidence Reference row -- unlike
/// `sqlite-action-link-completion-evidence.rs`'s own shortcut fixture, this
/// carries a complete `verification` shape (`last_verified_at`+
/// `integrity_digest` for `'verified'`), since the real
/// `PersistedActionEvidenceAuthority` this file exercises parses it
/// strictly.
fn seed_evidence_reference(
    path: &std::path::Path,
    id: &str,
    classification: &str,
    verification: &str,
) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'evidence_reference',1,?2,500,500)",
            rusqlite::params![id, classification],
        )
        .unwrap();
    match verification {
        "verified" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES (?1,NULL,'verified',?2,500)",
                    rusqlite::params![id, "a".repeat(64)],
                )
                .unwrap();
        }
        "unverified" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification) VALUES (?1,NULL,'unverified')",
                    [id],
                )
                .unwrap();
        }
        "integrity_mismatch" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification) VALUES (?1,NULL,'integrity_mismatch')",
                    [id],
                )
                .unwrap();
        }
        other => panic!("unsupported synthetic verification fixture: {other}"),
    }
}

/// Seeds a ledger through H1 create+submit, H2a accept, `StartAction`, and
/// one `LinkActionCompletionEvidence` (evidence id
/// `synthetic-completion-evidence-1`, `verified`), returning the handle and
/// the resulting InProgress Action's id (always `action-1`, classification
/// `Internal`, version 3: accept=1, start=2, link=3).
fn seeded_ledger_with_in_progress_action_and_linked_evidence(
) -> (SyntheticLedger, SqliteProductLedger, ActionId) {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
    ).unwrap();
    drop(connection);
    seed_evidence_reference(
        &ledger.0,
        "synthetic-completion-evidence-1",
        "internal",
        "verified",
    );
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_action_request_draft(
            create_command(
                "action-create-1",
                "action-create-correlation-1",
                "request-1",
            ),
            AuditEventId::parse("action-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_action_request(
            submit_command(
                "action-submit-1",
                "action-submit-correlation-1",
                "request-1",
            ),
            AuditEventId::parse("action-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepared = prepared_accept_fixture("request-1", "action-prepared-1", "action-1");
    writer
        .prepare_accept_action_request(
            prepare_accept_command(
                "action-prepare-1",
                "action-prepare-correlation-1",
                "request-1",
            ),
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency = IdempotencyId::parse("action-accept-execute-1").unwrap();
    let outcome = writer
        .approve_and_execute_accept_action_request(
            ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    execute_idempotency.clone(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: execute_idempotency,
                    correlation_id: CorrelationId::parse("action-accept-execute-correlation-1")
                        .unwrap(),
                },
            },
            [
                AuditEventId::parse("action-accept-request-audit-1").unwrap(),
                AuditEventId::parse("action-accept-action-audit-1").unwrap(),
                AuditEventId::parse("action-accept-link-audit-1").unwrap(),
            ],
            ApprovalReceiptId::parse("action-accept-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    let action_id = outcome.action.id().clone();
    writer
        .start_action(
            start_command(
                "action-start-1",
                "action-start-correlation-1",
                &action_id,
                AggregateVersion::initial(),
            ),
            AuditEventId::parse("action-start-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
        )
        .unwrap();
    writer
        .link_action_completion_evidence(
            link_command(
                "action-link-1",
                "action-link-correlation-1",
                &action_id,
                AggregateVersion::initial().next().unwrap(),
                "synthetic-completion-evidence-1",
            ),
            AuditEventId::parse("action-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
    (ledger, writer, action_id)
}

/// The regression itself: before the fix, this PREPARE call would fail with
/// a generic `ledger.persistence_failed` for ANY Action carrying linked
/// completion evidence, regardless of whether cancelling it should
/// legitimately succeed.
#[test]
fn cancel_succeeds_on_an_action_with_linked_completion_evidence() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence();
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let prepared = writer
        .prepare_cancel_action(
            cancel_command(
                "action-cancel-prepare-1",
                "action-cancel-prepare-correlation-1",
                &action_id,
                version_at_link,
            ),
            PreparedIntentId::parse("action-cancel-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect("Cancel PREPARE must succeed once real evidence resolution is wired in");

    let approval = approval_for(
        "action-cancel-prepared-1",
        prepared.payload_digest(),
        "action-cancel-execute-1",
    );
    let outcome = writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-cancel-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Cancel EXECUTE must succeed once real evidence resolution is wired in");
    assert_eq!(outcome.record.state(), ActionState::Cancelled);
    // The linked evidence's own classification (Internal) is already the
    // Action's current classification, so `combine` does not change it --
    // this still proves resolution happened (a Deny/Unavailable failure
    // would never have reached here at all).
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().contains(&outcome.record));
}

/// Continues past Cancel into Reopen, exercising all four repaired
/// construction sites (Cancel PREPARE/EXECUTE, Reopen PREPARE/EXECUTE) in
/// one Action lifecycle, then survives a restart.
#[test]
fn reopen_succeeds_on_a_cancelled_action_that_had_linked_completion_evidence() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence();
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let cancel_prepared = writer
        .prepare_cancel_action(
            cancel_command(
                "action-cancel-prepare-1",
                "action-cancel-prepare-correlation-1",
                &action_id,
                version_at_link,
            ),
            PreparedIntentId::parse("action-cancel-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    let cancel_approval = approval_for(
        "action-cancel-prepared-1",
        cancel_prepared.payload_digest(),
        "action-cancel-execute-1",
    );
    let cancelled = writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval: cancel_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-cancel-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(cancelled.record.state(), ActionState::Cancelled);
    let version_at_cancel = cancelled.record.version();

    let reopen_prepared = writer
        .prepare_reopen_action(
            reopen_command(
                "action-reopen-prepare-1",
                "action-reopen-prepare-correlation-1",
                &action_id,
                version_at_cancel,
            ),
            PreparedIntentId::parse("action-reopen-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_400),
        )
        .expect("Reopen PREPARE must succeed once real evidence resolution is wired in");
    let reopen_approval = approval_for(
        "action-reopen-prepared-1",
        reopen_prepared.payload_digest(),
        "action-reopen-execute-1",
    );
    let reopened_outcome = writer
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction {
                approval: reopen_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-reopen-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-reopen-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-reopen-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_450),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Reopen EXECUTE must succeed once real evidence resolution is wired in");
    assert_eq!(reopened_outcome.record.state(), ActionState::InProgress);
    drop(writer);

    let reopened_ledger = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened_ledger.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().contains(&reopened_outcome.record));
}

/// A second Cancel-then-Reopen cycle on the same Action.
/// Before the activity-decoder redesign, `decode_cancel_action_activity`/
/// `decode_reopen_action_activity` each built their own `previous_by_action`
/// snapshot ONCE at function entry and assumed at most one successful
/// execute per Action -- a second cycle's `expected_version` cross-check
/// would not match, and decode failed closed even though the ledger's own
/// data was completely legitimate (documented explicitly on both old
/// functions' own doc comments). The new unified fold reads `actions` live
/// at each event's own ordinal, so `Cancel -> Reopen -> Cancel -> Reopen`
/// now folds correctly with no such limit.
#[test]
fn cancel_reopen_cancel_reopen_cycle_decodes_losslessly() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence();
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let first_cancel_prepared = writer
        .prepare_cancel_action(
            cancel_command(
                "action-cancel-prepare-1",
                "action-cancel-prepare-correlation-1",
                &action_id,
                version_at_link,
            ),
            PreparedIntentId::parse("action-cancel-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    let first_cancel_approval = approval_for(
        "action-cancel-prepared-1",
        first_cancel_prepared.payload_digest(),
        "action-cancel-execute-1",
    );
    let first_cancelled = writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval: first_cancel_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-cancel-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(first_cancelled.record.state(), ActionState::Cancelled);

    let first_reopen_prepared = writer
        .prepare_reopen_action(
            reopen_command(
                "action-reopen-prepare-1",
                "action-reopen-prepare-correlation-1",
                &action_id,
                first_cancelled.record.version(),
            ),
            PreparedIntentId::parse("action-reopen-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_400),
        )
        .unwrap();
    let first_reopen_approval = approval_for(
        "action-reopen-prepared-1",
        first_reopen_prepared.payload_digest(),
        "action-reopen-execute-1",
    );
    let first_reopened = writer
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction {
                approval: first_reopen_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-reopen-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-reopen-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-reopen-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_450),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(first_reopened.record.state(), ActionState::InProgress);

    // Second cycle: Cancel again, then Reopen again.
    let second_cancel_prepared = writer
        .prepare_cancel_action(
            cancel_command(
                "action-cancel-prepare-2",
                "action-cancel-prepare-correlation-2",
                &action_id,
                first_reopened.record.version(),
            ),
            PreparedIntentId::parse("action-cancel-prepared-2").unwrap(),
            UtcTimestamp::from_unix_millis(1_500),
        )
        .expect("second Cancel PREPARE must succeed -- no more artificial one-cycle limit");
    let second_cancel_approval = approval_for(
        "action-cancel-prepared-2",
        second_cancel_prepared.payload_digest(),
        "action-cancel-execute-2",
    );
    let second_cancelled = writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval: second_cancel_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-cancel-execute-2").unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-correlation-2")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-audit-2").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-2").unwrap(),
            UtcTimestamp::from_unix_millis(1_550),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("second Cancel EXECUTE must succeed");
    assert_eq!(second_cancelled.record.state(), ActionState::Cancelled);

    let second_reopen_prepared = writer
        .prepare_reopen_action(
            reopen_command(
                "action-reopen-prepare-2",
                "action-reopen-prepare-correlation-2",
                &action_id,
                second_cancelled.record.version(),
            ),
            PreparedIntentId::parse("action-reopen-prepared-2").unwrap(),
            UtcTimestamp::from_unix_millis(1_600),
        )
        .expect("second Reopen PREPARE must succeed");
    let second_reopen_approval = approval_for(
        "action-reopen-prepared-2",
        second_reopen_prepared.payload_digest(),
        "action-reopen-execute-2",
    );
    let second_reopened = writer
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction {
                approval: second_reopen_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-reopen-execute-2").unwrap(),
                    correlation_id: CorrelationId::parse("action-reopen-execute-correlation-2")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-reopen-execute-audit-2").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-2").unwrap(),
            UtcTimestamp::from_unix_millis(1_650),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("second Reopen EXECUTE must succeed");
    assert_eq!(second_reopened.record.state(), ActionState::InProgress);
    // Setup ends at v3 (Accept=1, Start=2, Link=3); each of the four
    // subsequent Cancel/Reopen executes advances by exactly one: v4, v5,
    // v6, v7.
    assert_eq!(second_reopened.record.version().get(), 7);
    drop(writer);

    let reopened_ledger = SqliteProductLedger::open(&ledger.0)
        .expect("a ledger with a full two-cycle Cancel/Reopen history must still open cleanly");
    let snapshot = reopened_ledger
        .load_action_persistence_snapshot()
        .expect("the whole two-cycle history must decode losslessly under the new global fold");
    assert!(snapshot.actions().contains(&second_reopened.record));
}

/// Proves verification state is not a Cancel/Reopen gate (only Complete's
/// own, not-yet-implemented PREPARE evaluates
/// `EvidenceOrJudgment::evaluate_evidence_required`, which does look at
/// verification) -- Cancel must succeed on an Action whose linked evidence
/// is `Unverified`, not just `Verified`.
#[test]
fn cancel_succeeds_with_unverified_linked_completion_evidence() {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
    ).unwrap();
    drop(connection);
    seed_evidence_reference(
        &ledger.0,
        "synthetic-completion-evidence-unverified",
        "internal",
        "unverified",
    );
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_action_request_draft(
            create_command(
                "action-create-1",
                "action-create-correlation-1",
                "request-1",
            ),
            AuditEventId::parse("action-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_action_request(
            submit_command(
                "action-submit-1",
                "action-submit-correlation-1",
                "request-1",
            ),
            AuditEventId::parse("action-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepared = prepared_accept_fixture("request-1", "action-prepared-1", "action-1");
    writer
        .prepare_accept_action_request(
            prepare_accept_command(
                "action-prepare-1",
                "action-prepare-correlation-1",
                "request-1",
            ),
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency = IdempotencyId::parse("action-accept-execute-1").unwrap();
    let accepted = writer
        .approve_and_execute_accept_action_request(
            ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    execute_idempotency.clone(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: execute_idempotency,
                    correlation_id: CorrelationId::parse("action-accept-execute-correlation-1")
                        .unwrap(),
                },
            },
            [
                AuditEventId::parse("action-accept-request-audit-1").unwrap(),
                AuditEventId::parse("action-accept-action-audit-1").unwrap(),
                AuditEventId::parse("action-accept-link-audit-1").unwrap(),
            ],
            ApprovalReceiptId::parse("action-accept-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    let action_id = accepted.action.id().clone();
    writer
        .start_action(
            start_command(
                "action-start-1",
                "action-start-correlation-1",
                &action_id,
                AggregateVersion::initial(),
            ),
            AuditEventId::parse("action-start-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
        )
        .unwrap();
    writer
        .link_action_completion_evidence(
            link_command(
                "action-link-1",
                "action-link-correlation-1",
                &action_id,
                AggregateVersion::initial().next().unwrap(),
                "synthetic-completion-evidence-unverified",
            ),
            AuditEventId::parse("action-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();

    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();
    let prepared = writer
        .prepare_cancel_action(
            cancel_command(
                "action-cancel-prepare-1",
                "action-cancel-prepare-correlation-1",
                &action_id,
                version_at_link,
            ),
            PreparedIntentId::parse("action-cancel-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect("Cancel PREPARE must succeed regardless of linked evidence's verification state");
    let approval = approval_for(
        "action-cancel-prepared-1",
        prepared.payload_digest(),
        "action-cancel-execute-1",
    );
    let outcome = writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-cancel-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Cancel EXECUTE must succeed regardless of linked evidence's verification state");
    assert_eq!(outcome.record.state(), ActionState::Cancelled);
}
