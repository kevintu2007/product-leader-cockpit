//! Action Complete's own SQLite persistence -- the last of the
//! Cancel/Complete/Reopen trio, unblocked only once both of its
//! prerequisites (`StartAction`, `LinkActionCompletionEvidence`) shipped,
//! since `prepare_complete_action_cause` requires `ActionState::InProgress`
//! and non-empty, resolvable `completion_evidence`.
//!
//! Setup mirrors `sqlite-action-cancel-reopen-with-linked-evidence.rs`
//! (Accept -> Start -> Link), then exercises Complete's PREPARE/EXECUTE on
//! top. Evidence fixtures carry COMPLETE, valid verification data (not the
//! shortcut `sqlite-action-link-completion-evidence.rs`'s own fixture uses)
//! since `EvidenceOrJudgment::evaluate_evidence_required()` -- which
//! Complete's PREPARE always calls -- inspects verification strictly.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    actions::{
        ActionDetails, ActionExecutionPolicy, ActionExecutionPolicyPort, ActionOperationContext,
        ActionTitle, ApproveAndExecuteAcceptActionRequest, ApproveAndExecuteCompleteAction,
        CreateActionRequestDraft, LinkActionCompletionEvidence, PrepareAcceptActionRequest,
        PrepareCompleteAction, StartAction, SubmitActionRequest,
    },
    audit::AuditActor,
    classification::DataClassification,
    error::ErrorCode,
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, EvidenceReferenceId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, HumanJudgment, HumanJudgmentDisposition,
        WorkManagementApproval, WorkManagementOperation, WorkManagementPreparedIntent,
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
            "pmc-synthetic-action-complete-{nonce}-{sequence}.sqlite3"
        )))
    }
}

#[derive(Clone, Copy)]
struct H2aPolicy(ActionExecutionPolicy);

impl ApprovalAuthorizationPort for H2aPolicy {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

impl ActionExecutionPolicyPort for H2aPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        self.0
    }
}

fn create_command(idempotency: &str, correlation: &str, request: &str) -> CreateActionRequestDraft {
    CreateActionRequestDraft {
        id: ActionRequestId::parse(request).unwrap(),
        title: ActionTitle::parse("Synthetic Complete request").unwrap(),
        details: ActionDetails::parse("Persist an Action through the durable writer.").unwrap(),
        intended_owner: Some(StakeholderId::parse("stakeholder-owner-1").unwrap()),
        response_due_at: Some(UtcTimestamp::from_unix_millis(200_000)),
        intended_action_due_at: Some(UtcTimestamp::from_unix_millis(300_000)),
        classification: DataClassification::Internal,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn submit_command(idempotency: &str, correlation: &str, request: &str) -> SubmitActionRequest {
    SubmitActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::initial(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn prepare_accept_command(
    idempotency: &str,
    correlation: &str,
    request: &str,
) -> PrepareAcceptActionRequest {
    PrepareAcceptActionRequest {
        request_id: ActionRequestId::parse(request).unwrap(),
        expected_version: AggregateVersion::initial().next().unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn prepared_accept_fixture(
    request: &str,
    prepared_id: &str,
    action_id: &str,
) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::AcceptActionRequest {
            request_id: ActionRequestId::parse(request).unwrap(),
            request_version: AggregateVersion::initial().next().unwrap(),
            action_id: ActionId::parse(action_id).unwrap(),
            action_classification: DataClassification::Internal,
            action_subject: ActionTitle::parse("Synthetic Complete request").unwrap(),
            commitment_details: ActionDetails::parse(
                "Persist an Action through the durable writer.",
            )
            .unwrap(),
            intended_owner: StakeholderId::parse("stakeholder-owner-1").unwrap(),
            intended_due_at: UtcTimestamp::from_unix_millis(300_000),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

fn start_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
) -> StartAction {
    StartAction {
        action_id: action_id.clone(),
        expected_version,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn link_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
    evidence_id: &str,
) -> LinkActionCompletionEvidence {
    LinkActionCompletionEvidence {
        action_id: action_id.clone(),
        expected_version,
        evidence_id: EvidenceReferenceId::parse(evidence_id).unwrap(),
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn complete_command(
    idempotency: &str,
    correlation: &str,
    action_id: &ActionId,
    expected_version: AggregateVersion,
    judgment: Option<HumanJudgment>,
) -> PrepareCompleteAction {
    PrepareCompleteAction {
        action_id: action_id.clone(),
        expected_version,
        judgment,
        context: ActionOperationContext {
            idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
            correlation_id: CorrelationId::parse(correlation).unwrap(),
        },
    }
}

fn approval_for(
    prepared_id: &str,
    payload_digest: &pmc_domain::work_management::WorkManagementPayloadDigest,
    idempotency_id: &str,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        PreparedIntentId::parse(prepared_id).unwrap(),
        AuditActor::HeadOfProducts,
        payload_digest.clone(),
        IdempotencyId::parse(idempotency_id).unwrap(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

/// Seeds one raw, truthful Evidence Reference row -- carries a complete
/// `verification` shape (`last_verified_at`+`integrity_digest` for
/// `'verified'`), since `EvidenceOrJudgment::evaluate_evidence_required()`
/// parses it strictly.
fn seed_evidence_reference(
    path: &std::path::Path,
    id: &str,
    classification: &str,
    verification: &str,
) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'evidence_reference',1,?2,500,500)",
            rusqlite::params![id, classification],
        )
        .unwrap();
    match verification {
        "verified" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES (?1,NULL,'verified',?2,500)",
                    rusqlite::params![id, "a".repeat(64)],
                )
                .unwrap();
        }
        "degraded_last_verified" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES (?1,NULL,'degraded_last_verified',?2,500)",
                    rusqlite::params![id, "b".repeat(64)],
                )
                .unwrap();
        }
        "unverified" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification) VALUES (?1,NULL,'unverified')",
                    [id],
                )
                .unwrap();
        }
        "observed_unpinned" => {
            connection
                .execute(
                    "INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at) VALUES (?1,NULL,'observed_unpinned',?2,500)",
                    rusqlite::params![id, "c".repeat(64)],
                )
                .unwrap();
        }
        other => panic!("unsupported synthetic verification fixture: {other}"),
    }
}

/// Seeds a ledger through H1 create+submit, H2a accept, `StartAction`, and
/// one `LinkActionCompletionEvidence` (evidence id
/// `synthetic-completion-evidence-1`, classification/verification as given),
/// returning the handle and the resulting InProgress Action's id (always
/// `action-1`, version 3: accept=1, start=2, link=3).
fn seeded_ledger_with_in_progress_action_and_linked_evidence(
    evidence_classification: &str,
    evidence_verification: &str,
) -> (SyntheticLedger, SqliteProductLedger, ActionId) {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('stakeholder-owner-1','stakeholder',1,'internal',0,0); INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES ('stakeholder-owner-1','Synthetic Product Owner','person','synthetic_fixture','public-safe-fixture');",
    ).unwrap();
    drop(connection);
    seed_evidence_reference(
        &ledger.0,
        "synthetic-completion-evidence-1",
        evidence_classification,
        evidence_verification,
    );
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_action_request_draft(
            create_command(
                "action-create-1",
                "action-create-correlation-1",
                "request-1",
            ),
            AuditEventId::parse("action-create-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_action_request(
            submit_command(
                "action-submit-1",
                "action-submit-correlation-1",
                "request-1",
            ),
            AuditEventId::parse("action-submit-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    let prepared = prepared_accept_fixture("request-1", "action-prepared-1", "action-1");
    writer
        .prepare_accept_action_request(
            prepare_accept_command(
                "action-prepare-1",
                "action-prepare-correlation-1",
                "request-1",
            ),
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency = IdempotencyId::parse("action-accept-execute-1").unwrap();
    let outcome = writer
        .approve_and_execute_accept_action_request(
            ApproveAndExecuteAcceptActionRequest {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    execute_idempotency.clone(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: ActionOperationContext {
                    idempotency_id: execute_idempotency,
                    correlation_id: CorrelationId::parse("action-accept-execute-correlation-1")
                        .unwrap(),
                },
            },
            [
                AuditEventId::parse("action-accept-request-audit-1").unwrap(),
                AuditEventId::parse("action-accept-action-audit-1").unwrap(),
                AuditEventId::parse("action-accept-link-audit-1").unwrap(),
            ],
            ApprovalReceiptId::parse("action-accept-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    let action_id = outcome.action.id().clone();
    writer
        .start_action(
            start_command(
                "action-start-1",
                "action-start-correlation-1",
                &action_id,
                AggregateVersion::initial(),
            ),
            AuditEventId::parse("action-start-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_150),
        )
        .unwrap();
    writer
        .link_action_completion_evidence(
            link_command(
                "action-link-1",
                "action-link-correlation-1",
                &action_id,
                AggregateVersion::initial().next().unwrap(),
                "synthetic-completion-evidence-1",
            ),
            AuditEventId::parse("action-link-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_200),
        )
        .unwrap();
    (ledger, writer, action_id)
}

/// The core happy path: Complete succeeds (PREPARE+EXECUTE) on an
/// InProgress Action with one Verified linked completion evidence
/// reference, and the resulting Completed record -- including its
/// `SupportWitness` -- survives a full ledger restart and reload.
#[test]
fn complete_succeeds_with_verified_linked_completion_evidence() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "verified");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let prepared = writer
        .prepare_complete_action(
            complete_command(
                "action-complete-prepare-1",
                "action-complete-prepare-correlation-1",
                &action_id,
                version_at_link,
                None,
            ),
            PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect("Complete PREPARE must succeed with one Verified linked evidence reference");

    let approval = approval_for(
        "action-complete-prepared-1",
        prepared.payload_digest(),
        "action-complete-execute-1",
    );
    let outcome = writer
        .approve_and_execute_complete_action(
            ApproveAndExecuteCompleteAction {
                approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-complete-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-complete-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-complete-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-complete-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Complete EXECUTE must succeed with one Verified linked evidence reference");
    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::ActionState::Completed
    );
    assert!(outcome.record.support().is_some());
    assert_eq!(
        outcome.record.support().unwrap().evidence().len(),
        1,
        "the completed record's witness must carry the one linked evidence reference"
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().contains(&outcome.record));
}

/// Complete succeeds with a judgment backing a `DegradedLastVerified`
/// linked evidence reference -- `evaluate_evidence_required()` accepts this
/// combination (unlike bare `Unverified`/`IntegrityMismatch`, which it
/// always rejects regardless of judgment).
#[test]
fn complete_succeeds_with_degraded_evidence_backed_by_a_judgment() {
    let (ledger, mut writer, action_id) = seeded_ledger_with_in_progress_action_and_linked_evidence(
        "internal",
        "degraded_last_verified",
    );
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();
    let judgment = HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Manually reviewed; evidence integrity re-check pending but content confirmed sound.",
        DataClassification::Internal,
    )
    .unwrap();

    let prepared = writer
        .prepare_complete_action(
            complete_command(
                "action-complete-prepare-1",
                "action-complete-prepare-correlation-1",
                &action_id,
                version_at_link,
                Some(judgment),
            ),
            PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect("Complete PREPARE must succeed: degraded evidence backed by a judgment");

    let approval = approval_for(
        "action-complete-prepared-1",
        prepared.payload_digest(),
        "action-complete-execute-1",
    );
    let outcome = writer
        .approve_and_execute_complete_action(
            ApproveAndExecuteCompleteAction {
                approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-complete-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-complete-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-complete-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-complete-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Complete EXECUTE must succeed: degraded evidence backed by a judgment");
    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::ActionState::Completed
    );
    let support = outcome
        .record
        .support()
        .expect("a witness must be recorded");
    assert_eq!(support.judgments().len(), 1);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().contains(&outcome.record));
}

/// `ObservedUnpinned` evidence takes the Degraded path end to end
/// through the SQLite writer -- PREPARE snapshots the fifth value into
/// `action_h2a_support_evidence_snapshots`, EXECUTE revalidates it, and the
/// witness records the judgment that carried it.
#[test]
fn complete_succeeds_with_observed_unpinned_evidence_backed_by_a_judgment() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "observed_unpinned");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();
    let judgment = HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Manually reviewed; the reference has no pinned fingerprint yet.",
        DataClassification::Internal,
    )
    .unwrap();

    let prepared = writer
        .prepare_complete_action(
            complete_command(
                "action-complete-prepare-1",
                "action-complete-prepare-correlation-1",
                &action_id,
                version_at_link,
                Some(judgment),
            ),
            PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect("Complete PREPARE must succeed: unpinned evidence backed by a judgment");

    let approval = approval_for(
        "action-complete-prepared-1",
        prepared.payload_digest(),
        "action-complete-execute-1",
    );
    let outcome = writer
        .approve_and_execute_complete_action(
            ApproveAndExecuteCompleteAction {
                approval,
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-complete-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-complete-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-complete-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-complete-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .expect("Complete EXECUTE must succeed: unpinned evidence backed by a judgment");
    assert_eq!(
        outcome.record.state(),
        pmc_domain::work_management::ActionState::Completed
    );
    let support = outcome
        .record
        .support()
        .expect("a witness must be recorded");
    assert_eq!(support.judgments().len(), 1);
    assert!(
        support.evidence().iter().any(|item| matches!(
            item.verification(),
            pmc_domain::work_management::EvidenceVerification::ObservedUnpinned { .. }
        )),
        "the witness must carry the fifth state as observed, not a relabelled one"
    );
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().contains(&outcome.record));
}

/// Without a judgment, an unpinned observation is not enough. This is
/// the user-visible gate flip the product owner accepted: a completion that
/// used to pass on an unpinned reference now waits for a pin or a judgment.
#[test]
fn complete_prepare_denies_when_linked_evidence_is_observed_unpinned_and_unjudged() {
    let (_ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "observed_unpinned");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let result = writer.prepare_complete_action(
        complete_command(
            "action-complete-prepare-1",
            "action-complete-prepare-correlation-1",
            &action_id,
            version_at_link,
            None,
        ),
        PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
    );
    assert!(
        result.is_err(),
        "Complete PREPARE must deny ObservedUnpinned, unjudged evidence: {result:?}"
    );
}

/// H3 denial: `Unverified` evidence with no judgment can never satisfy
/// `evaluate_evidence_required()` -- PREPARE must deny, not hard-fail.
#[test]
fn complete_prepare_denies_when_linked_evidence_is_unverified_and_unjudged() {
    let (_ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "unverified");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let result = writer.prepare_complete_action(
        complete_command(
            "action-complete-prepare-1",
            "action-complete-prepare-correlation-1",
            &action_id,
            version_at_link,
            None,
        ),
        PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
    );
    assert!(
        result.is_err(),
        "Complete PREPARE must deny Unverified, unjudged evidence: {result:?}"
    );
}

/// The PREPARE-time H3 denial itself must be a durable,
/// replay-safe Terminal capsule, not a transaction that always rolls back.
/// This test found the gap was deeper than its own name suggested: before
/// this fix, the domain layer's own `record_h3_denial` never even reached
/// `store_terminal_failure` in the first place --
/// `PersistedActionTransitionPrepareIds::next_audit_event_id` (the ledger's
/// own PREPARE-time id source) unconditionally returned an error, because
/// nothing had ever needed to mint a FRESH audit event id mid-PREPARE before
/// (a successful PREPARE only ever needs the caller-supplied
/// `prepared_intent_id`). So the domain snapshot never actually contained a
/// capsule for `persist_prepare_complete_h3_denied` to find, independent of
/// whether decode existed -- fixed by deterministically deriving that audit
/// id from `prepared_intent_id` (see `derive_prepare_h3_denial_audit_id`).
/// Only once that was fixed did the originally-suspected gap (decode support
/// for the Terminal capsule) become reachable and provable. This test proves
/// the full, now-working chain: the caller sees the REAL, correctly-typed
/// denial (not a generic `ledger.persistence_failed` fallback), and a ledger
/// restart can decode the durable capsule and replay the identical denial on
/// retry.
#[test]
fn complete_prepare_h3_denial_persists_and_replays_losslessly() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "unverified");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();

    let error = writer
        .prepare_complete_action(
            complete_command(
                "action-complete-prepare-h3-1",
                "action-complete-prepare-h3-correlation-1",
                &action_id,
                version_at_link,
                None,
            ),
            PreparedIntentId::parse("action-complete-prepared-h3-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect_err("Complete PREPARE must deny Unverified, unjudged evidence");
    let LedgerTransactionError::Operation(domain_error) = &error else {
        panic!("expected an Operation error, got {error:?}");
    };
    assert_eq!(
        domain_error.code(),
        ErrorCode::SecurityPolicyDenied,
        "the caller must see the real H3 denial, not a generic persistence_failed: {domain_error:?}"
    );
    assert_eq!(
        domain_error.message_key().as_str(),
        "action.approval_denied"
    );
    drop(writer);

    let mut reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("the durable H3-denial Terminal capsule must decode losslessly on restart");
    assert!(
        snapshot
            .actions()
            .iter()
            .any(|record| record.id() == &action_id),
        "the underlying Action itself must still decode -- the denial never mutated it"
    );

    let replay_error = reopened
        .prepare_complete_action(
            complete_command(
                "action-complete-prepare-h3-1",
                "action-complete-prepare-h3-correlation-1",
                &action_id,
                version_at_link,
                None,
            ),
            PreparedIntentId::parse("action-complete-prepared-h3-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .expect_err("an idempotent PREPARE replay must return the identical denial");
    assert_eq!(
        replay_error, error,
        "idempotent replay must reproduce the exact same denial, not merely another error"
    );
}

/// PREPARE rejects a stale `expected_version`, mirroring the equivalent
/// Cancel/Reopen/StartAction tests.
#[test]
fn complete_prepare_rejects_a_stale_expected_version() {
    let (_ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "verified");

    let result = writer.prepare_complete_action(
        complete_command(
            "action-complete-prepare-1",
            "action-complete-prepare-correlation-1",
            &action_id,
            AggregateVersion::initial(),
            None,
        ),
        PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_300),
    );
    assert!(
        result.is_err(),
        "a stale expected_version must be rejected: {result:?}"
    );
}

/// v45: the prepared witness binds each Evidence reference's aggregate
/// version, so a change that leaves the verification tuple identical -- here
/// a bare registry version advance -- still invalidates the preview at
/// approval time, and the changed preview is retained rather than discarded.
#[test]
fn complete_execute_rejects_a_preview_whose_evidence_revision_advanced_without_visible_change() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "verified");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();
    let prepared = writer
        .prepare_complete_action(
            complete_command(
                "action-complete-prepare-1",
                "action-complete-prepare-correlation-1",
                &action_id,
                version_at_link,
                None,
            ),
            PreparedIntentId::parse("action-complete-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    drop(writer);

    let connection = Connection::open(&ledger.0).unwrap();
    let registry_version: i64 = connection
        .query_row(
            "SELECT version FROM aggregate_registry WHERE id='synthetic-completion-evidence-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let witness = prepared.preview().support().unwrap();
    assert_eq!(
        witness.evidence()[0].source_version().get(),
        u64::try_from(registry_version).unwrap(),
        "the witness must bind the reference's version at prepare time"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT evidence_version FROM action_h2a_support_evidence_snapshots WHERE prepared_intent_id='action-complete-prepared-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        registry_version
    );
    connection
        .execute(
            "UPDATE aggregate_registry SET version=version+1 WHERE id='synthetic-completion-evidence-1'",
            [],
        )
        .unwrap();
    drop(connection);

    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let error = match writer.approve_and_execute_complete_action(
        ApproveAndExecuteCompleteAction {
            approval: approval_for(
                "action-complete-prepared-1",
                prepared.payload_digest(),
                "action-complete-execute-1",
            ),
            context: ActionOperationContext {
                idempotency_id: IdempotencyId::parse("action-complete-execute-1").unwrap(),
                correlation_id: CorrelationId::parse("action-complete-execute-correlation-1")
                    .unwrap(),
            },
        },
        AuditEventId::parse("action-complete-execute-audit-1").unwrap(),
        ApprovalReceiptId::parse("action-complete-receipt-1").unwrap(),
        UtcTimestamp::from_unix_millis(1_350),
        H2aPolicy(ActionExecutionPolicy::Allowed),
        H2aPolicy(ActionExecutionPolicy::Allowed),
    ) {
        Err(LedgerTransactionError::Operation(error)) => error,
        other => panic!("a source revision advance must invalidate the preview: {other:?}"),
    };
    assert_eq!(error.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    drop(writer);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_action_persistence_snapshot().unwrap();
    assert_eq!(snapshot.prepared(), &[prepared]);
    assert!(snapshot
        .actions()
        .iter()
        .all(|action| action.state() != pmc_domain::work_management::ActionState::Completed));
}

/// History is what was decided: completion Evidence that later loses its
/// verification must neither rewrite nor break the completed Action's
/// support, which is read from the execution-time snapshot.
#[test]
fn a_completed_action_keeps_its_support_after_its_evidence_degrades() {
    let (ledger, mut writer, action_id) =
        seeded_ledger_with_in_progress_action_and_linked_evidence("internal", "verified");
    let version_at_link = AggregateVersion::initial().next().unwrap().next().unwrap();
    let prepared = writer
        .prepare_complete_action(
            complete_command(
                "action-degrade-prepare-1",
                "action-degrade-prepare-correlation-1",
                &action_id,
                version_at_link,
                None,
            ),
            PreparedIntentId::parse("action-degrade-prepared-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_300),
        )
        .unwrap();
    let outcome = writer
        .approve_and_execute_complete_action(
            ApproveAndExecuteCompleteAction {
                approval: approval_for(
                    "action-degrade-prepared-1",
                    prepared.payload_digest(),
                    "action-degrade-execute-1",
                ),
                context: ActionOperationContext {
                    idempotency_id: IdempotencyId::parse("action-degrade-execute-1").unwrap(),
                    correlation_id: CorrelationId::parse("action-degrade-execute-correlation-1")
                        .unwrap(),
                },
            },
            AuditEventId::parse("action-degrade-execute-audit-1").unwrap(),
            ApprovalReceiptId::parse("action-degrade-receipt-1").unwrap(),
            UtcTimestamp::from_unix_millis(1_350),
            H2aPolicy(ActionExecutionPolicy::Allowed),
            H2aPolicy(ActionExecutionPolicy::Allowed),
        )
        .unwrap();
    drop(writer);

    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "UPDATE evidence_references SET verification='unverified',integrity_digest=NULL,last_verified_at=NULL WHERE id='synthetic-completion-evidence-1';
             UPDATE aggregate_registry SET version=version+1 WHERE id='synthetic-completion-evidence-1';",
        )
        .unwrap();
    drop(connection);

    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened
        .load_action_persistence_snapshot()
        .expect("a degraded Evidence must not break the Action snapshot");
    assert!(
        snapshot.actions().contains(&outcome.record),
        "the completed Action keeps the support it executed on"
    );
}
