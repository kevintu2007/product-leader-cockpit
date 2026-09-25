//! v46: durable, audited rejection of a Risk or Issue H2a Prepared Intent
//! (the Action/Decision half is `sqlite-h2a-rejection.rs`, v45). A rejection
//! is a successful command with no effect: it consumes the intent, records
//! one zero-effect audit, mints no receipt, changes no aggregate, and
//! replays exactly. Risk and Issue keep their own per-table ordinal streams.
//!
//! Risk scaffolding mirrors `sqlite-risk-repository.rs` up through the
//! record-occurrence PREPARE; Issue scaffolding mirrors
//! `sqlite-issue-repository.rs`'s evidence-supported Resolve preview.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::{AuditActor, AuditApprovalOutcome, AuditEffectScope, AuditExecutionOutcome},
    classification::DataClassification,
    error::ErrorCode,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, IssueId, PreparedIntentId, RiskId,
    },
    issues::{
        ApproveAndExecuteIssueTransition, CreateIssue, IssueDetails, IssueEvidenceAuthorityError,
        IssueEvidenceAuthorityPort, IssueExecutionPolicy, IssueExecutionPolicyPort,
        IssueOperationContext, IssueTitle, PrepareResolveIssue, RecordedIssueClassification,
        RejectIssuePreparedIntent,
    },
    risks::{
        AllowRiskEvidence, ApproveAndExecuteRecordRiskOccurrence, CreateRisk,
        PrepareRecordRiskOccurrence, RecordedRiskClassification, RejectRiskPreparedIntent,
        RiskDetails, RiskExecutionPolicy, RiskExecutionPolicyPort, RiskOperationContext, RiskTitle,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceOrJudgment,
        EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification, IntegrityDigest,
        IssueResolutionType, RiskState, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, WorkManagementRationale, ISSUE_PREPARED_REJECTED_AUDIT_CODE,
        RISK_PREPARED_REJECTED_AUDIT_CODE,
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
            "pmc-synthetic-risk-issue-h2a-rejection-{nonce}-{sequence}.sqlite3"
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

impl RiskExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

impl IssueExecutionPolicyPort for Gate {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

impl IssueEvidenceAuthorityPort for Gate {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        let metadata = verified_resolution_evidence();
        if metadata.id() == id {
            Ok(metadata)
        } else {
            Err(IssueEvidenceAuthorityError::NotFound)
        }
    }
}

fn risk_context(idempotency: &str, correlation: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn issue_context(idempotency: &str, correlation: &str) -> IssueOperationContext {
    IssueOperationContext {
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

// -------------------------------------------------------------------- Risk

fn risk_id() -> RiskId {
    RiskId::parse("risk-1").unwrap()
}

fn prepared_occurrence_fixture(prepared_id: &str) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: risk_id(),
            risk_version: AggregateVersion::initial(),
            issue_id: IssueId::parse("risk-occurrence-issue-1").unwrap(),
            issue_classification: DataClassification::Internal,
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

/// H1 create and a record-occurrence PREPARE for `risk-1`, returning the
/// pending preview (`risk-prepared-1`, prepared at 1_000).
fn ledger_with_pending_occurrence() -> (
    SyntheticLedger,
    SqliteProductLedger,
    WorkManagementPreparedIntent,
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_risk(
            CreateRisk {
                id: risk_id(),
                title: RiskTitle::parse("Synthetic rejection risk").unwrap(),
                details: RiskDetails::parse("Refuse this preview durably.").unwrap(),
                classification: DataClassification::Internal,
                context: risk_context("risk-create-1", "risk-create-correlation-1"),
            },
            AuditEventId::parse("risk-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let prepared = prepared_occurrence_fixture("risk-prepared-1");
    writer
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: risk_id(),
                expected_version: AggregateVersion::initial(),
                issue_id: IssueId::parse("risk-occurrence-issue-1").unwrap(),
                context: risk_context("risk-prepare-1", "risk-prepare-correlation-1"),
            },
            prepared.clone(),
        )
        .unwrap();
    (ledger, writer, prepared)
}

fn reject_risk(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> RejectRiskPreparedIntent {
    RejectRiskPreparedIntent {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        context: risk_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

fn occurrence_execution(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> ApproveAndExecuteRecordRiskOccurrence {
    ApproveAndExecuteRecordRiskOccurrence {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse(idempotency).unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: risk_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

fn occurrence_audit_ids(prefix: &str) -> [AuditEventId; 3] {
    [
        AuditEventId::parse(format!("{prefix}-audit-0")).unwrap(),
        AuditEventId::parse(format!("{prefix}-audit-1")).unwrap(),
        AuditEventId::parse(format!("{prefix}-audit-2")).unwrap(),
    ]
}

#[test]
fn rejecting_a_pending_occurrence_preview_consumes_it_audits_nothing_and_reopens_losslessly() {
    let (ledger, mut writer, prepared) = ledger_with_pending_occurrence();
    let revision_before = writer.revision().unwrap();

    let outcome = writer
        .reject_risk_prepared_intent(
            reject_risk(&prepared, "risk-reject-1"),
            AuditEventId::parse("risk-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
            Gate,
        )
        .expect("a pending occurrence preview must be rejectable");
    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(1_150));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.id().as_str(), "risk-reject-audit-1");
    assert_eq!(audit.code().as_str(), RISK_PREPARED_REJECTED_AUDIT_CODE);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());
    assert_eq!(writer.revision().unwrap(), revision_before + 1);
    drop(writer);

    // Durable shape: consumed at the rejection instant, one result row on the
    // Risk stream (ordinal zero: the stream is its own), claimed in the risk
    // namespace, no receipt, no Issue created, one effect-free audit.
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM prepared_intents WHERE id='risk-prepared-1' AND consumed_at=1150"
        ),
        1
    );
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM risk_reject_prepared_command_results WHERE idempotency_id='risk-reject-1' AND prepared_intent_id='risk-prepared-1' AND rejected_at=1150 AND operation_ordinal=0 AND audit_event_id='risk-reject-audit-1'"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='risk-reject-1' AND namespace='risk' AND operation='reject_prepared'"), 1);
    assert_eq!(
        count(&ledger.0, "SELECT count(*) FROM approval_receipts"),
        0
    );
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM issues"), 0);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM audit_events WHERE id='risk-reject-audit-1' AND event_code='risk.prepared_rejected' AND target_type='risk' AND target_id='risk-1' AND correlation_id='risk-reject-1-correlation' AND policy_outcome='allowed' AND approval_outcome='rejected' AND execution_outcome='not_attempted' AND effect_scope='none'"), 1);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_effects WHERE audit_event_id='risk-reject-audit-1'"
        ),
        0
    );

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let h2a = reopened.load_risk_h2a_runtime_snapshot().unwrap();
    assert!(h2a.prepared().is_empty());
    let snapshot = reopened.load_risk_persistence_snapshot().unwrap();
    assert_eq!(snapshot.risks()[0].state(), RiskState::Open);
    assert_eq!(snapshot.risks()[0].version(), AggregateVersion::initial());

    // Exact replay; a different id against the consumed intent conflicts;
    // approval after rejection is refused and leaves the Risk untouched.
    assert_eq!(
        reopened
            .reject_risk_prepared_intent(
                reject_risk(&prepared, "risk-reject-1"),
                AuditEventId::parse("risk-reject-audit-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(9_999),
                Gate,
            )
            .unwrap(),
        outcome
    );
    let again = operation_error(reopened.reject_risk_prepared_intent(
        reject_risk(&prepared, "risk-reject-2"),
        AuditEventId::parse("risk-reject-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(again.code(), ErrorCode::DomainConflict);
    let executed = operation_error(reopened.approve_and_execute_record_risk_occurrence(
        occurrence_execution(&prepared, "risk-occurrence-after-reject"),
        occurrence_audit_ids("risk-occurrence-after-reject"),
        ApprovalReceiptId::parse("risk-occurrence-receipt-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
        Gate,
        Gate,
        AllowRiskEvidence,
        RecordedRiskClassification,
    ));
    assert_eq!(executed.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    let snapshot = reopened.load_risk_persistence_snapshot().unwrap();
    assert_eq!(snapshot.risks()[0].state(), RiskState::Open);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM issues"), 0);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM risk_reject_prepared_command_results"
        ),
        1
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_events WHERE event_code='risk.prepared_rejected'"
        ),
        1
    );
}

#[test]
fn an_expired_but_unconsumed_occurrence_preview_is_still_rejectable_and_says_so() {
    let (ledger, mut writer, prepared) = ledger_with_pending_occurrence();
    let after_expiry =
        UtcTimestamp::from_unix_millis(prepared.preview().expires_at().unix_millis() + 1);
    let outcome = writer
        .reject_risk_prepared_intent(
            reject_risk(&prepared, "risk-reject-late"),
            AuditEventId::parse("risk-reject-audit-late").unwrap(),
            after_expiry,
            Gate,
        )
        .unwrap();
    assert!(outcome.expired_at_rejection());
    assert_eq!(outcome.rejected_at(), after_expiry);
    drop(writer);
    assert_eq!(
        count(&ledger.0, &format!("SELECT count(*) FROM prepared_intents WHERE id='risk-prepared-1' AND consumed_at={}", after_expiry.unix_millis())),
        1
    );
    // The expiry verdict is re-derived on reopen, not stored.
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replayed = reopened
        .reject_risk_prepared_intent(
            reject_risk(&prepared, "risk-reject-late"),
            AuditEventId::parse("risk-reject-audit-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
            Gate,
        )
        .unwrap();
    assert_eq!(replayed, outcome);
}

#[test]
fn risk_rejection_is_refused_after_execute_and_a_claimed_idempotency_id_conflicts() {
    let (ledger, mut writer, prepared) = ledger_with_pending_occurrence();
    writer
        .approve_and_execute_record_risk_occurrence(
            occurrence_execution(&prepared, "risk-occurrence-execute-1"),
            occurrence_audit_ids("risk-occurrence-execute-1"),
            ApprovalReceiptId::parse("risk-occurrence-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            Gate,
            Gate,
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    let late = operation_error(writer.reject_risk_prepared_intent(
        reject_risk(&prepared, "risk-reject-after-execute"),
        AuditEventId::parse("risk-reject-audit-x").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(late.code(), ErrorCode::DomainConflict);
    let unknown = operation_error(writer.reject_risk_prepared_intent(
        RejectRiskPreparedIntent {
            prepared_id: PreparedIntentId::parse("risk-prepared-missing").unwrap(),
            actor: AuditActor::HeadOfProducts,
            context: risk_context("risk-reject-missing", "risk-reject-missing-correlation"),
        },
        AuditEventId::parse("risk-reject-audit-y").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(unknown.code(), ErrorCode::DomainNotFound);
    // An id another Risk operation already claimed cannot name a rejection.
    let mut claimed = reject_risk(&prepared, "risk-prepare-1");
    claimed.prepared_id = PreparedIntentId::parse("risk-prepared-missing").unwrap();
    let conflict = operation_error(writer.reject_risk_prepared_intent(
        claimed,
        AuditEventId::parse("risk-reject-audit-z").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(conflict.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM risk_reject_prepared_command_results"
        ),
        0
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_events WHERE event_code='risk.prepared_rejected'"
        ),
        0
    );
}

#[test]
fn a_risk_rejection_whose_consumption_instant_was_tampered_fails_rehydration() {
    let (ledger, mut writer, prepared) = ledger_with_pending_occurrence();
    writer
        .reject_risk_prepared_intent(
            reject_risk(&prepared, "risk-reject-1"),
            AuditEventId::parse("risk-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
            Gate,
        )
        .unwrap();
    drop(writer);
    // The result row cannot be re-pointed (binding trigger)...
    let connection = Connection::open(&ledger.0).unwrap();
    assert!(connection
        .execute(
            "UPDATE risk_reject_prepared_command_results SET rejected_at=1151 WHERE idempotency_id='risk-reject-1'",
            []
        )
        .is_err());
    // ...but a raw edit of the intent's consumption instant is caught on load
    // and on the next write.
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=1151 WHERE id='risk-prepared-1'",
            [],
        )
        .unwrap();
    drop(connection);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.load_risk_h2a_runtime_snapshot().is_err());
    let refused = operation_error(reopened.reject_risk_prepared_intent(
        reject_risk(&prepared, "risk-reject-1"),
        AuditEventId::parse("risk-reject-audit-ignored").unwrap(),
        UtcTimestamp::from_unix_millis(9_999),
        Gate,
    ));
    assert_eq!(refused.code(), ErrorCode::PlatformInternal);
    drop(reopened);

    // The audit row is not trigger-protected either; a forged code, actor,
    // or target is caught by re-deriving the audit and comparing wholesale.
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=1150 WHERE id='risk-prepared-1'",
            [],
        )
        .unwrap();
    drop(connection);
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_h2a_runtime_snapshot()
        .is_ok());
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE audit_events SET event_code='risk.closed' WHERE id='risk-reject-audit-1'",
            [],
        )
        .unwrap();
    drop(connection);
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_h2a_runtime_snapshot()
        .is_err());
}

// ------------------------------------------------------------------- Issue

fn issue_id() -> IssueId {
    IssueId::parse("issue-1").unwrap()
}

fn verified_resolution_evidence() -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("issue-resolution-evidence-1").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    )
}

/// Seeds the `evidence_references` row (and its `aggregate_registry` parent)
/// the support-gated Resolve PREPARE writer's foreign keys require.
fn seed_verified_resolution_evidence(ledger_path: &std::path::Path) {
    let connection = Connection::open(ledger_path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('issue-resolution-evidence-1','evidence_reference',1,'internal',0,0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES ('issue-resolution-evidence-1','issue_resolution','verified',?1,0)",
            [&"a".repeat(64)],
        )
        .unwrap();
}

fn prepared_resolve_fixture(prepared_id: &str) -> WorkManagementPreparedIntent {
    let support = EvidenceOrJudgment::new(vec![verified_resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id(),
            issue_version: AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: WorkManagementRationale::parse("Synthetic resolution rationale").unwrap(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

/// H1 create and an evidence-supported Resolve PREPARE for `issue-1`,
/// returning the pending preview (`issue-prepared-1`, prepared at 1_000).
fn ledger_with_pending_resolve() -> (
    SyntheticLedger,
    SqliteProductLedger,
    WorkManagementPreparedIntent,
) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_issue(
            CreateIssue {
                id: issue_id(),
                title: IssueTitle::parse("Synthetic rejection issue").unwrap(),
                details: IssueDetails::parse("Refuse this preview durably.").unwrap(),
                classification: DataClassification::Internal,
                recurrence_of: None,
                context: issue_context("issue-create-1", "issue-create-correlation-1"),
            },
            AuditEventId::parse("issue-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    let prepared = prepared_resolve_fixture("issue-prepared-1");
    writer
        .prepare_resolve_issue(
            PrepareResolveIssue {
                issue_id: issue_id(),
                expected_version: AggregateVersion::initial(),
                resolution_type: IssueResolutionType::Resolved,
                rationale: WorkManagementRationale::parse("Synthetic resolution rationale")
                    .unwrap(),
                evidence_ids: vec![
                    EvidenceReferenceId::parse("issue-resolution-evidence-1").unwrap()
                ],
                judgment: None,
                context: issue_context("issue-prepare-1", "issue-prepare-correlation-1"),
            },
            prepared.clone(),
        )
        .unwrap();
    (ledger, writer, prepared)
}

fn reject_issue(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> RejectIssuePreparedIntent {
    RejectIssuePreparedIntent {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        context: issue_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

fn resolve_execution(
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
) -> ApproveAndExecuteIssueTransition {
    ApproveAndExecuteIssueTransition {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse(idempotency).unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: issue_context(idempotency, &format!("{idempotency}-correlation")),
    }
}

#[test]
fn rejecting_a_pending_resolve_preview_consumes_it_audits_nothing_and_reopens_losslessly() {
    let (ledger, mut writer, prepared) = ledger_with_pending_resolve();
    let revision_before = writer.revision().unwrap();

    let outcome = writer
        .reject_issue_prepared_intent(
            reject_issue(&prepared, "issue-reject-1"),
            AuditEventId::parse("issue-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
            Gate,
        )
        .expect("a pending Resolve preview must be rejectable");
    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert_eq!(outcome.rejected_at(), UtcTimestamp::from_unix_millis(1_150));
    assert!(!outcome.expired_at_rejection());
    let audit = outcome.audit_event();
    assert_eq!(audit.id().as_str(), "issue-reject-audit-1");
    assert_eq!(audit.code().as_str(), ISSUE_PREPARED_REJECTED_AUDIT_CODE);
    assert_eq!(audit.approval_outcome(), AuditApprovalOutcome::Rejected);
    assert_eq!(
        audit.execution_outcome(),
        AuditExecutionOutcome::NotAttempted
    );
    assert_eq!(audit.effect_scope(), AuditEffectScope::None);
    assert!(audit.actual_effects().is_empty());
    assert_eq!(writer.revision().unwrap(), revision_before + 1);
    drop(writer);

    // Durable shape on the Issue's own stream.
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM prepared_intents WHERE id='issue-prepared-1' AND consumed_at=1150"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM issue_reject_prepared_command_results WHERE idempotency_id='issue-reject-1' AND prepared_intent_id='issue-prepared-1' AND rejected_at=1150 AND operation_ordinal=0 AND audit_event_id='issue-reject-audit-1'"), 1);
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='issue-reject-1' AND namespace='issue' AND operation='reject_prepared'"), 1);
    assert_eq!(
        count(&ledger.0, "SELECT count(*) FROM approval_receipts"),
        0
    );
    assert_eq!(count(&ledger.0, "SELECT count(*) FROM audit_events WHERE id='issue-reject-audit-1' AND event_code='issue.prepared_rejected' AND target_type='issue' AND target_id='issue-1' AND correlation_id='issue-reject-1-correlation' AND policy_outcome='allowed' AND approval_outcome='rejected' AND execution_outcome='not_attempted' AND effect_scope='none'"), 1);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_effects WHERE audit_event_id='issue-reject-audit-1'"
        ),
        0
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM issues WHERE id='issue-1' AND state='open'"
        ),
        1
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM aggregate_registry WHERE id='issue-1' AND version=1"
        ),
        1
    );

    // Exact replay from a reopened Ledger (the rejection rehydrates); a
    // different id against the consumed intent conflicts; approval after
    // rejection is refused and leaves the Issue untouched.
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(
        reopened
            .reject_issue_prepared_intent(
                reject_issue(&prepared, "issue-reject-1"),
                AuditEventId::parse("issue-reject-audit-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(9_999),
                Gate,
            )
            .unwrap(),
        outcome
    );
    let again = operation_error(reopened.reject_issue_prepared_intent(
        reject_issue(&prepared, "issue-reject-2"),
        AuditEventId::parse("issue-reject-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(again.code(), ErrorCode::DomainConflict);
    let executed = operation_error(reopened.approve_and_execute_resolve_issue(
        resolve_execution(&prepared, "issue-resolve-after-reject"),
        AuditEventId::parse("issue-resolve-audit-1").unwrap(),
        ApprovalReceiptId::parse("issue-resolve-receipt-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
        Gate,
        Gate,
        Gate,
        RecordedIssueClassification,
    ));
    assert_eq!(executed.code(), ErrorCode::DomainConflict);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM issues WHERE id='issue-1' AND state='open'"
        ),
        1
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM issue_reject_prepared_command_results"
        ),
        1
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_events WHERE event_code='issue.prepared_rejected'"
        ),
        1
    );
}

#[test]
fn an_expired_but_unconsumed_resolve_preview_is_still_rejectable_and_says_so() {
    let (ledger, mut writer, prepared) = ledger_with_pending_resolve();
    let after_expiry =
        UtcTimestamp::from_unix_millis(prepared.preview().expires_at().unix_millis() + 1);
    let outcome = writer
        .reject_issue_prepared_intent(
            reject_issue(&prepared, "issue-reject-late"),
            AuditEventId::parse("issue-reject-audit-late").unwrap(),
            after_expiry,
            Gate,
        )
        .unwrap();
    assert!(outcome.expired_at_rejection());
    drop(writer);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replayed = reopened
        .reject_issue_prepared_intent(
            reject_issue(&prepared, "issue-reject-late"),
            AuditEventId::parse("issue-reject-audit-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
            Gate,
        )
        .unwrap();
    assert_eq!(replayed, outcome);
}

#[test]
fn issue_rejection_is_refused_after_execute_and_a_claimed_idempotency_id_conflicts() {
    let (ledger, mut writer, prepared) = ledger_with_pending_resolve();
    writer
        .approve_and_execute_resolve_issue(
            resolve_execution(&prepared, "issue-resolve-execute-1"),
            AuditEventId::parse("issue-resolve-audit-1").unwrap(),
            ApprovalReceiptId::parse("issue-resolve-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            Gate,
            Gate,
            Gate,
            RecordedIssueClassification,
        )
        .unwrap();
    let late = operation_error(writer.reject_issue_prepared_intent(
        reject_issue(&prepared, "issue-reject-after-execute"),
        AuditEventId::parse("issue-reject-audit-x").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(late.code(), ErrorCode::DomainConflict);
    let unknown = operation_error(writer.reject_issue_prepared_intent(
        RejectIssuePreparedIntent {
            prepared_id: PreparedIntentId::parse("issue-prepared-missing").unwrap(),
            actor: AuditActor::HeadOfProducts,
            context: issue_context("issue-reject-missing", "issue-reject-missing-correlation"),
        },
        AuditEventId::parse("issue-reject-audit-y").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(unknown.code(), ErrorCode::DomainNotFound);
    // An id another Issue operation already claimed cannot name a rejection.
    let mut claimed = reject_issue(&prepared, "issue-prepare-1");
    claimed.prepared_id = PreparedIntentId::parse("issue-prepared-missing").unwrap();
    let conflict = operation_error(writer.reject_issue_prepared_intent(
        claimed,
        AuditEventId::parse("issue-reject-audit-z").unwrap(),
        UtcTimestamp::from_unix_millis(1_200),
        Gate,
    ));
    assert_eq!(conflict.code(), ErrorCode::DomainIdempotencyConflict);
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM issue_reject_prepared_command_results"
        ),
        0
    );
    assert_eq!(
        count(
            &ledger.0,
            "SELECT count(*) FROM audit_events WHERE event_code='issue.prepared_rejected'"
        ),
        0
    );
}

#[test]
fn an_issue_rejection_whose_consumption_instant_was_tampered_fails_rehydration() {
    let (ledger, mut writer, prepared) = ledger_with_pending_resolve();
    writer
        .reject_issue_prepared_intent(
            reject_issue(&prepared, "issue-reject-1"),
            AuditEventId::parse("issue-reject-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
            Gate,
        )
        .unwrap();
    drop(writer);
    let connection = Connection::open(&ledger.0).unwrap();
    assert!(connection
        .execute(
            "UPDATE issue_reject_prepared_command_results SET rejected_at=1151 WHERE idempotency_id='issue-reject-1'",
            []
        )
        .is_err());
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=1151 WHERE id='issue-prepared-1'",
            [],
        )
        .unwrap();
    drop(connection);
    // There is no standalone Issue H2a loader; the next write against this
    // Issue is where the tampered rejection must be caught -- a replay of the
    // rejection and a fresh Resolve execute alike.
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let refused = operation_error(reopened.reject_issue_prepared_intent(
        reject_issue(&prepared, "issue-reject-1"),
        AuditEventId::parse("issue-reject-audit-ignored").unwrap(),
        UtcTimestamp::from_unix_millis(9_999),
        Gate,
    ));
    assert_eq!(refused.code(), ErrorCode::PlatformInternal);
    let executed = operation_error(reopened.approve_and_execute_resolve_issue(
        resolve_execution(&prepared, "issue-resolve-after-tamper"),
        AuditEventId::parse("issue-resolve-audit-t").unwrap(),
        ApprovalReceiptId::parse("issue-resolve-receipt-t").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
        Gate,
        Gate,
        Gate,
        RecordedIssueClassification,
    ));
    assert!(matches!(
        executed.code(),
        ErrorCode::PlatformInternal | ErrorCode::DomainConflict
    ));
    drop(reopened);

    // The audit row is not trigger-protected either; a forged code is caught
    // by re-deriving the audit and comparing wholesale.
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE prepared_intents SET consumed_at=1150 WHERE id='issue-prepared-1'",
            [],
        )
        .unwrap();
    drop(connection);
    let mut restored = SqliteProductLedger::open(&ledger.0).unwrap();
    restored
        .reject_issue_prepared_intent(
            reject_issue(&prepared, "issue-reject-1"),
            AuditEventId::parse("issue-reject-audit-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(9_999),
            Gate,
        )
        .expect("restoring the consumption instant restores the replay");
    drop(restored);
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE audit_events SET event_code='issue.resolved' WHERE id='issue-reject-audit-1'",
            [],
        )
        .unwrap();
    drop(connection);
    let mut forged = SqliteProductLedger::open(&ledger.0).unwrap();
    let refused = operation_error(forged.reject_issue_prepared_intent(
        reject_issue(&prepared, "issue-reject-1"),
        AuditEventId::parse("issue-reject-audit-ignored").unwrap(),
        UtcTimestamp::from_unix_millis(9_999),
        Gate,
    ));
    assert_eq!(refused.code(), ErrorCode::PlatformInternal);
}
