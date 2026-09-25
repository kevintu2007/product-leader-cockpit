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
        ApproveAndExecuteResolveDecisionRequest, ApproveAndExecuteSupersedeDecision,
        CreateDecisionRequestDraft, DecisionEvidenceAuthorityError, DecisionEvidenceAuthorityPort,
        DecisionExecutionPolicy, DecisionExecutionPolicyPort, DecisionOperationContext,
        DecisionSubject, DecisionText, PrepareResolveDecisionRequest, PrepareSupersedeDecision,
        SubmitDecisionRequest,
    },
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, DecisionId,
        DecisionRequestId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, DecisionResultingActionRequest,
        EvidenceOrJudgment, IncompleteDownstreamWork, WorkManagementApproval,
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
            "pmc-synthetic-decision-supersede-{nonce}-{sequence}.sqlite3"
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

#[derive(Clone, Copy)]
struct SupersedeGate;
impl ApprovalAuthorizationPort for SupersedeGate {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
impl DecisionExecutionPolicyPort for SupersedeGate {
    fn current_policy(&self, _: &WorkManagementOperation) -> DecisionExecutionPolicy {
        DecisionExecutionPolicy::Allowed
    }
}
impl DecisionEvidenceAuthorityPort for SupersedeGate {
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

/// Bring a fresh ledger up to "one Effective Decision exists" (synthetic id
/// `synthetic-decision-1`), the shared prerequisite every Supersede test
/// needs -- goes through the real H1 create/submit -> H2a resolve path,
/// mirroring `sqlite-decision-repository.rs`'s own fixtures.
fn seed_resolved_decision(ledger: &SyntheticLedger, owner: &StakeholderId) {
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_decision_request_draft(
            CreateDecisionRequestDraft {
                id: DecisionRequestId::parse("decision-request-1").unwrap(),
                subject: DecisionSubject::parse("Synthetic Decision Request").unwrap(),
                details: DecisionText::parse("Persist a public-safe Decision Request draft.")
                    .unwrap(),
                intended_owner: Some(owner.clone()),
                classification: DataClassification::Internal,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("decision-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("decision-create-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            SubmitDecisionRequest {
                request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
                expected_version: AggregateVersion::initial(),
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("decision-submit-1").unwrap(),
                    correlation_id: CorrelationId::parse("decision-submit-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let resolve_command = PrepareResolveDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
        expected_version: AggregateVersion::initial().next().unwrap(),
        statement: DecisionText::parse("Choose synthetic option A.").unwrap(),
        rationale: DecisionText::parse("Synthetic tradeoff is documented.").unwrap(),
        impact: DecisionText::parse("Synthetic delivery remains on track.").unwrap(),
        evidence_ids: Vec::new(),
        judgments: vec![pmc_domain::work_management::HumanJudgment::new(
            pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale,
            "Synthetic owner judgment for the original resolve.",
            DataClassification::Internal,
        )
        .unwrap()],
        resulting_action_requests: Vec::new(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-resolve-prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-resolve-prepare-correlation-1").unwrap(),
        },
    };
    let resolve_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("decision-resolve-prepared-1").unwrap(),
        WorkManagementOperation::ResolveDecisionRequest {
            request_id: resolve_command.request_id.clone(),
            request_version: resolve_command.expected_version,
            decision_id: DecisionId::parse("synthetic-decision-1").unwrap(),
            decision_classification: DataClassification::Internal,
            statement: resolve_command.statement.clone(),
            rationale: resolve_command.rationale.clone(),
            impact: resolve_command.impact.clone(),
            decision_owner: owner.clone(),
            decided_at: UtcTimestamp::from_unix_millis(300),
            resulting_action_requests: Vec::new(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(Vec::new(), resolve_command.judgments.clone())
                .unwrap()
                .evaluate_evidence_or_judgment()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(300),
    )
    .unwrap();
    opened
        .prepare_resolve_decision_request(resolve_command, resolve_prepared.clone())
        .unwrap();
    opened
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    resolve_prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    resolve_prepared.payload_digest().clone(),
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
            Vec::new(),
            ApprovalReceiptId::parse("decision-resolve-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            SupersedeGate,
            SupersedeGate,
            SupersedeGate,
        )
        .unwrap();
}

/// Same shared prerequisite as `seed_resolved_decision`, but the ORIGINAL
/// Resolve also creates one resulting Action Request
/// (`synthetic-downstream-request-1`) that is deliberately left untouched
/// afterward -- still Open, version 1, classification Internal, tied to
/// `synthetic-decision-1` via `source_decision_id`
/// (`create_resulting_action_request_from_decision_transition` always
/// starts these at `ActionRequestState::Open`/`AggregateVersion::initial()`,
/// see `pmc-domain/src/actions.rs`). This is exactly what `Decision::downstream()`
/// (`pmc-domain/src/decisions.rs`) flags as `IncompleteDownstreamWork::ActionRequest`
/// for any Supersede of that same decision -- the fixture Supersede's own
/// downstream-marking integration test needs.
fn seed_resolved_decision_with_open_downstream_request(
    ledger: &SyntheticLedger,
    owner: &StakeholderId,
) {
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    opened
        .create_decision_request_draft(
            CreateDecisionRequestDraft {
                id: DecisionRequestId::parse("decision-request-1").unwrap(),
                subject: DecisionSubject::parse("Synthetic Decision Request").unwrap(),
                details: DecisionText::parse("Persist a public-safe Decision Request draft.")
                    .unwrap(),
                intended_owner: Some(owner.clone()),
                classification: DataClassification::Internal,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("decision-create-1").unwrap(),
                    correlation_id: CorrelationId::parse("decision-create-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    opened
        .submit_decision_request(
            SubmitDecisionRequest {
                request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
                expected_version: AggregateVersion::initial(),
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse("decision-submit-1").unwrap(),
                    correlation_id: CorrelationId::parse("decision-submit-correlation-1").unwrap(),
                },
            },
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let downstream_spec = DecisionResultingActionRequest {
        id: pmc_domain::identity::ActionRequestId::parse("synthetic-downstream-request-1").unwrap(),
        subject: pmc_domain::actions::ActionTitle::parse("Synthetic downstream follow-up").unwrap(),
        details: pmc_domain::actions::ActionDetails::parse("Track synthetic downstream follow-up.")
            .unwrap(),
        intended_owner: owner.clone(),
        due_at: UtcTimestamp::from_unix_millis(10_000),
        classification: DataClassification::Internal,
    };
    let resolve_command = PrepareResolveDecisionRequest {
        request_id: DecisionRequestId::parse("decision-request-1").unwrap(),
        expected_version: AggregateVersion::initial().next().unwrap(),
        statement: DecisionText::parse("Choose synthetic option A.").unwrap(),
        rationale: DecisionText::parse("Synthetic tradeoff is documented.").unwrap(),
        impact: DecisionText::parse("Synthetic delivery remains on track.").unwrap(),
        evidence_ids: Vec::new(),
        judgments: vec![pmc_domain::work_management::HumanJudgment::new(
            pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale,
            "Synthetic owner judgment for the original resolve.",
            DataClassification::Internal,
        )
        .unwrap()],
        resulting_action_requests: vec![downstream_spec.clone()],
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-resolve-prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-resolve-prepare-correlation-1").unwrap(),
        },
    };
    let resolve_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("decision-resolve-prepared-1").unwrap(),
        WorkManagementOperation::ResolveDecisionRequest {
            request_id: resolve_command.request_id.clone(),
            request_version: resolve_command.expected_version,
            decision_id: DecisionId::parse("synthetic-decision-1").unwrap(),
            decision_classification: DataClassification::Internal,
            statement: resolve_command.statement.clone(),
            rationale: resolve_command.rationale.clone(),
            impact: resolve_command.impact.clone(),
            decision_owner: owner.clone(),
            decided_at: UtcTimestamp::from_unix_millis(300),
            resulting_action_requests: vec![downstream_spec],
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(Vec::new(), resolve_command.judgments.clone())
                .unwrap()
                .evaluate_evidence_or_judgment()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(300),
    )
    .unwrap();
    opened
        .prepare_resolve_decision_request(resolve_command, resolve_prepared.clone())
        .unwrap();
    opened
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest {
                approval: WorkManagementApproval::new(
                    resolve_prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    resolve_prepared.payload_digest().clone(),
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
            ApprovalReceiptId::parse("decision-resolve-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(400),
            SupersedeGate,
            SupersedeGate,
            SupersedeGate,
        )
        .unwrap();
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

fn supersede_command(owner: StakeholderId) -> PrepareSupersedeDecision {
    PrepareSupersedeDecision {
        decision_id: DecisionId::parse("synthetic-decision-1").unwrap(),
        expected_version: AggregateVersion::initial(),
        replacement_statement: DecisionText::parse("Choose synthetic option B instead.").unwrap(),
        replacement_rationale: DecisionText::parse("Synthetic circumstances changed.").unwrap(),
        replacement_impact: DecisionText::parse("Synthetic delivery re-plans around option B.")
            .unwrap(),
        replacement_owner: owner.clone(),
        evidence_ids: Vec::new(),
        judgments: vec![pmc_domain::work_management::HumanJudgment::new(
            pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale,
            "Synthetic owner judgment for the supersede.",
            DataClassification::Internal,
        )
        .unwrap()],
        resulting_action_requests: vec![DecisionResultingActionRequest {
            id: pmc_domain::identity::ActionRequestId::parse("synthetic-supersede-request-1")
                .unwrap(),
            subject: pmc_domain::actions::ActionTitle::parse("Synthetic supersede follow-up")
                .unwrap(),
            details: pmc_domain::actions::ActionDetails::parse(
                "Track synthetic supersede follow-up.",
            )
            .unwrap(),
            intended_owner: owner,
            due_at: UtcTimestamp::from_unix_millis(20_000),
            classification: DataClassification::Internal,
        }],
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-supersede-prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-supersede-prepare-correlation-1")
                .unwrap(),
        },
    }
}

fn supersede_preview(command: &PrepareSupersedeDecision) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("decision-supersede-prepared-1").unwrap(),
        WorkManagementOperation::SupersedeDecision {
            decision_id: command.decision_id.clone(),
            decision_version: command.expected_version,
            replacement_decision_id: DecisionId::parse("synthetic-decision-2").unwrap(),
            replacement_decision_version: AggregateVersion::initial(),
            replacement_decision_classification: DataClassification::Internal,
            replacement_statement: command.replacement_statement.clone(),
            replacement_rationale: command.replacement_rationale.clone(),
            replacement_impact: command.replacement_impact.clone(),
            replacement_owner: command.replacement_owner.clone(),
            replacement_decided_at: UtcTimestamp::from_unix_millis(500),
            resulting_action_requests: command.resulting_action_requests.clone(),
            incomplete_downstream: Vec::new(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(Vec::new(), command.judgments.clone())
                .unwrap()
                .evaluate_evidence_or_judgment()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(500),
    )
    .unwrap()
}

/// `supersede_preview` with a non-empty `incomplete_downstream` -- the shape
/// its own hardcoded `Vec::new()` never exercises.
fn supersede_preview_with_downstream(
    command: &PrepareSupersedeDecision,
    incomplete_downstream: Vec<IncompleteDownstreamWork>,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("decision-supersede-prepared-1").unwrap(),
        WorkManagementOperation::SupersedeDecision {
            decision_id: command.decision_id.clone(),
            decision_version: command.expected_version,
            replacement_decision_id: DecisionId::parse("synthetic-decision-2").unwrap(),
            replacement_decision_version: AggregateVersion::initial(),
            replacement_decision_classification: DataClassification::Internal,
            replacement_statement: command.replacement_statement.clone(),
            replacement_rationale: command.replacement_rationale.clone(),
            replacement_impact: command.replacement_impact.clone(),
            replacement_owner: command.replacement_owner.clone(),
            replacement_decided_at: UtcTimestamp::from_unix_millis(500),
            resulting_action_requests: command.resulting_action_requests.clone(),
            incomplete_downstream,
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(Vec::new(), command.judgments.clone())
                .unwrap()
                .evaluate_evidence_or_judgment()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(500),
    )
    .unwrap()
}

#[test]
fn writer_persists_supersede_preview_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-1").unwrap();
    seed_resolved_decision(&ledger, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let command = supersede_command(owner);
    let prepared = supersede_preview(&command);
    let outcome = opened
        .prepare_supersede_decision(command.clone(), prepared.clone())
        .expect("canonical supersede preview must persist");
    assert_eq!(outcome, prepared);
    drop(opened);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert!(snapshot.prepared().contains(&prepared));
    assert_eq!(
        reopened
            .prepare_supersede_decision(command, prepared.clone())
            .unwrap(),
        prepared
    );
}

#[test]
fn writer_rejects_supersede_prepare_idempotency_conflict_on_altered_replay() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-2").unwrap();
    seed_resolved_decision(&ledger, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let command = supersede_command(owner);
    let prepared = supersede_preview(&command);
    opened
        .prepare_supersede_decision(command.clone(), prepared.clone())
        .unwrap();
    let mut altered = command;
    altered.replacement_rationale =
        DecisionText::parse("A materially different rationale.").unwrap();
    assert_eq!(
        opened
            .prepare_supersede_decision(altered, prepared)
            .unwrap_err(),
        LedgerTransactionError::Operation(pmc_domain::error::DomainError::new(
            ErrorCode::DomainIdempotencyConflict,
            pmc_domain::error::MessageKey::parse("ledger.idempotency_conflict").unwrap(),
            CorrelationId::parse("decision-supersede-prepare-correlation-1").unwrap(),
            false,
        ))
    );
}

#[test]
fn writer_executes_exact_persisted_supersede_and_reopens_decision_and_action_namespaces() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-3").unwrap();
    seed_resolved_decision(&ledger, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();
    let command = supersede_command(owner);
    let prepared = supersede_preview(&command);
    opened
        .prepare_supersede_decision(command, prepared.clone())
        .unwrap();
    let execute = ApproveAndExecuteSupersedeDecision {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("decision-supersede-execute-1").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-supersede-execute-1").unwrap(),
            correlation_id: CorrelationId::parse("decision-supersede-execute-correlation-1")
                .unwrap(),
        },
    };
    let outcome = opened
        .approve_and_execute_supersede_decision(
            execute.clone(),
            [
                AuditEventId::parse("decision-supersede-audit-1").unwrap(),
                AuditEventId::parse("decision-supersede-audit-2").unwrap(),
                AuditEventId::parse("decision-supersede-audit-3").unwrap(),
            ],
            vec![AuditEventId::parse("decision-supersede-action-audit-1").unwrap()],
            ApprovalReceiptId::parse("decision-supersede-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(600),
            SupersedeGate,
            SupersedeGate,
            SupersedeGate,
        )
        .expect("canonical supersede execution must persist");
    assert_eq!(outcome.superseded.id().as_str(), "synthetic-decision-1");
    assert_eq!(outcome.replacement.id().as_str(), "synthetic-decision-2");
    assert_eq!(
        outcome.superseded.superseded_by_decision_id(),
        Some(outcome.replacement.id())
    );
    assert_eq!(
        outcome.replacement.supersedes_decision_id(),
        Some(outcome.superseded.id())
    );
    assert_eq!(outcome.resulting_action_request_ids.len(), 1);
    assert!(outcome.flagged_downstream.is_empty());
    assert_eq!(outcome.audit_events.len(), 3);
    drop(opened);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert!(snapshot.decisions().contains(&outcome.superseded));
    assert!(snapshot.decisions().contains(&outcome.replacement));
    assert!(snapshot.prepared().is_empty());
    // Action namespace must also reopen losslessly -- the resulting action
    // request Supersede created is a decision-triggered capsule (same
    // provenance-gap-fix machinery Resolve already exercises).
    reopened.load_action_persistence_snapshot().unwrap();

    assert_eq!(
        reopened
            .approve_and_execute_supersede_decision(
                execute,
                [
                    AuditEventId::parse("ignored-audit-1").unwrap(),
                    AuditEventId::parse("ignored-audit-2").unwrap(),
                    AuditEventId::parse("ignored-audit-3").unwrap(),
                ],
                vec![AuditEventId::parse("ignored-action-audit-1").unwrap()],
                ApprovalReceiptId::parse("ignored-receipt-1").unwrap(),
                UtcTimestamp::from_unix_millis(700),
                SupersedeGate,
                SupersedeGate,
                SupersedeGate,
            )
            .unwrap(),
        outcome
    );
}

/// Decision Supersede's own downstream-marking
/// (`IncompleteDownstreamWork`) had no SQLite integration test. Seeds a
/// decision with one still-Open resulting Action Request from its ORIGINAL
/// Resolve, Supersedes it with that request flagged as incomplete
/// downstream work, and proves the whole round trip: PREPARE persists and
/// decodes the exact `incomplete_downstream` list, EXECUTE actually marks
/// the real `action_requests` row `superseded_premise=1` (via
/// `persist_supersede_downstream_action_rows`, already-shipped machinery
/// this test is the first to exercise), and both the Decision and Action
/// namespaces reopen losslessly afterward.
#[test]
fn writer_marks_incomplete_downstream_action_request_superseded_premise_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-downstream").unwrap();
    seed_resolved_decision_with_open_downstream_request(&ledger, &owner);
    let mut opened = SqliteProductLedger::open(&ledger.0).unwrap();

    let downstream_request_id =
        pmc_domain::identity::ActionRequestId::parse("synthetic-downstream-request-1").unwrap();
    let downstream = vec![IncompleteDownstreamWork::ActionRequest(
        downstream_request_id.clone(),
        AggregateVersion::initial(),
        DataClassification::Internal,
    )];
    let command = supersede_command(owner);
    let prepared = supersede_preview_with_downstream(&command, downstream);
    let outcome = opened
        .prepare_supersede_decision(command, prepared.clone())
        .expect("supersede PREPARE with one flagged downstream Action Request must persist");
    assert_eq!(outcome, prepared);
    drop(opened);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    assert!(
        snapshot.prepared().contains(&prepared),
        "PREPARE's incomplete_downstream must decode back exactly, not just round-trip the digest"
    );

    let execute = ApproveAndExecuteSupersedeDecision {
        approval: WorkManagementApproval::new(
            prepared.id().clone(),
            AuditActor::HeadOfProducts,
            prepared.payload_digest().clone(),
            IdempotencyId::parse("decision-supersede-downstream-execute-1").unwrap(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-supersede-downstream-execute-1")
                .unwrap(),
            correlation_id: CorrelationId::parse(
                "decision-supersede-downstream-execute-correlation-1",
            )
            .unwrap(),
        },
    };
    let outcome = reopened
        .approve_and_execute_supersede_decision(
            execute.clone(),
            [
                AuditEventId::parse("decision-supersede-downstream-audit-1").unwrap(),
                AuditEventId::parse("decision-supersede-downstream-audit-2").unwrap(),
                AuditEventId::parse("decision-supersede-downstream-audit-3").unwrap(),
            ],
            vec![
                AuditEventId::parse("decision-supersede-downstream-action-audit-1").unwrap(),
                AuditEventId::parse("decision-supersede-downstream-action-audit-2").unwrap(),
            ],
            ApprovalReceiptId::parse("decision-supersede-downstream-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(600),
            SupersedeGate,
            SupersedeGate,
            SupersedeGate,
        )
        .expect("supersede EXECUTE must mark the flagged downstream Action Request");
    assert_eq!(outcome.resulting_action_request_ids.len(), 1);
    assert_eq!(
        outcome.flagged_downstream,
        vec![IncompleteDownstreamWork::ActionRequest(
            downstream_request_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )]
    );
    drop(reopened);

    let connection = Connection::open(&ledger.0).unwrap();
    let (superseded_premise, version): (i64, i64) = connection
        .query_row(
            "SELECT ar.superseded_premise, reg.version FROM action_requests ar JOIN aggregate_registry reg ON reg.id=ar.id AND reg.aggregate_type='action_request' WHERE ar.id=?1",
            [downstream_request_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        superseded_premise, 1,
        "the real action_requests row must be marked superseded_premise=1"
    );
    assert_eq!(
        version, 2,
        "marking bumps the downstream request's own version"
    );
    drop(connection);

    let mut reopened_again = SqliteProductLedger::open(&ledger.0).unwrap();
    let decision_snapshot = reopened_again.load_decision_persistence_snapshot().unwrap();
    assert!(decision_snapshot.decisions().contains(&outcome.superseded));
    assert!(decision_snapshot.decisions().contains(&outcome.replacement));
    // Action namespace must also reopen losslessly with the marked request
    // reflected in its own decoded state.
    let action_snapshot = reopened_again.load_action_persistence_snapshot().unwrap();
    let marked_request = action_snapshot
        .requests()
        .iter()
        .find(|request| request.id() == &downstream_request_id)
        .expect("the downstream request must still decode after being marked");
    assert!(marked_request.has_superseded_premise());

    assert_eq!(
        reopened_again
            .approve_and_execute_supersede_decision(
                execute,
                [
                    AuditEventId::parse("ignored-downstream-audit-1").unwrap(),
                    AuditEventId::parse("ignored-downstream-audit-2").unwrap(),
                    AuditEventId::parse("ignored-downstream-audit-3").unwrap(),
                ],
                vec![
                    AuditEventId::parse("ignored-downstream-action-audit-1").unwrap(),
                    AuditEventId::parse("ignored-downstream-action-audit-2").unwrap(),
                ],
                ApprovalReceiptId::parse("ignored-downstream-receipt-1").unwrap(),
                UtcTimestamp::from_unix_millis(700),
                SupersedeGate,
                SupersedeGate,
                SupersedeGate,
            )
            .unwrap(),
        outcome
    );
}
