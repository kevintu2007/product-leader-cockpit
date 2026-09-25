//! Decision H2a "Lower Data Classification" persistence.
//!
//! Unlike Portfolio/Product/Roadmap/Kpi/KpiObservation/Risk, Decision's H2a
//! machinery is a fully event-sourced, closed-set replay-capsule model
//! (`DecisionPersistenceCommand`/`DecisionPersistenceResult`/
//! `validate_decision_capsule`), and Decision has no durable post-start
//! terminal for any H2a operation -- a policy denial here is an ordinary
//! rollback, not a persisted terminal like Risk's V11.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    decisions::{
        ApproveAndExecuteLowerDecisionClassification, ApproveAndExecuteResolveDecisionRequest,
        CreateDecisionRequestDraft, DecisionEvidenceAuthorityError, DecisionEvidenceAuthorityPort,
        DecisionExecutionPolicy, DecisionExecutionPolicyPort, DecisionOperationContext,
        DecisionSubject, DecisionText, PrepareLowerDecisionClassification,
        PrepareResolveDecisionRequest, SubmitDecisionRequest,
    },
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, DecisionId,
        DecisionRequestId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, DecisionResultingActionRequest,
        HumanJudgment, HumanJudgmentDisposition, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, WorkManagementRationale,
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
            "pmc-synthetic-decision-h2a-lower-classification-{nonce}-{sequence}.sqlite3"
        )))
    }
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
            decision_owner: owner,
            decided_at: UtcTimestamp::from_unix_millis(300),
            resulting_action_requests: command.resulting_action_requests.clone(),
        },
        DataClassification::Internal,
        Some(
            pmc_domain::work_management::EvidenceOrJudgment::new(
                Vec::new(),
                command.judgments.clone(),
            )
            .unwrap()
            .evaluate_evidence_or_judgment()
            .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap()
}

/// Seeds a ledger through H1 create+submit and H2a resolve, returning the
/// handle and the resulting effective Decision's id (always
/// `synthetic-decision-1`, classification `Internal`).
fn seeded_ledger_with_resolved_decision() -> (SyntheticLedger, SqliteProductLedger, DecisionId) {
    let ledger = SyntheticLedger::new();
    let owner = StakeholderId::parse("synthetic-decision-owner-1").unwrap();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_synthetic_stakeholder(&ledger.0, &owner);
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut create = create_command();
    create.intended_owner = Some(owner.clone());
    writer
        .create_decision_request_draft(
            create,
            AuditEventId::parse("decision-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_decision_request(
            submit_command(),
            AuditEventId::parse("decision-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let command = resolve_preview_command(owner.clone());
    let prepared = resolve_preview(&command, owner);
    writer
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
    let outcome = writer
        .approve_and_execute_resolve_decision_request(
            execute,
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
    assert_eq!(
        outcome.decision.classification(),
        DataClassification::Internal
    );
    (ledger, writer, outcome.decision.id().clone())
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

fn lowering_prepared(
    prepared_id: &str,
    decision_id: &DecisionId,
    proposed: DataClassification,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerDecisionClassification {
            decision_id: decision_id.clone(),
            decision_version: AggregateVersion::initial(),
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
fn prepare_lower_decision_classification_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer, decision_id) = seeded_ledger_with_resolved_decision();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "decision-h2a-lower-prepared-1",
        &decision_id,
        DataClassification::Public,
        &rationale,
    );
    let command = PrepareLowerDecisionClassification {
        decision_id: decision_id.clone(),
        expected_version: AggregateVersion::initial(),
        proposed_classification: DataClassification::Public,
        rationale: rationale.clone(),
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-h2a-lower-prepare-idempotency-1")
                .unwrap(),
            correlation_id: CorrelationId::parse("decision-h2a-lower-prepare-correlation-1")
                .unwrap(),
        },
    };
    let first = writer
        .prepare_lower_decision_classification(command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = writer
        .prepare_lower_decision_classification(command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_lower_decision_classification_rejects_a_non_lowering_proposal() {
    let (_ledger, mut writer, decision_id) = seeded_ledger_with_resolved_decision();
    let rationale = WorkManagementRationale::parse("Synthetic non-lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "decision-h2a-lower-prepared-2",
        &decision_id,
        DataClassification::Restricted,
        &rationale,
    );
    let command = PrepareLowerDecisionClassification {
        decision_id,
        expected_version: AggregateVersion::initial(),
        proposed_classification: DataClassification::Public,
        rationale,
        context: DecisionOperationContext {
            idempotency_id: IdempotencyId::parse("decision-h2a-lower-prepare-idempotency-2")
                .unwrap(),
            correlation_id: CorrelationId::parse("decision-h2a-lower-prepare-correlation-2")
                .unwrap(),
        },
    };
    assert!(writer
        .prepare_lower_decision_classification(command, prepared)
        .is_err());
}

#[test]
fn approve_and_execute_lower_decision_classification_persists_and_replays() {
    let (_ledger, mut writer, decision_id) = seeded_ledger_with_resolved_decision();
    let rationale = WorkManagementRationale::parse("Synthetic execute lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "decision-h2a-lower-prepared-3",
        &decision_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_decision_classification(
            PrepareLowerDecisionClassification {
                decision_id: decision_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "decision-h2a-lower-prepare-idempotency-3",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-prepare-correlation-3",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("decision-h2a-lower-execute-idempotency-3").unwrap();
    let command_execute = ApproveAndExecuteLowerDecisionClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: DecisionOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("decision-h2a-lower-execute-correlation-3")
                .unwrap(),
        },
    };
    let audit_id = AuditEventId::parse("decision-h2a-lower-audit-execute-3").unwrap();
    let receipt_id = ApprovalReceiptId::parse("decision-h2a-lower-receipt-3").unwrap();
    let outcome = writer
        .approve_and_execute_lower_decision_classification(
            command_execute.clone(),
            audit_id.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(500),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();
    assert_eq!(outcome.record.classification(), DataClassification::Public);
    assert_eq!(outcome.approval_receipt_id, Some(receipt_id.clone()));

    let replay = writer
        .approve_and_execute_lower_decision_classification(
            command_execute,
            audit_id,
            receipt_id,
            UtcTimestamp::from_unix_millis(999),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();
    assert_eq!(replay, outcome);
}

#[test]
fn approve_and_execute_lower_decision_classification_rejects_a_stale_preview() {
    // Decision's decode is a full event-sourced replay: a decision's
    // current version comes from the replay capsule timeline, not from
    // directly tampering `aggregate_registry` (unlike Risk's decoder).
    // Genuine staleness here means a second, still-outstanding preview
    // whose `expected_version` was overtaken by an earlier execute against
    // the same decision -- so this test prepares two lowerings against
    // version 1, executes the first (advancing the decision to version 2),
    // then attempts to execute the second, still pinned to version 1.
    let (_ledger, mut writer, decision_id) = seeded_ledger_with_resolved_decision();
    let rationale = WorkManagementRationale::parse("Synthetic first lowering rationale").unwrap();
    let first_prepared = lowering_prepared(
        "decision-h2a-lower-prepared-4a",
        &decision_id,
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_decision_classification(
            PrepareLowerDecisionClassification {
                decision_id: decision_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "decision-h2a-lower-prepare-idempotency-4a",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-prepare-correlation-4a",
                    )
                    .unwrap(),
                },
            },
            first_prepared.clone(),
        )
        .unwrap();
    let stale_rationale = WorkManagementRationale::parse("Synthetic stale rationale").unwrap();
    let stale_prepared = lowering_prepared(
        "decision-h2a-lower-prepared-4b",
        &decision_id,
        DataClassification::Public,
        &stale_rationale,
    );
    writer
        .prepare_lower_decision_classification(
            PrepareLowerDecisionClassification {
                decision_id: decision_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale: stale_rationale,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "decision-h2a-lower-prepare-idempotency-4b",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-prepare-correlation-4b",
                    )
                    .unwrap(),
                },
            },
            stale_prepared.clone(),
        )
        .unwrap();

    let first_execute_idempotency_id =
        IdempotencyId::parse("decision-h2a-lower-execute-idempotency-4a").unwrap();
    writer
        .approve_and_execute_lower_decision_classification(
            ApproveAndExecuteLowerDecisionClassification {
                approval: approval_for(&first_prepared, &first_execute_idempotency_id),
                context: DecisionOperationContext {
                    idempotency_id: first_execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-execute-correlation-4a",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("decision-h2a-lower-audit-execute-4a").unwrap(),
            ApprovalReceiptId::parse("decision-h2a-lower-receipt-4a").unwrap(),
            UtcTimestamp::from_unix_millis(500),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();

    let stale_execute_idempotency_id =
        IdempotencyId::parse("decision-h2a-lower-execute-idempotency-4b").unwrap();
    let result = writer.approve_and_execute_lower_decision_classification(
        ApproveAndExecuteLowerDecisionClassification {
            approval: approval_for(&stale_prepared, &stale_execute_idempotency_id),
            context: DecisionOperationContext {
                idempotency_id: stale_execute_idempotency_id,
                correlation_id: CorrelationId::parse("decision-h2a-lower-execute-correlation-4b")
                    .unwrap(),
            },
        },
        AuditEventId::parse("decision-h2a-lower-audit-execute-4b").unwrap(),
        ApprovalReceiptId::parse("decision-h2a-lower-receipt-4b").unwrap(),
        UtcTimestamp::from_unix_millis(600),
        ResolveGate,
        ResolveGate,
        ResolveGate,
    );
    assert!(result.is_err());
}

#[test]
fn restart_recovers_the_lowered_decision_after_execute() {
    let (ledger, mut writer, decision_id) = seeded_ledger_with_resolved_decision();
    let rationale = WorkManagementRationale::parse("Synthetic restart rationale").unwrap();
    let prepared = lowering_prepared(
        "decision-h2a-lower-prepared-5",
        &decision_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_decision_classification(
            PrepareLowerDecisionClassification {
                decision_id: decision_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "decision-h2a-lower-prepare-idempotency-5",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-prepare-correlation-5",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("decision-h2a-lower-execute-idempotency-5").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_decision_classification(
            ApproveAndExecuteLowerDecisionClassification {
                approval,
                context: DecisionOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-execute-correlation-5",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("decision-h2a-lower-audit-execute-5").unwrap(),
            ApprovalReceiptId::parse("decision-h2a-lower-receipt-5").unwrap(),
            UtcTimestamp::from_unix_millis(500),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_decision_persistence_snapshot().unwrap();
    let decision = snapshot
        .decisions()
        .iter()
        .find(|item| item.id() == &decision_id)
        .unwrap();
    assert_eq!(decision.classification(), DataClassification::Public);
    assert_eq!(decision.version().get(), 2);
}

#[test]
fn restart_recovers_an_outstanding_lower_classification_preview() {
    let (ledger, mut writer, decision_id) = seeded_ledger_with_resolved_decision();
    let rationale = WorkManagementRationale::parse("Synthetic outstanding rationale").unwrap();
    let prepared = lowering_prepared(
        "decision-h2a-lower-prepared-6",
        &decision_id,
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_decision_classification(
            PrepareLowerDecisionClassification {
                decision_id: decision_id.clone(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: DecisionOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "decision-h2a-lower-prepare-idempotency-6",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-prepare-correlation-6",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    drop(writer);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("decision-h2a-lower-execute-idempotency-6").unwrap();
    let outcome = reopened
        .approve_and_execute_lower_decision_classification(
            ApproveAndExecuteLowerDecisionClassification {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: DecisionOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "decision-h2a-lower-execute-correlation-6",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("decision-h2a-lower-audit-execute-6").unwrap(),
            ApprovalReceiptId::parse("decision-h2a-lower-receipt-6").unwrap(),
            UtcTimestamp::from_unix_millis(500),
            ResolveGate,
            ResolveGate,
            ResolveGate,
        )
        .unwrap();
    assert_eq!(outcome.record.classification(), DataClassification::Public);
}
