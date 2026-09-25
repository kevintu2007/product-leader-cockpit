//! Issue H2a "Lower Data Classification" persistence.
//!
//! Issue's existing H2a architecture reconstructs `IssueRecord`s directly
//! from typed columns (like Risk), not via full event-sourced replay (like
//! Decision) -- and `IssueRecord::from_persisted` accepts any lifecycle
//! state/version generically, so unlike Risk this needed no additive
//! rehydration constructor. The only domain-layer gap was a small forwarding
//! method on `WorkManagementRuntimeComposition` (Issue has no bypass
//! constructor analogous to Risk's `rehydrate_with_h2a`: `SharedIssueAuthority`
//! is `pub(crate)`). Issue also has no durable post-start terminal for any
//! H2a operation, so a policy denial here is an ordinary rollback.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId,
        IssueId, PreparedIntentId,
    },
    issues::{
        ApproveAndExecuteLowerIssueClassification, CreateIssue, IssueDetails,
        IssueEvidenceAuthorityError, IssueEvidenceAuthorityPort, IssueExecutionPolicy,
        IssueExecutionPolicyPort, IssueOperationContext, IssueTitle,
        PrepareLowerIssueClassification, RecordedIssueClassification,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceReferenceMetadata,
        WorkManagementApproval, WorkManagementOperation, WorkManagementPreparedIntent,
        WorkManagementRationale,
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
            "pmc-synthetic-issue-h2a-lower-classification-{nonce}-{sequence}.sqlite3"
        )))
    }
}

#[derive(Clone, Copy)]
struct AllowApproval;
impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize(&self, _: AuditActor) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct AllowIssuePolicy;
impl IssueExecutionPolicyPort for AllowIssuePolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

#[derive(Clone, Copy)]
struct DenyIssuePolicy;
impl IssueExecutionPolicyPort for DenyIssuePolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Denied
    }
}

/// This operation never resolves evidence (see `PrepareLowerIssueClassification`'s
/// own doc comment), but the execute method's generic bound still requires an
/// `IssueEvidenceAuthorityPort` implementor.
#[derive(Clone, Copy)]
struct UnusedIssueEvidence;
impl IssueEvidenceAuthorityPort for UnusedIssueEvidence {
    fn resolve(
        &self,
        _: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        Err(IssueEvidenceAuthorityError::Unavailable)
    }
}

fn create_command() -> CreateIssue {
    CreateIssue {
        id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
        title: IssueTitle::parse("Synthetic independent issue").unwrap(),
        details: IssueDetails::parse("Synthetic only; no organizational data.").unwrap(),
        classification: DataClassification::Confidential,
        recurrence_of: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-issue-h2a-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-issue-h2a-create-correlation-1")
                .unwrap(),
        },
    }
}

fn seeded_ledger() -> (SyntheticLedger, SqliteProductLedger) {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_issue(
            create_command(),
            AuditEventId::parse("synthetic-issue-h2a-audit-create-1").unwrap(),
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
        WorkManagementOperation::LowerIssueClassification {
            issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
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
fn prepare_lower_issue_classification_persists_and_replays_the_exact_preview() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-1",
        DataClassification::Internal,
        &rationale,
    );
    let command = PrepareLowerIssueClassification {
        issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale: rationale.clone(),
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-issue-h2a-prepare-idempotency-1")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-issue-h2a-prepare-correlation-1")
                .unwrap(),
        },
    };
    let first = writer
        .prepare_lower_issue_classification(command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = writer
        .prepare_lower_issue_classification(command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_lower_issue_classification_rejects_a_non_lowering_proposal() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic non-lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-2",
        DataClassification::Restricted,
        &rationale,
    );
    let command = PrepareLowerIssueClassification {
        issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        proposed_classification: DataClassification::Internal,
        rationale,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-issue-h2a-prepare-idempotency-2")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-issue-h2a-prepare-correlation-2")
                .unwrap(),
        },
    };
    assert!(writer
        .prepare_lower_issue_classification(command, prepared)
        .is_err());
}

#[test]
fn approve_and_execute_lower_issue_classification_persists_and_replays() {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic execute lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-3",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-issue-h2a-prepare-idempotency-3",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-prepare-correlation-3",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-3").unwrap();
    let command_execute = ApproveAndExecuteLowerIssueClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: IssueOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-issue-h2a-execute-correlation-3")
                .unwrap(),
        },
    };
    let audit_id = AuditEventId::parse("synthetic-issue-h2a-audit-execute-3").unwrap();
    let receipt_id = ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-3").unwrap();
    let outcome = writer
        .approve_and_execute_lower_issue_classification(
            command_execute.clone(),
            audit_id.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            UnusedIssueEvidence,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
    assert_eq!(outcome.approval_receipt_id, Some(receipt_id.clone()));

    let replay = writer
        .approve_and_execute_lower_issue_classification(
            command_execute,
            audit_id,
            receipt_id,
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            AllowIssuePolicy,
            UnusedIssueEvidence,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
}

#[test]
fn approve_and_execute_lower_issue_classification_rejects_a_stale_preview() {
    let (ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic stale rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-4",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-issue-h2a-prepare-idempotency-4",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-prepare-correlation-4",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    // Issue's decode reconstructs the record directly from typed columns
    // (like Risk's), so directly advancing aggregate_registry's version is
    // enough to make the stored preview genuinely stale.
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE aggregate_registry SET version=2 WHERE id='synthetic-issue-h2a-1'",
            [],
        )
        .unwrap();
    drop(connection);

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-4").unwrap();
    let command_execute = ApproveAndExecuteLowerIssueClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: IssueOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-issue-h2a-execute-correlation-4")
                .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_issue_classification(
        command_execute,
        AuditEventId::parse("synthetic-issue-h2a-audit-execute-4").unwrap(),
        ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-4").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        AllowIssuePolicy,
        UnusedIssueEvidence,
        RecordedIssueClassification,
    );
    assert!(result.is_err());
}

#[test]
fn approve_and_execute_lower_issue_classification_denies_without_persisting_a_terminal() {
    // Issue has no durable post-start terminal for any H2a operation (see
    // this file's module doc comment): a policy denial at execute time is
    // an ordinary rollback, not a persisted denial row like Risk's V11.
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic denied rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-5",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-issue-h2a-prepare-idempotency-5",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-prepare-correlation-5",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-5").unwrap();
    let command_execute = ApproveAndExecuteLowerIssueClassification {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: IssueOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-issue-h2a-execute-correlation-5")
                .unwrap(),
        },
    };
    let result = writer.approve_and_execute_lower_issue_classification(
        command_execute,
        AuditEventId::parse("synthetic-issue-h2a-audit-execute-5").unwrap(),
        ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-5").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        DenyIssuePolicy,
        UnusedIssueEvidence,
        RecordedIssueClassification,
    );
    assert!(result.is_err());

    // The prepared intent must still be outstanding (not consumed) after an
    // ordinary rollback, so the same preview can be retried once allowed.
    let reopened = SqliteProductLedger::open(&_ledger.0).unwrap();
    let connection = Connection::open(&_ledger.0).unwrap();
    let consumed_at: Option<i64> = connection
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(consumed_at.is_none());
    drop(reopened);
}

#[test]
fn restart_recovers_the_lowered_issue_after_execute() {
    let (ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic restart rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-6",
        DataClassification::Public,
        &rationale,
    );
    writer
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Public,
                rationale,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-issue-h2a-prepare-idempotency-6",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-prepare-correlation-6",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-6").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    writer
        .approve_and_execute_lower_issue_classification(
            ApproveAndExecuteLowerIssueClassification {
                approval,
                context: IssueOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-execute-correlation-6",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-issue-h2a-audit-execute-6").unwrap(),
            ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-6").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            UnusedIssueEvidence,
            RecordedIssueClassification,
        )
        .unwrap();
    drop(writer);

    // Verified directly against durable columns, not `load_issue_h1_runtime_
    // snapshot`: that loader's `IssueH1RuntimeSnapshot::try_new` still pins
    // every standalone issue to `version == AggregateVersion::initial()`,
    // which this operation is the first to violate for an Open issue with
    // no lifecycle transition (see the follow-up flagged after this slice).
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    let (classification, version): (String, i64) = connection
        .query_row(
            "SELECT registry.classification,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.id='synthetic-issue-h2a-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(classification, "public");
    assert_eq!(version, 2);
    drop(reopened);
}

#[test]
fn approve_and_execute_lower_issue_classification_recovers_an_outstanding_preview_after_restart() {
    let (ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic outstanding rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-7",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-issue-h2a-prepare-idempotency-7",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-prepare-correlation-7",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    drop(writer);

    // Reopening and executing against the just-reopened handle proves the
    // outstanding preview survives a restart through real durable columns
    // (prepared_intents + this operation's own V28 command table), not
    // through in-process state carried across the `drop`.
    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-7").unwrap();
    let outcome = reopened
        .approve_and_execute_lower_issue_classification(
            ApproveAndExecuteLowerIssueClassification {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: IssueOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-execute-correlation-7",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-issue-h2a-audit-execute-7").unwrap(),
            ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-7").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            UnusedIssueEvidence,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(
        outcome.record.classification(),
        DataClassification::Internal
    );
}

// Regression added after a fresh-context review found that Delivery's H2a
// execute idempotent-replay branch never
// checked `command.approval.idempotency_id() == command.context.
// idempotency_id`, and a follow-up audit found the identical gap here.
#[test]
fn approve_and_execute_lower_issue_classification_rejects_a_replay_with_a_mismatched_approval_idempotency_id(
) {
    let (_ledger, mut writer) = seeded_ledger();
    let rationale = WorkManagementRationale::parse("Synthetic execute lowering rationale").unwrap();
    let prepared = lowering_prepared(
        "synthetic-issue-h2a-prepared-8",
        DataClassification::Internal,
        &rationale,
    );
    writer
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("synthetic-issue-h2a-1").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-issue-h2a-prepare-idempotency-8",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-prepare-correlation-8",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-8").unwrap();
    writer
        .approve_and_execute_lower_issue_classification(
            ApproveAndExecuteLowerIssueClassification {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: IssueOperationContext {
                    idempotency_id: execute_idempotency_id.clone(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-issue-h2a-execute-correlation-8",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-issue-h2a-audit-execute-8").unwrap(),
            ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-8").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            UnusedIssueEvidence,
            RecordedIssueClassification,
        )
        .unwrap();

    let mismatched_approval = WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-issue-h2a-execute-idempotency-8-different").unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap();
    let result = writer.approve_and_execute_lower_issue_classification(
        ApproveAndExecuteLowerIssueClassification {
            approval: mismatched_approval,
            context: IssueOperationContext {
                idempotency_id: execute_idempotency_id,
                correlation_id: CorrelationId::parse(
                    "synthetic-issue-h2a-execute-correlation-8-replay",
                )
                .unwrap(),
            },
        },
        AuditEventId::parse("synthetic-issue-h2a-audit-execute-8-replay").unwrap(),
        ApprovalReceiptId::parse("synthetic-issue-h2a-receipt-8-replay").unwrap(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        AllowIssuePolicy,
        UnusedIssueEvidence,
        RecordedIssueClassification,
    );
    assert!(result.is_err());
}
