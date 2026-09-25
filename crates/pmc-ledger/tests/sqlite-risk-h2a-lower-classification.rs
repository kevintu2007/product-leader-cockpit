//! Risk H2a "Lower Data Classification" persistence.
//!
//! Unlike Portfolio/Product/Roadmap/Kpi/KpiObservation, Risk's existing H2a
//! machinery already persists outstanding previews durably (V13) and has an
//! established durable post-start terminal for policy denials (V11, closed
//! to exactly `record_occurrence`/`close`). This operation gets its own
//! fresh-root prepare/execute/terminal-denial authority (V24/V25) rather
//! than extending either.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, PreparedIntentId, RiskId,
    },
    risks::{
        AllowRiskEvidence, ApproveAndExecuteLowerRiskClassification, CreateRisk,
        PrepareLowerRiskClassification, RecordedRiskClassification, RiskDetails,
        RiskExecutionPolicy, RiskExecutionPolicyPort, RiskOperationContext, RiskTitle,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
    },
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::Connection;

#[derive(Clone, Copy)]
struct AllowApproval;
impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

#[derive(Clone, Copy)]
struct FixedRiskPolicy(RiskExecutionPolicy);
impl RiskExecutionPolicyPort for FixedRiskPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        self.0
    }
}

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-risk-h2a-lower-classification-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn create_risk() -> CreateRisk {
    CreateRisk {
        id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
        title: RiskTitle::parse("Synthetic delivery risk").unwrap(),
        details: RiskDetails::parse("Synthetic only; no organizational data.").unwrap(),
        classification: DataClassification::Confidential,
        context: RiskOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-risk-h2a-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-risk-h2a-create-correlation-1")
                .unwrap(),
        },
    }
}

fn seeded_ledger() -> (SyntheticLedger, SqliteProductLedger) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_risk(
            create_risk(),
            AuditEventId::parse("synthetic-risk-h2a-audit-create-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    (ledger, writer)
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
    proposed: DataClassification,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerRiskClassification {
            risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: proposed,
            rationale: rationale.clone(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap()
}

#[test]
fn prepare_lower_risk_classification_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-1",
        DataClassification::Internal,
        &rationale,
    );
    let command = PrepareLowerRiskClassification {
        risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale.clone(),
        context: RiskOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-risk-h2a-prepare-idempotency-1")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-risk-h2a-prepare-correlation-1")
                .unwrap(),
        },
    };
    let first = writer
        .prepare_lower_risk_classification(command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = writer
        .prepare_lower_risk_classification(command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
    assert_eq!(writer.revision().unwrap(), 2);
}

#[test]
fn prepare_lower_risk_classification_rejects_a_non_lowering_proposal() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic non-lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-2",
        DataClassification::Restricted,
        &rationale,
    );
    let command = PrepareLowerRiskClassification {
        risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale,
        context: RiskOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-risk-h2a-prepare-idempotency-2")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-risk-h2a-prepare-correlation-2")
                .unwrap(),
        },
    };
    assert!(writer
        .prepare_lower_risk_classification(command, prepared)
        .is_err());
}

#[test]
fn approve_and_execute_lower_risk_classification_persists_and_replays() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic execute lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-3",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-3",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-3",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-3").unwrap();
    let command_execute = ApproveAndExecuteLowerRiskClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: RiskOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-risk-h2a-execute-correlation-3")
                .unwrap(),
        },
    };
    let audit_id = AuditEventId::parse("synthetic-risk-h2a-audit-execute-3").unwrap();
    let receipt_id = ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-3").unwrap();
    let outcome = writer
        .approve_and_execute_lower_risk_classification(
            command_execute.clone(),
            audit_id.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.approval_receipt_id, Some(receipt_id.clone()));

    let replay = writer
        .approve_and_execute_lower_risk_classification(
            command_execute,
            audit_id,
            receipt_id,
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
    assert_eq!(writer.revision().unwrap(), 3);
}

#[test]
fn approve_and_execute_lower_risk_classification_rejects_a_stale_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic stale rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-4",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-4",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-4",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    // Advance the risk's version out from under the preview via an unrelated,
    // still-outstanding prepare-and-execute-close cycle is unnecessary here --
    // directly bumping aggregate_registry is the minimal way to make the
    // preview stale without depending on another operation's H2a machinery.
    let connection = Connection::open(&_ledger.0).unwrap();
    connection
        .execute(
            "UPDATE aggregate_registry SET version=2 WHERE id='synthetic-risk-h2a-1'",
            [],
        )
        .unwrap();
    drop(connection);

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-4").unwrap();
    let command_execute = ApproveAndExecuteLowerRiskClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: RiskOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-risk-h2a-execute-correlation-4")
                .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_risk_classification(
        command_execute,
        AuditEventId::parse("synthetic-risk-h2a-audit-execute-4").unwrap(),
        ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-4").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Allowed),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(result.is_err());
}

#[test]
fn approve_and_execute_lower_risk_classification_denies_and_persists_the_terminal() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic denied rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-5",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-5",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-5",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-5").unwrap();
    let command_execute = ApproveAndExecuteLowerRiskClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: RiskOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-risk-h2a-execute-correlation-5")
                .unwrap(),
        },
    };
    let audit_id = AuditEventId::parse("synthetic-risk-h2a-audit-execute-5").unwrap();
    let receipt_id = ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-5").unwrap();
    let first = writer.approve_and_execute_lower_risk_classification(
        command_execute.clone(),
        audit_id.clone(),
        receipt_id.clone(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Denied),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(first.is_err());

    let connection = Connection::open(&_ledger.0).unwrap();
    let denial_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM risk_h2a_lower_classification_terminal_denials WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(denial_count, 1);
    drop(connection);

    let replay = writer.approve_and_execute_lower_risk_classification(
        command_execute,
        audit_id,
        receipt_id,
        UtcTimestamp::from_unix_millis(999),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Denied),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(replay.is_err());
    let connection = Connection::open(&_ledger.0).unwrap();
    let denial_count_after_replay: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM risk_h2a_lower_classification_terminal_denials",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(denial_count_after_replay, 1);
}

#[test]
fn restart_recovers_the_lowered_risk_after_execute() {
    let (ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic restart rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-6",
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-6",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-6",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-6").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_risk_classification(
            ApproveAndExecuteLowerRiskClassification {
                approval,
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-execute-correlation-6",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-risk-h2a-audit-execute-6").unwrap(),
            ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-6").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 3);
    let snapshot = reopened.load_risk_persistence_snapshot().unwrap();
    let record = snapshot
        .risks()
        .iter()
        .find(|risk| risk.id().as_str() == "synthetic-risk-h2a-1")
        .unwrap();
    assert_eq!(record.classification(), DataClassification::Public);
}

#[test]
fn restart_recovers_an_outstanding_lower_classification_preview() {
    let (ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic outstanding rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-7",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-7",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-7",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    drop(writer);

    // Reopening and executing against the just-reopened handle proves the
    // outstanding preview survived the restart through the typed V24 decode
    // path (`decode_lower_risk_classification_prepared_previews`), not just
    // through in-memory state carried across the `drop`.
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-7").unwrap();
    let outcome = reopened
        .approve_and_execute_lower_risk_classification(
            ApproveAndExecuteLowerRiskClassification {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-execute-correlation-7",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-risk-h2a-audit-execute-7").unwrap(),
            ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-7").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
}

// Regression added after a review found that Delivery's H2a execute
// idempotent-replay branch never checked `command.approval.idempotency_id() == command.context.
// idempotency_id`, and a follow-up audit found the identical gap in both of
// Risk's replay branches (success execute and terminal-denial execute).

#[test]
fn approve_and_execute_lower_risk_classification_rejects_a_replay_with_a_mismatched_approval_idempotency_id(
) {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic execute lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-8",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-8",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-8",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-8").unwrap();
    writer
        .approve_and_execute_lower_risk_classification(
            ApproveAndExecuteLowerRiskClassification {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency_id.clone(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-execute-correlation-8",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-risk-h2a-audit-execute-8").unwrap(),
            ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-8").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();

    let mismatched_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-8-different").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let result = writer.approve_and_execute_lower_risk_classification(
        ApproveAndExecuteLowerRiskClassification {
            approval: mismatched_approval,
            context: RiskOperationContext {
                idempotency_id: execute_idempotency_id,
                correlation_id: CorrelationId::parse(
                    "synthetic-risk-h2a-execute-correlation-8-replay",
                )
                .unwrap(),
            },
        },
        AuditEventId::parse("synthetic-risk-h2a-audit-execute-8-replay").unwrap(),
        ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-8-replay").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Allowed),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(result.is_err());
}

#[test]
fn approve_and_execute_lower_risk_classification_terminal_denial_rejects_a_replay_with_a_mismatched_approval_idempotency_id(
) {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic denied rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-risk-h2a-prepared-9",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_risk_classification(
            PrepareLowerRiskClassification {
                risk_id: RiskId::parse("synthetic-risk-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-risk-h2a-prepare-idempotency-9",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-risk-h2a-prepare-correlation-9",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-9").unwrap();
    let first = writer.approve_and_execute_lower_risk_classification(
        ApproveAndExecuteLowerRiskClassification {
            approval: approval_for(&prepared, &execute_idempotency_id),
            context: RiskOperationContext {
                idempotency_id: execute_idempotency_id.clone(),
                correlation_id: CorrelationId::parse("synthetic-risk-h2a-execute-correlation-9")
                    .unwrap(),
            },
        },
        AuditEventId::parse("synthetic-risk-h2a-audit-execute-9").unwrap(),
        ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-9").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Denied),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(first.is_err());

    // Same prepared_id/digest as the committed terminal denial, but a
    // different idempotency_id than the one it was actually denied under.
    let mismatched_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-risk-h2a-execute-idempotency-9-different").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let replay = writer.approve_and_execute_lower_risk_classification(
        ApproveAndExecuteLowerRiskClassification {
            approval: mismatched_approval,
            context: RiskOperationContext {
                idempotency_id: execute_idempotency_id,
                correlation_id: CorrelationId::parse(
                    "synthetic-risk-h2a-execute-correlation-9-replay",
                )
                .unwrap(),
            },
        },
        AuditEventId::parse("synthetic-risk-h2a-audit-execute-9-replay").unwrap(),
        ApprovalReceiptId::parse("synthetic-risk-h2a-receipt-9-replay").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Denied),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    // Both the original (policy-denied) and the mismatched replay must be
    // errors -- the terminal-denial branch's own stored outcome is itself a
    // denial, so `is_err()` alone can't distinguish "caught by the identity
    // check" from "naturally replayed its own stored denial". What *would*
    // regress without the fix is a second denial row getting persisted for
    // the mismatched approval instead of the mismatch being rejected before
    // ever reaching the replay/persist path -- assert that didn't happen.
    assert!(replay.is_err());
    let connection = Connection::open(&_ledger.0).unwrap();
    let denial_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM risk_h2a_lower_classification_terminal_denials",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(denial_count, 1);
}
