//! Action H2a "Lower Data Classification" persistence.
//!
//! Mirrors `sqlite-decision-h2a-lower-classification.rs`'s structure: Action
//! shares Decision's closed-set replay-capsule model
//! (`ActionPersistenceCommand`/`ActionPersistenceResult`/
//! `validate_with_discarded`), and Action's Lower operation has no durable
//! post-start terminal either (see
//! `approve_and_execute_lower_action_classification`'s doc comment in
//! `actions.rs`) -- a policy denial here is an ordinary rollback, not a
//! persisted terminal like Accept/Complete/Cancel/Reopen's.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionTitle, ApproveAndExecuteAcceptActionRequest,
        ApproveAndExecuteLowerActionClassification, CreateActionRequestDraft,
        PrepareAcceptActionRequest, PrepareLowerActionClassification, StartAction,
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
            "pmc-synthetic-action-h2a-lower-classification-{nonce}-{sequence}.sqlite3"
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
        title: ActionTitle::parse("Synthetic H2a lower classification request").unwrap(),
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
            action_subject: ActionTitle::parse("Synthetic H2a lower classification request")
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

fn approval_for(
    prepared: &WorkManagementPreparedIntent,
    idempotency_id: &IdempotencyId,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
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

fn lowering_prepared(
    prepared_id: &str,
    action_id: &ActionId,
    proposed: DataClassification,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerActionClassification {
            action_id: action_id.clone(),
            action_version: AggregateVersion::initial(),
            current_classification: DataClassification::Internal,
            proposed_classification: proposed,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap()
}

#[test]
fn prepare_lower_action_classification_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "action-h2a-lower-prepared-1",
        &action_id,
        DataClassification::Public,
        &rationale,
    );
    let command = PrepareLowerActionClassification {
        action_id: action_id.clone(),
        expected_version: AggregateVersion::initial(),
        proposed_classification: DataClassification::Public,
        rationale: rationale.clone(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-1").unwrap(),
            correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-1").unwrap(),
        },
    };
    let first = writer
        .prepare_lower_action_classification(command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = writer
        .prepare_lower_action_classification(command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_lower_action_classification_rejects_a_non_lowering_proposal() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic non-lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "action-h2a-lower-prepared-2",
        &action_id,
        DataClassification::Restricted,
        &rationale,
    );
    let command = PrepareLowerActionClassification {
        action_id,
        expected_version: AggregateVersion::initial(),
        proposed_classification: DataClassification::Public,
        rationale,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-2").unwrap(),
            correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-2").unwrap(),
        },
    };
    assert!(writer
        .prepare_lower_action_classification(command, prepared)
        .is_err());
}

#[test]
fn approve_and_execute_lower_action_classification_persists_and_replays() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic execute lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "action-h2a-lower-prepared-3",
        &action_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-3")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-3")
                        .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("action-h2a-lower-execute-idempotency-3").unwrap();
    let command_execute = ApproveAndExecuteLowerActionClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: ActionOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("action-h2a-lower-execute-correlation-3").unwrap(),
        },
    };
    let audit_id = AuditEventId::parse("action-h2a-lower-audit-execute-3").unwrap();
    let receipt_id = ApprovalReceiptId::parse("action-h2a-lower-receipt-3").unwrap();
    let outcome = writer
        .approve_and_execute_lower_action_classification(
            command_execute.clone(),
            audit_id.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(outcome.record.classification(), DataClassification::Public);
    assert_eq!(outcome.record.version().get(), 2);
    assert_eq!(outcome.approval_receipt_id, Some(receipt_id.clone()));

    let replay = writer
        .approve_and_execute_lower_action_classification(
            command_execute,
            audit_id,
            receipt_id,
            UtcTimestamp::from_unix_millis(1_300),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(replay, outcome);
}

#[test]
fn approve_and_execute_lower_action_classification_rejects_a_stale_preview() {
    // Action's decode is a full closed-set replay validation (mirrors
    // Decision): a stale preview here means a second, still-outstanding
    // preview whose `expected_version` was overtaken by an earlier execute
    // against the same action -- so this test prepares two lowerings
    // against version 1, executes the first (advancing the action to
    // version 2), then attempts to execute the second, still pinned to
    // version 1.
    let (_ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic first lowering rationale").unwrap();
    let first_prepared = lowering_prepared(
        "action-h2a-lower-prepared-4a",
        &action_id,
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-4a")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-4a")
                        .unwrap(),
                },
            },
            first_prepared.clone(),
        )
        .unwrap();
    let stale_rationale = WorkManagementRationale::parse("Synthetic stale rationale").unwrap();
    let stale_prepared = lowering_prepared(
        "action-h2a-lower-prepared-4b",
        &action_id,
        DataClassification::Public,
        &stale_rationale,
    );
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale: stale_rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-4b")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-4b")
                        .unwrap(),
                },
            },
            stale_prepared.clone(),
        )
        .unwrap();

    let first_execute_idempotency_id =
        IdempotencyId::parse("action-h2a-lower-execute-idempotency-4a").unwrap();
    writer
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval: approval_for(&first_prepared, &first_execute_idempotency_id),
                context: ActionOperationContext {
                    idempotency_id: first_execute_idempotency_id,
                    correlation_id: CorrelationId::parse("action-h2a-lower-execute-correlation-4a")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-h2a-lower-audit-execute-4a").unwrap(),
            ApprovalReceiptId::parse("action-h2a-lower-receipt-4a").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();

    let stale_execute_idempotency_id =
        IdempotencyId::parse("action-h2a-lower-execute-idempotency-4b").unwrap();
    let result = writer.approve_and_execute_lower_action_classification(
        ApproveAndExecuteLowerActionClassification {
            approval: approval_for(&stale_prepared, &stale_execute_idempotency_id),
            context: ActionOperationContext {
                idempotency_id: stale_execute_idempotency_id,
                correlation_id: CorrelationId::parse("action-h2a-lower-execute-correlation-4b")
                    .unwrap(),
            },
        },
        AuditEventId::parse("action-h2a-lower-audit-execute-4b").unwrap(),
        ApprovalReceiptId::parse("action-h2a-lower-receipt-4b").unwrap(),
        UtcTimestamp::from_unix_millis(1_250),
        H2aPolicy(ActionExecutionPolicy::Allowed),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    );
    assert!(result.is_err());
}

#[test]
fn restart_recovers_the_lowered_action_after_execute() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic restart rationale").unwrap();
    let prepared = lowering_prepared(
        "action-h2a-lower-prepared-5",
        &action_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-5")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-5")
                        .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("action-h2a-lower-execute-idempotency-5").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval,
                context: ActionOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse("action-h2a-lower-execute-correlation-5")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-h2a-lower-audit-execute-5").unwrap(),
            ApprovalReceiptId::parse("action-h2a-lower-receipt-5").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    let action = snapshot
        .actions()
        .iter()
        .find(|item| item.id() == &action_id)
        .unwrap();
    assert_eq!(action.classification(), DataClassification::Public);
    assert_eq!(action.version().get(), 2);
}

#[test]
fn restart_recovers_an_outstanding_lower_classification_preview() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic outstanding rationale").unwrap();
    let prepared = lowering_prepared(
        "action-h2a-lower-prepared-6",
        &action_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-idempotency-6")
                        .unwrap(),
                    correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-6")
                        .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    drop(writer);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("action-h2a-lower-execute-idempotency-6").unwrap();
    let outcome = reopened
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: ActionOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse("action-h2a-lower-execute-correlation-6")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-h2a-lower-audit-execute-6").unwrap(),
            ApprovalReceiptId::parse("action-h2a-lower-receipt-6").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    assert_eq!(outcome.record.classification(), DataClassification::Public);
}

/// `Lower Classification -> Start`, the exact class of
/// sequence the old fixed six-pass pipeline could not handle -- Lower
/// Classification and Start (Open -> InProgress) are BOTH legal while an
/// Action is still Open, so a real ledger can do either first, but the old
/// pipeline always folded ALL of Start before ANY Lower Classification
/// activity, regardless of each capsule's real `operation_ordinal`. This
/// specific direction (Lower before Start) was the "corrected" blast radius
/// a later design review found -- broader than the `Start -> Lower -> Link`
/// example that originally motivated the unified fold (which only proved
/// Start could safely go first). The new unified fold reads `actions` live
/// at each event's own ordinal, so this now decodes correctly regardless of
/// which of the two happens first.
#[test]
fn lower_classification_then_start_decodes_losslessly() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_open_action();
    let rationale = WorkManagementRationale::parse("Synthetic lower-then-start rationale").unwrap();
    let prepared = lowering_prepared(
        "action-h2a-lower-prepared-7",
        &action_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_action_classification(
            PrepareLowerActionClassification {
                action_id: action_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-h2a-lower-prepare-7").unwrap(),
                    correlation_id: CorrelationId::parse("action-h2a-lower-prepare-correlation-7")
                        .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let lower_execute_idempotency_id =
        IdempotencyId::parse("action-h2a-lower-execute-idempotency-7").unwrap();
    let lowered = writer
        .approve_and_execute_lower_action_classification(
            ApproveAndExecuteLowerActionClassification {
                approval: approval_for(&prepared, &lower_execute_idempotency_id),
                context: ActionOperationContext {
                    idempotency_id: lower_execute_idempotency_id,
                    correlation_id: CorrelationId::parse("action-h2a-lower-execute-correlation-7")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-h2a-lower-audit-execute-7").unwrap(),
            ApprovalReceiptId::parse("action-h2a-lower-receipt-7").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Lower Classification EXECUTE must succeed on an Open Action");
    assert_eq!(lowered.record.classification(), DataClassification::Public);
    assert_eq!(lowered.record.version().get(), 2);

    let started = writer
        .start_action(
            start_command(
                "action-start-1",
                "action-start-correlation-1",
                &action_id,
                lowered.record.version(),
            ),
            AuditEventId::parse("action-start-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect("Start must succeed on an already-lowered Open Action");
    assert_eq!(started.record.state(), ActionState::InProgress);
    // Start's own reconstruction preserves whatever classification the
    // Action already carried -- the lowered value must survive, not revert.
    assert_eq!(started.record.classification(), DataClassification::Public);
    assert_eq!(started.record.version().get(), 3);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0)
        .expect("a ledger with Lower-then-Start history must still open cleanly");
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("Lower -> Start must decode losslessly under the new global fold");
    assert!(snapshot.actions().contains(&started.record));
}
