//! The Action lifecycle facade: Start -> link Evidence
//! -> Complete through the exact preview, Cancel, Reopen, and rejection of
//! any of the three previews, all through the host's own id source.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_application::action_lifecycle::{
    action_completion_context, approve_and_execute_cancel_action,
    approve_and_execute_complete_action, approve_and_execute_reopen_action,
    link_action_completion_evidence, prepare_cancel_action, prepare_complete_action,
    prepare_reopen_action, ActionCompletionContextError,
};
use pmc_application::action_requests::{
    approve_and_execute_accept_action_request, prepare_accept_action_request,
    reject_action_prepared_intent, start_action, ActionRequestFlowError,
};
use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_domain::actions::{
    ActionDetails, ActionOperationContext, ActionTitle, CreateActionRequestDraft,
    LinkActionCompletionEvidence, PrepareAcceptActionRequest, PrepareCancelAction,
    PrepareCompleteAction, PrepareReopenAction, StartAction, SubmitActionRequest,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::evidence::{
    CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm,
    OperationContext as EvidenceContext, VaultRelativePath,
};
use pmc_domain::identity::{
    ActionId, ActionRequestId, AggregateVersion, AuditEventId, CorrelationId, EvidenceReferenceId,
    IdempotencyId, StakeholderId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, OperationContext as RelationshipContext, StakeholderKind, StakeholderName,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionReopenMode, ActionState, EvidenceVerification, HumanJudgment, HumanJudgmentDisposition,
    IntegrityDigest, WorkManagementOperation, WorkManagementPayloadDigest,
};
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-action-lifecycle-facade-{nonce}-{sequence}.sqlite3"
        )))
    }
}

impl Drop for SyntheticLedger {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn context(key: &str) -> ActionOperationContext {
    ActionOperationContext {
        idempotency_id: IdempotencyId::parse(format!("lifecycle-idem-{key}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("lifecycle-corr-{key}")).unwrap(),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn domain_refusal(error: ActionRequestFlowError) -> pmc_domain::error::DomainError {
    match error {
        ActionRequestFlowError::Ledger(LedgerTransactionError::Operation(domain))
        | ActionRequestFlowError::Domain(domain) => domain,
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

/// Stakeholder + Action Request accepted through the facade + Verified
/// Evidence reference created through the Ledger; returns the Open Action.
fn seeded_open_action(
    ledger: &SyntheticLedger,
    ids: &mut OpaqueIdSource,
) -> (SqliteProductLedger, ActionId) {
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_stakeholder(
            CreateStakeholder {
                id: StakeholderId::parse("stakeholder-owner-1").unwrap(),
                name: StakeholderName::parse("Synthetic Product Owner").unwrap(),
                kind: StakeholderKind::Person,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::SyntheticFixture(
                    ProvenanceReference::parse("public-safe-fixture").unwrap(),
                ),
                context: RelationshipContext {
                    idempotency_id: IdempotencyId::parse("lifecycle-idem-owner").unwrap(),
                    correlation_id: CorrelationId::parse("lifecycle-corr-owner").unwrap(),
                },
            },
            AuditEventId::parse("lifecycle-audit-owner").unwrap(),
            at(50),
        )
        .unwrap();
    let request_id = ActionRequestId::parse("request-1").unwrap();
    writer
        .create_action_request_draft(
            CreateActionRequestDraft {
                id: request_id.clone(),
                title: ActionTitle::parse("Synthetic lifecycle request").unwrap(),
                details: ActionDetails::parse("Complete me through the facade.").unwrap(),
                intended_owner: Some(StakeholderId::parse("stakeholder-owner-1").unwrap()),
                response_due_at: Some(at(200_000)),
                intended_action_due_at: Some(at(300_000)),
                classification: DataClassification::Internal,
                context: context("create"),
            },
            AuditEventId::parse("lifecycle-audit-create").unwrap(),
            at(100),
        )
        .unwrap();
    writer
        .submit_action_request(
            SubmitActionRequest {
                request_id: request_id.clone(),
                expected_version: AggregateVersion::initial(),
                context: context("submit"),
            },
            AuditEventId::parse("lifecycle-audit-submit").unwrap(),
            at(200),
        )
        .unwrap();
    let prepared = prepare_accept_action_request(
        &mut writer,
        PrepareAcceptActionRequest {
            request_id,
            expected_version: AggregateVersion::new(2).unwrap(),
            context: context("prepare-accept"),
        },
        ids,
        at(1_000),
    )
    .unwrap();
    let accepted = approve_and_execute_accept_action_request(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("approve-accept"),
        ids,
        at(1_100),
    )
    .unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: EvidenceReferenceId::parse("evidence-1").unwrap(),
                vault_path: VaultRelativePath::parse("Research/notes.md").unwrap(),
                fingerprint: Some(EvidenceFingerprint::new(
                    FingerprintAlgorithm::Sha256,
                    IntegrityDigest::parse("a".repeat(64)).unwrap(),
                )),
                verification: EvidenceVerification::Verified {
                    verified_at: at(900),
                    integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                },
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: EvidenceContext {
                    idempotency_id: IdempotencyId::parse("lifecycle-idem-evidence").unwrap(),
                    correlation_id: CorrelationId::parse("lifecycle-corr-evidence").unwrap(),
                },
            },
            AuditEventId::parse("lifecycle-audit-evidence").unwrap(),
            at(900),
        )
        .unwrap();
    (writer, accepted.action.id().clone())
}

fn in_progress_with_linked_evidence(
    writer: &mut SqliteProductLedger,
    action_id: &ActionId,
    ids: &mut OpaqueIdSource,
) -> AggregateVersion {
    let started = start_action(
        writer,
        StartAction {
            action_id: action_id.clone(),
            expected_version: AggregateVersion::initial(),
            context: context("start"),
        },
        ids,
        at(1_200),
    )
    .unwrap();
    let linked = link_action_completion_evidence(
        writer,
        LinkActionCompletionEvidence {
            action_id: action_id.clone(),
            expected_version: started.record.version(),
            evidence_id: EvidenceReferenceId::parse("evidence-1").unwrap(),
            context: context("link"),
        },
        ids,
        at(1_300),
    )
    .unwrap();
    assert_eq!(linked.record.completion_evidence().len(), 1);
    linked.record.version()
}

#[test]
fn the_completion_context_reports_the_action_and_every_evidence_reference_without_paths() {
    let ledger = SyntheticLedger::new();
    let mut ids = OpaqueIdSource::with_nonce(0xb5);
    let (writer, action_id) = seeded_open_action(&ledger, &mut ids);

    let context = action_completion_context(&writer, &action_id, at(1_150)).unwrap();
    assert_eq!(context.action.id(), &action_id);
    assert_eq!(context.action.state(), ActionState::Open);
    assert_eq!(context.evidence_references.len(), 1);
    let evidence = &context.evidence_references[0];
    assert_eq!(evidence.id.as_str(), "evidence-1");
    assert!(evidence.pinned);
    assert_eq!(evidence.version, AggregateVersion::initial());
    assert!(matches!(
        action_completion_context(
            &writer,
            &ActionId::parse("action-missing").unwrap(),
            at(1_150)
        ),
        Err(ActionCompletionContextError::NotFound)
    ));
}

#[test]
fn link_then_prepare_then_approve_completes_the_action_with_its_evidence_witness() {
    let ledger = SyntheticLedger::new();
    let mut ids = OpaqueIdSource::with_nonce(0xb5);
    let (mut writer, action_id) = seeded_open_action(&ledger, &mut ids);
    let version = in_progress_with_linked_evidence(&mut writer, &action_id, &mut ids);

    let prepared = prepare_complete_action(
        &mut writer,
        PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: version,
            judgment: None,
            context: context("prepare-complete"),
        },
        &mut ids,
        at(1_400),
    )
    .unwrap();
    assert!(matches!(
        prepared.operation(),
        WorkManagementOperation::CompleteAction { .. }
    ));
    let witness = prepared
        .preview()
        .support()
        .expect("Complete binds its Evidence");
    assert_eq!(witness.evidence().len(), 1);
    assert_eq!(
        witness.evidence()[0].source_version(),
        AggregateVersion::initial()
    );

    let completed = approve_and_execute_complete_action(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("approve-complete"),
        &mut ids,
        at(1_500),
    )
    .unwrap();
    assert_eq!(completed.record.state(), ActionState::Completed);
    assert!(completed.record.support().is_some());
    assert!(completed.approval_receipt_id.is_some());

    // The Ledger's own gates still hold through the facade: approving the
    // consumed preview again with a fresh id is refused with no effect.
    let again = domain_refusal(
        approve_and_execute_complete_action(
            &mut writer,
            prepared.id().clone(),
            prepared.payload_digest().clone(),
            context("approve-complete-again"),
            &mut ids,
            at(1_600),
        )
        .unwrap_err(),
    );
    assert_eq!(again.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
}

#[test]
fn a_complete_preview_can_be_rejected_and_a_judgment_is_carried_into_the_witness() {
    let ledger = SyntheticLedger::new();
    let mut ids = OpaqueIdSource::with_nonce(0xb5);
    let (mut writer, action_id) = seeded_open_action(&ledger, &mut ids);
    let version = in_progress_with_linked_evidence(&mut writer, &action_id, &mut ids);

    let prepared = prepare_complete_action(
        &mut writer,
        PrepareCompleteAction {
            action_id: action_id.clone(),
            expected_version: version,
            judgment: Some(
                HumanJudgment::new(
                    HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                    "Owner reviewed the deliverable in person.",
                    DataClassification::Internal,
                )
                .unwrap(),
            ),
            context: context("prepare-complete-judged"),
        },
        &mut ids,
        at(1_400),
    )
    .unwrap();
    assert_eq!(prepared.preview().support().unwrap().judgments().len(), 1);

    let rejected = reject_action_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        context("reject-complete"),
        &mut ids,
        at(1_450),
    )
    .unwrap();
    assert_eq!(rejected.prepared_intent_id(), prepared.id());
    let snapshot = writer.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    assert_eq!(
        snapshot
            .actions()
            .iter()
            .find(|a| a.id() == &action_id)
            .unwrap()
            .state(),
        ActionState::InProgress
    );
}

#[test]
fn cancel_then_reopen_round_trips_through_their_exact_previews() {
    let ledger = SyntheticLedger::new();
    let mut ids = OpaqueIdSource::with_nonce(0xb5);
    let (mut writer, action_id) = seeded_open_action(&ledger, &mut ids);

    let prepared = prepare_cancel_action(
        &mut writer,
        PrepareCancelAction {
            action_id: action_id.clone(),
            expected_version: AggregateVersion::initial(),
            reason: ActionDetails::parse("Scope moved to another initiative.").unwrap(),
            context: context("prepare-cancel"),
        },
        &mut ids,
        at(1_200),
    )
    .unwrap();
    let cancelled = approve_and_execute_cancel_action(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("approve-cancel"),
        &mut ids,
        at(1_300),
    )
    .unwrap();
    assert_eq!(cancelled.record.state(), ActionState::Cancelled);

    let prepared = prepare_reopen_action(
        &mut writer,
        PrepareReopenAction {
            action_id: action_id.clone(),
            expected_version: cancelled.record.version(),
            mode: ActionReopenMode::RestartCancelled,
            reason: ActionDetails::parse("The scope came back.").unwrap(),
            context: context("prepare-reopen"),
        },
        &mut ids,
        at(1_400),
    )
    .unwrap();
    let reopened = approve_and_execute_reopen_action(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("approve-reopen"),
        &mut ids,
        at(1_500),
    )
    .unwrap();
    // Restarting a cancelled Action resumes it: InProgress, not Open.
    assert_eq!(reopened.record.state(), ActionState::InProgress);

    // A digest the person did not acknowledge is refused on every kind.
    let prepared = prepare_cancel_action(
        &mut writer,
        PrepareCancelAction {
            action_id: action_id.clone(),
            expected_version: reopened.record.version(),
            reason: ActionDetails::parse("Second thoughts.").unwrap(),
            context: context("prepare-cancel-2"),
        },
        &mut ids,
        at(1_600),
    )
    .unwrap();
    let wrong = WorkManagementPayloadDigest::from_persisted("0".repeat(64)).unwrap();
    let refused = domain_refusal(
        approve_and_execute_cancel_action(
            &mut writer,
            prepared.id().clone(),
            wrong,
            context("approve-cancel-wrong"),
            &mut ids,
            at(1_700),
        )
        .unwrap_err(),
    );
    assert_eq!(refused.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
}
