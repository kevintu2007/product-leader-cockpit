//! `StartAction` (Open -> InProgress) -- one of the two prerequisites of
//! Action Complete: Complete's PREPARE needs both this and
//! `LinkActionCompletionEvidence`.
//!
//! Mirrors `sqlite-action-reopen.rs`'s setup exactly up through Accept, then
//! calls `start_action` directly instead of continuing into Cancel.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionTitle, ApproveAndExecuteAcceptActionRequest, CreateActionRequestDraft,
        PrepareAcceptActionRequest, StartAction, SubmitActionRequest,
    },
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent,
    },
};
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};
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
            "pmc-synthetic-action-start-{nonce}-{sequence}.sqlite3"
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
        title: ActionTitle::parse("Synthetic Start request").unwrap(),
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
            action_subject: ActionTitle::parse("Synthetic Start request").unwrap(),
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

/// Seeds a ledger through H1 create+submit and H2a accept, returning the
/// handle and the resulting Open Action's id (always `action-1`,
/// classification `Internal`, version 1).
fn seeded_ledger_with_accepted_action() -> (SyntheticLedger, SqliteProductLedger, ActionId) {
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
    (ledger, writer, action_id)
}

#[test]
fn writer_starts_an_open_action_and_reopens_losslessly() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_accepted_action();

    let outcome = writer
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
        .expect("an Open Action must start atomically");
    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::ActionState::InProgress
    );
    assert_eq!(
        outcome.record.version(),
        AggregateVersion::initial().next().unwrap()
    );
    assert_eq!(outcome.audit_events.len(), 1);
    drop(writer);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().contains(&outcome.record));

    assert_eq!(
        reopened
            .start_action(
                start_command(
                    "action-start-1",
                    "action-start-correlation-1",
                    &action_id,
                    AggregateVersion::initial(),
                ),
                AuditEventId::parse("action-start-replay-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(1_200),
            )
            .unwrap(),
        outcome
    );
}

#[test]
fn writer_rejects_start_action_on_a_stale_expected_version() {
    let (_ledger, mut writer, action_id) = seeded_ledger_with_accepted_action();
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

    let stale = writer.start_action(
        start_command(
            "action-start-2",
            "action-start-correlation-2",
            &action_id,
            AggregateVersion::initial(),
        ),
        AuditEventId::parse("action-start-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
    );
    assert!(matches!(stale, Err(LedgerTransactionError::Operation(_))));
}
