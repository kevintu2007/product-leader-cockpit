use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionPersistenceCommand, ActionPersistenceResult, ActionTitle, CreateActionRequestDraft,
        DeclineActionRequest, PrepareAcceptActionRequest, SubmitActionRequest,
        WithdrawActionRequest,
    },
    audit::AuditActor,
    classification::DataClassification,
    error::{DomainError, ErrorCode},
    identity::{
        ActionId, ActionRequestId, AggregateVersion, AuditEventId, CorrelationId, IdempotencyId,
        PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ActionRequestState, ApprovalAuthorizationPort, ApprovalConfirmation,
        WorkManagementApproval, WorkManagementOperation, WorkManagementPayloadDigest,
        WorkManagementPreparedIntent,
    },
};
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};
use rusqlite::{params, Connection};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
struct H2aPolicy(ActionExecutionPolicy);

#[derive(Clone, Copy)]
struct H2aAuthorization(bool);

impl ApprovalAuthorizationPort for H2aAuthorization {
    fn authorize(&self, _: AuditActor) -> bool {
        self.0
    }
}

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

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-action-repository-{nonce}-{sequence}.sqlite3"
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
            let _ = fs::remove_file(path);
        }
    }
}

#[test]
fn new_v2_ledger_loads_empty_action_snapshot_through_public_seam() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");

    let snapshot = opened
        .load_action_persistence_snapshot()
        .expect("new v2 ledger must expose a valid empty Action snapshot");

    assert!(snapshot.requests().is_empty());
    assert!(snapshot.actions().is_empty());
    assert!(snapshot.prepared().is_empty());
    assert!(snapshot.discarded_prepared().is_empty());
    assert!(snapshot.replay().is_empty());
    assert!(snapshot.audits().is_empty());
}

#[test]
fn empty_action_snapshot_survives_application_reopen() {
    let ledger = SyntheticLedger::new();
    {
        let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
        let snapshot = opened
            .load_action_persistence_snapshot()
            .expect("first empty Action load must succeed");
        assert!(snapshot.requests().is_empty());
    }

    let reopened = SqliteProductLedger::open(&ledger.0).expect("v2 ledger must reopen");
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("reopened empty Action load must succeed");
    assert!(snapshot.requests().is_empty());
    assert!(snapshot.actions().is_empty());
}

#[test]
fn action_owned_shared_rows_fail_closed_after_reopen() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);

    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    connection
        .execute(
            "INSERT INTO aggregate_registry (id, aggregate_type, version, classification, created_at, updated_at) VALUES ('synthetic-action-request', 'action_request', 1, 'internal', 0, 0)",
            [],
        )
        .expect("synthetic Action Request registry row must insert");
    connection
        .execute(
            "INSERT INTO prepared_intents (id, contract_version, intent_kind, payload_digest, classification, policy, cancellation_policy, authority, expires_at, created_at) VALUES (?1, 1, 'complete_action', 'synthetic-digest', 'internal', 'allowed', 'not_cancellable_after_submit', 'head_of_products', 100, 0)",
            params!["synthetic-action-prepared"],
        )
        .expect("synthetic Action prepared intent must insert");
    drop(connection);

    let reopened = SqliteProductLedger::open(&ledger.0).expect("v2 ledger must reopen");
    assert!(matches!(
        reopened.load_action_persistence_snapshot(),
        Err(pmc_ledger::sqlite::ActionPersistenceLoadError::InvalidActionSnapshot)
    ));
}

#[test]
fn action_classification_source_role_fails_closed_without_an_action_parent() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);

    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    insert_non_action_prepared_intent(&connection, "synthetic-decision-prepared");
    connection
        .execute(
            "INSERT INTO prepared_intent_classification_sources (prepared_intent_id, ordinal, role, source_id, classification) VALUES (?1, 0, 'created_action', 'synthetic-action', 'internal')",
            params!["synthetic-decision-prepared"],
        )
        .expect("synthetic Action classification role must insert");
    drop(connection);

    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn action_audit_effect_fails_closed_without_an_action_audit_parent() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);

    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    connection
        .execute(
            "INSERT INTO audit_events (id, occurred_at, actor, module, event_code, target_type, target_id, correlation_id, policy_outcome, approval_outcome, execution_outcome, effect_scope) VALUES ('synthetic-decision-audit', 0, 'head_of_products', 'work_management', 'synthetic', 'decision', 'synthetic-decision', 'synthetic-correlation', 'not_required', 'not_required', 'not_attempted', 'none')",
            [],
        )
        .expect("synthetic non-Action audit must insert");
    connection
        .execute(
            "INSERT INTO audit_effects (audit_event_id, ordinal, effect_code, scope, target_type, target_id) VALUES ('synthetic-decision-audit', 0, 'synthetic', 'none', 'action', 'synthetic-action')",
            [],
        )
        .expect("synthetic Action audit effect must insert");
    drop(connection);

    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn action_operation_effect_fails_closed_without_an_action_operation_parent() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);

    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    connection
        .execute(
            "INSERT INTO operations (id, namespace, operation, correlation_id, idempotency_id, status, started_at) VALUES ('synthetic-decision-operation', 'decision', 'resolve', 'synthetic-correlation', 'synthetic-idempotency', 'succeeded', 0)",
            [],
        )
        .expect("synthetic non-Action operation must insert");
    connection
        .execute(
            "INSERT INTO operation_effects (operation_id, ordinal, effect_code, scope, target_type, target_id) VALUES ('synthetic-decision-operation', 0, 'synthetic', 'none', 'action', 'synthetic-action')",
            [],
        )
        .expect("synthetic Action operation effect must insert");
    drop(connection);

    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn action_operation_namespace_fails_closed_without_a_prepared_intent() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);

    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    connection
        .execute(
            "INSERT INTO operations (id, namespace, operation, correlation_id, idempotency_id, status, started_at) VALUES ('synthetic-action-operation', 'action', 'start', 'synthetic-correlation', 'synthetic-idempotency', 'succeeded', 0)",
            [],
        )
        .expect("synthetic Action operation must insert");
    drop(connection);

    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn action_idempotency_namespace_fails_closed_without_an_action_record() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);

    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    connection
        .execute(
            "INSERT INTO idempotency_outcomes (namespace, operation, idempotency_id, payload_digest, outcome_code, created_at) VALUES ('action', 'start', 'synthetic-idempotency', 'synthetic-digest', 'succeeded', 0)",
            [],
        )
        .expect("synthetic Action idempotency outcome must insert");
    drop(connection);

    assert_invalid_action_snapshot(&ledger.0);
}

fn insert_non_action_prepared_intent(connection: &Connection, id: &str) {
    connection
        .execute(
            "INSERT INTO prepared_intents (id, contract_version, intent_kind, payload_digest, classification, policy, cancellation_policy, authority, expires_at, created_at) VALUES (?1, 1, 'resolve_decision_request', 'synthetic-digest', 'internal', 'allowed', 'not_cancellable_after_submit', 'head_of_products', 100, 0)",
            params![id],
        )
        .expect("synthetic non-Action prepared intent must insert");
}

fn assert_invalid_action_snapshot(path: &std::path::Path) {
    let reopened = SqliteProductLedger::open(path).expect("v2 ledger must reopen");
    assert!(matches!(
        reopened.load_action_persistence_snapshot(),
        Err(pmc_ledger::sqlite::ActionPersistenceLoadError::InvalidActionSnapshot)
    ));
}

#[test]
fn canonical_create_request_draft_is_the_next_lossless_decode_shape() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).expect("synthetic v2 ledger must open");
    drop(opened);
    let connection = Connection::open(&ledger.0).expect("synthetic database must open");
    let transaction = connection
        .unchecked_transaction()
        .expect("synthetic seed transaction must start");
    transaction
        .execute(
            "INSERT INTO aggregate_registry (id, aggregate_type, version, classification, created_at, updated_at) VALUES ('stakeholder-owner-1', 'stakeholder', 1, 'internal', 99, 99)",
            [],
        )
        .expect("synthetic owner registry row must insert");
    transaction
        .execute(
            "INSERT INTO stakeholders (id, name, kind, provenance_kind, provenance_reference) VALUES ('stakeholder-owner-1', 'Synthetic Product Owner', 'person', 'synthetic_fixture', 'public-safe-fixture')",
            [],
        )
        .expect("synthetic owner row must insert");
    transaction
        .execute(
            "INSERT INTO aggregate_registry (id, aggregate_type, version, classification, created_at, updated_at) VALUES ('request-draft-1', 'action_request', 1, 'internal', 100, 100)",
            [],
        )
        .expect("synthetic Action Request registry row must insert");
    transaction
        .execute(
            "INSERT INTO action_requests (id, title, details, intended_owner_id, response_due_at, intended_action_due_at, state, terminal_rationale, linked_action_id, source_decision_id, superseded_premise) VALUES ('request-draft-1', 'Synthetic roadmap review', 'Review the public-safe roadmap draft and record the next governed action.', 'stakeholder-owner-1', 200, 300, 'draft', NULL, NULL, NULL, 0)",
            [],
        )
        .expect("synthetic draft Action Request row must insert");
    transaction
        .execute(
            "INSERT INTO action_replay_operations (idempotency_id, operation, correlation_id, operation_ordinal, result_kind, result_reference, prepared_disposition) VALUES ('create-request-1', 'create_request', 'create-correlation-1', 0, 'request', 'request-draft-1', 'not_applicable')",
            [],
        )
        .expect("synthetic create replay row must insert");
    transaction
        .execute(
            "INSERT INTO action_command_create_requests (idempotency_id, request_id, title, details, intended_owner_id, response_due_at, intended_action_due_at, classification) VALUES ('create-request-1', 'request-draft-1', 'Synthetic roadmap review', 'Review the public-safe roadmap draft and record the next governed action.', 'stakeholder-owner-1', 200, 300, 'internal')",
            [],
        )
        .expect("synthetic create command row must insert");
    transaction
        .execute(
            "INSERT INTO audit_events (id, occurred_at, actor, module, event_code, target_type, target_id, correlation_id, policy_outcome, approval_outcome, execution_outcome, effect_scope) VALUES ('audit-create-1', 101, 'head_of_products', 'work_management', 'action_request.created', 'action_request', 'request-draft-1', 'create-correlation-1', 'allowed', 'not_required', 'succeeded', 'complete')",
            [],
        )
        .expect("synthetic create audit row must insert");
    transaction
        .execute(
            "INSERT INTO audit_effects (audit_event_id, ordinal, effect_code, scope, target_type, target_id) VALUES ('audit-create-1', 0, 'action_request.created', 'complete', 'action_request', 'request-draft-1')",
            [],
        )
        .expect("synthetic create audit effect must insert");
    transaction
        .execute(
            "INSERT INTO action_replay_audits (idempotency_id, ordinal, audit_event_id, correlation_id) VALUES ('create-request-1', 0, 'audit-create-1', 'create-correlation-1')",
            [],
        )
        .expect("synthetic replay audit row must insert");
    transaction
        .execute(
            "INSERT INTO prepared_intents (id, contract_version, intent_kind, payload_digest, classification, policy, cancellation_policy, authority, expires_at, created_at) VALUES ('decision-prepared-1', 1, 'resolve_decision_request', 'synthetic-digest', 'internal', 'allowed', 'not_cancellable_after_submit', 'head_of_products', 100, 0)",
            [],
        )
        .expect("unrelated synthetic Decision prepared intent must insert");
    transaction
        .commit()
        .expect("synthetic seed transaction must commit");

    let reopened = SqliteProductLedger::open(&ledger.0).expect("v2 ledger must reopen");
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("canonical create-request draft must decode losslessly");
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.requests()[0].id().as_str(), "request-draft-1");
    assert_eq!(
        snapshot.requests()[0].title().as_str(),
        "Synthetic roadmap review"
    );
    assert_eq!(
        snapshot.requests()[0].details().as_str(),
        "Review the public-safe roadmap draft and record the next governed action."
    );
    assert_eq!(
        snapshot.requests()[0]
            .intended_owner()
            .expect("synthetic owner must rehydrate")
            .as_str(),
        "stakeholder-owner-1"
    );
    assert_eq!(
        snapshot.requests()[0]
            .response_due_at()
            .unwrap()
            .unix_millis(),
        200
    );
    assert_eq!(
        snapshot.requests()[0]
            .intended_action_due_at()
            .unwrap()
            .unix_millis(),
        300
    );
    assert_eq!(
        snapshot.requests()[0].classification(),
        pmc_domain::classification::DataClassification::Internal
    );
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);
    assert_eq!(
        snapshot.replay()[0].original_correlation_id().as_str(),
        "create-correlation-1"
    );
    assert_eq!(
        snapshot.replay()[0].audit_event_ids()[0].as_str(),
        "audit-create-1"
    );
    assert!(matches!(
        snapshot.replay()[0].command(),
        ActionPersistenceCommand::CreateRequest {
            id,
            title,
            details,
            intended_owner: Some(owner),
            response_due_at: Some(response_due_at),
            intended_action_due_at: Some(intended_action_due_at),
            classification,
        } if id.as_str() == "request-draft-1"
            && title.as_str() == "Synthetic roadmap review"
            && details.as_str() == "Review the public-safe roadmap draft and record the next governed action."
            && owner.as_str() == "stakeholder-owner-1"
            && response_due_at.unix_millis() == 200
            && intended_action_due_at.unix_millis() == 300
            && *classification == pmc_domain::classification::DataClassification::Internal
    ));
    assert!(matches!(
        snapshot.replay()[0].result(),
        ActionPersistenceResult::Request(outcome)
            if outcome.record.id().as_str() == "request-draft-1"
                && outcome.audit_events[0].id().as_str() == "audit-create-1"
                && outcome.audit_events[0].correlation_id().as_str() == "create-correlation-1"
    ));
    assert_eq!(snapshot.audits()[0].occurred_at().unix_millis(), 101);
    assert_eq!(
        snapshot.audits()[0].code().as_str(),
        "action_request.created"
    );
    assert_eq!(
        snapshot.audits()[0].policy_outcome().as_persisted(),
        "allowed"
    );
    assert_eq!(
        snapshot.audits()[0].approval_outcome().as_persisted(),
        "not_required"
    );
    assert_eq!(
        snapshot.audits()[0].execution_outcome().as_persisted(),
        "succeeded"
    );
    assert_eq!(
        snapshot.audits()[0].effect_scope().as_persisted(),
        "complete"
    );
    assert_eq!(snapshot.audits()[0].actual_effects().len(), 1);
    assert_eq!(
        snapshot.audits()[0].actual_effects()[0].as_str(),
        "action_request.created"
    );

    let connection = Connection::open(&ledger.0).expect("synthetic database must reopen");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON; UPDATE action_replay_operations SET result_kind='action' WHERE idempotency_id='create-request-1';",
        )
        .expect("synthetic result-kind tamper must be applied");
    let tampered_result_kind = SqliteProductLedger::open(&ledger.0).expect("v2 ledger must reopen");
    assert!(matches!(
        tampered_result_kind.load_action_persistence_snapshot(),
        Err(pmc_ledger::sqlite::ActionPersistenceLoadError::InvalidActionSnapshot)
    ));
    connection
        .execute(
            "UPDATE action_replay_operations SET result_kind='request' WHERE idempotency_id='create-request-1'",
            [],
        )
        .expect("synthetic result-kind must be restored");
    connection
        .execute(
            "INSERT INTO prepared_intent_targets(prepared_intent_id, ordinal, target_type, target_id, expected_version) VALUES ('decision-prepared-1', 0, 'action', 'request-draft-1', 1)",
            [],
        )
        .expect("synthetic Action target on unrelated Decision intent must insert");
    connection
        .execute(
            "INSERT INTO prepared_intent_effects(prepared_intent_id, ordinal, effect_code, target_type, target_id) VALUES ('decision-prepared-1', 0, 'unexpected.action_effect', 'action', 'request-draft-1')",
            [],
        )
        .expect("synthetic Action effect on unrelated Decision intent must insert");
    let action_topology_tampered =
        SqliteProductLedger::open(&ledger.0).expect("v2 ledger must reopen");
    assert!(matches!(
        action_topology_tampered.load_action_persistence_snapshot(),
        Err(pmc_ledger::sqlite::ActionPersistenceLoadError::InvalidActionSnapshot)
    ));
    connection
        .execute(
            "DELETE FROM prepared_intent_effects WHERE prepared_intent_id='decision-prepared-1'",
            [],
        )
        .expect("synthetic Action effect tamper must be removed");
    connection
        .execute(
            "DELETE FROM prepared_intent_targets WHERE prepared_intent_id='decision-prepared-1'",
            [],
        )
        .expect("synthetic Action target tamper must be removed");

    connection
        .execute(
            "INSERT INTO audit_effects (audit_event_id, ordinal, effect_code, scope, target_type, target_id) VALUES ('audit-create-1', 1, 'unexpected.extra', 'complete', 'action_request', 'request-draft-1')",
            [],
        )
        .expect("synthetic extra audit effect must insert");
    let tampered = SqliteProductLedger::open(&ledger.0).expect("v2 ledger must reopen");
    assert!(matches!(
        tampered.load_action_persistence_snapshot(),
        Err(pmc_ledger::sqlite::ActionPersistenceLoadError::InvalidActionSnapshot)
    ));
}

fn writer_command(idempotency: &str, correlation: &str, request: &str) -> CreateActionRequestDraft {
    CreateActionRequestDraft {
        id: ActionRequestId::parse(request).unwrap(),
        title: pmc_domain::actions::ActionTitle::parse("Synthetic writer request").unwrap(),
        details: pmc_domain::actions::ActionDetails::parse(
            "Persist a public-safe draft through the durable Action writer.",
        )
        .unwrap(),
        intended_owner: None,
        response_due_at: Some(UtcTimestamp::from_unix_millis(200)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(300)),
        classification: DataClassification::Internal,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn writer_owned_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
) -> CreateActionRequestDraft {
    let mut command = writer_command(idempotency, correlation, request);
    command.intended_owner = Some(StakeholderId::parse("stakeholder-owner-1").unwrap());
    command
}

#[test]
fn writer_commits_create_draft_before_success_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let command = writer_command(
        "writer-create-1",
        "writer-correlation-1",
        "writer-request-1",
    );
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let outcome = opened
        .create_action_request_draft(
            command.clone(),
            AuditEventId::parse("writer-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .expect("writer must commit the typed draft");
    assert_eq!(outcome.record.id().as_str(), "writer-request-1");
    assert_eq!(opened.revision().unwrap(), 1);
    drop(opened);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("committed writer state must decode after reopen");
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);
    assert_eq!(snapshot.replay()[0].operation_ordinal(), 0);
    assert!(matches!(
        snapshot.replay()[0].result(),
        ActionPersistenceResult::Request(result)
            if result.record.id().as_str() == "writer-request-1"
                && result.audit_events[0].id().as_str() == "writer-audit-1"
    ));
}

fn submit_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
    version: u64,
) -> SubmitActionRequest {
    SubmitActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::new(version).unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn prepared_accept_fixture(
    request: &str,
    version: u64,
    prepared_id: &str,
    action_id: &str,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::AcceptActionRequest {
            request_id: ActionRequestId::parse(request).unwrap(),
            request_version: AggregateVersion::new(version).unwrap(),
            action_id: ActionId::parse(action_id).unwrap(),
            action_classification: DataClassification::Internal,
            action_subject: ActionTitle::parse("Synthetic writer request").unwrap(),
            commitment_details: ActionDetails::parse(
                "Persist a public-safe draft through the durable Action writer.",
            )
            .unwrap(),
            intended_owner: StakeholderId::parse("stakeholder-owner-1").unwrap(),
            intended_due_at: UtcTimestamp::from_unix_millis(300),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

fn seeded_h2a_accept(
    prefix: &str,
) -> (
    SyntheticLedger,
    SqliteProductLedger,
    WorkManagementPreparedIntent,
) {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
    ).unwrap();
    drop(connection);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let request = format!("{prefix}-request");
    opened
        .create_action_request_draft(
            writer_owned_command(
                &format!("{prefix}-create"),
                &format!("{prefix}-create-correlation"),
                &request,
            ),
            AuditEventId::parse(format!("{prefix}-create-audit")).unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                &format!("{prefix}-submit"),
                &format!("{prefix}-submit-correlation"),
                &request,
                1,
            ),
            AuditEventId::parse(format!("{prefix}-submit-audit")).unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepared = prepared_accept_fixture(
        &request,
        2,
        &format!("{prefix}-prepared"),
        &format!("{prefix}-action"),
    );
    opened
        .prepare_accept_action_request(
            prepare_command(
                &format!("{prefix}-prepare"),
                &format!("{prefix}-prepare-correlation"),
                &request,
                2,
            ),
            prepared.clone(),
        )
        .unwrap();
    (ledger, opened, prepared)
}

fn execute_h2a_accept(
    ledger: &mut SqliteProductLedger,
    prefix: &str,
    prepared: &WorkManagementPreparedIntent,
    digest: WorkManagementPayloadDigest,
    at: UtcTimestamp,
    authorization: impl ApprovalAuthorizationPort,
    policy: impl ActionExecutionPolicyPort,
) -> Result<pmc_domain::actions::AcceptedActionOutcome, Box<DomainError>> {
    ledger
        .approve_and_execute_accept_action_request(
            pmc_domain::actions::ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    digest,
                    IdempotencyId::parse(format!("{prefix}-execute")).unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse(format!("{prefix}-execute")).unwrap(),
                    correlation_id: CorrelationId::parse(format!("{prefix}-execute-correlation"))
                        .unwrap(),
                },
            },
            [
                AuditEventId::parse(format!("{prefix}-request-audit")).unwrap(),
                AuditEventId::parse(format!("{prefix}-action-audit")).unwrap(),
                AuditEventId::parse(format!("{prefix}-link-audit")).unwrap(),
            ],
            pmc_domain::identity::ApprovalReceiptId::parse(format!("{prefix}-receipt")).unwrap(),
            at,
            authorization,
            policy,
        )
        .map_err(|error| match error {
            LedgerTransactionError::Operation(error) => Box::new(error),
            other => {
                panic!("synthetic terminal test must not fail at the storage boundary: {other:?}")
            }
        })
}

fn prepare_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
    version: u64,
) -> PrepareAcceptActionRequest {
    PrepareAcceptActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::new(version).unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn decline_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
    version: u64,
    rationale: &str,
) -> DeclineActionRequest {
    DeclineActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::new(version).unwrap(),
        rationale: pmc_domain::actions::ActionDetails::parse(rationale).unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn withdraw_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
    version: u64,
    rationale: &str,
) -> WithdrawActionRequest {
    WithdrawActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::new(version).unwrap(),
        rationale: pmc_domain::actions::ActionDetails::parse(rationale).unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

#[test]
fn writer_declines_open_request_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "decline-create",
                "decline-create-correlation",
                "decline-request",
            ),
            AuditEventId::parse("decline-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "decline-submit",
                "decline-submit-correlation",
                "decline-request",
                1,
            ),
            AuditEventId::parse("decline-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = decline_command(
        "decline-terminal",
        "decline-terminal-correlation",
        "decline-request",
        2,
        "Synthetic scope is not approved",
    );
    let first = opened
        .decline_action_request(
            command.clone(),
            AuditEventId::parse("decline-terminal-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();
    assert_eq!(
        first.record.state(),
        pmc_domain::work_management::ActionRequestState::Declined
    );
    assert_eq!(first.record.version().get(), 3);
    assert_eq!(
        first.record.terminal_rationale().unwrap().as_str(),
        "Synthetic scope is not approved"
    );
    assert_eq!(opened.revision().unwrap(), 3);
    let replay = opened
        .decline_action_request(
            command,
            AuditEventId::parse("decline-terminal-ignored-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(opened.revision().unwrap(), 3);
    drop(opened);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(
        snapshot.requests()[0].state(),
        pmc_domain::work_management::ActionRequestState::Declined
    );
    assert_eq!(snapshot.requests()[0].version().get(), 3);
    assert_eq!(
        snapshot.requests()[0]
            .terminal_rationale()
            .unwrap()
            .as_str(),
        "Synthetic scope is not approved"
    );
    assert_eq!(snapshot.replay().len(), 3);
    assert!(matches!(
        snapshot.replay()[2].command(),
        ActionPersistenceCommand::TransitionRequest { request_id, expected_version, target_state, rationale }
            if request_id.as_str() == "decline-request"
                && expected_version.get() == 2
                && *target_state == pmc_domain::work_management::ActionRequestState::Declined
                && rationale.as_ref().map(|value| value.as_str()) == Some("Synthetic scope is not approved")
    ));
    assert_eq!(
        snapshot.audits()[2].code().as_str(),
        "action_request.declined"
    );
}

#[test]
fn writer_decline_rejects_stale_illegal_and_changed_commands() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "decline-errors-create",
                "decline-errors-create-correlation",
                "decline-errors-request",
            ),
            AuditEventId::parse("decline-errors-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let missing = opened
        .decline_action_request(
            decline_command(
                "decline-missing",
                "decline-missing-correlation",
                "missing-request",
                2,
                "Synthetic missing",
            ),
            AuditEventId::parse("decline-missing-audit").unwrap(),
            UtcTimestamp::from_unix_millis(101),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(missing) = missing else {
        panic!("missing request must return a domain error");
    };
    assert_eq!(missing.code(), ErrorCode::DomainNotFound);
    let stale = opened
        .decline_action_request(
            decline_command(
                "decline-stale",
                "decline-stale-correlation",
                "decline-errors-request",
                2,
                "Synthetic stale",
            ),
            AuditEventId::parse("decline-stale-audit").unwrap(),
            UtcTimestamp::from_unix_millis(102),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(stale) = stale else {
        panic!("stale request must return a domain error");
    };
    assert_eq!(stale.code(), ErrorCode::DomainConflict);
    opened
        .submit_action_request(
            submit_command(
                "decline-errors-submit",
                "decline-errors-submit-correlation",
                "decline-errors-request",
                1,
            ),
            AuditEventId::parse("decline-errors-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = decline_command(
        "decline-errors-terminal",
        "decline-errors-terminal-correlation",
        "decline-errors-request",
        2,
        "Synthetic first rationale",
    );
    opened
        .decline_action_request(
            command.clone(),
            AuditEventId::parse("decline-errors-terminal-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();
    let changed = opened
        .decline_action_request(
            decline_command(
                "decline-errors-terminal",
                "decline-errors-changed-correlation",
                "decline-errors-request",
                2,
                "Synthetic changed rationale",
            ),
            AuditEventId::parse("decline-errors-changed-audit").unwrap(),
            UtcTimestamp::from_unix_millis(301),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(changed) = changed else {
        panic!("changed rationale must return a domain error");
    };
    assert_eq!(changed.code(), ErrorCode::DomainIdempotencyConflict);
    let illegal = opened
        .decline_action_request(
            decline_command(
                "decline-errors-illegal",
                "decline-errors-illegal-correlation",
                "decline-errors-request",
                3,
                "Synthetic second attempt",
            ),
            AuditEventId::parse("decline-errors-illegal-audit").unwrap(),
            UtcTimestamp::from_unix_millis(302),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(illegal) = illegal else {
        panic!("terminal request must return a domain error");
    };
    assert_eq!(illegal.code(), ErrorCode::DomainConflict);
    assert_eq!(opened.revision().unwrap(), 3);
}

#[test]
fn decline_audit_collision_rolls_back_terminal_bundle() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "decline-collision-create",
                "decline-collision-create-correlation",
                "decline-collision-request",
            ),
            AuditEventId::parse("decline-collision-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "decline-collision-submit",
                "decline-collision-submit-correlation",
                "decline-collision-request",
                1,
            ),
            AuditEventId::parse("decline-collision-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let error = opened
        .decline_action_request(
            decline_command(
                "decline-collision-terminal",
                "decline-collision-terminal-correlation",
                "decline-collision-request",
                2,
                "Synthetic audit collision",
            ),
            AuditEventId::parse("decline-collision-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LedgerTransactionError::Operation(DomainError { .. })
    ));
    assert_eq!(opened.revision().unwrap(), 2);
    drop(opened);
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(
        snapshot.requests()[0].state(),
        pmc_domain::work_management::ActionRequestState::Open
    );
    assert_eq!(snapshot.requests()[0].version().get(), 2);
    assert_eq!(snapshot.replay().len(), 2);
    assert_eq!(snapshot.audits().len(), 2);
}

#[test]
fn writer_withdraws_open_request_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "withdraw-create",
                "withdraw-create-correlation",
                "withdraw-request",
            ),
            AuditEventId::parse("withdraw-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "withdraw-submit",
                "withdraw-submit-correlation",
                "withdraw-request",
                1,
            ),
            AuditEventId::parse("withdraw-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = withdraw_command(
        "withdraw-terminal",
        "withdraw-terminal-correlation",
        "withdraw-request",
        2,
        "Synthetic owner withdrew the request",
    );
    let first = opened
        .withdraw_action_request(
            command.clone(),
            AuditEventId::parse("withdraw-terminal-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();
    assert_eq!(first.record.state(), ActionRequestState::Withdrawn);
    assert_eq!(first.record.version().get(), 3);
    assert_eq!(
        first.record.terminal_rationale().unwrap().as_str(),
        "Synthetic owner withdrew the request"
    );
    assert_eq!(
        first.audit_events[0].code().as_str(),
        "action_request.withdrawn"
    );
    let replay = opened
        .withdraw_action_request(
            command,
            AuditEventId::parse("withdraw-terminal-unused-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(opened.revision().unwrap(), 3);
    drop(opened);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(
        snapshot.requests()[0].state(),
        ActionRequestState::Withdrawn
    );
    assert_eq!(snapshot.requests()[0].version().get(), 3);
    assert_eq!(snapshot.replay().len(), 3);
    assert_eq!(
        snapshot.audits()[2].code().as_str(),
        "action_request.withdrawn"
    );
    assert!(matches!(
        snapshot.replay()[2].command(),
        ActionPersistenceCommand::TransitionRequest {
            request_id,
            expected_version,
            target_state,
            rationale
        } if request_id.as_str() == "withdraw-request"
            && expected_version.get() == 2
            && *target_state == ActionRequestState::Withdrawn
            && rationale.as_ref().map(|value| value.as_str())
                == Some("Synthetic owner withdrew the request")
    ));
}

#[test]
fn withdrawn_request_rejects_submit_and_second_withdraw_while_decline_remains_distinct() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "withdraw-illegal-create",
                "withdraw-illegal-create-correlation",
                "withdraw-illegal-request",
            ),
            AuditEventId::parse("withdraw-illegal-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "withdraw-illegal-submit",
                "withdraw-illegal-submit-correlation",
                "withdraw-illegal-request",
                1,
            ),
            AuditEventId::parse("withdraw-illegal-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    opened
        .withdraw_action_request(
            withdraw_command(
                "withdraw-illegal-terminal",
                "withdraw-illegal-terminal-correlation",
                "withdraw-illegal-request",
                2,
                "Synthetic withdrawal",
            ),
            AuditEventId::parse("withdraw-illegal-terminal-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap();
    let submit = opened
        .submit_action_request(
            submit_command(
                "withdraw-illegal-after",
                "withdraw-illegal-after-correlation",
                "withdraw-illegal-request",
                3,
            ),
            AuditEventId::parse("withdraw-illegal-after-audit").unwrap(),
            UtcTimestamp::from_unix_millis(400),
        )
        .unwrap_err();
    assert!(
        matches!(submit, LedgerTransactionError::Operation(error) if error.code() == ErrorCode::DomainConflict)
    );
    let second = opened
        .withdraw_action_request(
            withdraw_command(
                "withdraw-illegal-second",
                "withdraw-illegal-second-correlation",
                "withdraw-illegal-request",
                3,
                "Synthetic second withdrawal",
            ),
            AuditEventId::parse("withdraw-illegal-second-audit").unwrap(),
            UtcTimestamp::from_unix_millis(401),
        )
        .unwrap_err();
    assert!(
        matches!(second, LedgerTransactionError::Operation(error) if error.code() == ErrorCode::DomainConflict)
    );

    opened
        .create_action_request_draft(
            writer_command(
                "decline-distinct-create",
                "decline-distinct-create-correlation",
                "decline-distinct-request",
            ),
            AuditEventId::parse("decline-distinct-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(500),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "decline-distinct-submit",
                "decline-distinct-submit-correlation",
                "decline-distinct-request",
                1,
            ),
            AuditEventId::parse("decline-distinct-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(600),
        )
        .unwrap();
    opened
        .decline_action_request(
            decline_command(
                "decline-distinct-terminal",
                "decline-distinct-terminal-correlation",
                "decline-distinct-request",
                2,
                "Synthetic decline",
            ),
            AuditEventId::parse("decline-distinct-terminal-audit").unwrap(),
            UtcTimestamp::from_unix_millis(700),
        )
        .unwrap();
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot
        .requests()
        .iter()
        .any(
            |request| request.id().as_str() == "withdraw-illegal-request"
                && request.state() == ActionRequestState::Withdrawn
        ));
    assert!(snapshot
        .requests()
        .iter()
        .any(
            |request| request.id().as_str() == "decline-distinct-request"
                && request.state() == ActionRequestState::Declined
        ));
}

#[test]
fn withdraw_audit_collision_rolls_back_terminal_bundle() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "withdraw-collision-create",
                "withdraw-collision-create-correlation",
                "withdraw-collision-request",
            ),
            AuditEventId::parse("withdraw-collision-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "withdraw-collision-submit",
                "withdraw-collision-submit-correlation",
                "withdraw-collision-request",
                1,
            ),
            AuditEventId::parse("withdraw-collision-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let error = opened
        .withdraw_action_request(
            withdraw_command(
                "withdraw-collision-terminal",
                "withdraw-collision-terminal-correlation",
                "withdraw-collision-request",
                2,
                "Synthetic audit collision",
            ),
            AuditEventId::parse("withdraw-collision-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LedgerTransactionError::Operation(DomainError { .. })
    ));
    assert_eq!(opened.revision().unwrap(), 2);
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Open);
    assert_eq!(snapshot.requests()[0].version().get(), 2);
    assert_eq!(snapshot.replay().len(), 2);
    assert_eq!(snapshot.audits().len(), 2);
}

#[test]
fn writer_submits_draft_to_open_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let create = writer_command(
        "submit-create-1",
        "submit-create-correlation",
        "submit-request-1",
    );
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let create_outcome = opened
        .create_action_request_draft(
            create,
            AuditEventId::parse("submit-audit-create").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let draft_wrong_version = opened
        .submit_action_request(
            submit_command(
                "submit-draft-wrong-version",
                "submit-draft-wrong-version-correlation",
                "submit-request-1",
                2,
            ),
            AuditEventId::parse("submit-draft-wrong-version-audit").unwrap(),
            UtcTimestamp::from_unix_millis(99),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(draft_error) = draft_wrong_version else {
        panic!("wrong Draft version must be a domain conflict");
    };
    assert_eq!(draft_error.code(), ErrorCode::DomainConflict);
    assert_eq!(draft_error.message_key().as_str(), "action.domain_conflict");
    assert!(draft_error.extensions().iter().any(|extension| matches!(
        extension,
        pmc_domain::error::SafeErrorExtension::CurrentVersion(version) if version.get() == 1
    )));
    assert_eq!(
        draft_error
            .params()
            .iter()
            .map(|param| param.key())
            .collect::<Vec<_>>(),
        vec!["current_state", "allowed_next_intent_1"]
    );
    let command = submit_command(
        "submit-open-1",
        "submit-open-correlation",
        "submit-request-1",
        1,
    );
    let outcome = opened
        .submit_action_request(
            command.clone(),
            AuditEventId::parse("submit-audit-open").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .expect("typed H1 submit must commit");
    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::ActionRequestState::Open
    );
    assert_eq!(outcome.record.version().get(), 2);
    assert_eq!(opened.revision().unwrap(), 2);
    let create_conflict = opened
        .create_action_request_draft(
            writer_command(
                "submit-open-1",
                "submit-open-used-for-create",
                "submit-request-1",
            ),
            AuditEventId::parse("submit-open-used-for-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(998),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(create_error) = create_conflict else {
        panic!("submit idempotency reused by Create must be a domain conflict");
    };
    assert_eq!(create_error.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(
        create_error.correlation_id().as_str(),
        "submit-open-used-for-create"
    );
    assert_eq!(opened.revision().unwrap(), 2);
    let create_replay = opened
        .create_action_request_draft(
            writer_command(
                "submit-create-1",
                "submit-create-retry-correlation",
                "submit-request-1",
            ),
            AuditEventId::parse("submit-audit-create-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .expect("create idempotency must replay after submit");
    assert_eq!(create_replay, create_outcome);
    assert_eq!(opened.revision().unwrap(), 2);
    drop(opened);
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(
        snapshot.requests()[0].state(),
        pmc_domain::work_management::ActionRequestState::Open
    );
    assert_eq!(snapshot.requests()[0].version().get(), 2);
    assert_eq!(snapshot.replay().len(), 2);
    assert_eq!(snapshot.replay()[0].operation_ordinal(), 0);
    assert_eq!(snapshot.replay()[1].operation_ordinal(), 1);
    assert!(
        matches!(snapshot.replay()[1].command(), ActionPersistenceCommand::TransitionRequest { request_id, expected_version, target_state, rationale } if request_id.as_str() == "submit-request-1" && expected_version.get() == 1 && *target_state == pmc_domain::work_management::ActionRequestState::Open && rationale.is_none())
    );
    assert_eq!(
        snapshot.audits()[1].code().as_str(),
        "action_request.submitted"
    );
}

#[test]
fn writer_prepares_accept_intent_without_mutating_request_or_action_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
        )
        .unwrap();
    drop(connection);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_owned_command(
                "prepare-create",
                "prepare-create-correlation",
                "prepare-request",
            ),
            AuditEventId::parse("prepare-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "prepare-submit",
                "prepare-submit-correlation",
                "prepare-request",
                1,
            ),
            AuditEventId::parse("prepare-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = prepare_command(
        "prepare-accept",
        "prepare-accept-correlation",
        "prepare-request",
        2,
    );
    let prepared =
        prepared_accept_fixture("prepare-request", 2, "prepared-accept-1", "action-accept-1");
    let result = opened
        .prepare_accept_action_request(command.clone(), prepared.clone())
        .expect("typed H2a prepare must persist");
    assert_eq!(result, prepared);
    assert_eq!(opened.revision().unwrap(), 3);

    drop(opened);
    let mut opened = SqliteProductLedger::open(&ledger.0)
        .expect("prepared H2a ledger must reopen through the public adapter");
    let snapshot = opened
        .load_action_persistence_snapshot()
        .expect("prepared H2a snapshot must decode");
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Open);
    assert_eq!(snapshot.requests()[0].version().get(), 2);
    assert!(snapshot.actions().is_empty());
    assert_eq!(snapshot.prepared(), &[prepared.clone()]);
    assert!(matches!(
        snapshot.replay().last().unwrap().result(),
        ActionPersistenceResult::Prepared(intent) if intent.id().as_str() == "prepared-accept-1"
    ));

    let replay = opened
        .prepare_accept_action_request(command.clone(), prepared.clone())
        .expect("exact H2a prepare retry must replay");
    assert_eq!(replay, prepared);
    assert_eq!(opened.revision().unwrap(), 3);

    opened
        .decline_action_request(
            decline_command(
                "prepare-decline",
                "prepare-decline-correlation",
                "prepare-request",
                2,
                "Synthetic owner declined the proposed commitment",
            ),
            AuditEventId::parse("prepare-decline-audit").unwrap(),
            UtcTimestamp::from_unix_millis(400),
        )
        .expect("later H1 decline must not invalidate a prior H2a replay");
    let replay_after_terminal = opened
        .prepare_accept_action_request(command.clone(), prepared.clone())
        .expect("exact H2a prepare retry after terminal transition must replay");
    assert_eq!(replay_after_terminal, prepared);
    assert_eq!(opened.revision().unwrap(), 4);

    let changed =
        prepared_accept_fixture("prepare-request", 2, "prepared-accept-2", "action-accept-2");
    let conflict = opened
        .prepare_accept_action_request(command, changed)
        .expect_err("changed same-key H2a prepare must conflict");
    let LedgerTransactionError::Operation(error) = conflict else {
        panic!("changed same-key prepare must return a domain conflict");
    };
    assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);

    drop(opened);
    let connection =
        Connection::open(&ledger.0).expect("synthetic ledger must reopen for tampering");
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=2000 WHERE id='prepared-accept-1'",
            [],
        )
        .expect("synthetic consumed H2a intent corruption must apply");
    drop(connection);
    assert_invalid_action_snapshot(&ledger.0);

    let connection =
        Connection::open(&ledger.0).expect("synthetic ledger must reopen for tampering");
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=NULL WHERE id='prepared-accept-1'",
            [],
        )
        .expect("synthetic H2a intent corruption cleanup must apply");
    connection
        .execute(
            "INSERT INTO prepared_intents (id, contract_version, intent_kind, payload_digest, classification, policy, cancellation_policy, authority, expires_at, created_at) VALUES ('unsupported-action-prepare', 1, 'complete_action', 'synthetic-digest', 'internal', 'allowed', 'not_cancellable_after_submit', 'head_of_products', 100, 0)",
            [],
        )
        .expect("synthetic unsupported Action prepared intent must insert");
    drop(connection);
    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn writer_executes_approved_accept_atomically_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
        )
        .unwrap();
    drop(connection);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_owned_command(
                "execute-create",
                "execute-create-correlation",
                "execute-request",
            ),
            AuditEventId::parse("execute-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "execute-submit",
                "execute-submit-correlation",
                "execute-request",
                1,
            ),
            AuditEventId::parse("execute-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepare = prepare_command(
        "execute-prepare",
        "execute-prepare-correlation",
        "execute-request",
        2,
    );
    let prepared = prepared_accept_fixture(
        "execute-request",
        2,
        "execute-prepared-1",
        "execute-action-1",
    );
    opened
        .prepare_accept_action_request(prepare, prepared.clone())
        .expect("H2a preview must persist before approval");
    opened
        .create_action_request_draft(
            writer_owned_command(
                "execute-second-create",
                "execute-second-create-correlation",
                "execute-request-2",
            ),
            AuditEventId::parse("execute-second-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "execute-second-submit",
                "execute-second-submit-correlation",
                "execute-request-2",
                1,
            ),
            AuditEventId::parse("execute-second-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    opened
        .prepare_accept_action_request(
            prepare_command(
                "execute-second-prepare",
                "execute-second-prepare-correlation",
                "execute-request-2",
                2,
            ),
            prepared_accept_fixture(
                "execute-request-2",
                2,
                "execute-prepared-2",
                "execute-action-2",
            ),
        )
        .expect("second pending H2a preview must persist");
    let approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("execute-accept").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let execute_command = pmc_domain::actions::ApproveAndExecuteAcceptActionRequest {
        approval,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse("execute-accept").unwrap(),
            correlation_id: CorrelationId::parse("execute-accept-correlation").unwrap(),
        },
    };
    let accepted = opened
        .approve_and_execute_accept_action_request(
            execute_command.clone(),
            [
                AuditEventId::parse("execute-accept-request-audit").unwrap(),
                AuditEventId::parse("execute-accept-action-audit").unwrap(),
                AuditEventId::parse("execute-accept-link-audit").unwrap(),
            ],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-accept-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("approved H2a accept must atomically create the linked Action");
    assert_eq!(accepted.request.state(), ActionRequestState::Accepted);
    assert_eq!(accepted.request.version().get(), 3);
    assert_eq!(accepted.action.id().as_str(), "execute-action-1");
    assert_eq!(accepted.action.version().get(), 1);
    assert_eq!(
        accepted.action.source_request_id().as_str(),
        "execute-request"
    );
    assert_eq!(opened.revision().unwrap(), 7);

    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0)
        .expect("approved H2a ledger must reopen through the public adapter");
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("accepted H2a history must rehydrate exactly");
    assert_eq!(snapshot.requests().len(), 2);
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Accepted);
    assert_eq!(snapshot.actions().len(), 1);
    assert_eq!(snapshot.prepared().len(), 1);
    assert_eq!(snapshot.prepared()[0].id().as_str(), "execute-prepared-2");
    assert_eq!(snapshot.audits().len(), 7);
    let replayed = reopened
        .approve_and_execute_accept_action_request(
            execute_command,
            [
                AuditEventId::parse("execute-replay-request-audit").unwrap(),
                AuditEventId::parse("execute-replay-action-audit").unwrap(),
                AuditEventId::parse("execute-replay-link-audit").unwrap(),
            ],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-replay-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("exact execute retry must replay without consuming new authority");
    assert_eq!(replayed, accepted);
    assert_eq!(reopened.revision().unwrap(), 7);
    let changed_approval = WorkManagementApproval::new(
        PreparedIntentId::parse("execute-prepared-2").unwrap(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("execute-accept").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let changed = reopened
        .approve_and_execute_accept_action_request(
            pmc_domain::actions::ApproveAndExecuteAcceptActionRequest {
                approval: changed_approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("execute-accept").unwrap(),
                    correlation_id: CorrelationId::parse("execute-changed-correlation").unwrap(),
                },
            },
            [
                AuditEventId::parse("execute-changed-request-audit").unwrap(),
                AuditEventId::parse("execute-changed-action-audit").unwrap(),
                AuditEventId::parse("execute-changed-link-audit").unwrap(),
            ],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-changed-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect_err("changed execute approval under the same key must conflict");
    let LedgerTransactionError::Operation(error) = changed else {
        panic!("changed H2a execute must return a domain conflict");
    };
    assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(reopened.revision().unwrap(), 7);
    drop(reopened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=NULL WHERE id='execute-prepared-1'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=1100 WHERE id='execute-prepared-2'",
            [],
        )
        .unwrap();
    drop(connection);
    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn writer_persists_post_prepare_policy_denial_as_a_consumed_h2a_terminal() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
        )
        .unwrap();
    drop(connection);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_owned_command("deny-create", "deny-create-correlation", "deny-request"),
            AuditEventId::parse("deny-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command("deny-submit", "deny-submit-correlation", "deny-request", 1),
            AuditEventId::parse("deny-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepared = prepared_accept_fixture("deny-request", 2, "deny-prepared", "deny-action");
    opened
        .prepare_accept_action_request(
            prepare_command(
                "deny-prepare",
                "deny-prepare-correlation",
                "deny-request",
                2,
            ),
            prepared.clone(),
        )
        .unwrap();
    let denied = opened
        .approve_and_execute_accept_action_request(
            pmc_domain::actions::ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("deny-execute").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("deny-execute").unwrap(),
                    correlation_id: CorrelationId::parse("deny-execute-correlation").unwrap(),
                },
            },
            [
                AuditEventId::parse("deny-request-audit").unwrap(),
                AuditEventId::parse("deny-action-audit").unwrap(),
                AuditEventId::parse("deny-link-audit").unwrap(),
            ],
            pmc_domain::identity::ApprovalReceiptId::parse("deny-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Denied),
        )
        .expect_err("current policy denial must prevent execution");
    let LedgerTransactionError::Operation(error) = denied else {
        panic!("policy denial must return a safe domain error");
    };
    assert_eq!(error.code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(opened.revision().unwrap(), 4);
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Open);
    assert!(snapshot.actions().is_empty());
    assert!(snapshot.prepared().is_empty());
    assert_eq!(
        snapshot.discarded_prepared(),
        std::slice::from_ref(&prepared)
    );
    assert_eq!(snapshot.audits().len(), 3);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replayed = reopened
        .approve_and_execute_accept_action_request(
            pmc_domain::actions::ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    PreparedIntentId::parse("deny-prepared").unwrap(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("deny-execute").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("deny-execute").unwrap(),
                    correlation_id: CorrelationId::parse("deny-retry-correlation").unwrap(),
                },
            },
            [
                AuditEventId::parse("deny-retry-request-audit").unwrap(),
                AuditEventId::parse("deny-retry-action-audit").unwrap(),
                AuditEventId::parse("deny-retry-link-audit").unwrap(),
            ],
            pmc_domain::identity::ApprovalReceiptId::parse("deny-retry-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect_err("a persisted policy denial must replay and cannot execute after re-enable");
    let LedgerTransactionError::Operation(replayed_error) = replayed else {
        panic!("terminal H2a replay must return the stored domain error");
    };
    assert_eq!(replayed_error, error);
    assert_eq!(reopened.revision().unwrap(), 4);
}

#[test]
fn writer_persists_unauthorized_h2a_terminal_across_reopen_and_exact_replay() {
    let (ledger, mut opened, prepared) = seeded_h2a_accept("unauthorized");
    let first = execute_h2a_accept(
        &mut opened,
        "unauthorized",
        &prepared,
        prepared.payload_digest().clone(),
        UtcTimestamp::from_unix_millis(1_100),
        H2aAuthorization(false),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("unauthorized approval must be terminal");
    assert_eq!(first.code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(opened.revision().unwrap(), 4);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().is_empty());
    assert!(snapshot.prepared().is_empty());
    assert_eq!(
        snapshot.discarded_prepared(),
        std::slice::from_ref(&prepared)
    );
    let replay = execute_h2a_accept(
        &mut reopened,
        "unauthorized",
        &prepared,
        prepared.payload_digest().clone(),
        UtcTimestamp::from_unix_millis(1_200),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("exact terminal replay must remain safe");
    assert_eq!(replay, first);
    assert_eq!(reopened.revision().unwrap(), 4);
}

#[test]
fn writer_persists_digest_mismatch_h2a_terminal_across_reopen_and_exact_replay() {
    let (ledger, mut opened, prepared) = seeded_h2a_accept("mismatch");
    let mismatch = WorkManagementPayloadDigest::from_persisted(
        "0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
    )
    .unwrap();
    let first = execute_h2a_accept(
        &mut opened,
        "mismatch",
        &prepared,
        mismatch.clone(),
        UtcTimestamp::from_unix_millis(1_100),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("digest mismatch must be terminal");
    assert_eq!(first.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(opened.revision().unwrap(), 4);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().is_empty());
    assert_eq!(
        snapshot.discarded_prepared(),
        std::slice::from_ref(&prepared)
    );
    let replay = execute_h2a_accept(
        &mut reopened,
        "mismatch",
        &prepared,
        mismatch,
        UtcTimestamp::from_unix_millis(1_200),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("exact digest terminal replay must be stable");
    assert_eq!(replay, first);
    assert_eq!(reopened.revision().unwrap(), 4);
}

#[test]
fn writer_persists_expired_h2a_terminal_across_reopen_and_exact_replay() {
    let (ledger, mut opened, prepared) = seeded_h2a_accept("expired");
    let first = execute_h2a_accept(
        &mut opened,
        "expired",
        &prepared,
        prepared.payload_digest().clone(),
        UtcTimestamp::from_unix_millis(prepared.preview().expires_at().unix_millis() + 1),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("expired approval must be terminal");
    assert_eq!(first.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(opened.revision().unwrap(), 4);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(
        reopened
            .load_action_persistence_snapshot()
            .unwrap()
            .discarded_prepared(),
        std::slice::from_ref(&prepared)
    );
    let replay = execute_h2a_accept(
        &mut reopened,
        "expired",
        &prepared,
        prepared.payload_digest().clone(),
        UtcTimestamp::from_unix_millis(prepared.preview().expires_at().unix_millis() + 2),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("exact expiry replay must be stable");
    assert_eq!(replay, first);
    assert_eq!(reopened.revision().unwrap(), 4);
}

#[test]
fn writer_retains_changed_h2a_preview_across_reopen_and_exact_replay() {
    let (ledger, mut opened, prepared) = seeded_h2a_accept("changed-preview");
    opened
        .decline_action_request(
            decline_command(
                "changed-preview-decline",
                "changed-preview-decline-correlation",
                "changed-preview-request",
                2,
                "Synthetic request changed after preview preparation",
            ),
            AuditEventId::parse("changed-preview-decline-audit").unwrap(),
            UtcTimestamp::from_unix_millis(400),
        )
        .unwrap();
    let first = execute_h2a_accept(
        &mut opened,
        "changed-preview",
        &prepared,
        prepared.payload_digest().clone(),
        UtcTimestamp::from_unix_millis(1_100),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("changed authoritative request must retain the preview safely");
    assert_eq!(first.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(opened.revision().unwrap(), 5);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Declined);
    assert!(snapshot.actions().is_empty());
    assert_eq!(snapshot.prepared(), std::slice::from_ref(&prepared));
    assert!(snapshot.discarded_prepared().is_empty());
    let replay = execute_h2a_accept(
        &mut reopened,
        "changed-preview",
        &prepared,
        prepared.payload_digest().clone(),
        UtcTimestamp::from_unix_millis(1_200),
        H2aAuthorization(true),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    )
    .expect_err("exact retained terminal replay must be stable");
    assert_eq!(replay, first);
    assert_eq!(reopened.revision().unwrap(), 5);
}

#[test]
fn writer_submits_exactly_once_and_rejects_changed_or_stale_commands() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "submit-replay-create",
                "submit-replay-create-correlation",
                "submit-replay-request",
            ),
            AuditEventId::parse("submit-replay-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let command = submit_command(
        "submit-replay-open",
        "submit-replay-open-correlation",
        "submit-replay-request",
        1,
    );
    let first = opened
        .submit_action_request(
            command.clone(),
            AuditEventId::parse("submit-replay-open-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let replay = opened
        .submit_action_request(
            command,
            AuditEventId::parse("submit-replay-ignored-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(opened.revision().unwrap(), 2);
    let illegal_open = opened
        .submit_action_request(
            submit_command(
                "submit-illegal-open",
                "submit-illegal-open-correlation",
                "submit-replay-request",
                1,
            ),
            AuditEventId::parse("submit-illegal-open-audit").unwrap(),
            UtcTimestamp::from_unix_millis(250),
        )
        .unwrap_err();
    let LedgerTransactionError::Operation(illegal_error) = illegal_open else {
        panic!("Open request cannot be submitted again");
    };
    assert_eq!(illegal_error.code(), ErrorCode::DomainConflict);
    assert_eq!(
        illegal_error.message_key().as_str(),
        "action.domain_conflict"
    );
    assert!(illegal_error.extensions().iter().any(|extension| matches!(
        extension,
        pmc_domain::error::SafeErrorExtension::CurrentVersion(version) if version.get() == 2
    )));
    assert_eq!(
        illegal_error
            .params()
            .iter()
            .map(|param| param.key())
            .collect::<Vec<_>>(),
        vec![
            "current_state",
            "allowed_next_intent_1",
            "allowed_next_intent_2",
            "allowed_next_intent_3"
        ]
    );
    let changed = submit_command(
        "submit-replay-open",
        "submit-replay-changed-correlation",
        "submit-replay-request",
        2,
    );
    let conflict = opened
        .submit_action_request(
            changed,
            AuditEventId::parse("submit-replay-changed-audit").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .unwrap_err();
    assert!(matches!(
        conflict,
        LedgerTransactionError::Operation(DomainError { .. })
    ));
    let stale = submit_command(
        "submit-stale-open",
        "submit-stale-correlation",
        "submit-replay-request",
        1,
    );
    let error = opened
        .submit_action_request(
            stale,
            AuditEventId::parse("submit-stale-audit").unwrap(),
            UtcTimestamp::from_unix_millis(301),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LedgerTransactionError::Operation(DomainError { .. })
    ));
    if let LedgerTransactionError::Operation(error) = error {
        assert_eq!(error.code(), ErrorCode::DomainConflict);
        assert!(error.extensions().iter().any(|extension| matches!(
            extension,
            pmc_domain::error::SafeErrorExtension::CurrentVersion(version) if version.get() == 2
        )));
        assert_eq!(
            error
                .params()
                .iter()
                .map(|param| param.key())
                .collect::<Vec<_>>(),
            vec![
                "current_state",
                "allowed_next_intent_1",
                "allowed_next_intent_2",
                "allowed_next_intent_3"
            ]
        );
    }
    let reused_create_id = opened
        .submit_action_request(
            submit_command(
                "submit-replay-create",
                "submit-replay-create-as-submit",
                "submit-replay-request",
                1,
            ),
            AuditEventId::parse("submit-replay-create-as-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(302),
        )
        .unwrap_err();
    if let LedgerTransactionError::Operation(error) = reused_create_id {
        assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);
    } else {
        panic!("reusing create idempotency must be a domain conflict");
    }
    assert_eq!(opened.revision().unwrap(), 2);
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.replay().len(), 2);
    assert_eq!(snapshot.audits().len(), 2);
}

#[test]
fn writer_interleaves_multiple_create_and_submit_operations_losslessly() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    for (suffix, request) in [("a", "interleave-request-a"), ("b", "interleave-request-b")] {
        opened
            .create_action_request_draft(
                writer_command(
                    &format!("interleave-create-{suffix}"),
                    &format!("interleave-create-correlation-{suffix}"),
                    request,
                ),
                AuditEventId::parse(format!("interleave-create-audit-{suffix}")).unwrap(),
                UtcTimestamp::from_unix_millis(if suffix == "a" { 100 } else { 101 }),
            )
            .unwrap();
    }
    opened
        .submit_action_request(
            submit_command(
                "interleave-submit-b",
                "interleave-submit-correlation-b",
                "interleave-request-b",
                1,
            ),
            AuditEventId::parse("interleave-submit-audit-b").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "interleave-submit-a",
                "interleave-submit-correlation-a",
                "interleave-request-a",
                1,
            ),
            AuditEventId::parse("interleave-submit-audit-a").unwrap(),
            UtcTimestamp::from_unix_millis(201),
        )
        .unwrap();
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests().len(), 2);
    assert_eq!(snapshot.replay().len(), 4);
    assert_eq!(
        snapshot
            .replay()
            .iter()
            .map(|capsule| capsule.operation_ordinal())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(
        snapshot
            .replay()
            .iter()
            .map(|capsule| capsule.idempotency_id().as_str())
            .collect::<Vec<_>>(),
        vec![
            "interleave-create-a",
            "interleave-create-b",
            "interleave-submit-b",
            "interleave-submit-a"
        ]
    );
}

#[test]
fn submitted_snapshot_tampering_fails_closed_for_transition_audit_effect_and_correlation() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "tamper-submit-create",
                "tamper-submit-create-correlation",
                "tamper-submit-request",
            ),
            AuditEventId::parse("tamper-submit-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_action_request(
            submit_command(
                "tamper-submit-open",
                "tamper-submit-open-correlation",
                "tamper-submit-request",
                1,
            ),
            AuditEventId::parse("tamper-submit-open-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    drop(opened);

    for (column, value) in [
        ("target_state", "declined"),
        ("rationale", "synthetic tamper"),
    ] {
        let connection = Connection::open(&ledger.0).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
            .unwrap();
        connection
            .execute(
                &format!("UPDATE action_command_transition_requests SET {column}=?1 WHERE idempotency_id='tamper-submit-open'"),
                [value],
            )
            .unwrap();
        assert_invalid_action_snapshot(&ledger.0);
        if column == "target_state" {
            connection
                .execute("UPDATE action_command_transition_requests SET target_state='open' WHERE idempotency_id='tamper-submit-open'", [])
                .unwrap();
        } else {
            connection
                .execute("UPDATE action_command_transition_requests SET rationale=NULL WHERE idempotency_id='tamper-submit-open'", [])
                .unwrap();
        }
    }
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON; UPDATE audit_effects SET effect_code='tampered.effect' WHERE audit_event_id='tamper-submit-open-audit';")
        .unwrap();
    assert_invalid_action_snapshot(&ledger.0);
    connection
        .execute("UPDATE audit_effects SET effect_code='action_request.submitted' WHERE audit_event_id='tamper-submit-open-audit'", [])
        .unwrap();
    connection
        .execute("INSERT INTO audit_effects (audit_event_id, ordinal, effect_code, scope, target_type, target_id) VALUES ('tamper-submit-open-audit', 1, 'unexpected.extra', 'complete', 'action_request', 'tamper-submit-request')", [])
        .unwrap();
    assert_invalid_action_snapshot(&ledger.0);
    connection
        .execute("DELETE FROM audit_effects WHERE audit_event_id='tamper-submit-open-audit' AND ordinal=1", [])
        .unwrap();
    connection
        .execute("UPDATE audit_effects SET target_id=NULL WHERE audit_event_id='tamper-submit-open-audit' AND ordinal=0", [])
        .unwrap();
    assert_invalid_action_snapshot(&ledger.0);
    connection
        .execute("UPDATE audit_effects SET target_id='tamper-submit-request' WHERE audit_event_id='tamper-submit-open-audit' AND ordinal=0", [])
        .unwrap();
    connection
        .execute("UPDATE audit_events SET correlation_id='tampered-correlation' WHERE id='tamper-submit-open-audit'", [])
        .unwrap();
    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn mixed_history_create_replay_cross_reference_fails_closed() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    for (suffix, request) in [
        ("a", "cross-reference-request-a"),
        ("b", "cross-reference-request-b"),
    ] {
        opened
            .create_action_request_draft(
                writer_command(
                    &format!("cross-reference-create-{suffix}"),
                    &format!("cross-reference-correlation-{suffix}"),
                    request,
                ),
                AuditEventId::parse(format!("cross-reference-audit-{suffix}")).unwrap(),
                UtcTimestamp::from_unix_millis(100),
            )
            .unwrap();
    }
    drop(opened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON; UPDATE action_replay_operations SET result_reference='cross-reference-request-b' WHERE idempotency_id='cross-reference-create-a';")
        .unwrap();
    assert_invalid_action_snapshot(&ledger.0);
}

#[test]
fn submit_audit_collision_rolls_back_request_version_and_replay_bundle() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "late-collision-create",
                "late-collision-create-correlation",
                "late-collision-request",
            ),
            AuditEventId::parse("late-collision-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let error = opened
        .submit_action_request(
            submit_command(
                "late-collision-submit",
                "late-collision-submit-correlation",
                "late-collision-request",
                1,
            ),
            AuditEventId::parse("late-collision-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        LedgerTransactionError::Operation(DomainError { .. })
    ));
    assert_eq!(opened.revision().unwrap(), 1);
    drop(opened);
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(
        snapshot.requests()[0].state(),
        pmc_domain::work_management::ActionRequestState::Draft
    );
    assert_eq!(snapshot.requests()[0].version().get(), 1);
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);
    assert_eq!(snapshot.audits()[0].id().as_str(), "late-collision-audit");
}

#[test]
fn two_distinct_writer_creates_reopen_and_replay_without_new_effects() {
    let ledger = SyntheticLedger::new();
    let first_command = writer_command(
        "writer-multi-1",
        "writer-multi-correlation-1",
        "writer-multi-request-z",
    );
    let second_command = writer_command(
        "writer-multi-2",
        "writer-multi-correlation-2",
        "writer-multi-request-a",
    );
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = opened
        .create_action_request_draft(
            first_command.clone(),
            AuditEventId::parse("writer-multi-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let second = opened
        .create_action_request_draft(
            second_command.clone(),
            AuditEventId::parse("writer-multi-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(101),
        )
        .unwrap();
    assert_eq!(opened.revision().unwrap(), 2);
    drop(opened);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests().len(), 2);
    assert_eq!(snapshot.replay().len(), 2);
    assert_eq!(snapshot.audits().len(), 2);
    assert_eq!(
        snapshot
            .requests()
            .iter()
            .map(|request| request.id().as_str())
            .collect::<Vec<_>>(),
        vec!["writer-multi-request-a", "writer-multi-request-z"]
    );
    assert_eq!(
        snapshot
            .replay()
            .iter()
            .map(|capsule| capsule.operation_ordinal())
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(
        snapshot
            .audits()
            .iter()
            .map(|audit| audit.id().as_str())
            .collect::<Vec<_>>(),
        vec!["writer-multi-audit-1", "writer-multi-audit-2"]
    );
    let first_replay = reopened
        .create_action_request_draft(
            first_command,
            AuditEventId::parse("writer-multi-audit-ignored-1").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    let second_replay = reopened
        .create_action_request_draft(
            second_command,
            AuditEventId::parse("writer-multi-audit-ignored-2").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    assert_eq!(first_replay, first);
    assert_eq!(second_replay, second);
    assert_eq!(reopened.revision().unwrap(), 2);
    let after_replay = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(after_replay.requests().len(), 2);
    assert_eq!(after_replay.replay().len(), 2);
    assert_eq!(after_replay.audits().len(), 2);
}

#[test]
fn writer_replays_exact_idempotency_without_duplicate_effects_and_rejects_changed_payload() {
    let ledger = SyntheticLedger::new();
    let command = writer_command(
        "writer-replay-1",
        "writer-correlation-1",
        "writer-request-1",
    );
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let first = opened
        .create_action_request_draft(
            command.clone(),
            AuditEventId::parse("writer-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let mut replay = command.clone();
    replay.context.correlation_id = CorrelationId::parse("writer-correlation-2").unwrap();
    let second = opened
        .create_action_request_draft(
            replay,
            AuditEventId::parse("writer-audit-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .expect("same business command must replay");
    assert_eq!(first, second);
    assert_eq!(opened.revision().unwrap(), 1);
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);

    let mut changed = command;
    changed.details = pmc_domain::actions::ActionDetails::parse(
        "A changed public-safe payload must not overwrite the original request.",
    )
    .unwrap();
    let conflict = opened
        .create_action_request_draft(
            changed,
            AuditEventId::parse("writer-audit-changed").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap_err();
    assert!(matches!(
        conflict,
        LedgerTransactionError::Operation(DomainError { .. })
    ));
    if let LedgerTransactionError::Operation(error) = conflict {
        assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);
    }
    assert_eq!(opened.revision().unwrap(), 1);
    let unchanged = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(unchanged.requests().len(), 1);
    assert_eq!(unchanged.audits().len(), 1);
}

#[test]
fn writer_rolls_back_all_rows_when_audit_identity_collides() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_action_request_draft(
            writer_command(
                "writer-rollback-1",
                "writer-correlation-1",
                "writer-request-1",
            ),
            AuditEventId::parse("writer-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let error = opened
        .create_action_request_draft(
            writer_command(
                "writer-rollback-2",
                "writer-correlation-2",
                "writer-request-2",
            ),
            AuditEventId::parse("writer-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap_err();
    assert!(matches!(error, LedgerTransactionError::Operation(_)));
    assert_eq!(opened.revision().unwrap(), 1);
    let snapshot = opened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests().len(), 1);
    assert_eq!(snapshot.requests()[0].id().as_str(), "writer-request-1");
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);
}

#[test]
fn writer_rolls_back_when_existing_action_namespace_is_malformed() {
    let ledger = SyntheticLedger::new();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('malformed-existing', 'action_request', 2, 'internal', 0, 0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO action_requests (id,title,details,intended_owner_id,response_due_at,intended_action_due_at,state,terminal_rationale,linked_action_id,source_decision_id,superseded_premise) VALUES ('malformed-existing','Malformed synthetic request','This row has an unsupported registry version.',NULL,NULL,NULL,'draft',NULL,NULL,NULL,0)",
            [],
        )
        .unwrap();
    drop(connection);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let error = reopened
        .create_action_request_draft(
            writer_command(
                "writer-malformed-existing",
                "writer-malformed-correlation",
                "writer-malformed-new",
            ),
            AuditEventId::parse("writer-malformed-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap_err();
    assert!(matches!(error, LedgerTransactionError::Operation(_)));
    assert_eq!(reopened.revision().unwrap(), 0);
    let connection = Connection::open(&ledger.0).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM action_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM action_replay_operations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}

#[test]
fn writer_rejects_decoder_invalid_negative_timestamp_without_partial_rows() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut command = writer_command(
        "writer-negative-time",
        "writer-negative-correlation",
        "writer-negative-request",
    );
    command.response_due_at = Some(UtcTimestamp::from_unix_millis(-1));
    let error = opened
        .create_action_request_draft(
            command,
            AuditEventId::parse("writer-negative-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap_err();
    assert!(matches!(error, LedgerTransactionError::Operation(_)));
    assert_eq!(opened.revision().unwrap(), 0);
    let connection = Connection::open(&ledger.0).unwrap();
    for table in [
        "action_requests",
        "action_replay_operations",
        "action_command_create_requests",
        "audit_events",
        "audit_effects",
        "action_replay_audits",
    ] {
        let query = format!("SELECT count(*) FROM {table}");
        assert_eq!(
            connection
                .query_row(&query, [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
