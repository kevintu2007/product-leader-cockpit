//! Action's Reopen lifecycle transition -- the second of Cancel/Complete/
//! Reopen to gain SQLite persistence (Reopen came before Complete, since
//! Complete's PREPARE was unconditionally blocked on completion
//! evidence that has no persistence of its own yet, while Reopen's
//! `RestartCancelled` mode is fully reachable today via Cancel's own
//! already-shipped persistence).
//!
//! Mirrors `sqlite-action-cancel.rs`'s structure closely: same closed-set
//! replay-capsule model, same full-rehydrate PREPARE (see
//! `SqliteProductLedger::prepare_reopen_action`'s doc comment). Every test
//! here exercises `ActionReopenMode::RestartCancelled` -- `ReopenCompleted`
//! is implemented identically but untestable until Complete's own
//! persistence lands (`ActionState::Completed` is currently unreachable).

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionPersistenceResult, ActionTitle, ApproveAndExecuteAcceptActionRequest,
        ApproveAndExecuteCancelAction, ApproveAndExecuteLowerActionClassification,
        ApproveAndExecuteReopenAction, CreateActionRequestDraft, PrepareAcceptActionRequest,
        PrepareCancelAction, PrepareLowerActionClassification, PrepareReopenAction,
        SubmitActionRequest,
    },
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ActionReopenMode, ActionState, ApprovalAuthorizationPort, ApprovalConfirmation,
        WorkManagementApproval, WorkManagementOperation, WorkManagementPreparedIntent,
        WorkManagementRationale,
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
            "pmc-synthetic-action-reopen-{nonce}-{sequence}.sqlite3"
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
        title: ActionTitle::parse("Synthetic Reopen request").unwrap(),
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
            action_subject: ActionTitle::parse("Synthetic Reopen request").unwrap(),
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

/// Seeds a ledger through H1 create+submit, H2a accept, and a full Cancel
/// (prepare+execute), returning the handle and the resulting Cancelled
/// Action's id (always `action-1`, classification `Internal`, version 2).
fn seeded_ledger_with_cancelled_action() -> (SyntheticLedger, SqliteProductLedger, ActionId) {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
    ).unwrap();
    drop(connection);
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

    let cancel_prepare = cancel_command(
        "action-cancel-prepare-seed",
        "action-cancel-prepare-seed-correlation",
        &action_id,
        AggregateVersion::initial(),
    );
    let cancel_prepared_id = PreparedIntentId::parse("action-cancel-prepared-seed").unwrap();
    let cancel_prepared = writer
        .prepare_cancel_action(
            cancel_prepare,
            cancel_prepared_id,
            UtcTimestamp::from_unix_millis(1_150),
        )
        .unwrap();
    let cancel_execute_idempotency = "action-cancel-execute-seed";
    let cancel_approval = approval_for(
        "action-cancel-prepared-seed",
        cancel_prepared.payload_digest(),
        cancel_execute_idempotency,
    );
    let cancelled = writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval: cancel_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse(cancel_execute_idempotency).unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-seed-correlation")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-seed-audit").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-seed").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(cancelled.record.state(), ActionState::Cancelled);
    assert_eq!(
        cancelled.record.version(),
        AggregateVersion::initial().next().unwrap()
    );
    (ledger, writer, action_id)
}

#[test]
fn prepare_reopen_action_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();
    let command = reopen_command(
        "action-reopen-prepare-1",
        "action-reopen-prepare-correlation-1",
        &action_id,
        AggregateVersion::initial().next().unwrap(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-reopen-prepared-1").unwrap();
    let first = writer
        .prepare_reopen_action(
            command.clone(),
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    assert_eq!(first.id(), &prepared_intent_id);
    assert_eq!(first.classification(), DataClassification::Internal);
    let replay = writer
        .prepare_reopen_action(
            command,
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    assert_eq!(replay, first);
}

#[test]
fn prepare_reopen_action_rejects_a_stale_expected_version() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();
    let command = reopen_command(
        "action-reopen-prepare-stale-1",
        "action-reopen-prepare-stale-correlation-1",
        &action_id,
        AggregateVersion::initial(),
    );
    let result = writer.prepare_reopen_action(
        command,
        PreparedIntentId::parse("action-reopen-prepared-stale-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
    );
    assert!(result.is_err());
}

#[allow(clippy::too_many_arguments)]
fn reopen_action(
    writer: &mut SqliteProductLedger,
    action_id: &ActionId,
    expected_version: AggregateVersion,
    prepare_idempotency: &str,
    prepared_id: &str,
    execute_idempotency: &str,
    receipt_id: &str,
    occurred_at_prepare: i64,
    occurred_at_execute: i64,
) -> pmc_domain::actions::ActionMutationOutcome<pmc_domain::actions::ActionRecord> {
    let prepare = reopen_command(
        prepare_idempotency,
        &format!("{prepare_idempotency}-correlation"),
        action_id,
        expected_version,
    );
    let prepared_intent_id = PreparedIntentId::parse(prepared_id).unwrap();
    let prepared = writer
        .prepare_reopen_action(
            prepare,
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(occurred_at_prepare),
        )
        .unwrap();
    let approval = approval_for(prepared_id, prepared.payload_digest(), execute_idempotency);
    writer
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction {
                approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
                    correlation_id: CorrelationId::parse(format!(
                        "{execute_idempotency}-correlation"
                    ))
                    .unwrap(),
                },
            },
            AuditEventId::parse(format!("{execute_idempotency}-audit")).unwrap(),
            ApprovalReceiptId::parse(receipt_id).unwrap(),
            UtcTimestamp::from_unix_millis(occurred_at_execute),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap()
}

#[test]
fn approve_and_execute_reopen_action_persists_and_survives_a_restart() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();
    let outcome = reopen_action(
        &mut writer,
        &action_id,
        AggregateVersion::initial().next().unwrap(),
        "action-reopen-prepare-2",
        "action-reopen-prepared-2",
        "action-reopen-execute-2",
        "action-reopen-receipt-2",
        1_300,
        1_400,
    );
    assert_eq!(outcome.record.state(), ActionState::InProgress);
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.transition_history().len(), 2);
    assert_eq!(
        outcome.record.transition_history()[1].to(),
        ActionState::InProgress
    );
    drop(writer);

    let restarted = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = restarted.load_action_persistence_snapshot().unwrap();
    let restarted_action = snapshot
        .actions()
        .iter()
        .find(|record| record.id() == &action_id)
        .unwrap();
    assert_eq!(restarted_action.state(), ActionState::InProgress);
    assert_eq!(
        restarted_action.classification(),
        DataClassification::Internal
    );
    assert_eq!(restarted_action.version(), outcome.record.version());
    assert!(snapshot
        .replay()
        .iter()
        .any(|capsule| matches!(capsule.result(), ActionPersistenceResult::Action(r) if r.record == outcome.record)));
}

#[test]
fn approve_and_execute_reopen_action_replay_returns_the_original_outcome() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();
    let prepare = reopen_command(
        "action-reopen-prepare-3",
        "action-reopen-prepare-correlation-3",
        &action_id,
        AggregateVersion::initial().next().unwrap(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-reopen-prepared-3").unwrap();
    let prepared = writer
        .prepare_reopen_action(
            prepare,
            prepared_intent_id,
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    let execute_idempotency = "action-reopen-execute-3";
    let approval = approval_for(
        "action-reopen-prepared-3",
        prepared.payload_digest(),
        execute_idempotency,
    );
    let command = ApproveAndExecuteReopenAction {
        approval,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
            correlation_id: CorrelationId::parse("action-reopen-execute-correlation-3").unwrap(),
        },
    };
    let first = writer
        .approve_and_execute_reopen_action(
            command.clone(),
            AuditEventId::parse("action-reopen-execute-audit-3").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-3").unwrap(),
            UtcTimestamp::from_unix_millis(1_400),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    let replay = writer
        .approve_and_execute_reopen_action(
            command,
            AuditEventId::parse("action-reopen-execute-audit-3").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-3").unwrap(),
            UtcTimestamp::from_unix_millis(1_400),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(replay.record, first.record);
    assert_eq!(replay.approval_receipt_id, first.approval_receipt_id);
}

#[test]
fn approve_and_execute_reopen_action_rejects_a_replay_with_a_mismatched_approval_idempotency_id() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();
    let prepare = reopen_command(
        "action-reopen-prepare-4",
        "action-reopen-prepare-correlation-4",
        &action_id,
        AggregateVersion::initial().next().unwrap(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-reopen-prepared-4").unwrap();
    let prepared = writer
        .prepare_reopen_action(
            prepare,
            prepared_intent_id,
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    let execute_idempotency = "action-reopen-execute-4";
    let approval = approval_for(
        "action-reopen-prepared-4",
        prepared.payload_digest(),
        execute_idempotency,
    );
    writer
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction {
                approval: approval.clone(),
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
                    correlation_id: CorrelationId::parse("action-reopen-execute-correlation-4")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-reopen-execute-audit-4").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-4").unwrap(),
            UtcTimestamp::from_unix_millis(1_400),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    // Same approval-idempotency regression check already applied to
    // Cancel: the early idempotency-lookup replay branch must still bind
    // the approval's own idempotency_id, not just the context's.
    let mismatched_approval = WorkManagementApproval::new(
        PreparedIntentId::parse("action-reopen-prepared-4").unwrap(),
        AuditActor::HeadOfProducts,
        approval.acknowledged_payload_digest().clone(),
        IdempotencyId::parse("action-reopen-execute-4-different").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let result = writer.approve_and_execute_reopen_action(
        ApproveAndExecuteReopenAction {
            approval: mismatched_approval,
            context: ActionOperationContext {
                idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
                correlation_id: CorrelationId::parse("action-reopen-execute-correlation-4")
                    .unwrap(),
            },
        },
        AuditEventId::parse("action-reopen-execute-audit-4b").unwrap(),
        ApprovalReceiptId::parse("action-reopen-receipt-4b").unwrap(),
        UtcTimestamp::from_unix_millis(1_500),
        H2aPolicy(ActionExecutionPolicy::Allowed),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    );
    assert!(result.is_err());
}

/// Exercises the EXECUTE-time Terminal/Retained path without touching the
/// documented Lower-Classification-after-Cancel gap: two independent Reopen
/// previews are prepared against the SAME Cancelled action/version under
/// different idempotency keys, the first is executed successfully (bumping
/// the action to InProgress/v3), then the second -- still targeting v2 --
/// is executed and hits `StaleVersion` -> `PreparedIntentChanged` ->
/// `Retained` Terminal, exactly like Cancel's own retained-terminal test.
#[test]
fn approve_and_execute_reopen_action_persists_a_retained_terminal_on_a_stale_prepared_intent_and_survives_a_restart(
) {
    let (ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();
    let expected_version = AggregateVersion::initial().next().unwrap();

    let prepared_a = writer
        .prepare_reopen_action(
            reopen_command(
                "action-reopen-prepare-5a",
                "action-reopen-prepare-5a-correlation",
                &action_id,
                expected_version,
            ),
            PreparedIntentId::parse("action-reopen-prepared-5a").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    let prepared_b = writer
        .prepare_reopen_action(
            reopen_command(
                "action-reopen-prepare-5b",
                "action-reopen-prepare-5b-correlation",
                &action_id,
                expected_version,
            ),
            PreparedIntentId::parse("action-reopen-prepared-5b").unwrap(),
            UtcTimestamp::from_unix_millis(1_310),
        )
        .unwrap();

    // Execute A first -- succeeds, bumping the action to InProgress/v3.
    let execute_a_idempotency = "action-reopen-execute-5a";
    let approval_a = approval_for(
        "action-reopen-prepared-5a",
        prepared_a.payload_digest(),
        execute_a_idempotency,
    );
    let outcome_a = writer
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction {
                approval: approval_a,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse(execute_a_idempotency).unwrap(),
                    correlation_id: CorrelationId::parse("action-reopen-execute-5a-correlation")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-reopen-execute-5a-audit").unwrap(),
            ApprovalReceiptId::parse("action-reopen-receipt-5a").unwrap(),
            UtcTimestamp::from_unix_millis(1_400),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(outcome_a.record.state(), ActionState::InProgress);

    // Execute B second -- still targets v2, now stale -- expect a Retained
    // Terminal, not a success.
    let execute_b_idempotency = "action-reopen-execute-5b";
    let approval_b = approval_for(
        "action-reopen-prepared-5b",
        prepared_b.payload_digest(),
        execute_b_idempotency,
    );
    let result = writer.approve_and_execute_reopen_action(
        ApproveAndExecuteReopenAction {
            approval: approval_b,
            context: ActionOperationContext {
                idempotency_id: IdempotencyId::parse(execute_b_idempotency).unwrap(),
                correlation_id: CorrelationId::parse("action-reopen-execute-5b-correlation")
                    .unwrap(),
            },
        },
        AuditEventId::parse("action-reopen-execute-5b-audit").unwrap(),
        ApprovalReceiptId::parse("action-reopen-receipt-5b").unwrap(),
        UtcTimestamp::from_unix_millis(1_410),
        H2aPolicy(ActionExecutionPolicy::Allowed),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    );
    assert!(result.is_err());

    // B's prepared intent must have survived (Retained), still visible as
    // outstanding after a restart, and the action itself reflects only A's
    // successful reopen.
    drop(writer);
    let restarted = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = restarted.load_action_persistence_snapshot().unwrap();
    let restarted_action = snapshot
        .actions()
        .iter()
        .find(|record| record.id() == &action_id)
        .unwrap();
    assert_eq!(restarted_action.state(), ActionState::InProgress);
    assert_eq!(restarted_action.version(), outcome_a.record.version());
    assert!(snapshot
        .prepared()
        .iter()
        .any(|intent| intent.id().as_str() == "action-reopen-prepared-5b"));
    assert!(snapshot.discarded_prepared().is_empty());
}

/// Reopen half of the lowered-classification snapshot fix. A Cancelled
/// Action that has since been
/// lower-classified must still be reopenable: the same re-export of its own
/// history that broke Cancel breaks Reopen, because all three operations share
/// the `persistence_snapshot()` seam.
#[test]
fn prepare_reopen_action_succeeds_after_the_action_was_lower_classified() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_cancelled_action();

    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let lower_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("action-lower-prepared-57r").unwrap(),
        WorkManagementOperation::LowerActionClassification {
            action_id: action_id.clone(),
            action_version: AggregateVersion::new(2).unwrap(),
            current_classification: DataClassification::Internal,
            proposed_classification: DataClassification::Public,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_100),
    )
    .unwrap();
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::new(2).unwrap(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-lower-prepare-57r").unwrap(),
                    correlation_id: CorrelationId::parse("action-lower-prepare-correlation-57r")
                        .unwrap(),
                },
            },
            lower_prepared.clone(),
        )
        .unwrap();
    let lower_execute_idempotency = IdempotencyId::parse("action-lower-execute-57r").unwrap();
    writer
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval: WorkManagementApproval::new(
                    lower_prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    lower_prepared.payload_digest().clone(),
                    lower_execute_idempotency.clone(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: lower_execute_idempotency,
                    correlation_id: CorrelationId::parse("action-lower-execute-correlation-57r")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-lower-execute-audit-57r").unwrap(),
            ApprovalReceiptId::parse("action-lower-receipt-57r").unwrap(),
            UtcTimestamp::from_unix_millis(1_110),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();

    let outcome = writer.prepare_reopen_action(
        reopen_command(
            "action-reopen-prepare-57r",
            "action-reopen-prepare-correlation-57r",
            &action_id,
            AggregateVersion::new(3).unwrap(),
        ),
        PreparedIntentId::parse("action-reopen-prepared-57r").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
    );

    assert!(
        outcome.is_ok(),
        "a Lower-Classified Action must still be reopenable; got {:?}",
        outcome.err()
    );
}
