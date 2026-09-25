//! SQLite persistence for Portfolio's governed "Lower Data Classification"
//! H2a operation -- the first of the twelve record types' domain-layer
//! lowering work to be wired into real `pmc-ledger` persistence.
//! Mirrors `sqlite-risk-repository.rs`'s prepare/execute round-trip and
//! replay coverage.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, PortfolioId,
        PreparedIntentId,
    },
    portfolio::{
        ApproveAndExecuteLowerPortfolioClassification, CreatePortfolio, LongText, OperationContext,
        PrepareLowerPortfolioClassification, ShortText,
    },
    provenance::Provenance,
    time::UtcTimestamp,
    work_management::{
        ApprovalConfirmation, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, WorkManagementRationale,
    },
};
use pmc_ledger::sqlite::SqliteProductLedger;

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
            "pmc-synthetic-portfolio-h2a-lower-classification-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn create_command() -> CreatePortfolio {
    CreatePortfolio {
        id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        name: ShortText::parse("Synthetic Portfolio").unwrap(),
        details: LongText::parse("Synthetic only; no organizational data.").unwrap(),
        classification: Some(DataClassification::Confidential),
        provenance: Provenance::UserEntered,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-portfolio-h2a-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-correlation-create-1")
                .unwrap(),
        },
    }
}

fn seeded_ledger() -> (SyntheticLedger, SqliteProductLedger) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_portfolio(
            create_command(),
            AuditEventId::parse("synthetic-portfolio-h2a-audit-create").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    (ledger, writer)
}

fn prepared_intent(
    prepared_id: &str,
    rationale: &WorkManagementRationale,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::LowerPortfolioClassification {
            portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
            portfolio_version: pmc_domain::identity::AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Internal,
            rationale: rationale.clone(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap()
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

#[test]
fn prepare_lower_portfolio_classification_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = prepared_intent("synthetic-portfolio-h2a-prepared-1", &rationale);
    let command = PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale.clone(),
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-portfolio-h2a-prepare-idempotency-1")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-prepare-correlation-1")
                .unwrap(),
        },
    };
    let first = writer
        .prepare_lower_portfolio_classification(command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = writer
        .prepare_lower_portfolio_classification(command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_lower_portfolio_classification_rejects_a_non_lowering_proposal() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Not actually a lowering").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-portfolio-h2a-prepared-not-lowering").unwrap(),
        WorkManagementOperation::LowerPortfolioClassification {
            portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
            portfolio_version: pmc_domain::identity::AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Confidential,
            rationale: rationale.clone(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap();
    let command = PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Confidential,
        rationale,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse(
                "synthetic-portfolio-h2a-prepare-not-lowering-idempotency",
            )
            .unwrap(),
            correlation_id: CorrelationId::parse(
                "synthetic-portfolio-h2a-prepare-not-lowering-correlation",
            )
            .unwrap(),
        },
    };
    assert!(writer
        .prepare_lower_portfolio_classification(command, prepared)
        .is_err());
}

#[test]
fn approve_and_execute_lower_portfolio_classification_persists_and_replays() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = prepared_intent("synthetic-portfolio-h2a-prepared-2", &rationale);
    let prepare_command = PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-portfolio-h2a-prepare-idempotency-2")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-prepare-correlation-2")
                .unwrap(),
        },
    };
    writer
        .prepare_lower_portfolio_classification(prepare_command, prepared.clone())
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-portfolio-h2a-execute-idempotency-2").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerPortfolioClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-execute-correlation-2")
                .unwrap(),
        },
    };
    let audit_event_id = AuditEventId::parse("synthetic-portfolio-h2a-audit-execute-2").unwrap();
    let approval_receipt_id =
        ApprovalReceiptId::parse("synthetic-portfolio-h2a-receipt-2").unwrap();
    let outcome = writer
        .approve_and_execute_lower_portfolio_classification(
            execute_command.clone(),
            audit_event_id.clone(),
            approval_receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(outcome.record.classification, DataClassification::Internal);
    assert_eq!(outcome.record.version.get(), 2);
    assert_eq!(writer.revision().unwrap(), 3);

    let replay = writer
        .approve_and_execute_lower_portfolio_classification(
            execute_command,
            audit_event_id,
            approval_receipt_id,
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    assert_eq!(replay.record.classification, DataClassification::Internal);
    assert_eq!(replay.record.version.get(), 2);
    assert_eq!(writer.revision().unwrap(), 3);

    drop(writer);
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 3);
}

#[test]
fn approve_and_execute_lower_portfolio_classification_rejects_a_stale_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = prepared_intent("synthetic-portfolio-h2a-prepared-3", &rationale);
    let prepare_command = PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-portfolio-h2a-prepare-idempotency-3")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-prepare-correlation-3")
                .unwrap(),
        },
    };
    writer
        .prepare_lower_portfolio_classification(prepare_command, prepared.clone())
        .unwrap();

    // Mutate the portfolio out from under the prepared preview via an
    // ordinary H1 update that only raises/holds classification (never a
    // lowering, so it is legal through the ordinary path).
    writer
        .update_portfolio_details(
            pmc_domain::portfolio::UpdatePortfolioDetails {
                id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                name: ShortText::parse("Synthetic Portfolio").unwrap(),
                details: LongText::parse("Synthetic only; no organizational data.").unwrap(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-portfolio-h2a-interleaved-update",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-portfolio-h2a-interleaved-update-correlation",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-portfolio-h2a-interleaved-audit").unwrap(),
            UtcTimestamp::from_unix_millis(150),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-portfolio-h2a-execute-idempotency-3").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let execute_command = ApproveAndExecuteLowerPortfolioClassification {
        approval,
        context: OperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-execute-correlation-3")
                .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_portfolio_classification(
        execute_command,
        AuditEventId::parse("synthetic-portfolio-h2a-audit-execute-3").unwrap(),
        ApprovalReceiptId::parse("synthetic-portfolio-h2a-receipt-3").unwrap(),
        UtcTimestamp::from_unix_millis(200),
    );
    assert!(result.is_err());
}

#[test]
fn restart_recovers_the_lowered_portfolio_after_execute() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = prepared_intent("synthetic-portfolio-h2a-prepared-4", &rationale);
    let prepare_command = PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-portfolio-h2a-prepare-idempotency-4")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-prepare-correlation-4")
                .unwrap(),
        },
    };
    writer
        .prepare_lower_portfolio_classification(prepare_command, prepared.clone())
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-portfolio-h2a-execute-idempotency-4").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval,
                context: OperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-portfolio-h2a-execute-correlation-4",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-portfolio-h2a-audit-execute-4").unwrap(),
            ApprovalReceiptId::parse("synthetic-portfolio-h2a-receipt-4").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    drop(writer);

    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    assert_eq!(reopened.revision().unwrap(), 3);
}

// Regression added after a fresh-context review of the Delivery family
// found that Delivery's H2a execute idempotent-replay branch never
// checked `command.approval.idempotency_id() == command.context.
// idempotency_id`, and a follow-up audit found the identical gap here.
#[test]
fn approve_and_execute_lower_portfolio_classification_rejects_a_replay_with_a_mismatched_approval_idempotency_id(
) {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = prepared_intent("synthetic-portfolio-h2a-prepared-5", &rationale);
    let prepare_command = PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId::parse("synthetic-portfolio-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale,
        context: OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-portfolio-h2a-prepare-idempotency-5")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-portfolio-h2a-prepare-correlation-5")
                .unwrap(),
        },
    };
    writer
        .prepare_lower_portfolio_classification(prepare_command, prepared.clone())
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-portfolio-h2a-execute-idempotency-5").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_portfolio_classification(
            ApproveAndExecuteLowerPortfolioClassification {
                approval,
                context: OperationContext {
                    idempotency_id: execute_idempotency_id.clone(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-portfolio-h2a-execute-correlation-5",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-portfolio-h2a-audit-execute-5").unwrap(),
            ApprovalReceiptId::parse("synthetic-portfolio-h2a-receipt-5").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();

    // Same prepared_id/actor/digest as the committed approval, but a
    // different idempotency_id than the one it was actually approved
    // under -- submitted alongside a context that still claims the
    // original (already-committed) idempotency_id, to hit the replay
    // branch's stored-row lookup.
    let mismatched_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-portfolio-h2a-execute-idempotency-5-different").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let result = writer.approve_and_execute_lower_portfolio_classification(
        ApproveAndExecuteLowerPortfolioClassification {
            approval: mismatched_approval,
            context: OperationContext {
                idempotency_id: execute_idempotency_id,
                correlation_id: CorrelationId::parse(
                    "synthetic-portfolio-h2a-execute-correlation-5-replay",
                )
                .unwrap(),
            },
        },
        AuditEventId::parse("synthetic-portfolio-h2a-audit-execute-5-replay").unwrap(),
        ApprovalReceiptId::parse("synthetic-portfolio-h2a-receipt-5-replay").unwrap(),
        UtcTimestamp::from_unix_millis(200),
    );
    assert!(result.is_err());
}
