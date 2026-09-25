//! The Decision Request facade: withdraw, and the
//! Resolve H2a loop (prepare with Ledger-read Evidence, approve with every
//! host-minted id, reject durably), all through the host's own id source.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_application::decision_requests::{
    approve_and_execute_resolve_decision_request, prepare_resolve_decision_request,
    reject_decision_prepared_intent, withdraw_decision_request, DecisionFlowError,
};
use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_domain::actions::{ActionDetails, ActionTitle};
use pmc_domain::classification::DataClassification;
use pmc_domain::decisions::{
    CreateDecisionRequestDraft, DecisionOperationContext, DecisionSubject, DecisionText,
    DecisionWithdrawalRationale, PrepareResolveDecisionRequest, SubmitDecisionRequest,
    WithdrawDecisionRequest,
};
use pmc_domain::error::ErrorCode;
use pmc_domain::evidence::{
    CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm,
    OperationContext as EvidenceContext, VaultRelativePath,
};
use pmc_domain::identity::{
    ActionRequestId, AggregateVersion, AuditEventId, CorrelationId, DecisionRequestId,
    EvidenceReferenceId, IdempotencyId, StakeholderId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, OperationContext as RelationshipContext, StakeholderKind, StakeholderName,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    DecisionRequestState, DecisionResultingActionRequest, EvidenceVerification, HumanJudgment,
    HumanJudgmentDisposition, IntegrityDigest, WorkManagementOperation,
    WorkManagementPayloadDigest,
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
            "pmc-synthetic-decision-request-facade-{nonce}-{sequence}.sqlite3"
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

fn context(key: &str) -> DecisionOperationContext {
    DecisionOperationContext {
        idempotency_id: IdempotencyId::parse(format!("decision-idem-{key}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("decision-corr-{key}")).unwrap(),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn text<const N: usize>(value: &str) -> pmc_domain::BoundedText<N> {
    pmc_domain::BoundedText::parse(value.to_owned()).unwrap()
}

fn domain_refusal(error: DecisionFlowError) -> pmc_domain::error::DomainError {
    match error {
        DecisionFlowError::Ledger(LedgerTransactionError::Operation(domain))
        | DecisionFlowError::Domain(domain) => domain,
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

/// Stakeholder, one Verified Evidence reference, and an Open Decision
/// Request whose intended owner is that stakeholder.
fn seeded_open_request(ledger: &SyntheticLedger) -> (SqliteProductLedger, DecisionRequestId) {
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    writer
        .create_stakeholder(
            CreateStakeholder {
                id: StakeholderId::parse("owner-1").unwrap(),
                name: StakeholderName::parse("Synthetic Decision Owner").unwrap(),
                kind: StakeholderKind::Person,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::SyntheticFixture(
                    ProvenanceReference::parse("public-safe-fixture").unwrap(),
                ),
                context: RelationshipContext {
                    idempotency_id: IdempotencyId::parse("decision-idem-owner").unwrap(),
                    correlation_id: CorrelationId::parse("decision-corr-owner").unwrap(),
                },
            },
            AuditEventId::parse("decision-audit-owner").unwrap(),
            at(50),
        )
        .unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: EvidenceReferenceId::parse("evidence-1").unwrap(),
                vault_path: VaultRelativePath::parse("Research/decision.md").unwrap(),
                fingerprint: Some(EvidenceFingerprint::new(
                    FingerprintAlgorithm::Sha256,
                    IntegrityDigest::parse("a".repeat(64)).unwrap(),
                )),
                verification: EvidenceVerification::Verified {
                    verified_at: at(60),
                    integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
                },
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: EvidenceContext {
                    idempotency_id: IdempotencyId::parse("decision-idem-evidence").unwrap(),
                    correlation_id: CorrelationId::parse("decision-corr-evidence").unwrap(),
                },
            },
            AuditEventId::parse("decision-audit-evidence").unwrap(),
            at(60),
        )
        .unwrap();
    let request_id = DecisionRequestId::parse("decision-request-1").unwrap();
    writer
        .create_decision_request_draft(
            CreateDecisionRequestDraft {
                id: request_id.clone(),
                subject: DecisionSubject::parse("Choose the synthetic direction").unwrap(),
                details: DecisionText::parse("Resolve me through the facade.").unwrap(),
                intended_owner: Some(StakeholderId::parse("owner-1").unwrap()),
                classification: DataClassification::Internal,
                context: context("create"),
            },
            AuditEventId::parse("decision-audit-create").unwrap(),
            at(100),
        )
        .unwrap();
    writer
        .submit_decision_request(
            SubmitDecisionRequest {
                request_id: request_id.clone(),
                expected_version: AggregateVersion::initial(),
                context: context("submit"),
            },
            AuditEventId::parse("decision-audit-submit").unwrap(),
            at(200),
        )
        .unwrap();
    (writer, request_id)
}

fn resolve_command(request_id: &DecisionRequestId, key: &str) -> PrepareResolveDecisionRequest {
    PrepareResolveDecisionRequest {
        request_id: request_id.clone(),
        expected_version: AggregateVersion::new(2).unwrap(),
        statement: text("Proceed with option A."),
        rationale: text("Best synthetic tradeoff."),
        impact: text("Synthetic delivery remains on track."),
        evidence_ids: vec![EvidenceReferenceId::parse("evidence-1").unwrap()],
        judgments: vec![],
        resulting_action_requests: vec![DecisionResultingActionRequest {
            id: ActionRequestId::parse(format!("resulting-{key}")).unwrap(),
            subject: ActionTitle::parse("Synthetic follow-up").unwrap(),
            details: ActionDetails::parse("Track the synthetic follow-up.").unwrap(),
            intended_owner: StakeholderId::parse("owner-1").unwrap(),
            due_at: at(900_000),
            classification: DataClassification::Internal,
        }],
        context: context(&format!("prepare-{key}")),
    }
}

#[test]
fn prepare_binds_the_ledgers_evidence_then_approve_resolves_and_creates_the_resulting_request() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xb6);

    let prepared = prepare_resolve_decision_request(
        &mut writer,
        resolve_command(&request_id, "1"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let WorkManagementOperation::ResolveDecisionRequest {
        decision_id,
        resulting_action_requests,
        ..
    } = prepared.operation()
    else {
        panic!("a Resolve preview must carry the Resolve operation");
    };
    assert!(decision_id.as_str().starts_with("decision-"));
    assert_eq!(resulting_action_requests.len(), 1);
    let witness = prepared
        .preview()
        .support()
        .expect("Resolve binds its Evidence");
    assert_eq!(witness.evidence().len(), 1);
    assert_eq!(
        witness.evidence()[0].source_version(),
        AggregateVersion::initial()
    );

    let resolved = approve_and_execute_resolve_decision_request(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("approve-1"),
        &mut ids,
        at(1_100),
    )
    .unwrap();
    assert_eq!(resolved.request.state(), DecisionRequestState::Resolved);
    assert_eq!(resolved.decision.id(), decision_id);
    assert_eq!(resolved.resulting_action_request_ids.len(), 1);
    assert_eq!(resolved.audit_events.len(), 3);

    // The resulting Action Request exists in the Action namespace.
    let actions = writer.load_action_persistence_snapshot().unwrap();
    assert_eq!(actions.requests().len(), 1);
    assert_eq!(
        actions.requests()[0].source_decision_id(),
        Some(decision_id)
    );

    // Approving the consumed preview again with a fresh id is refused.
    let again = domain_refusal(
        approve_and_execute_resolve_decision_request(
            &mut writer,
            prepared.id().clone(),
            prepared.payload_digest().clone(),
            context("approve-again"),
            &mut ids,
            at(1_200),
        )
        .unwrap_err(),
    );
    assert_eq!(again.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
}

/// The authority answers only for references the Ledger holds. The positive
/// control matters: without it, a refusal here could come from the support
/// gate (an unverified witness is refused on its own) rather than from the
/// lookup, and the test would pass even if the authority invented metadata.
#[test]
fn an_evidence_reference_the_ledger_does_not_hold_is_refused_at_prepare() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xb6);

    // Positive control: the seeded reference is Verified, and preparing with
    // it succeeds, so the support gate is satisfied by this shape.
    let prepared = prepare_resolve_decision_request(
        &mut writer,
        resolve_command(&request_id, "known"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    assert_eq!(
        prepared
            .preview()
            .support()
            .expect("Resolve binds its Evidence")
            .evidence()
            .len(),
        1
    );
    reject_decision_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        context("reject-known"),
        &mut ids,
        at(1_050),
    )
    .unwrap();

    // The same shape with an id the Ledger does not hold is refused, and
    // nothing is persisted.
    let mut command = resolve_command(&request_id, "missing");
    command.evidence_ids = vec![EvidenceReferenceId::parse("evidence-missing").unwrap()];
    let error = domain_refusal(
        prepare_resolve_decision_request(&mut writer, command, &mut ids, at(1_100)).unwrap_err(),
    );
    assert_ne!(error.code(), ErrorCode::PlatformInternal);
    assert!(writer
        .load_decision_persistence_snapshot()
        .unwrap()
        .prepared()
        .is_empty());
}

#[test]
fn a_resolve_preview_can_be_rejected_durably_and_a_judgment_is_carried() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xb6);
    let mut command = resolve_command(&request_id, "judged");
    command.evidence_ids = vec![];
    command.judgments = vec![HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        "Owner weighed the options in person.",
        DataClassification::Internal,
    )
    .unwrap()];
    let prepared =
        prepare_resolve_decision_request(&mut writer, command, &mut ids, at(1_000)).unwrap();
    assert_eq!(prepared.preview().support().unwrap().judgments().len(), 1);

    let rejected = reject_decision_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        context("reject"),
        &mut ids,
        at(1_050),
    )
    .unwrap();
    assert_eq!(rejected.prepared_intent_id(), prepared.id());
    assert_eq!(
        rejected.audit_event().code().as_str(),
        "decision.prepared_rejected"
    );
    let snapshot = writer.load_decision_persistence_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    assert_eq!(snapshot.requests()[0].state(), DecisionRequestState::Open);

    let wrong = WorkManagementPayloadDigest::from_persisted("0".repeat(64)).unwrap();
    let refused = domain_refusal(
        approve_and_execute_resolve_decision_request(
            &mut writer,
            prepared.id().clone(),
            wrong,
            context("approve-rejected"),
            &mut ids,
            at(1_100),
        )
        .unwrap_err(),
    );
    assert_eq!(refused.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
}

#[test]
fn withdrawing_is_a_plain_h1_write_with_a_rationale() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xb6);
    let outcome = withdraw_decision_request(
        &mut writer,
        WithdrawDecisionRequest {
            request_id,
            expected_version: AggregateVersion::new(2).unwrap(),
            rationale: DecisionWithdrawalRationale::parse("No longer needed.").unwrap(),
            context: context("withdraw"),
        },
        &mut ids,
        at(1_000),
    )
    .unwrap();
    assert_eq!(outcome.record.state(), DecisionRequestState::Withdrawn);
    assert_eq!(outcome.audit_events.len(), 1);
}
