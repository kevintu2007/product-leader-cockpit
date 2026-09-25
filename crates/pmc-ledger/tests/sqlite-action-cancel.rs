//! SQLite persistence for Action's Cancel lifecycle transition -- the first
//! of Cancel/Complete/Reopen to gain one.
//!
//! Mirrors `sqlite-action-h2a-lower-classification.rs`'s structure: Cancel
//! shares the same closed-set replay-capsule model, but unlike Lower
//! Classification's caller-supplied-canonical PREPARE, Cancel's PREPARE
//! fully rehydrates the real domain service (see
//! `SqliteProductLedger::prepare_cancel_action`'s doc comment for why), so
//! its own `prepared` fixture below is only used to build the approval
//! digest for EXECUTE, never passed into `prepare_cancel_action` itself.

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
        CreateActionRequestDraft, PrepareAcceptActionRequest, PrepareCancelAction,
        PrepareLowerActionClassification, SubmitActionRequest,
    },
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ActionState, ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
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
            "pmc-synthetic-action-cancel-{nonce}-{sequence}.sqlite3"
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
        title: ActionTitle::parse("Synthetic Cancel request").unwrap(),
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
            action_subject: ActionTitle::parse("Synthetic Cancel request").unwrap(),
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

/// Seeds a ledger through H1 create+submit and H2a accept, returning the
/// handle and the resulting Open Action's id (always `action-1`,
/// classification `Internal`, version 1).
fn seeded_ledger_with_open_action() -> (SyntheticLedger, SqliteProductLedger, ActionId) {
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
    assert_eq!(
        outcome.action.classification(),
        DataClassification::Internal
    );
    (ledger, writer, outcome.action.id().clone())
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

#[test]
fn prepare_cancel_action_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let command = cancel_command(
        "action-cancel-prepare-1",
        "action-cancel-prepare-correlation-1",
        &action_id,
        AggregateVersion::initial(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-cancel-prepared-1").unwrap();
    let first = writer
        .prepare_cancel_action(
            command.clone(),
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
    assert_eq!(first.id(), &prepared_intent_id);
    assert_eq!(first.classification(), DataClassification::Internal);
    let replay = writer
        .prepare_cancel_action(
            command,
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
    assert_eq!(replay, first);
}

#[test]
fn prepare_cancel_action_rejects_a_stale_expected_version() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let command = cancel_command(
        "action-cancel-prepare-stale-1",
        "action-cancel-prepare-stale-correlation-1",
        &action_id,
        AggregateVersion::initial().next().unwrap(),
    );
    let result = writer.prepare_cancel_action(
        command,
        PreparedIntentId::parse("action-cancel-prepared-stale-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
    );
    assert!(result.is_err());
}

/// Prepares and executes a Cancel against `action_id`, returning the
/// receipt id used so callers can assert on it if needed.
#[allow(clippy::too_many_arguments)]
fn cancel_action(
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
    let prepare = cancel_command(
        prepare_idempotency,
        &format!("{prepare_idempotency}-correlation"),
        action_id,
        expected_version,
    );
    let prepared_intent_id = PreparedIntentId::parse(prepared_id).unwrap();
    let prepared = writer
        .prepare_cancel_action(
            prepare,
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(occurred_at_prepare),
        )
        .unwrap();
    let approval = approval_for(prepared_id, prepared.payload_digest(), execute_idempotency);
    writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
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
fn approve_and_execute_cancel_action_persists_and_survives_a_restart() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let outcome = cancel_action(
        &mut writer,
        &action_id,
        AggregateVersion::initial(),
        "action-cancel-prepare-2",
        "action-cancel-prepared-2",
        "action-cancel-execute-2",
        "action-cancel-receipt-2",
        1_200,
        1_300,
    );
    assert_eq!(outcome.record.state(), ActionState::Cancelled);
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.record.transition_history().len(), 1);
    assert_eq!(
        outcome.record.transition_history()[0].to(),
        ActionState::Cancelled
    );
    drop(writer);

    let restarted = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = restarted.load_action_persistence_snapshot().unwrap();
    let restarted_action = snapshot
        .actions()
        .iter()
        .find(|record| record.id() == &action_id)
        .unwrap();
    assert_eq!(restarted_action.state(), ActionState::Cancelled);
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
fn approve_and_execute_cancel_action_replay_returns_the_original_outcome() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let prepare = cancel_command(
        "action-cancel-prepare-3",
        "action-cancel-prepare-correlation-3",
        &action_id,
        AggregateVersion::initial(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-cancel-prepared-3").unwrap();
    let prepared = writer
        .prepare_cancel_action(
            prepare,
            prepared_intent_id,
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
    let execute_idempotency = "action-cancel-execute-3";
    let approval = approval_for(
        "action-cancel-prepared-3",
        prepared.payload_digest(),
        execute_idempotency,
    );
    let command = ApproveAndExecuteCancelAction {
        approval,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
            correlation_id: CorrelationId::parse("action-cancel-execute-correlation-3").unwrap(),
        },
    };
    let first = writer
        .approve_and_execute_cancel_action(
            command.clone(),
            AuditEventId::parse("action-cancel-execute-audit-3").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-3").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    let replay = writer
        .approve_and_execute_cancel_action(
            command,
            AuditEventId::parse("action-cancel-execute-audit-3").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-3").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(replay.record, first.record);
    assert_eq!(replay.approval_receipt_id, first.approval_receipt_id);
}

#[test]
fn approve_and_execute_cancel_action_rejects_a_replay_with_a_mismatched_approval_idempotency_id() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let prepare = cancel_command(
        "action-cancel-prepare-4",
        "action-cancel-prepare-correlation-4",
        &action_id,
        AggregateVersion::initial(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-cancel-prepared-4").unwrap();
    let prepared = writer
        .prepare_cancel_action(
            prepare,
            prepared_intent_id,
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
    let execute_idempotency = "action-cancel-execute-4";
    let approval = approval_for(
        "action-cancel-prepared-4",
        prepared.payload_digest(),
        execute_idempotency,
    );
    writer
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction {
                approval: approval.clone(),
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
                    correlation_id: CorrelationId::parse("action-cancel-execute-correlation-4")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-cancel-execute-audit-4").unwrap(),
            ApprovalReceiptId::parse("action-cancel-receipt-4").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    // Replay the SAME `execute_idempotency` (so the repository's early
    // idempotency-lookup branch is exercised) but with an `approval` whose
    // OWN `idempotency_id` differs from the context's -- mirrors the
    // mismatched-approval regression pattern already applied to the other 8
    // families.
    let mismatched_approval = WorkManagementApproval::new(
        PreparedIntentId::parse("action-cancel-prepared-4").unwrap(),
        AuditActor::HeadOfProducts,
        approval.acknowledged_payload_digest().clone(),
        IdempotencyId::parse("action-cancel-execute-4-different").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let result = writer.approve_and_execute_cancel_action(
        ApproveAndExecuteCancelAction {
            approval: mismatched_approval,
            context: ActionOperationContext {
                idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
                correlation_id: CorrelationId::parse("action-cancel-execute-correlation-4")
                    .unwrap(),
            },
        },
        AuditEventId::parse("action-cancel-execute-audit-4b").unwrap(),
        ApprovalReceiptId::parse("action-cancel-receipt-4b").unwrap(),
        UtcTimestamp::from_unix_millis(1_400),
        H2aPolicy(ActionExecutionPolicy::Allowed),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    );
    assert!(result.is_err());
}

/// Exercises the EXECUTE-time Terminal/Retained path: a Cancel is prepared
/// against version 1, then a Lower Classification is prepared+executed
/// against the SAME action, bumping it to version 2 -- so by the time the
/// stale Cancel approval is executed, `execute_h2`'s `exact_action` lookup
/// fails with `StaleVersion`, `map_post_prepare_state` remaps that to
/// `PreparedIntentChanged`, and `record_retryable_preview_failure` persists
/// a `Retained` Terminal capsule (the prepared intent survives for a retry
/// against the new version, unlike a `ConsumedAndDiscarded` failure).
#[test]
fn approve_and_execute_cancel_action_persists_a_retained_terminal_on_a_stale_prepared_intent_and_survives_a_restart(
) {
    let (ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let prepare = cancel_command(
        "action-cancel-prepare-5",
        "action-cancel-prepare-correlation-5",
        &action_id,
        AggregateVersion::initial(),
    );
    let prepared_intent_id = PreparedIntentId::parse("action-cancel-prepared-5").unwrap();
    let prepared = writer
        .prepare_cancel_action(
            prepare,
            prepared_intent_id.clone(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();

    // Lower the SAME action's classification, bumping it to version 2, so
    // the version-1 Cancel preview is now stale by the time it executes.
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let lower_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("action-lower-prepared-5").unwrap(),
        WorkManagementOperation::LowerActionClassification {
            action_id: action_id.clone(),
            action_version: AggregateVersion::initial(),
            current_classification: DataClassification::Internal,
            proposed_classification: DataClassification::Public,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_250),
    )
    .unwrap();
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-lower-prepare-5").unwrap(),
                    correlation_id: CorrelationId::parse("action-lower-prepare-correlation-5")
                        .unwrap(),
                },
            },
            lower_prepared.clone(),
        )
        .unwrap();
    let lower_execute_idempotency = IdempotencyId::parse("action-lower-execute-5").unwrap();
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
                    correlation_id: CorrelationId::parse("action-lower-execute-correlation-5")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-lower-execute-audit-5").unwrap(),
            ApprovalReceiptId::parse("action-lower-receipt-5").unwrap(),
            UtcTimestamp::from_unix_millis(1_260),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();

    // Now execute the now-stale Cancel preview -- expect a Retained Terminal.
    let execute_idempotency = "action-cancel-execute-5";
    let approval = approval_for(
        "action-cancel-prepared-5",
        prepared.payload_digest(),
        execute_idempotency,
    );
    let result = writer.approve_and_execute_cancel_action(
        ApproveAndExecuteCancelAction {
            approval,
            context: ActionOperationContext {
                idempotency_id: IdempotencyId::parse(execute_idempotency).unwrap(),
                correlation_id: CorrelationId::parse("action-cancel-execute-correlation-5")
                    .unwrap(),
            },
        },
        AuditEventId::parse("action-cancel-execute-audit-5").unwrap(),
        ApprovalReceiptId::parse("action-cancel-receipt-5").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
        H2aPolicy(ActionExecutionPolicy::Allowed),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    );
    assert!(result.is_err());

    // The prepared intent must have survived (Retained), still visible as
    // outstanding after a restart, and the action itself is still at
    // version 2 / Public / Open (the failed Cancel never mutated it).
    drop(writer);
    let restarted = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = restarted.load_action_persistence_snapshot().unwrap();
    let restarted_action = snapshot
        .actions()
        .iter()
        .find(|record| record.id() == &action_id)
        .unwrap();
    assert_eq!(restarted_action.state(), ActionState::Open);
    assert_eq!(
        restarted_action.classification(),
        DataClassification::Public
    );
    assert!(snapshot
        .prepared()
        .iter()
        .any(|intent| intent.id() == &prepared_intent_id));
    assert!(snapshot.discarded_prepared().is_empty());
}

/// Regression: once an Action had been Lower-Classified, Cancel's PREPARE
/// failed permanently.
///
/// The existing stale-preview test above lowers *after* preparing, so the
/// rehydrated history has no Lower Classification capsule in it when
/// `persistence_snapshot()` re-exports. This is the other order -- lower
/// first, then prepare -- which is the one a user actually hits, and which
/// leaves the Action permanently uncancellable.
#[test]
fn prepare_cancel_action_succeeds_after_the_action_was_lower_classified() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();

    // Lower the classification first. This is the step that puts a Lower
    // Classification capsule into the Action's replay history.
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let lower_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("action-lower-prepared-57").unwrap(),
        WorkManagementOperation::LowerActionClassification {
            action_id: action_id.clone(),
            action_version: AggregateVersion::initial(),
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
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-lower-prepare-57").unwrap(),
                    correlation_id: CorrelationId::parse("action-lower-prepare-correlation-57")
                        .unwrap(),
                },
            },
            lower_prepared.clone(),
        )
        .unwrap();
    let lower_execute_idempotency = IdempotencyId::parse("action-lower-execute-57").unwrap();
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
                    correlation_id: CorrelationId::parse("action-lower-execute-correlation-57")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-lower-execute-audit-57").unwrap(),
            ApprovalReceiptId::parse("action-lower-receipt-57").unwrap(),
            UtcTimestamp::from_unix_millis(1_110),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();

    // Now try to cancel it. The Action is at version 2 after the lowering.
    let outcome = writer.prepare_cancel_action(
        cancel_command(
            "action-cancel-prepare-57",
            "action-cancel-prepare-correlation-57",
            &action_id,
            AggregateVersion::new(2).unwrap(),
        ),
        PreparedIntentId::parse("action-cancel-prepared-57").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
    );

    assert!(
        outcome.is_ok(),
        "a Lower-Classified Action must still be cancellable; got {:?}",
        outcome.err()
    );
}
