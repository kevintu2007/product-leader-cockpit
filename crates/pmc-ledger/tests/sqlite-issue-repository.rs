use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, IssueId, PreparedIntentId,
    },
    issues::{
        ApproveAndExecuteIssueTransition, CreateIssue, IssueDetails, IssueEvidenceAuthorityError,
        IssueEvidenceAuthorityPort, IssueExecutionPolicy, IssueExecutionPolicyPort,
        IssueOperationContext, IssueTitle, PrepareCloseIssue, PrepareReopenIssue,
        PrepareResolveIssue, RecordedIssueClassification,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, EvidenceOrJudgment,
        EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification, HumanJudgment,
        HumanJudgmentDisposition, IntegrityDigest, IssueResolutionType, IssueState,
        SupportDisposition, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPreparedIntent, WorkManagementRationale,
    },
};
use pmc_ledger::sqlite::SqliteProductLedger;

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

#[derive(Clone, Default)]
struct FixedIssueEvidence(
    std::collections::HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>,
);
impl FixedIssueEvidence {
    fn with(mut self, metadata: EvidenceReferenceMetadata) -> Self {
        self.0.insert(metadata.id().clone(), metadata);
        self
    }
}
impl IssueEvidenceAuthorityPort for FixedIssueEvidence {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        self.0
            .get(id)
            .cloned()
            .ok_or(IssueEvidenceAuthorityError::NotFound)
    }
}

fn approval_for(
    prepared: &WorkManagementPreparedIntent,
    idempotency_id: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse(idempotency_id).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
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
            "pmc-synthetic-issue-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn command() -> CreateIssue {
    CreateIssue {
        id: IssueId::parse("synthetic-issue-1").unwrap(),
        title: IssueTitle::parse("Synthetic independent issue").unwrap(),
        details: IssueDetails::parse("Synthetic only; no organizational data.").unwrap(),
        classification: DataClassification::Internal,
        recurrence_of: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-issue-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-issue-correlation-1").unwrap(),
        },
    }
}

#[test]
fn writer_creates_independent_issue_and_reopens_exactly() {
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    assert_eq!(writer.revision().unwrap(), 0);
    let created = writer
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    assert_eq!(writer.revision().unwrap(), 1);
    assert_eq!(created.record.id().as_str(), "synthetic-issue-1");
    assert_eq!(created.record.version().get(), 1);
    assert!(created.record.source_risk_id().is_none());
    assert!(created.record.recurrence_of().is_none());

    let replay = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("ignored-synthetic-issue-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    assert_eq!(replay, created);

    let mut altered = command();
    altered.details = IssueDetails::parse("Altered synthetic details.").unwrap();
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            altered,
            AuditEventId::parse("altered-synthetic-issue-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .is_err());
}

#[test]
fn exact_replay_rejects_a_tampered_audit_correlation() {
    let ledger = SyntheticLedger::new();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();

    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE audit_events SET correlation_id='tampered-synthetic-correlation' WHERE id='synthetic-issue-audit-1'",
            [],
        )
        .unwrap();
    drop(connection);

    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("ignored-synthetic-issue-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .is_err());
}

#[test]
fn exact_replay_rejects_an_extra_audit_effect() {
    let ledger = SyntheticLedger::new();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES('synthetic-issue-audit-1',1,'issue.created','complete','issue','synthetic-issue-1')",
            [],
        )
        .unwrap();
    drop(connection);

    assert_exact_replay_rejected(&ledger.0);
}

#[test]
fn exact_replay_rejects_an_extra_linked_audit() {
    let ledger = SyntheticLedger::new();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-issue-audit-extra',101,'head_of_products','work_management','issue.created','issue','synthetic-issue-1','synthetic-issue-correlation-1','allowed','not_required','succeeded','complete');
             INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES('synthetic-issue-audit-extra',0,'issue.created','complete','issue','synthetic-issue-1');
             INSERT INTO issue_replay_audits(idempotency_id,audit_event_id,correlation_id) VALUES('synthetic-issue-create-1','synthetic-issue-audit-extra','synthetic-issue-correlation-1');",
        )
        .unwrap();
    drop(connection);

    assert_exact_replay_rejected(&ledger.0);
}

#[test]
fn load_issue_h1_runtime_snapshot_recovers_standalone_issues() {
    let ledger = SyntheticLedger::new();
    let created = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_issue_h1_runtime_snapshot()
        .unwrap();
    assert_eq!(snapshot.len(), 1);
    let expected =
        pmc_domain::issues::IssueH1RuntimeSnapshot::try_new(vec![created.record]).unwrap();
    assert_eq!(snapshot, expected);
}

#[test]
fn load_issue_h1_runtime_snapshot_is_empty_for_a_fresh_ledger() {
    let ledger = SyntheticLedger::new();
    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_issue_h1_runtime_snapshot()
        .unwrap();
    assert!(snapshot.is_empty());
}

fn assert_exact_replay_rejected(path: &std::path::Path) {
    assert!(SqliteProductLedger::open(path)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("ignored-synthetic-issue-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .is_err());
}

fn verified_resolution_evidence() -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap(),
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
/// a support-gated prepare writer's foreign keys require. Evidence creation
/// is owned by a different persistence vertical; these tests only consume it.
fn seed_verified_resolution_evidence(ledger_path: &std::path::Path) {
    let connection = rusqlite::Connection::open(ledger_path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('synthetic-resolution-evidence','evidence_reference',1,'internal',0,0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES ('synthetic-resolution-evidence','issue_resolution','verified',?1,0)",
            [&"a".repeat(64)],
        )
        .unwrap();
}

#[test]
fn prepare_resolve_issue_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-prepare-resolve").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    let support = EvidenceOrJudgment::new(vec![verified_resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic resolution rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-resolve-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareResolveIssue {
        issue_id,
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        resolution_type: IssueResolutionType::Resolved,
        rationale,
        evidence_ids: vec![EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-prepare-resolve-idempotency").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-prepare-resolve-correlation").unwrap(),
        },
    };
    let first = ledger_handle
        .prepare_resolve_issue(prepare_command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = ledger_handle
        .prepare_resolve_issue(prepare_command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_resolve_issue_rejects_a_stale_version() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-prepare-resolve-stale").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    let support = EvidenceOrJudgment::new(vec![verified_resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic resolution rationale").unwrap();
    let stale_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-resolve-stale-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: stale_version,
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareResolveIssue {
        issue_id,
        expected_version: stale_version,
        resolution_type: IssueResolutionType::Resolved,
        rationale,
        evidence_ids: vec![EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-prepare-resolve-stale-idempotency")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-prepare-resolve-stale-correlation")
                .unwrap(),
        },
    };
    assert!(ledger_handle
        .prepare_resolve_issue(prepare_command, prepared)
        .is_err());
}

/// A second call with the exact same idempotency ID, command scalars, and
/// even the same `prepared.id()` must still be rejected as a conflict if the
/// supplied preview's actual support differs from what was durably
/// persisted -- `PrepareResolveIssue.evidence_ids` is never compared by the
/// replay check, so only the payload digest (which embeds the full support)
/// can catch a caller-supplied `prepared` built from different evidence.
#[test]
fn prepare_resolve_issue_rejects_a_same_id_replay_with_a_different_payload_digest() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-prepare-resolve-digest").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    seed_evidence(
        &ledger.0,
        "synthetic-prepare-resolve-digest-other-evidence",
        "issue_resolution",
        'f',
    );
    let other_evidence = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-prepare-resolve-digest-other-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("f".repeat(64)).unwrap(),
        },
    );
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic resolution rationale").unwrap();
    let prepared_id = PreparedIntentId::parse("synthetic-prepare-resolve-digest-prepared").unwrap();
    let support = EvidenceOrJudgment::new(vec![verified_resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let context = IssueOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-prepare-resolve-digest-idempotency")
            .unwrap(),
        correlation_id: CorrelationId::parse("synthetic-prepare-resolve-digest-correlation")
            .unwrap(),
    };
    ledger_handle
        .prepare_resolve_issue(
            PrepareResolveIssue {
                issue_id: issue_id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                resolution_type: IssueResolutionType::Resolved,
                rationale: rationale.clone(),
                evidence_ids: vec![
                    EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()
                ],
                judgment: None,
                context: context.clone(),
            },
            prepared,
        )
        .unwrap();

    // Same prepared ID, same command scalars -- but built from different
    // support (a different evidence item), which `evidence_ids` on the
    // command is never compared against on replay.
    let differing_digest_support = EvidenceOrJudgment::new(vec![other_evidence], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let differing_digest_prepared = WorkManagementPreparedIntent::prepare(
        prepared_id,
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(differing_digest_support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    assert!(ledger_handle
        .prepare_resolve_issue(
            PrepareResolveIssue {
                issue_id,
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                resolution_type: IssueResolutionType::Resolved,
                rationale,
                evidence_ids: vec![EvidenceReferenceId::parse(
                    "synthetic-prepare-resolve-digest-other-evidence"
                )
                .unwrap(),],
                judgment: None,
                context,
            },
            differing_digest_prepared,
        )
        .is_err());
}

fn seed_evidence(ledger_path: &std::path::Path, id: &str, role: &str, digest_byte: char) {
    let connection = rusqlite::Connection::open(ledger_path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'evidence_reference',1,'internal',0,0)",
            [id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES (?1,?2,'verified',?3,0)",
            rusqlite::params![id, role, digest_byte.to_string().repeat(64)],
        )
        .unwrap();
}

/// Forces an existing issue to `resolved` at version 2 via raw SQL. Used only
/// by the `prepare_close_issue`/`prepare_reopen_issue` precondition tests
/// below, which intentionally stay decoupled from the execute writers
/// exercised further down this file.
fn force_issue_resolved(ledger_path: &std::path::Path, issue_id: &str) {
    let connection = rusqlite::Connection::open(ledger_path).unwrap();
    connection
        .execute(
            "UPDATE issues SET state='resolved',resolution_type='resolved',resolution_rationale='Synthetic forced resolution' WHERE id=?1",
            [issue_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE aggregate_registry SET version=2 WHERE id=?1",
            [issue_id],
        )
        .unwrap();
}

#[test]
fn prepare_close_issue_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-prepare-close").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    force_issue_resolved(&ledger.0, "synthetic-issue-1");
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('synthetic-closure-evidence','evidence_reference',1,'internal',0,0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES ('synthetic-closure-evidence','issue_closure_verification','verified',?1,0)",
            [&"b".repeat(64)],
        )
        .unwrap();
    drop(connection);
    let evidence = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-closure-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueClosureVerification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("b".repeat(64)).unwrap(),
        },
    );
    let support = EvidenceOrJudgment::new(vec![evidence], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let expected_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-close-prepared").unwrap(),
        WorkManagementOperation::CloseIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareCloseIssue {
        issue_id,
        expected_version,
        evidence_ids: vec![EvidenceReferenceId::parse("synthetic-closure-evidence").unwrap()],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-prepare-close-idempotency").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-prepare-close-correlation").unwrap(),
        },
    };
    let first = ledger_handle
        .prepare_close_issue(prepare_command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = ledger_handle
        .prepare_close_issue(prepare_command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_reopen_issue_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-prepare-reopen").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    force_issue_resolved(&ledger.0, "synthetic-issue-1");
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('synthetic-failed-verification-evidence','evidence_reference',1,'internal',0,0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES ('synthetic-failed-verification-evidence','issue_failed_verification','verified',?1,0)",
            [&"c".repeat(64)],
        )
        .unwrap();
    drop(connection);
    let evidence = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-failed-verification-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueFailedVerification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("c".repeat(64)).unwrap(),
        },
    );
    let support = EvidenceOrJudgment::new(vec![evidence], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let expected_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic reopen rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-reopen-prepared").unwrap(),
        WorkManagementOperation::ReopenIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareReopenIssue {
        issue_id,
        expected_version,
        rationale,
        evidence_ids: vec![
            EvidenceReferenceId::parse("synthetic-failed-verification-evidence").unwrap(),
        ],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-prepare-reopen-idempotency").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-prepare-reopen-correlation").unwrap(),
        },
    };
    let first = ledger_handle
        .prepare_reopen_issue(prepare_command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = ledger_handle
        .prepare_reopen_issue(prepare_command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

fn unpinned_resolution_evidence() -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueResolution,
        EvidenceVerification::ObservedUnpinned {
            observed_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    )
}

fn issue_judgment(rationale: &str, classification: DataClassification) -> HumanJudgment {
    HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        rationale,
        classification,
    )
    .unwrap()
}

fn judged_resolve_preview(id: &str, judgment: HumanJudgment) -> WorkManagementPreparedIntent {
    let support = EvidenceOrJudgment::new(vec![unpinned_resolution_evidence()], vec![judgment])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(id).unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: IssueId::parse("synthetic-issue-1").unwrap(),
            issue_version: AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: WorkManagementRationale::parse("Synthetic resolution rationale").unwrap(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap()
}

fn judged_resolve_command(key: &str, judgment: Option<HumanJudgment>) -> PrepareResolveIssue {
    PrepareResolveIssue {
        issue_id: IssueId::parse("synthetic-issue-1").unwrap(),
        expected_version: AggregateVersion::initial(),
        resolution_type: IssueResolutionType::Resolved,
        rationale: WorkManagementRationale::parse("Synthetic resolution rationale").unwrap(),
        evidence_ids: vec![EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()],
        judgment,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse(key).unwrap(),
            correlation_id: CorrelationId::parse(format!("{key}-correlation")).unwrap(),
        },
    }
}

/// Issue transitions are Evidence-required: a preview whose support stands
/// on a Judgment alone cannot even be constructed, so no caller can hand one
/// to the repository.
#[test]
fn a_judgment_only_issue_preview_cannot_be_prepared() {
    let written = issue_judgment(
        "Synthetic: no Evidence, only words.",
        DataClassification::Internal,
    );
    let support = EvidenceOrJudgment::new(Vec::new(), vec![written])
        .unwrap()
        .evaluate_evidence_or_judgment()
        .unwrap();
    assert_eq!(support.disposition(), SupportDisposition::JudgmentSatisfied);
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-judgment-only-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: IssueId::parse("synthetic-issue-1").unwrap(),
            issue_version: AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: WorkManagementRationale::parse("Synthetic resolution rationale").unwrap(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    );
    assert!(prepared.is_err());
}
/// A resolve prepared on partly verified Evidence with a written Judgment:
/// the Judgment is persisted, survives a restart exactly (so the digest
/// re-derives), binds replay and topology, executes, and stays reachable
/// from the Issue's support history afterwards.
#[test]
fn an_issue_judgment_persists_replays_binds_and_executes_after_restart() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-judged").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    let written = issue_judgment(
        "Synthetic: read but not pinned; the owner accepts it.",
        DataClassification::Confidential,
    );
    let prepared = judged_resolve_preview("synthetic-judged-prepared", written.clone());
    assert_eq!(
        prepared.preview().support().unwrap().disposition(),
        SupportDisposition::VerificationPending
    );
    assert_eq!(prepared.classification(), DataClassification::Confidential);
    let key = "synthetic-judged-prepare";
    ledger_handle
        .prepare_resolve_issue(
            judged_resolve_command(key, Some(written.clone())),
            prepared.clone(),
        )
        .unwrap();
    drop(ledger_handle);

    // Restart: the outstanding preview decodes with its Judgment intact.
    let mut restarted = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = restarted.load_issue_h2a_runtime_snapshot().unwrap();
    let reloaded = snapshot
        .prepared()
        .iter()
        .find(|intent| intent.id() == prepared.id())
        .expect("the preview survives the restart");
    assert_eq!(reloaded, &prepared);
    assert_eq!(
        reloaded.preview().support().unwrap().judgments(),
        &[written.clone()]
    );

    // Exact replay returns the preview; the same key with a different
    // Judgment (or none) is a conflict even when the preview itself matches.
    assert_eq!(
        restarted
            .prepare_resolve_issue(
                judged_resolve_command(key, Some(written.clone())),
                prepared.clone()
            )
            .unwrap(),
        prepared
    );
    for changed in [
        Some(issue_judgment(
            "Synthetic: a different reason.",
            DataClassification::Confidential,
        )),
        Some(issue_judgment(
            "Synthetic: read but not pinned; the owner accepts it.",
            DataClassification::Internal,
        )),
        None,
    ] {
        assert!(restarted
            .prepare_resolve_issue(judged_resolve_command(key, changed), prepared.clone())
            .is_err());
    }
    // A fresh request whose command Judgment differs from the preview's
    // support is refused before anything is stored.
    let other = judged_resolve_preview(
        "synthetic-judged-other-prepared",
        issue_judgment("Synthetic: other.", DataClassification::Internal),
    );
    assert!(restarted
        .prepare_resolve_issue(
            judged_resolve_command("synthetic-judged-topology", Some(written.clone())),
            other,
        )
        .is_err());
    assert!(restarted
        .issue_prepared_intent_for_client_request(
            &IdempotencyId::parse("synthetic-judged-topology").unwrap()
        )
        .unwrap()
        .is_none());

    // Execute after the restart: the host re-derives the same digest from
    // the stored Evidence and Judgment.
    let outcome = restarted
        .approve_and_execute_resolve_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval_for(&prepared, "synthetic-judged-execute"),
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-judged-execute").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-judged-execute-correlation")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-judged-execute-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-judged-execute-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            FixedIssueEvidence::default().with(unpinned_resolution_evidence()),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(outcome.record.state(), IssueState::Resolved);

    // The written basis stays reachable from the Issue's support history.
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    let (rationale, classification): (String, String) = connection
        .query_row(
            "SELECT j.rationale,j.classification FROM issue_support_history h JOIN support_judgments j ON j.support_id=h.support_id WHERE h.issue_id='synthetic-issue-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(rationale, written.rationale());
    assert_eq!(classification, "confidential");
}

/// Resolves an Issue on unpinned Evidence plus a Judgment and returns the
/// executed preview; the Ledger is closed again on return.
fn execute_judged_resolve(ledger_path: &std::path::Path) -> WorkManagementPreparedIntent {
    let mut ledger_handle = SqliteProductLedger::open(ledger_path).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-history").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(ledger_path);
    let written = issue_judgment(
        "Synthetic: read but not pinned; the owner accepts it.",
        DataClassification::Internal,
    );
    let prepared = judged_resolve_preview("synthetic-history-prepared", written.clone());
    ledger_handle
        .prepare_resolve_issue(
            judged_resolve_command("synthetic-history-prepare", Some(written)),
            prepared.clone(),
        )
        .unwrap();
    ledger_handle
        .approve_and_execute_resolve_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval_for(&prepared, "synthetic-history-execute"),
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-history-execute").unwrap(),
                    correlation_id: CorrelationId::parse("synthetic-history-execute-correlation")
                        .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-history-execute-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-history-execute-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            FixedIssueEvidence::default().with(unpinned_resolution_evidence()),
            RecordedIssueClassification,
        )
        .unwrap();
    prepared
}

/// History is what was decided: Evidence that later loses its verification
/// must neither rewrite nor break an executed transition's support.
#[test]
fn issue_history_keeps_the_support_it_executed_on_after_its_evidence_degrades() {
    let ledger = SyntheticLedger::new();
    let prepared = execute_judged_resolve(&ledger.0);
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "UPDATE evidence_references SET verification='unverified',integrity_digest=NULL,last_verified_at=NULL WHERE id='synthetic-resolution-evidence';
             UPDATE aggregate_registry SET version=version+1 WHERE id='synthetic-resolution-evidence';",
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .load_issue_h2a_runtime_snapshot()
        .expect("a degraded Evidence must not break the Issue snapshot");

    let record = snapshot
        .records()
        .iter()
        .find(|record| record.id().as_str() == "synthetic-issue-1")
        .unwrap();
    assert_eq!(
        record.support_history(),
        &[prepared.preview().support().unwrap().clone()]
    );
}

/// Reading history from the execution-time snapshot is still checked: a
/// tampered snapshot row is refused, not trusted.
#[test]
fn a_tampered_issue_history_snapshot_is_refused() {
    let ledger = SyntheticLedger::new();
    execute_judged_resolve(&ledger.0);
    let connection = rusqlite::Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE issue_h2a_support_evidence_snapshots SET verification='unverified',integrity_digest=NULL,last_verified_at=NULL WHERE prepared_intent_id='synthetic-history-prepared'",
            [],
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.load_issue_h2a_runtime_snapshot().is_err());
}

#[test]
fn approve_and_execute_resolve_issue_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-execute-resolve").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    let support = EvidenceOrJudgment::new(vec![verified_resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic resolution rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-resolve-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareResolveIssue {
        issue_id,
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        resolution_type: IssueResolutionType::Resolved,
        rationale,
        evidence_ids: vec![EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-execute-resolve-prepare-idempotency")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-execute-resolve-prepare-correlation")
                .unwrap(),
        },
    };
    ledger_handle
        .prepare_resolve_issue(prepare_command, prepared.clone())
        .unwrap();

    let approval = approval_for(&prepared, "synthetic-execute-resolve-execute");
    let evidence_port = FixedIssueEvidence::default().with(verified_resolution_evidence());
    let transition = ApproveAndExecuteIssueTransition {
        approval,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-execute-resolve-execute").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-execute-resolve-execute-correlation")
                .unwrap(),
        },
    };
    let outcome = ledger_handle
        .approve_and_execute_resolve_issue(
            transition.clone(),
            AuditEventId::parse("synthetic-execute-resolve-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-execute-resolve-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port.clone(),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(outcome.record.state(), IssueState::Resolved);
    assert_eq!(outcome.record.version().get(), 2);
    assert_eq!(
        outcome.record.resolution_evidence(),
        &[EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()]
    );

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replay = reopened
        .approve_and_execute_resolve_issue(
            transition,
            AuditEventId::parse("ignored-synthetic-execute-resolve-audit").unwrap(),
            ApprovalReceiptId::parse("ignored-synthetic-execute-resolve-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
}

/// A replay must return the persisted audit's own correlation ID, not the
/// one the replaying caller happens to supply -- the domain's own
/// idempotency signature never included correlation ID as part of "the same
/// call" (Risk and Issue both key execute replay on prepared ID, actor, and
/// acknowledged digest only), so a differing correlation ID here is a
/// legitimate replay, and the returned `AuditEvent` must reflect what
/// actually happened, not the new call's framing.
#[test]
fn approve_and_execute_resolve_issue_replay_returns_the_original_correlation_id() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-execute-resolve-corr").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    seed_verified_resolution_evidence(&ledger.0);
    let support = EvidenceOrJudgment::new(vec![verified_resolution_evidence()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic resolution rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-resolve-corr-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareResolveIssue {
        issue_id,
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        resolution_type: IssueResolutionType::Resolved,
        rationale,
        evidence_ids: vec![EvidenceReferenceId::parse("synthetic-resolution-evidence").unwrap()],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse(
                "synthetic-execute-resolve-corr-prepare-idempotency",
            )
            .unwrap(),
            correlation_id: CorrelationId::parse(
                "synthetic-execute-resolve-corr-prepare-correlation",
            )
            .unwrap(),
        },
    };
    ledger_handle
        .prepare_resolve_issue(prepare_command, prepared.clone())
        .unwrap();

    let approval = approval_for(&prepared, "synthetic-execute-resolve-corr-execute");
    let evidence_port = FixedIssueEvidence::default().with(verified_resolution_evidence());
    let original_context = IssueOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-execute-resolve-corr-execute").unwrap(),
        correlation_id: CorrelationId::parse("synthetic-execute-resolve-corr-original").unwrap(),
    };
    let outcome = ledger_handle
        .approve_and_execute_resolve_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval.clone(),
                context: original_context.clone(),
            },
            AuditEventId::parse("synthetic-execute-resolve-corr-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-execute-resolve-corr-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port.clone(),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(
        outcome.audit_events[0].correlation_id(),
        &original_context.correlation_id
    );

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replay_with_different_correlation = ApproveAndExecuteIssueTransition {
        approval,
        context: IssueOperationContext {
            idempotency_id: original_context.idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-execute-resolve-corr-different")
                .unwrap(),
        },
    };
    let replay = reopened
        .approve_and_execute_resolve_issue(
            replay_with_different_correlation,
            AuditEventId::parse("ignored-synthetic-execute-resolve-corr-audit").unwrap(),
            ApprovalReceiptId::parse("ignored-synthetic-execute-resolve-corr-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
    assert_eq!(
        replay.audit_events[0].correlation_id(),
        &original_context.correlation_id
    );
}

#[test]
fn approve_and_execute_close_issue_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-execute-close").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    force_issue_resolved(&ledger.0, "synthetic-issue-1");
    seed_evidence(
        &ledger.0,
        "synthetic-execute-closure-evidence",
        "issue_closure_verification",
        'd',
    );
    let evidence_metadata = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-execute-closure-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueClosureVerification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("d".repeat(64)).unwrap(),
        },
    );
    let support = EvidenceOrJudgment::new(vec![evidence_metadata.clone()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let expected_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-close-prepared").unwrap(),
        WorkManagementOperation::CloseIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareCloseIssue {
        issue_id,
        expected_version,
        evidence_ids: vec![
            EvidenceReferenceId::parse("synthetic-execute-closure-evidence").unwrap(),
        ],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-execute-close-prepare-idempotency")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-execute-close-prepare-correlation")
                .unwrap(),
        },
    };
    ledger_handle
        .prepare_close_issue(prepare_command, prepared.clone())
        .unwrap();

    let approval = approval_for(&prepared, "synthetic-execute-close-execute");
    let evidence_port = FixedIssueEvidence::default().with(evidence_metadata);
    let transition = ApproveAndExecuteIssueTransition {
        approval,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-execute-close-execute").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-execute-close-execute-correlation")
                .unwrap(),
        },
    };
    let outcome = ledger_handle
        .approve_and_execute_close_issue(
            transition.clone(),
            AuditEventId::parse("synthetic-execute-close-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-execute-close-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(300),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port.clone(),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(outcome.record.state(), IssueState::Closed);
    assert_eq!(outcome.record.version().get(), 3);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replay = reopened
        .approve_and_execute_close_issue(
            transition,
            AuditEventId::parse("ignored-synthetic-execute-close-audit").unwrap(),
            ApprovalReceiptId::parse("ignored-synthetic-execute-close-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
}

#[test]
fn approve_and_execute_reopen_issue_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-issue-audit-execute-reopen").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    force_issue_resolved(&ledger.0, "synthetic-issue-1");
    seed_evidence(
        &ledger.0,
        "synthetic-execute-failed-verification-evidence",
        "issue_failed_verification",
        'e',
    );
    let evidence_metadata = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-execute-failed-verification-evidence").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueFailedVerification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("e".repeat(64)).unwrap(),
        },
    );
    let support = EvidenceOrJudgment::new(vec![evidence_metadata.clone()], vec![])
        .unwrap()
        .evaluate_evidence_required()
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();
    let expected_version = pmc_domain::identity::AggregateVersion::new(2).unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic reopen rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-reopen-prepared").unwrap(),
        WorkManagementOperation::ReopenIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        Some(support),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareReopenIssue {
        issue_id,
        expected_version,
        rationale,
        evidence_ids: vec![EvidenceReferenceId::parse(
            "synthetic-execute-failed-verification-evidence",
        )
        .unwrap()],
        judgment: None,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-execute-reopen-prepare-idempotency")
                .unwrap(),
            correlation_id: CorrelationId::parse("synthetic-execute-reopen-prepare-correlation")
                .unwrap(),
        },
    };
    ledger_handle
        .prepare_reopen_issue(prepare_command, prepared.clone())
        .unwrap();

    let approval = approval_for(&prepared, "synthetic-execute-reopen-execute");
    let evidence_port = FixedIssueEvidence::default().with(evidence_metadata);
    let transition = ApproveAndExecuteIssueTransition {
        approval,
        context: IssueOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-execute-reopen-execute").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-execute-reopen-execute-correlation")
                .unwrap(),
        },
    };
    let outcome = ledger_handle
        .approve_and_execute_reopen_issue(
            transition.clone(),
            AuditEventId::parse("synthetic-execute-reopen-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-execute-reopen-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(300),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port.clone(),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(outcome.record.state(), IssueState::Open);
    assert_eq!(outcome.record.version().get(), 3);
    assert_eq!(
        outcome.record.failed_verification_evidence(),
        &[EvidenceReferenceId::parse("synthetic-execute-failed-verification-evidence").unwrap()]
    );
    assert_eq!(outcome.record.reopen_rationales().len(), 1);
    assert_eq!(outcome.record.support_history().len(), 1);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let replay = reopened
        .approve_and_execute_reopen_issue(
            transition,
            AuditEventId::parse("ignored-synthetic-execute-reopen-audit").unwrap(),
            ApprovalReceiptId::parse("ignored-synthetic-execute-reopen-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            AllowIssuePolicy,
            evidence_port,
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
}

/// Drives resolve -> reopen -> resolve -> close, reopening a fresh
/// `SqliteProductLedger` before every step. Each execute call must
/// rehydrate the Issue's full accumulated history (resolution attributes
/// overwritten across the two resolves, failed-verification evidence and
/// reopen rationale accumulated across the reopen, and a growing support
/// history) purely from durable state -- proving `decode_issue_h2a_record`
/// reconstructs a cycling Issue correctly, not just a fresh one.
#[test]
fn full_h2a_lifecycle_survives_restart_at_every_step_with_accumulated_history() {
    let ledger = SyntheticLedger::new();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_issue(
            command(),
            AuditEventId::parse("synthetic-cycle-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let issue_id = IssueId::parse("synthetic-issue-1").unwrap();

    seed_evidence(
        &ledger.0,
        "synthetic-cycle-resolution-1",
        "issue_resolution",
        '1',
    );
    let resolution_evidence_1 = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-cycle-resolution-1").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("1".repeat(64)).unwrap(),
        },
    );
    let rationale_1 = WorkManagementRationale::parse("Synthetic first resolution").unwrap();
    let prepared_resolve_1 = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-cycle-resolve-1-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::initial(),
            resolution_type: IssueResolutionType::Workaround,
            rationale: rationale_1.clone(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(vec![resolution_evidence_1.clone()], vec![])
                .unwrap()
                .evaluate_evidence_required()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .prepare_resolve_issue(
            PrepareResolveIssue {
                issue_id: issue_id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                resolution_type: IssueResolutionType::Workaround,
                rationale: rationale_1,
                evidence_ids: vec![
                    EvidenceReferenceId::parse("synthetic-cycle-resolution-1").unwrap()
                ],
                judgment: None,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-prepare-resolve-1")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-prepare-resolve-1-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared_resolve_1.clone(),
        )
        .unwrap();
    let after_resolve_1 = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .approve_and_execute_resolve_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval_for(&prepared_resolve_1, "synthetic-cycle-execute-resolve-1"),
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-execute-resolve-1")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-execute-resolve-1-correlation",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-cycle-execute-resolve-1-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-cycle-execute-resolve-1-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(110),
            AllowApproval,
            AllowIssuePolicy,
            FixedIssueEvidence::default().with(resolution_evidence_1),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(after_resolve_1.record.state(), IssueState::Resolved);
    assert_eq!(after_resolve_1.record.version().get(), 2);

    seed_evidence(
        &ledger.0,
        "synthetic-cycle-failed-verification-1",
        "issue_failed_verification",
        '2',
    );
    let failed_verification_evidence_1 = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-cycle-failed-verification-1").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueFailedVerification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("2".repeat(64)).unwrap(),
        },
    );
    let reopen_rationale_1 = WorkManagementRationale::parse("Synthetic reopen rationale").unwrap();
    let prepared_reopen_1 = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-cycle-reopen-1-prepared").unwrap(),
        WorkManagementOperation::ReopenIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::new(2).unwrap(),
            rationale: reopen_rationale_1.clone(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(vec![failed_verification_evidence_1.clone()], vec![])
                .unwrap()
                .evaluate_evidence_required()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .prepare_reopen_issue(
            PrepareReopenIssue {
                issue_id: issue_id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::new(2).unwrap(),
                rationale: reopen_rationale_1,
                evidence_ids: vec![EvidenceReferenceId::parse(
                    "synthetic-cycle-failed-verification-1",
                )
                .unwrap()],
                judgment: None,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-prepare-reopen-1")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-prepare-reopen-1-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared_reopen_1.clone(),
        )
        .unwrap();
    let after_reopen_1 = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .approve_and_execute_reopen_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval_for(&prepared_reopen_1, "synthetic-cycle-execute-reopen-1"),
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-execute-reopen-1")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-execute-reopen-1-correlation",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-cycle-execute-reopen-1-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-cycle-execute-reopen-1-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(120),
            AllowApproval,
            AllowIssuePolicy,
            FixedIssueEvidence::default().with(failed_verification_evidence_1),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(after_reopen_1.record.state(), IssueState::Open);
    assert_eq!(after_reopen_1.record.version().get(), 3);
    assert_eq!(after_reopen_1.record.reopen_rationales().len(), 1);
    assert_eq!(
        after_reopen_1.record.failed_verification_evidence().len(),
        1
    );
    assert_eq!(after_reopen_1.record.support_history().len(), 2);
    // The first resolve's attributes must survive the reopen untouched.
    assert_eq!(
        after_reopen_1.record.resolution_type(),
        Some(IssueResolutionType::Workaround)
    );
    assert_eq!(after_reopen_1.record.resolution_evidence().len(), 1);

    seed_evidence(
        &ledger.0,
        "synthetic-cycle-resolution-2",
        "issue_resolution",
        '3',
    );
    let resolution_evidence_2 = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-cycle-resolution-2").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueResolution,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("3".repeat(64)).unwrap(),
        },
    );
    let rationale_2 = WorkManagementRationale::parse("Synthetic second resolution").unwrap();
    let prepared_resolve_2 = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-cycle-resolve-2-prepared").unwrap(),
        WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::new(3).unwrap(),
            resolution_type: IssueResolutionType::Resolved,
            rationale: rationale_2.clone(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(vec![resolution_evidence_2.clone()], vec![])
                .unwrap()
                .evaluate_evidence_required()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .prepare_resolve_issue(
            PrepareResolveIssue {
                issue_id: issue_id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::new(3).unwrap(),
                resolution_type: IssueResolutionType::Resolved,
                rationale: rationale_2,
                evidence_ids: vec![
                    EvidenceReferenceId::parse("synthetic-cycle-resolution-2").unwrap()
                ],
                judgment: None,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-prepare-resolve-2")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-prepare-resolve-2-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared_resolve_2.clone(),
        )
        .unwrap();
    let after_resolve_2 = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .approve_and_execute_resolve_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval_for(&prepared_resolve_2, "synthetic-cycle-execute-resolve-2"),
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-execute-resolve-2")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-execute-resolve-2-correlation",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-cycle-execute-resolve-2-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-cycle-execute-resolve-2-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(130),
            AllowApproval,
            AllowIssuePolicy,
            FixedIssueEvidence::default().with(resolution_evidence_2),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(after_resolve_2.record.state(), IssueState::Resolved);
    assert_eq!(after_resolve_2.record.version().get(), 4);
    // Overwritten, not accumulated: the second resolve replaces the first.
    assert_eq!(
        after_resolve_2.record.resolution_type(),
        Some(IssueResolutionType::Resolved)
    );
    assert_eq!(
        after_resolve_2.record.resolution_evidence(),
        &[EvidenceReferenceId::parse("synthetic-cycle-resolution-2").unwrap()]
    );
    assert_eq!(after_resolve_2.record.reopen_rationales().len(), 1);
    assert_eq!(after_resolve_2.record.support_history().len(), 3);

    seed_evidence(
        &ledger.0,
        "synthetic-cycle-closure-1",
        "issue_closure_verification",
        '4',
    );
    let closure_evidence = EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("synthetic-cycle-closure-1").unwrap(),
        AggregateVersion::initial(),
        DataClassification::Internal,
        EvidenceRole::IssueClosureVerification,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(0),
            integrity_digest: IntegrityDigest::parse("4".repeat(64)).unwrap(),
        },
    );
    let prepared_close = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-cycle-close-prepared").unwrap(),
        WorkManagementOperation::CloseIssue {
            issue_id: issue_id.clone(),
            issue_version: pmc_domain::identity::AggregateVersion::new(4).unwrap(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(vec![closure_evidence.clone()], vec![])
                .unwrap()
                .evaluate_evidence_required()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .prepare_close_issue(
            PrepareCloseIssue {
                issue_id: issue_id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::new(4).unwrap(),
                evidence_ids: vec![
                    EvidenceReferenceId::parse("synthetic-cycle-closure-1").unwrap(),
                ],
                judgment: None,
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-prepare-close")
                        .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-prepare-close-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared_close.clone(),
        )
        .unwrap();
    let after_close = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .approve_and_execute_close_issue(
            ApproveAndExecuteIssueTransition {
                approval: approval_for(&prepared_close, "synthetic-cycle-execute-close"),
                context: IssueOperationContext {
                    idempotency_id: IdempotencyId::parse("synthetic-cycle-execute-close").unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cycle-execute-close-correlation",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-cycle-execute-close-audit").unwrap(),
            ApprovalReceiptId::parse("synthetic-cycle-execute-close-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(140),
            AllowApproval,
            AllowIssuePolicy,
            FixedIssueEvidence::default().with(closure_evidence),
            RecordedIssueClassification,
        )
        .unwrap();
    assert_eq!(after_close.record.state(), IssueState::Closed);
    assert_eq!(after_close.record.version().get(), 5);
    assert_eq!(after_close.record.reopen_rationales().len(), 1);
    assert_eq!(after_close.record.failed_verification_evidence().len(), 1);
    assert_eq!(after_close.record.support_history().len(), 4);
    assert_eq!(
        after_close.record.closure_verification_evidence(),
        &[EvidenceReferenceId::parse("synthetic-cycle-closure-1").unwrap()]
    );
    // Resolution evidence is overwritten by the second resolve, not
    // accumulated with the first; a rehydrated record after the second
    // resolve (and everything downstream, like this close) must reflect
    // only the current resolve's evidence.
    assert_eq!(
        after_close.record.resolution_evidence(),
        &[EvidenceReferenceId::parse("synthetic-cycle-resolution-2").unwrap()]
    );
}

#[test]
fn create_issue_refuses_recurrence_as_a_validation_error_not_a_storage_failure() {
    // This writer never persists recurrence_of; the refusal must say
    // so as a domain validation error, not as a retryable storage failure.
    let ledger = SyntheticLedger::new();
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let mut recurrence = command();
    recurrence.id = IssueId::parse("synthetic-issue-2").unwrap();
    recurrence.recurrence_of = Some(IssueId::parse("synthetic-issue-1").unwrap());
    recurrence.context.idempotency_id = IdempotencyId::parse("synthetic-issue-create-2").unwrap();

    let result = writer.create_issue(
        recurrence,
        AuditEventId::parse("synthetic-issue-audit-2").unwrap(),
        UtcTimestamp::from_unix_millis(2_000),
    );

    let Err(pmc_ledger::sqlite::LedgerTransactionError::Operation(error)) = result else {
        panic!("expected a domain refusal, got {result:?}");
    };
    assert_eq!(
        error.code(),
        pmc_domain::error::ErrorCode::ValidationInvalidField
    );
    assert_eq!(
        error.message_key().as_str(),
        "issue.recurrence_not_supported"
    );
    assert!(!error.retryable());
    assert_eq!(writer.revision().unwrap(), 0, "nothing was written");
}
