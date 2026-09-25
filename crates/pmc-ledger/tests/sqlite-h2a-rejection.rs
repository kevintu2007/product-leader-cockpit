//! v45: durable, audited rejection of a work-management Prepared Intent
//! (Action Accept and Decision Resolve). A rejection is a successful
//! command with no effect: it consumes the intent, records one zero-effect
//! audit, mints no receipt, changes no aggregate, and replays exactly.
//!
//! Action scaffolding mirrors `sqlite-action-start.rs` up through the
//! Accept PREPARE; Decision scaffolding mirrors
//! `sqlite-decision-repository.rs`'s judgment-supported Resolve preview.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionTitle, ApproveAndExecuteAcceptActionRequest, CreateActionRequestDraft,
        PrepareAcceptActionRequest, RejectActionPreparedIntent, SubmitActionRequest,
    },
    audit::{AuditActor, AuditApprovalOutcome, AuditEffectScope, AuditExecutionOutcome},
    classification::DataClassification,
    decisions::{
        ApproveAndExecuteResolveDecisionRequest, CreateDecisionRequestDraft,
        DecisionEvidenceAuthorityError, DecisionEvidenceAuthorityPort, DecisionExecutionPolicy,
        DecisionExecutionPolicyPort, DecisionOperationContext, DecisionSubject, DecisionText,
        PrepareResolveDecisionRequest, RejectDecisionPreparedIntent, SubmitDecisionRequest,
    },
    error::ErrorCode,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, DecisionId, DecisionRequestId, EvidenceReferenceId, IdempotencyId,
        PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ActionRequestState, ApprovalAuthorizationPort, ApprovalConfirmation, DecisionRequestState,
        DecisionResultingActionRequest, EvidenceOrJudgment, EvidenceReferenceMetadata,
        HumanJudgment, HumanJudgmentDisposition, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, ACTION_PREPARED_REJECTED_AUDIT_CODE,
        DECISION_PREPARED_REJECTED_AUDIT_CODE,
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
            "pmc-synthetic-h2a-rejection-{nonce}-{sequence}.sqlite3"
        )))
    }
}

#[derive(Clone, Copy)]
struct Gate;

impl ApprovalAuthorizationPort for Gate {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

impl ActionExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}

impl DecisionExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        DecisionExecutionPolicy::Allowed
    }
}

impl DecisionEvidenceAuthorityPort for Gate {
    fn resolve(
        &self,
        _: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        Err(DecisionEvidenceAuthorityError::Unavailable)
    }
}

fn action_context(idempotency: &str, correlation: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn decision_context(idempotency: &str, correlation: &str) -> DecisionOperationContext {
    DecisionOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn count(path: &std::path::Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}

fn operation_error<T: std::fmt::Debug>(
    result: Result<T, LedgerTransactionError<pmc_domain::error::DomainError>>,
) -> pmc_domain::error::DomainError {
    match result {
        Err(LedgerTransactionError::Operation(error)) => error,
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------- Action

fn prepared_accept_fixture(request: &str, prepared_id: &str) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::AcceptActionRequest {
            request_id: ActionRequestId::parse(request).unwrap(),
            request_version: AggregateVersion::initial().next().unwrap(),
            action_id: ActionId::parse("action-1").unwrap(),
            action_classification: DataClassification::Internal,
            action_subject: ActionTitle::parse("Synthetic rejection request").unwrap(),
            commitment_details: ActionDetails::parse("Refuse this preview durably.").unwrap(),
            intended_owner: StakeholderId::parse("stakeholder-owner-1").unwrap(),
            intended_due_at: UtcTimestamp::from_unix_millis(300_000),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

/// H1 create+submit and an Accept PREPARE for `request-1`, returning the
/// pending preview (`action-prepared-1`, prepared at 1_000).
fn ledger_with_pending_accept() -> (
    SyntheticLedger,
    SqliteProductLedger,
    WorkManagementPreparedIntent,
) {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    Connection::open(&ledger.0).unwrap().execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
    ).unwrap();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_action_request_draft(
            CreateActionRequestDraft {
                id: ActionRequestId::parse("request-1").unwrap(),
                title: ActionTitle::parse("Synthetic rejection request").unwrap(),
                details: ActionDetails::parse("Refuse this preview durably.").unwrap(),
                intended_owner: Some(StakeholderId::parse("stakeholder-owner-1").unwrap()),
                response_due_at: Some(UtcTimestamp::from_unix_millis(200_000)),
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(300_000)),
                classification: DataClassification::Internal,
                context: action_context("action-create-1", "action-create-correlation-1"),
            },
            AuditEventId::parse("action-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_action_request(
            SubmitActionRequest {
                request_id: ActionRequestId::parse("request-1").unwrap(),
                expected_version: AggregateVersion::initial(),
                context: action_context("action-submit-1", "action-submit-correlation-1"),
            },
            AuditEventId::parse("action-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepared = prepared_accept_fixture("request-1", "action-prepared-1");
    writer
        .prepare_accept_action_request(
            PrepareAcceptActionRequest {
                request_id: ActionRequestId::parse("request-1").unwrap(),
                expected_version: AggregateVersion::initial().next().unwrap(),
                context: action_context("action-prepare-1", "action-prepare-correlation-1"),
            },
            prepared.clone(),
        )
        .unwrap();
    (ledger, writer, prepared)
}

fn reject_action(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> RejectActionPreparedIntent {
    RejectActionPreparedIntent {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        context: action_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

fn accept_execution(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> ApproveAndExecuteAcceptActionRequest {
    ApproveAndExecuteAcceptActionRequest {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse(idempotency).unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: action_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

#[test]
fn rejecting_a_pending_accept_preview_consumes_it_audits_nothing_and_reopens_losslessly() {
    let (ledger, mut writer, prepared) = ledger_with_pending_accept();
    let revision_before = writer.revision().unwrap();

    let outcome = writer
        .reject_action_prepared_intent(
            reject_action(&prepared, "action-reject-1"),
            AuditEventId::parse("action-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
            Gate,
        )
        .expect("a pending Accept preview must be rejectable");
    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(1_150));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.id().as_str(), "action-reject-audit-1");
    assert_eq!(audit.code().as_str(), ACTION_PREPARED_REJECTED_AUDIT_CODE);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());
    assert_eq!(writer.revision().unwrap(), revision_before + 1);
    drop(writer);

    // Durable shape: consumed at the rejection instant, one result row that
    // claimed its idempotency key, no receipt, no discard-through-terminal.
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM prepared_intents WHERE id='action-prepared-1' AND consumed_at=1150"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM action_reject_prepared_command_results WHERE idempotency_id='action-reject-1' AND prepared_intent_id='action-prepared-1' AND rejected_at=1150 AND audit_event_id='action-reject-audit-1' AND operation_ordinal=3"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='action-reject-1' AND namespace='action' AND operation='reject_prepared'"), 1);
    assert_eq!(
        count(&ledger.0, "SELECT count(*) FROM approval_receipts"),
        0
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM action_discarded_prepared_intents"
        ),
        0
    );
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM audit_events WHERE id='action-reject-audit-1' AND event_code='action.prepared_rejected' AND target_type='action_request' AND target_id='request-1' AND approval_outcome='rejected' AND execution_outcome='not_attempted' AND effect_scope='none'"), 1);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_effects WHERE audit_event_id='action-reject-audit-1'"
        ),
        0
    );

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    assert_eq!(snapshot.discarded_prepared(), &[prepared.clone()]);
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Open);
    let last = snapshot.replay().last().unwrap();
    assert_eq!(last.operation_ordinal(), 3);
    assert_eq!(last.idempotency_id().as_str(), "action-reject-1");
    assert_eq!(
        last.result(),
        &pmc_domain::actions::ActionPersistenceResult::Rejected(outcome.clone())
    );
    assert_eq!(snapshot.audits().last(), Some(audit));

    // Exact replay; a different id against the consumed intent conflicts;
    // approval after rejection is refused and leaves the request untouched.
    assert_eq!(
        reopened
            .reject_action_prepared_intent(
                reject_action(&prepared, "action-reject-1"),
                AuditEventId::parse("action-reject-audit-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(9_999),
                Gate,
            )
            .unwrap(),
        outcome
    );
    let again = operation_error(reopened.reject_action_prepared_intent(
        reject_action(&prepared, "action-reject-2"),
        AuditEventId::parse("action-reject-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(again.code(), ErrorCode::DomainConflict);
    let executed = operation_error(reopened.approve_and_execute_accept_action_request(
        accept_execution(&prepared, "action-accept-after-reject"),
        [
            AuditEventId::parse("action-accept-request-audit-1").unwrap(),
            AuditEventId::parse("action-accept-action-audit-1").unwrap(),
            AuditEventId::parse("action-accept-link-audit-1").unwrap(),
        ],
        ApprovalReceiptId::parse("action-accept-receipt-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
        Gate,
        Gate,
    ));
    assert_eq!(executed.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Open);
    assert!(snapshot.actions().is_empty());
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM action_reject_prepared_command_results"
        ),
        1
    );
}

#[test]
fn an_expired_but_unconsumed_accept_preview_is_still_rejectable_and_says_so() {
    let (ledger, mut writer, prepared) = ledger_with_pending_accept();
    let after_expiry =
        UtcTimestamp::from_unix_millis(prepared.preview().expires_at().unix_millis() + 1);
    let outcome = writer
        .reject_action_prepared_intent(
            reject_action(&prepared, "action-reject-late"),
            AuditEventId::parse("action-reject-audit-late").unwrap(),
            after_expiry,
            Gate,
        )
        .expect("expiry limits approval, not refusal");
    assert!(outcome.expired_at_rejection());
    assert_eq!(outcome.rejected_at(), after_expiry);
    drop(writer);
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.discarded_prepared(), &[prepared]);
    assert!(snapshot.prepared().is_empty());
}

#[test]
fn rejection_is_refused_after_execute_and_a_claimed_idempotency_id_conflicts() {
    let (ledger, mut writer, prepared) = ledger_with_pending_accept();
    writer
        .approve_and_execute_accept_action_request(
            accept_execution(&prepared, "action-accept-execute-1"),
            [
                AuditEventId::parse("action-accept-request-audit-1").unwrap(),
                AuditEventId::parse("action-accept-action-audit-1").unwrap(),
                AuditEventId::parse("action-accept-link-audit-1").unwrap(),
            ],
            ApprovalReceiptId::parse("action-accept-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            Gate,
            Gate,
        )
        .unwrap();
    let late = operation_error(writer.reject_action_prepared_intent(
        reject_action(&prepared, "action-reject-after-execute"),
        AuditEventId::parse("action-reject-audit-x").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(late.code(), ErrorCode::DomainConflict);
    let unknown = operation_error(writer.reject_action_prepared_intent(
        RejectActionPreparedIntent {
            prepared_id: PreparedIntentId::parse("action-prepared-missing").unwrap(),
            actor: AuditActor::HeadOfProducts,
            context: action_context("action-reject-missing", "action-reject-missing-correlation"),
        },
        AuditEventId::parse("action-reject-audit-y").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(unknown.code(), ErrorCode::DomainNotFound);
    // An id another Action operation already claimed cannot name a rejection.
    let mut claimed = reject_action(&prepared, "action-create-1");
    claimed.prepared_id = PreparedIntentId::parse("action-prepared-missing").unwrap();
    let conflict = operation_error(writer.reject_action_prepared_intent(
        claimed,
        AuditEventId::parse("action-reject-audit-z").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(conflict.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM action_reject_prepared_command_results"
        ),
        0
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_events WHERE event_code='action.prepared_rejected'"
        ),
        0
    );
}

#[test]
fn a_rejection_whose_consumption_instant_was_tampered_fails_rehydration() {
    let (ledger, mut writer, prepared) = ledger_with_pending_accept();
    writer
        .reject_action_prepared_intent(
            reject_action(&prepared, "action-reject-1"),
            AuditEventId::parse("action-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
            Gate,
        )
        .unwrap();
    drop(writer);
    // The result row cannot be re-pointed (binding trigger)...
    let connection = Connection::open(&ledger.0).unwrap();
    assert!(connection
        .execute(
            "UPDATE action_reject_prepared_command_results SET rejected_at=1151 WHERE idempotency_id='action-reject-1'",
            []
        )
        .is_err());
    // ...but a raw edit of the intent's consumption instant is caught on load.
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=1151 WHERE id='action-prepared-1'",
            [],
        )
        .unwrap();
    drop(connection);
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.load_action_persistence_snapshot().is_err());
}

// ---------------------------------------------------------------- Decision

fn resolve_command() -> PrepareResolveDecisionRequest {
    PrepareResolveDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
        expected_version: AggregateVersion::initial().next().unwrap(),
        statement: DecisionText::parse("Choose synthetic option A.").unwrap(),
        rationale: DecisionText::parse("Synthetic tradeoff is documented.").unwrap(),
        impact: DecisionText::parse("Synthetic delivery remains on track.").unwrap(),
        evidence_ids: Vec::new(),
        judgments: vec![HumanJudgment::new(
            HumanJudgmentDisposition::ProceedWithDocumentedRationale,
            "Synthetic owner judgment.",
            DataClassification::Internal,
        )
        .unwrap()],
        resulting_action_requests: vec![DecisionResultingActionRequest {
            id: ActionRequestId::parse("synthetic-resulting-request-1").unwrap(),
            subject: ActionTitle::parse("Synthetic follow-up").unwrap(),
            details: ActionDetails::parse("Track synthetic follow-up.").unwrap(),
            intended_owner: StakeholderId::parse("synthetic-decision-owner-1").unwrap(),
            due_at: UtcTimestamp::from_unix_millis(10_000),
            classification: DataClassification::Internal,
        }],
        context: decision_context(
            "decision-resolve-prepare-1",
            "decision-resolve-prepare-correlation-1",
        ),
    }
}

fn prepared_resolve_fixture(
    command: &PrepareResolveDecisionRequest,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("decision-resolve-prepared-1").unwrap(),
        WorkManagementOperation::ResolveDecisionRequest {
            request_id: command.request_id.clone(),
            request_version: command.expected_version,
            decision_id: DecisionId::parse("synthetic-decision-1").unwrap(),
            decision_classification: DataClassification::Internal,
            statement: command.statement.clone(),
            rationale: command.rationale.clone(),
            impact: command.impact.clone(),
            decision_owner: StakeholderId::parse("synthetic-decision-owner-1").unwrap(),
            decided_at: UtcTimestamp::from_unix_millis(300),
            resulting_action_requests: command.resulting_action_requests.clone(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(Vec::new(), command.judgments.clone())
                .unwrap()
                .evaluate_evidence_or_judgment()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(300),
    )
    .unwrap()
}

/// H1 create+submit and a judgment-supported Resolve PREPARE for
/// `decision-request-1`, returning the pending preview.
fn ledger_with_pending_resolve() -> (
    SyntheticLedger,
    SqliteProductLedger,
    WorkManagementPreparedIntent,
) {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    Connection::open(&ledger.0).unwrap().execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('synthetic-decision-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('synthetic-decision-owner-1','Synthetic Decision Owner','person','synthetic_fixture','test-only');",
    ).unwrap();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_decision_request_draft(
            CreateDecisionRequestDraft {
                id: DecisionRequestId::parse("decision-request-1").unwrap(),
                subject: DecisionSubject::parse("Synthetic Decision Request").unwrap(),
                details: DecisionText::parse("Refuse this Resolve preview durably.").unwrap(),
                intended_owner: Some(StakeholderId::parse("synthetic-decision-owner-1").unwrap()),
                classification: DataClassification::Internal,
                context: decision_context("decision-create-1", "decision-create-correlation-1"),
            },
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_decision_request(
            SubmitDecisionRequest {
                request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
                expected_version: AggregateVersion::initial(),
                context: decision_context("decision-submit-1", "decision-submit-correlation-1"),
            },
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = resolve_command();
    let prepared = prepared_resolve_fixture(&command);
    writer
        .prepare_resolve_decision_request(command, prepared.clone())
        .unwrap();
    (ledger, writer, prepared)
}

fn reject_decision(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> RejectDecisionPreparedIntent {
    RejectDecisionPreparedIntent {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        context: decision_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

#[test]
fn rejecting_a_pending_resolve_preview_consumes_it_audits_nothing_and_reopens_losslessly() {
    let (ledger, mut writer, prepared) = ledger_with_pending_resolve();
    let revision_before = writer.revision().unwrap();
    let outcome = writer
        .reject_decision_prepared_intent(
            reject_decision(&prepared, "decision-reject-1"),
            AuditEventId::parse("decision-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(350),
            Gate,
        )
        .expect("a pending Resolve preview must be rejectable");
    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(350));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.code().as_str(), DECISION_PREPARED_REJECTED_AUDIT_CODE);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert!(audit.actual_effects().is_empty());
    assert_eq!(writer.revision().unwrap(), revision_before + 1);
    drop(writer);

    assert_eq!(count(&ledger.0, "SELECT count(*) FROM prepared_intents WHERE id='decision-resolve-prepared-1' AND consumed_at=350"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM decision_reject_prepared_command_results WHERE idempotency_id='decision-reject-1' AND prepared_intent_id='decision-resolve-prepared-1' AND operation_ordinal=3"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='decision-reject-1' AND namespace='decision' AND operation='reject_prepared'"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM decisions"), 0);
    assert_eq!(
        count(&ledger.0, "SELECT count(*) FROM approval_receipts"),
        0
    );
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM audit_events WHERE id='decision-reject-audit-1' AND event_code='decision.prepared_rejected' AND target_type='decision_request' AND target_id='decision-request-1' AND approval_outcome='rejected' AND execution_outcome='not_attempted' AND effect_scope='none'"), 1);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    assert_eq!(snapshot.requests()[0].state(), DecisionRequestState::Open);
    let last = snapshot.replay().last().unwrap();
    assert_eq!(last.operation_ordinal(), 3);
    assert_eq!(
        last.result(),
        &pmc_domain::decisions::DecisionPersistenceResult::Rejected(outcome.clone())
    );
    assert_eq!(snapshot.audits().last(), Some(audit));

    assert_eq!(
        reopened
            .reject_decision_prepared_intent(
                reject_decision(&prepared, "decision-reject-1"),
                AuditEventId::parse("decision-reject-audit-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(9_999),
                Gate,
            )
            .unwrap(),
        outcome
    );
    let again = operation_error(reopened.reject_decision_prepared_intent(
        reject_decision(&prepared, "decision-reject-2"),
        AuditEventId::parse("decision-reject-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(360),
        Gate,
    ));
    assert_eq!(again.code(), ErrorCode::DomainConflict);
    let executed = operation_error(
        reopened.approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("decision-resolve-after-reject").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: decision_context(
                    "decision-resolve-after-reject",
                    "decision-resolve-after-reject-correlation",
                ),
            },
            [
                AuditEventId::parse("decision-resolve-audit-1").unwrap(),
                AuditEventId::parse("decision-resolve-audit-2").unwrap(),
                AuditEventId::parse("decision-resolve-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("decision-resolve-action-audit-1").unwrap()],
            ApprovalReceiptId::parse("decision-resolve-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            Gate,
            Gate,
            Gate,
        ),
    );
    assert_eq!(executed.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM decisions"), 0);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM decision_reject_prepared_command_results"
        ),
        1
    );
}

#[test]
fn a_decision_rejection_whose_consumption_instant_was_tampered_fails_rehydration() {
    let (ledger, mut writer, prepared) = ledger_with_pending_resolve();
    writer
        .reject_decision_prepared_intent(
            reject_decision(&prepared, "decision-reject-1"),
            AuditEventId::parse("decision-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(350),
            Gate,
        )
        .unwrap();
    drop(writer);
    Connection::open(&ledger.0)
        .unwrap()
        .execute(
            "UPDATE prepared_intents SET consumed_at=351 WHERE id='decision-resolve-prepared-1'",
            [],
        )
        .unwrap();
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.load_decision_persistence_snapshot().is_err());
}
