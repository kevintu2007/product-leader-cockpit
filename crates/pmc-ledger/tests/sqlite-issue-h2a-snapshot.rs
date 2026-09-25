//! The complete Issue H2a snapshot (2b): every Issue at its current
//! lifecycle state, not only the standalone ones still at their create
//! state, and the Risk occurrence path that depends on it to refuse a
//! duplicate Issue identity.
//!
//! The conditions covered here are pristine-open, classification-lowered
//! open, resolved, and Risk-derived. Closed and reopened Issues travel the
//! same decoder (`decode_issue_h2a_record` reads every lifecycle column and
//! every evidence role for all of them) and are exercised by
//! `sqlite-issue-repository.rs`; they are not re-staged here.

// The Ledger's transaction error carries the whole safe envelope; the
// helpers below hand it straight to the assertions, as the other Ledger
// suites do.
#![allow(clippy::result_large_err)]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    error::ErrorCode,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, IssueId, PreparedIntentId, RiskId,
    },
    issues::{
        ApproveAndExecuteIssueTransition, ApproveAndExecuteLowerIssueClassification, CreateIssue,
        IssueDetails, IssueEvidenceAuthorityError, IssueEvidenceAuthorityPort,
        IssueExecutionPolicy, IssueExecutionPolicyPort, IssueOperationContext, IssueTitle,
        PrepareLowerIssueClassification, PrepareResolveIssue, RecordedIssueClassification,
    },
    risks::{
        AllowRiskEvidence, ApproveAndExecuteRecordRiskOccurrence, CreateRisk,
        PrepareRecordRiskOccurrence, RecordedRiskClassification, RiskDetails, RiskExecutionPolicy,
        RiskExecutionPolicyPort, RiskOperationContext, RiskTitle,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceOrJudgment,
        EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification, IntegrityDigest,
        IssueResolutionType, IssueState, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, WorkManagementRationale,
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
            "pmc-synthetic-issue-h2a-snapshot-{nonce}-{sequence}.sqlite3"
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
        let metadata = resolution_evidence();
        if metadata.id() == id {
            Ok(metadata)
        } else {
            Err(IssueEvidenceAuthorityError::NotFound)
        }
    }
}

fn issue_context(idempotency: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(format!("{idempotency}-correlation")).unwrap(),
    }
}

fn risk_context(idempotency: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(format!("{idempotency}-correlation")).unwrap(),
    }
}

fn operation_error<T: std::fmt::Debug>(
    result: Result<T, LedgerTransactionError<pmc_domain::error::DomainError>>,
) -> pmc_domain::error::DomainError {
    match result {
        Err(LedgerTransactionError::Operation(error)) => error,
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

fn resolution_evidence() -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("snapshot-resolution-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    )
}

fn seed_resolution_evidence(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('snapshot-resolution-evidence','evidence_reference',1,'internal',0,0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES ('snapshot-resolution-evidence','issue_resolution','verified',?1,0)",
            [&"a".repeat(64)],
        )
        .unwrap();
}

fn create_issue(handle: &mut SqliteProductLedger, id: &str, classification: DataClassification) {
    handle
        .create_issue(
            CreateIssue {
                id: IssueId::parse(id).unwrap(),
                title: IssueTitle::parse(format!("Synthetic issue {id}")).unwrap(),
                details: IssueDetails::parse("Synthetic only; no organizational data.").unwrap(),
                classification,
                recurrence_of: None,
                context: issue_context(&format!("{id}-create")),
            },
            AuditEventId::parse(format!("{id}-create-audit")).unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
}

/// Lowers `issue-lowered` to Internal: version 2, state and both resolution
/// columns exactly as created -- the condition that used to make the Issue
/// H1 snapshot, and with it every Risk H2a operation, unavailable.
fn lower_classification(handle: &mut SqliteProductLedger) {
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("snapshot-lower-prepared").unwrap(),
        WorkManagementOperation::LowerIssueClassification {
            issue_id: IssueId::parse("issue-lowered").unwrap(),
            issue_version: AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Internal,
            rationale: WorkManagementRationale::parse("Synthetic lowering rationale").unwrap(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    handle
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("issue-lowered").unwrap(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale: WorkManagementRationale::parse("Synthetic lowering rationale").unwrap(),
                context: issue_context("snapshot-lower-prepare"),
            },
            prepared.clone(),
        )
        .unwrap();
    handle
        .approve_and_execute_lower_issue_classification(
            ApproveAndExecuteLowerIssueClassification {
                approval: approval(&prepared, "snapshot-lower-execute"),
                context: issue_context("snapshot-lower-execute"),
            },
            AuditEventId::parse("snapshot-lower-audit").unwrap(),
            ApprovalReceiptId::parse("snapshot-lower-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            Gate,
            Gate,
            Gate,
            RecordedIssueClassification,
        )
        .unwrap();
}

/// Resolves `issue-resolved` through its own H2a loop.
fn resolve(handle: &mut SqliteProductLedger) {
    let support = EvidenceOrJudgment::new(vec![resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic resolution rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("snapshot-resolve-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: IssueId::parse("issue-resolved").unwrap(),
            issue_version: AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    handle
        .prepare_resolve_issue(
            PrepareResolveIssue {
                issue_id: IssueId::parse("issue-resolved").unwrap(),
                expected_version: AggregateVersion::initial(),
                resolution_type: IssueResolutionType::Resolved,
                rationale,
                evidence_ids: vec![
                    EvidenceReferenceId::parse("snapshot-resolution-evidence").unwrap()
                ],
                judgment: None,
                context: issue_context("snapshot-resolve-prepare"),
            },
            prepared.clone(),
        )
        .unwrap();
    handle
        .approve_and_execute_resolve_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval(&prepared, "snapshot-resolve-execute"),
                context: issue_context("snapshot-resolve-execute"),
            },
            AuditEventId::parse("snapshot-resolve-audit").unwrap(),
            ApprovalReceiptId::parse("snapshot-resolve-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
            Gate,
            Gate,
            Gate,
            RecordedIssueClassification,
        )
        .unwrap();
}

fn approval(prepared: &WorkManagementPreparedIntent, idempotency: &str) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(idempotency).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

fn occurrence_preview(prepared_id: &str, issue_id: &str) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: RiskId::parse("snapshot-risk").unwrap(),
            risk_version: AggregateVersion::initial(),
            issue_id: IssueId::parse(issue_id).unwrap(),
            issue_classification: DataClassification::Internal,
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .unwrap()
}

fn execute_occurrence(
    handle: &mut SqliteProductLedger,
    prepared: &WorkManagementPreparedIntent,
    idempotency: &str,
    at: i64,
) -> Result<
    pmc_domain::risks::OccurredRiskOutcome,
    LedgerTransactionError<pmc_domain::error::DomainError>,
> {
    handle.approve_and_execute_record_risk_occurrence(
        ApproveAndExecuteRecordRiskOccurrence {
            approval: approval(prepared, idempotency),
            context: risk_context(idempotency),
        },
        [
            AuditEventId::parse(format!("{idempotency}-audit-0")).unwrap(),
            AuditEventId::parse(format!("{idempotency}-audit-1")).unwrap(),
            AuditEventId::parse(format!("{idempotency}-audit-2")).unwrap(),
        ],
        ApprovalReceiptId::parse(format!("{idempotency}-receipt")).unwrap(),
        UtcTimestamp::from_unix_millis(at),
        Gate,
        Gate,
        AllowRiskEvidence,
        RecordedRiskClassification,
    )
}

/// A Ledger holding one Risk and three standalone Issues: pristine open,
/// classification-lowered open, and resolved.
fn ledger_with_mixed_issues() -> (SyntheticLedger, SqliteProductLedger) {
    let ledger = SyntheticLedger::new();
    let mut handle = SqliteProductLedger::open(&ledger.0).unwrap();
    handle
        .create_risk(
            CreateRisk {
                id: RiskId::parse("snapshot-risk").unwrap(),
                title: RiskTitle::parse("Synthetic snapshot risk").unwrap(),
                details: RiskDetails::parse("Synthetic only; no organizational data.").unwrap(),
                classification: DataClassification::Internal,
                context: risk_context("snapshot-risk-create"),
            },
            AuditEventId::parse("snapshot-risk-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    create_issue(&mut handle, "issue-pristine", DataClassification::Internal);
    create_issue(
        &mut handle,
        "issue-lowered",
        DataClassification::Confidential,
    );
    create_issue(&mut handle, "issue-resolved", DataClassification::Internal);
    seed_resolution_evidence(&ledger.0);
    lower_classification(&mut handle);
    resolve(&mut handle);
    (ledger, handle)
}

#[test]
fn the_complete_snapshot_carries_every_issue_whatever_state_it_reached() {
    let (ledger, handle) = ledger_with_mixed_issues();
    assert!(
        handle.load_issue_h1_runtime_snapshot().is_err(),
        "the create-state snapshot cannot answer for this Ledger -- the reason the complete one exists"
    );
    drop(handle);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .load_issue_h2a_runtime_snapshot()
        .expect("every Issue must survive a restart, whatever state it reached");
    let record = |id: &str| {
        snapshot
            .records()
            .iter()
            .find(|record| record.id().as_str() == id)
            .unwrap_or_else(|| panic!("{id} must be in the complete snapshot"))
            .clone()
    };

    let pristine = record("issue-pristine");
    assert_eq!(pristine.state(), IssueState::Open);
    assert_eq!(pristine.version(), AggregateVersion::initial());
    assert_eq!(pristine.classification(), DataClassification::Internal);

    // Invisible to the create-state snapshot; present here at version 2.
    let lowered = record("issue-lowered");
    assert_eq!(lowered.state(), IssueState::Open);
    assert_eq!(lowered.version().get(), 2);
    assert_eq!(lowered.classification(), DataClassification::Internal);

    // Invisible to the create-state snapshot; present here with its whole
    // resolution: type, rationale, Evidence, and the support witness that
    // admitted it.
    let resolved = record("issue-resolved");
    assert_eq!(resolved.state(), IssueState::Resolved);
    assert_eq!(resolved.version().get(), 2);
    assert_eq!(
        resolved.resolution_type(),
        Some(IssueResolutionType::Resolved)
    );
    assert_eq!(
        resolved
            .resolution_rationale()
            .map(|r| r.as_str().to_owned()),
        Some("Synthetic resolution rationale".to_owned())
    );
    assert_eq!(
        resolved.resolution_evidence(),
        &[EvidenceReferenceId::parse("snapshot-resolution-evidence").unwrap()]
    );
    assert_eq!(resolved.support_history().len(), 1);
    assert!(resolved.closure_verification_evidence().is_empty());
    assert!(resolved.reopen_rationales().is_empty());

    assert_eq!(snapshot.records().len(), 3);
    assert!(snapshot.prepared().is_empty(), "every preview was consumed");
    assert!(snapshot.rejections().is_empty());
}

#[test]
fn a_risk_occurrence_runs_again_and_still_refuses_an_identity_that_already_exists() {
    let (ledger, mut handle) = ledger_with_mixed_issues();

    // An occurrence naming an Issue identity that already exists is refused
    // by the domain, with nothing written -- including one that only the
    // complete snapshot can see.
    for (existing, idempotency) in [
        ("issue-pristine", "snapshot-collide-pristine"),
        ("issue-lowered", "snapshot-collide-lowered"),
        ("issue-resolved", "snapshot-collide-resolved"),
    ] {
        let prepared = occurrence_preview(&format!("{idempotency}-prepared"), existing);
        handle
            .prepare_record_risk_occurrence(
                PrepareRecordRiskOccurrence {
                    risk_id: RiskId::parse("snapshot-risk").unwrap(),
                    expected_version: AggregateVersion::initial(),
                    issue_id: IssueId::parse(existing).unwrap(),
                    context: risk_context(&format!("{idempotency}-prepare")),
                },
                prepared.clone(),
            )
            .unwrap();
        let refusal = operation_error(execute_occurrence(
            &mut handle,
            &prepared,
            idempotency,
            3_000,
        ));
        assert_ne!(
            refusal.code(),
            ErrorCode::PlatformInternal,
            "{existing}: a duplicate identity is a domain conflict, not a storage failure"
        );
        assert_eq!(
            count(&ledger.0, "SELECT count(*) FROM risk_issue_links"),
            0,
            "{existing}: a refused occurrence links nothing"
        );
    }

    // A fresh identity is admitted, and the Issue it creates joins the
    // snapshot at its own current state.
    let prepared = occurrence_preview("snapshot-fresh-prepared", "issue-from-occurrence");
    handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: RiskId::parse("snapshot-risk").unwrap(),
                expected_version: AggregateVersion::initial(),
                issue_id: IssueId::parse("issue-from-occurrence").unwrap(),
                context: risk_context("snapshot-fresh-prepare"),
            },
            prepared.clone(),
        )
        .unwrap();
    let outcome = execute_occurrence(&mut handle, &prepared, "snapshot-fresh-execute", 4_000)
        .expect("a lowered or resolved standalone Issue must not make occurrence unavailable");
    assert_eq!(outcome.issue.id().as_str(), "issue-from-occurrence");
    drop(handle);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_issue_h2a_runtime_snapshot().unwrap();
    let ids: Vec<String> = snapshot
        .prepared()
        .iter()
        .map(|intent| intent.id().as_str().to_owned())
        .collect();
    assert!(ids.is_empty(), "every preview here was consumed: {ids:?}");
}

fn count(path: &std::path::Path, sql: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(sql, [], |row| row.get(0))
        .unwrap()
}
