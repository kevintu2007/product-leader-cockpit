use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{audit::AuditActor, error::ErrorCode};
use pmc_domain::{
    classification::DataClassification,
    decisions::{
        ApproveAndExecuteResolveDecisionRequest, CreateDecisionRequestDraft,
        DecisionEvidenceAuthorityError, DecisionEvidenceAuthorityPort, DecisionExecutionPolicy,
        DecisionExecutionPolicyPort, DecisionOperationContext, DecisionSubject, DecisionText,
        DecisionWithdrawalRationale, PrepareResolveDecisionRequest, SubmitDecisionRequest,
        WithdrawDecisionRequest,
    },
    identity::{
        AggregateVersion, AuditEventId, CorrelationId, DecisionId, DecisionRequestId,
        IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, DecisionRequestState,
        DecisionResultingActionRequest, EvidenceOrJudgment, EvidenceReferenceMetadata,
        EvidenceRole, EvidenceVerification, HumanJudgment, HumanJudgmentDisposition,
        IntegrityDigest, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent,
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
            "pmc-synthetic-decision-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

#[test]
fn writer_submits_decision_request_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_decision_request_draft(
            create_command(),
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let submit = SubmitDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
        expected_version: AggregateVersion::initial(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-submit-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-submit-correlation-1").unwrap(),
        },
    };
    let outcome = opened
        .submit_decision_request(
            submit.clone(),
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .expect("Draft Decision Request must submit atomically");
    assert_eq!(outcome.record.state(), DecisionRequestState::Open);
    assert_eq!(outcome.record.version().get(), 2);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(
        reopened
            .load_decision_persistence_snapshot()
            .unwrap()
            .requests()[0]
            .state(),
        DecisionRequestState::Open
    );
    assert_eq!(
        reopened
            .submit_decision_request(
                submit,
                AuditEventId::parse("decision-submit-replay-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(300)
            )
            .unwrap(),
        outcome
    );
}

#[test]
fn writer_withdraws_open_decision_request_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_decision_request_draft(
            create_command(),
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let withdraw = WithdrawDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
        expected_version: AggregateVersion::initial().next().unwrap(),
        rationale: DecisionWithdrawalRationale::parse("Synthetic scope changed before resolution.")
            .unwrap(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-withdraw-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-withdraw-correlation-1").unwrap(),
        },
    };
    let outcome = opened
        .withdraw_decision_request(
            withdraw.clone(),
            AuditEventId::parse("decision-withdraw-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(300),
        )
        .expect("Open Decision Request must withdraw atomically");
    assert_eq!(outcome.record.state(), DecisionRequestState::Withdrawn);
    assert_eq!(outcome.record.version().get(), 3);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(
        reopened
            .load_decision_persistence_snapshot()
            .unwrap()
            .requests()[0]
            .state(),
        DecisionRequestState::Withdrawn
    );
    assert_eq!(
        reopened
            .withdraw_decision_request(
                withdraw,
                AuditEventId::parse("decision-withdraw-replay-ignored").unwrap(),
                UtcTimestamp::from_unix_millis(400)
            )
            .unwrap(),
        outcome
    );
}

#[test]
fn writer_persists_judgment_supported_resolve_preview_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-1").unwrap();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            create,
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();

    let command = PrepareResolveDecisionRequest {
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
            id: pmc_domain::identity::ActionRequestId::parse("synthetic-resulting-request-1")
                .unwrap(),
            subject: pmc_domain::actions::ActionTitle::parse("Synthetic follow-up").unwrap(),
            details: pmc_domain::actions::ActionDetails::parse("Track synthetic follow-up.")
                .unwrap(),
            intended_owner: owner.clone(),
            due_at: UtcTimestamp::from_unix_millis(10_000),
            classification: DataClassification::Internal,
        }],
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-resolve-prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-resolve-prepare-correlation-1").unwrap(),
        },
    };
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("decision-resolve-prepared-1").unwrap(),
        WorkManagementOperation::ResolveDecisionRequest {
            request_id: command.request_id.clone(),
            request_version: command.expected_version,
            decision_id: DecisionId::parse("synthetic-decision-1").unwrap(),
            decision_classification: DataClassification::Internal,
            statement: command.statement.clone(),
            rationale: command.rationale.clone(),
            impact: command.impact.clone(),
            decision_owner: owner,
            decided_at: UtcTimestamp::from_unix_millis(300),
            resulting_action_requests: command.resulting_action_requests.clone(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(
                command
                    .evidence_ids
                    .iter()
                    .map(|id| {
                        EvidenceReferenceMetadata::new(
                            id.clone(),
                            AggregateVersion::initial(),
                            DataClassification::Internal,
                            EvidenceRole::DecisionResolution,
                            EvidenceVerification::Verified {
                                verified_at: UtcTimestamp::from_unix_millis(200),
                                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                            },
                        )
                    })
                    .collect(),
                command.judgments.clone(),
            )
            .unwrap()
            .evaluate_evidence_or_judgment()
            .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(300),
    )
    .unwrap();

    let outcome = opened
        .prepare_resolve_decision_request(command.clone(), prepared.clone())
        .expect("canonical judgment-supported resolve preview must persist");
    assert_eq!(outcome, prepared);
    drop(opened);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert_eq!(snapshot.prepared(), &[prepared.clone()]);
    assert_eq!(snapshot.replay().len(), 3);
    assert_eq!(snapshot.replay()[2].operation_ordinal(), 2);
    assert_eq!(
        reopened
            .prepare_resolve_decision_request(command, prepared.clone())
            .unwrap(),
        prepared
    );
}

#[derive(Clone, Copy)]
struct ResolveGate;
impl ApprovalAuthorizationPort for ResolveGate {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
impl DecisionExecutionPolicyPort for ResolveGate {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        DecisionExecutionPolicy::Allowed
    }
}
impl DecisionEvidenceAuthorityPort for ResolveGate {
    fn resolve(
        &self,
        _: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<
        pmc_domain::work_management::EvidenceReferenceMetadata,
        DecisionEvidenceAuthorityError,
    > {
        Err(DecisionEvidenceAuthorityError::Unavailable)
    }
}

impl DecisionEvidenceAuthorityPort for EvidenceGate {
    fn resolve(
        &self,
        id: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        Ok(EvidenceReferenceMetadata::new(
            id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::DecisionResolution,
            EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(200),
                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
            },
        ))
    }
}

#[derive(Clone, Copy)]
struct EvidenceGate;

#[derive(Clone, Copy)]
struct UnavailableEvidenceGate;

#[derive(Clone, Copy)]
struct TamperedEvidenceGate;

#[derive(Clone, Copy)]
struct DegradedEvidenceGate;

impl DecisionEvidenceAuthorityPort for UnavailableEvidenceGate {
    fn resolve(
        &self,
        _: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        Err(DecisionEvidenceAuthorityError::Unavailable)
    }
}

impl DecisionEvidenceAuthorityPort for TamperedEvidenceGate {
    fn resolve(
        &self,
        id: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        Ok(EvidenceReferenceMetadata::new(
            id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::DecisionResolution,
            EvidenceVerification::IntegrityMismatch,
        ))
    }
}

impl DecisionEvidenceAuthorityPort for DegradedEvidenceGate {
    fn resolve(
        &self,
        id: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        Ok(EvidenceReferenceMetadata::new(
            id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
            EvidenceRole::DecisionResolution,
            EvidenceVerification::DegradedLastVerified {
                last_verified_at: UtcTimestamp::from_unix_millis(100),
                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
            },
        ))
    }
}

#[test]
fn writer_reopens_evidence_supported_resolution_preview() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-evidence").unwrap();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            create,
            AuditEventId::parse("evidence-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("evidence-submit-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let evidence_id =
        pmc_domain::identity::EvidenceReferenceId::parse("synthetic-evidence-1").unwrap();
    let second_evidence_id =
        pmc_domain::identity::EvidenceReferenceId::parse("synthetic-evidence-2").unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry VALUES(?1,'evidence_reference',1,'internal',1,1)",
            [evidence_id.as_str()],
        )
        .unwrap();
    connection.execute("INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at) VALUES(?1,'decision_resolution','verified',?2,200)", rusqlite::params![evidence_id.as_str(), "a".repeat(64)]).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry VALUES(?1,'evidence_reference',1,'internal',1,1)",
            [second_evidence_id.as_str()],
        )
        .unwrap();
    connection.execute("INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at) VALUES(?1,'decision_resolution','verified',?2,200)", rusqlite::params![second_evidence_id.as_str(), "b".repeat(64)]).unwrap();
    drop(connection);
    let mut command = resolve_preview_command(owner.clone());
    command.evidence_ids = vec![evidence_id, second_evidence_id];
    let prepared = resolve_preview(&command, owner);
    opened
        .prepare_resolve_decision_request(command.clone(), prepared.clone())
        .unwrap();
    let mut altered_evidence = command.clone();
    altered_evidence.evidence_ids =
        vec![
            pmc_domain::identity::EvidenceReferenceId::parse("synthetic-evidence-altered").unwrap(),
        ];
    assert!(opened
        .prepare_resolve_decision_request(altered_evidence, prepared.clone())
        .is_err());
    let mut reordered_evidence = command;
    reordered_evidence.evidence_ids.reverse();
    assert_eq!(
        opened
            .prepare_resolve_decision_request(reordered_evidence, prepared.clone())
            .unwrap_err(),
        LedgerTransactionError::Operation(pmc_domain::error::DomainError::new(
            ErrorCode::DomainIdempotencyConflict,
            pmc_domain::error::MessageKey::parse("ledger.idempotency_conflict").unwrap(),
            CorrelationId::parse("decision-resolve-prepare-correlation-1").unwrap(),
            false,
        ))
    );
    drop(opened);
    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_decision_persistence_snapshot()
        .unwrap();
    assert_eq!(snapshot.prepared(), &[prepared]);
}

#[test]
fn writer_executes_and_reopens_evidence_supported_resolution() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-execute-evidence").unwrap();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            create,
            AuditEventId::parse("execute-evidence-create").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("execute-evidence-submit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let evidence_id =
        pmc_domain::identity::EvidenceReferenceId::parse("synthetic-execute-evidence-1").unwrap();
    let c = Connection::open(&ledger.0).unwrap();
    c.execute(
        "INSERT INTO aggregate_registry VALUES(?1,'evidence_reference',1,'internal',1,1)",
        [evidence_id.as_str()],
    )
    .unwrap();
    c.execute("INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at) VALUES(?1,'decision_resolution','verified',?2,200)", rusqlite::params![evidence_id.as_str(), "a".repeat(64)]).unwrap();
    drop(c);
    let mut command = resolve_preview_command(owner);
    command.evidence_ids = vec![evidence_id];
    command.judgments.clear();
    let prepared = resolve_preview(
        &command,
        StakeholderId::parse("synthetic-decision-owner-execute-evidence").unwrap(),
    );
    opened
        .prepare_resolve_decision_request(command, prepared.clone())
        .unwrap();
    let execution = ApproveAndExecuteResolveDecisionRequest {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("execute-evidence-idempotency").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("execute-evidence-idempotency").unwrap(),
            correlation_id: CorrelationId::parse("execute-evidence-correlation").unwrap(),
        },
    };
    assert!(opened
        .approve_and_execute_resolve_decision_request(
            execution.clone(),
            [
                AuditEventId::parse("execute-evidence-unavailable-audit-1").unwrap(),
                AuditEventId::parse("execute-evidence-unavailable-audit-2").unwrap(),
                AuditEventId::parse("execute-evidence-unavailable-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("execute-evidence-unavailable-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-evidence-unavailable-receipt")
                .unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            UnavailableEvidenceGate,
        )
        .is_err());
    assert!(opened
        .approve_and_execute_resolve_decision_request(
            execution.clone(),
            [
                AuditEventId::parse("execute-evidence-tampered-audit-1").unwrap(),
                AuditEventId::parse("execute-evidence-tampered-audit-2").unwrap(),
                AuditEventId::parse("execute-evidence-tampered-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("execute-evidence-tampered-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-evidence-tampered-receipt")
                .unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            TamperedEvidenceGate,
        )
        .is_err());
    assert!(opened
        .approve_and_execute_resolve_decision_request(
            execution.clone(),
            [
                AuditEventId::parse("execute-evidence-degraded-audit-1").unwrap(),
                AuditEventId::parse("execute-evidence-degraded-audit-2").unwrap(),
                AuditEventId::parse("execute-evidence-degraded-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("execute-evidence-degraded-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-evidence-degraded-receipt")
                .unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            DegradedEvidenceGate,
        )
        .is_err());
    opened
        .approve_and_execute_resolve_decision_request(
            execution,
            [
                AuditEventId::parse("execute-evidence-audit-1").unwrap(),
                AuditEventId::parse("execute-evidence-audit-2").unwrap(),
                AuditEventId::parse("execute-evidence-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("execute-evidence-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("execute-evidence-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            EvidenceGate,
        )
        .unwrap();
    drop(opened);
    assert_eq!(
        SqliteProductLedger::open(&ledger.0)
            .unwrap()
            .load_decision_persistence_snapshot()
            .unwrap()
            .decisions()
            .len(),
        1
    );
}

#[test]
fn writer_executes_exact_persisted_resolution_and_reopens_decision_namespace() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-1").unwrap();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            create,
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = resolve_preview_command(owner.clone());
    let prepared = resolve_preview(&command, owner);
    opened
        .prepare_resolve_decision_request(command, prepared.clone())
        .unwrap();
    let execute = ApproveAndExecuteResolveDecisionRequest {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("decision-resolve-execute-1").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-resolve-execute-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-resolve-execute-correlation-1").unwrap(),
        },
    };
    let outcome = opened
        .approve_and_execute_resolve_decision_request(
            execute.clone(),
            [
                AuditEventId::parse("decision-resolve-audit-1").unwrap(),
                AuditEventId::parse("decision-resolve-audit-2").unwrap(),
                AuditEventId::parse("decision-resolve-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("decision-resolve-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("decision-resolve-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();
    assert_eq!(outcome.request.state(), DecisionRequestState::Resolved);
    assert_eq!(outcome.audit_events.len(), 3);
    drop(opened);
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert_eq!(snapshot.decisions(), &[outcome.decision.clone()]);
    assert!(snapshot.prepared().is_empty());
    assert_eq!(
        reopened
            .approve_and_execute_resolve_decision_request(
                execute,
                [
                    AuditEventId::parse("ignored-audit-1").unwrap(),
                    AuditEventId::parse("ignored-audit-2").unwrap(),
                    AuditEventId::parse("ignored-audit-3").unwrap()
                ],
                vec![AuditEventId::parse("ignored-action-audit-1").unwrap()],
                pmc_domain::identity::ApprovalReceiptId::parse("ignored-receipt-1").unwrap(),
                UtcTimestamp::from_unix_millis(500),
                ResolveGate,
                ResolveGate,
                ResolveGate
            )
            .unwrap(),
        outcome
    );
    drop(reopened);
    let connection = Connection::open(&ledger.0).unwrap();
    let tampering_cases = [
        (
            "UPDATE audit_events SET event_code='decision.created' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET event_code='decision_request.resolved' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET actor='policy_authorized_system' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET actor='head_of_products' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET module='execution' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET module='work_management' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET target_type='decision',target_id='synthetic-decision-1' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET target_type='decision_request',target_id='decision-request-1' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET policy_outcome='denied' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET policy_outcome='allowed' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET approval_outcome='not_required' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET approval_outcome='approved' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET execution_outcome='not_attempted' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET execution_outcome='succeeded' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_events SET effect_scope='none' WHERE id='decision-resolve-audit-1'",
            "UPDATE audit_events SET effect_scope='complete' WHERE id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_effects SET effect_code='decision.created' WHERE audit_event_id='decision-resolve-audit-1'",
            "UPDATE audit_effects SET effect_code='decision_request.resolved' WHERE audit_event_id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_effects SET scope='none' WHERE audit_event_id='decision-resolve-audit-1'",
            "UPDATE audit_effects SET scope='complete' WHERE audit_event_id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_effects SET target_type='decision',target_id='synthetic-decision-1' WHERE audit_event_id='decision-resolve-audit-1'",
            "UPDATE audit_effects SET target_type='decision_request',target_id='decision-request-1' WHERE audit_event_id='decision-resolve-audit-1'",
        ),
        (
            "UPDATE audit_effects SET ordinal=9 WHERE audit_event_id='decision-resolve-audit-1'",
            "UPDATE audit_effects SET ordinal=0 WHERE audit_event_id='decision-resolve-audit-1'",
        ),
    ];
    for (tamper, restore) in tampering_cases {
        connection.execute_batch(tamper).unwrap();
        assert!(SqliteProductLedger::open(&ledger.0)
            .unwrap()
            .load_decision_persistence_snapshot()
            .is_err());
        connection.execute_batch(restore).unwrap();
    }
    connection
        .execute(
            "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES ('decision-resolve-audit-1',1,'decision_request.resolved','complete','decision_request','decision-request-1')",
            [],
        )
        .unwrap();
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_decision_persistence_snapshot()
        .is_err());
    connection
        .execute(
            "DELETE FROM audit_effects WHERE audit_event_id='decision-resolve-audit-1' AND ordinal=1",
            [],
        )
        .unwrap();
    assert!(connection
        .execute(
            "UPDATE decision_h2a_replay_operations SET correlation_id='tampered-correlation' WHERE idempotency_id='decision-resolve-execute-1'",
            [],
        )
        .is_err());
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_decision_persistence_snapshot()
        .is_ok());
}

#[test]
fn writer_reopens_interleaved_h2a_and_h1_decision_timeline_losslessly() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-1").unwrap();
    let opened = SqliteProductLedger::open(&ledger.0).unwrap();
    drop(opened);
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();

    let mut first_create = create_command();
    first_create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            first_create,
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let first_command = resolve_preview_command(owner.clone());
    let first_prepared = resolve_preview(&first_command, owner.clone());
    opened
        .prepare_resolve_decision_request(first_command, first_prepared.clone())
        .unwrap();
    opened
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    first_prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    first_prepared.payload_digest().clone(),
                    IdempotencyId::parse("decision-resolve-execute-1").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("decision-resolve-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("decision-resolve-execute-correlation-1")
                        .unwrap(),
                },
            },
            [
                AuditEventId::parse("decision-resolve-audit-1").unwrap(),
                AuditEventId::parse("decision-resolve-audit-2").unwrap(),
                AuditEventId::parse("decision-resolve-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("decision-resolve-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("decision-resolve-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();

    let mut second_create = create_command();
    second_create.id = DecisionRequestId::parse("decision-request-2").unwrap();
    second_create.subject = DecisionSubject::parse("Second Synthetic Decision Request").unwrap();
    second_create.intended_owner = Some(owner.clone());
    second_create.context = DecisionOperationContext {
        idempotency_id: IdempotencyId::parse("decision-create-2").unwrap(),
        correlation_id: CorrelationId::parse("decision-create-correlation-2").unwrap(),
    };
    opened
        .create_decision_request_draft(
            second_create,
            AuditEventId::parse("decision-create-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(500),
        )
        .unwrap();
    let second_submit = SubmitDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-2").unwrap(),
        expected_version: AggregateVersion::initial(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-submit-2").unwrap(),
            correlation_id: CorrelationId::parse("decision-submit-correlation-2").unwrap(),
        },
    };
    opened
        .submit_decision_request(
            second_submit,
            AuditEventId::parse("decision-submit-audit-2").unwrap(),
            UtcTimestamp::from_unix_millis(600),
        )
        .unwrap();
    let mut second_command = resolve_preview_command(owner.clone());
    second_command.request_id = DecisionRequestId::parse("decision-request-2").unwrap();
    second_command.context = DecisionOperationContext {
        idempotency_id: IdempotencyId::parse("decision-resolve-prepare-2").unwrap(),
        correlation_id: CorrelationId::parse("decision-resolve-prepare-correlation-2").unwrap(),
    };
    second_command.resulting_action_requests[0].id =
        pmc_domain::identity::ActionRequestId::parse("synthetic-resulting-request-2").unwrap();
    let second_prepared = resolve_preview_for(
        &second_command,
        owner,
        "decision-resolve-prepared-2",
        "synthetic-decision-2",
        700,
    );
    opened
        .prepare_resolve_decision_request(second_command, second_prepared.clone())
        .unwrap();
    opened
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    second_prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    second_prepared.payload_digest().clone(),
                    IdempotencyId::parse("decision-resolve-execute-2").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("decision-resolve-execute-2").unwrap(),
                    correlation_id: CorrelationId::parse("decision-resolve-execute-correlation-2")
                        .unwrap(),
                },
            },
            [
                AuditEventId::parse("decision-resolve-audit-4").unwrap(),
                AuditEventId::parse("decision-resolve-audit-5").unwrap(),
                AuditEventId::parse("decision-resolve-audit-6").unwrap(),
            ],
            vec![AuditEventId::parse("decision-resolve-action-audit-2").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("decision-resolve-receipt-2").unwrap(),
            UtcTimestamp::from_unix_millis(800),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();
    drop(opened);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert_eq!(snapshot.decisions().len(), 2);
    assert!(snapshot
        .requests()
        .iter()
        .all(|request| request.state() == DecisionRequestState::Resolved));
    assert!(snapshot
        .replay()
        .iter()
        .enumerate()
        .all(|(ordinal, capsule)| capsule.operation_ordinal() == ordinal as u64));
}

fn resolve_preview_command(owner: StakeholderId) -> PrepareResolveDecisionRequest {
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
            id: pmc_domain::identity::ActionRequestId::parse("synthetic-resulting-request-1")
                .unwrap(),
            subject: pmc_domain::actions::ActionTitle::parse("Synthetic follow-up").unwrap(),
            details: pmc_domain::actions::ActionDetails::parse("Track synthetic follow-up.")
                .unwrap(),
            intended_owner: owner,
            due_at: UtcTimestamp::from_unix_millis(10_000),
            classification: DataClassification::Internal,
        }],
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-resolve-prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-resolve-prepare-correlation-1").unwrap(),
        },
    }
}

fn resolve_preview(
    command: &PrepareResolveDecisionRequest,
    owner: StakeholderId,
) -> WorkManagementPreparedIntent {
    resolve_preview_for(
        command,
        owner,
        "decision-resolve-prepared-1",
        "synthetic-decision-1",
        300,
    )
}

fn resolve_preview_for(
    command: &PrepareResolveDecisionRequest,
    owner: StakeholderId,
    prepared_id: &str,
    decision_id: &str,
    decided_at: i64,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::ResolveDecisionRequest {
            request_id: command.request_id.clone(),
            request_version: command.expected_version,
            decision_id: DecisionId::parse(decision_id).unwrap(),
            decision_classification: DataClassification::Internal,
            statement: command.statement.clone(),
            rationale: command.rationale.clone(),
            impact: command.impact.clone(),
            decision_owner: owner,
            decided_at: UtcTimestamp::from_unix_millis(decided_at),
            resulting_action_requests: command.resulting_action_requests.clone(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(
                command
                    .evidence_ids
                    .iter()
                    .map(|id| {
                        EvidenceReferenceMetadata::new(
                            id.clone(),
                            AggregateVersion::initial(),
                            DataClassification::Internal,
                            EvidenceRole::DecisionResolution,
                            EvidenceVerification::Verified {
                                verified_at: UtcTimestamp::from_unix_millis(200),
                                integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                            },
                        )
                    })
                    .collect(),
                command.judgments.clone(),
            )
            .unwrap()
            .evaluate_evidence_or_judgment()
            .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(decided_at),
    )
    .unwrap()
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

fn create_command() -> CreateDecisionRequestDraft {
    CreateDecisionRequestDraft {
        id: DecisionRequestId::parse("decision-request-1").unwrap(),
        subject: DecisionSubject::parse("Synthetic Decision Request").unwrap(),
        details: DecisionText::parse("Persist a public-safe Decision Request draft.").unwrap(),
        intended_owner: None,
        classification: DataClassification::Internal,
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-create-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-create-correlation-1").unwrap(),
        },
    }
}

fn submit_command() -> SubmitDecisionRequest {
    SubmitDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
        expected_version: AggregateVersion::initial(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-submit-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-submit-correlation-1").unwrap(),
        },
    }
}

fn seed_synthetic_stakeholder(path: &std::path::Path, id: &StakeholderId) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'stakeholder',1,'internal',0,0)",
            [id.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES (?1,'Synthetic Decision Owner','person','synthetic_fixture','test-only')",
            [id.as_str()],
        )
        .unwrap();
}

#[test]
fn writer_commits_create_decision_request_draft_before_success_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let command = create_command();
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let outcome = opened
        .create_decision_request_draft(
            command.clone(),
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .expect("typed Decision Request draft must commit before success");
    assert_eq!(outcome.record.id().as_str(), "decision-request-1");
    assert_eq!(outcome.record.state(), DecisionRequestState::Draft);
    assert_eq!(outcome.record.version().get(), 1);
    assert_eq!(opened.revision().unwrap(), 1);
    drop(opened);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .load_decision_persistence_snapshot()
        .expect("committed Decision Request state must decode after reopen");
    assert_eq!(snapshot.requests().len(), 1);
    assert!(snapshot.decisions().is_empty());
    assert!(snapshot.prepared().is_empty());
    assert_eq!(snapshot.replay().len(), 1);
    assert_eq!(snapshot.audits().len(), 1);
    assert_eq!(snapshot.replay()[0].operation_ordinal(), 0);

    let replay = reopened
        .create_decision_request_draft(
            command,
            AuditEventId::parse("decision-replay-audit-ignored").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .expect("exact Decision Request create must replay");
    assert_eq!(replay, outcome);
    assert_eq!(reopened.revision().unwrap(), 1);
}

/// A Decision resolution writes Action-side capsules into
/// `action_decision_replay_operations`, and the Action contiguity trigger counts
/// that table. The Action repository must allocate its next ordinal from the same
/// set, or the first Action write after any Decision resolution is aborted --
/// which is reachable from the desktop as soon as a resolution creates a
/// resulting Action Request.
#[test]
fn an_action_write_after_a_decision_resolution_allocates_a_contiguous_ordinal() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-contiguity").unwrap();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            create,
            AuditEventId::parse("contiguity-create").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("contiguity-submit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = resolve_preview_command(owner.clone());
    let prepared = resolve_preview(&command, owner.clone());
    opened
        .prepare_resolve_decision_request(command, prepared.clone())
        .unwrap();
    opened
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("contiguity-execute").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("contiguity-execute").unwrap(),
                    correlation_id: CorrelationId::parse("contiguity-execute-correlation").unwrap(),
                },
            },
            [
                AuditEventId::parse("contiguity-audit-1").unwrap(),
                AuditEventId::parse("contiguity-audit-2").unwrap(),
                AuditEventId::parse("contiguity-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("contiguity-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("contiguity-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();

    // The Action namespace now holds a durable capsule the Action repository did
    // not write. Its own next allocation must still be contiguous.
    opened
        .create_action_request_draft(
            pmc_domain::actions::CreateActionRequestDraft {
                id: pmc_domain::identity::ActionRequestId::parse("contiguity-request-1").unwrap(),
                title: pmc_domain::actions::ActionTitle::parse("Follow the resolution").unwrap(),
                details: pmc_domain::actions::ActionDetails::parse(
                    "An Action Request written after a Decision resolution.",
                )
                .unwrap(),
                intended_owner: Some(owner),
                response_due_at: Some(UtcTimestamp::from_unix_millis(20_000)),
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(30_000)),
                classification: DataClassification::Internal,
                context: pmc_domain::actions::ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("contiguity-action-create").unwrap(),
                    correlation_id: CorrelationId::parse("contiguity-action-correlation").unwrap(),
                },
            },
            AuditEventId::parse("contiguity-action-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(500),
        )
        .expect("an Action write after a Decision resolution must not be aborted");
}

/// History is what was decided: resolution Evidence that later loses its
/// verification must neither rewrite nor break the resolved Decision, whose
/// support is read from the execution-time snapshot. The whole snapshot
/// before and after the Evidence degrades must be equal.
#[test]
fn a_resolved_decision_keeps_its_support_after_its_evidence_degrades() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-degrade").unwrap();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    opened
        .create_decision_request_draft(
            create,
            AuditEventId::parse("degrade-create").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("degrade-submit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let evidence_id =
        pmc_domain::identity::EvidenceReferenceId::parse("synthetic-degrade-evidence-1").unwrap();
    let c = Connection::open(&ledger.0).unwrap();
    c.execute(
        "INSERT INTO aggregate_registry VALUES(?1,'evidence_reference',1,'internal',1,1)",
        [evidence_id.as_str()],
    )
    .unwrap();
    c.execute("INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at) VALUES(?1,'decision_resolution','verified',?2,200)", rusqlite::params![evidence_id.as_str(), "a".repeat(64)]).unwrap();
    drop(c);
    let mut command = resolve_preview_command(owner.clone());
    command.evidence_ids = vec![evidence_id];
    command.judgments.clear();
    let prepared = resolve_preview(&command, owner);
    opened
        .prepare_resolve_decision_request(command, prepared.clone())
        .unwrap();
    opened
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("degrade-execute-idempotency").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("degrade-execute-idempotency").unwrap(),
                    correlation_id: CorrelationId::parse("degrade-execute-correlation").unwrap(),
                },
            },
            [
                AuditEventId::parse("degrade-execute-audit-1").unwrap(),
                AuditEventId::parse("degrade-execute-audit-2").unwrap(),
                AuditEventId::parse("degrade-execute-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("degrade-execute-action-audit-1").unwrap()],
            pmc_domain::identity::ApprovalReceiptId::parse("degrade-execute-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            ResolveGate,
            ResolveGate,
            EvidenceGate,
        )
        .unwrap();
    drop(opened);
    let before = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_decision_persistence_snapshot()
        .unwrap();
    assert_eq!(before.decisions().len(), 1);

    let c = Connection::open(&ledger.0).unwrap();
    c.execute_batch(
        "UPDATE evidence_references SET verification='unverified',integrity_digest=NULL,last_verified_at=NULL WHERE id='synthetic-degrade-evidence-1';
         UPDATE aggregate_registry SET version=version+1 WHERE id='synthetic-degrade-evidence-1';",
    )
    .unwrap();
    drop(c);

    let after = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_decision_persistence_snapshot()
        .expect("a degraded Evidence must not break the Decision snapshot");
    assert_eq!(after.decisions(), before.decisions());
}
