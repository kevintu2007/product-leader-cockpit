//! The Risk and Issue H2a facades: prepare the exact preview
//! through the domain service, approve it with every host-minted id, refuse
//! it durably -- and, the guarantee this slice adds, survive a retry.
//!
//! The host mints the prepared-intent id, and for a Risk occurrence the id of
//! the Issue that occurrence will create. A preview cannot be rebuilt later:
//! its expiry is part of the payload digest. So a second attempt at the same
//! client request must return the first attempt's preview rather than mint a
//! second identity, and once that preview has been approved or refused the
//! facade must say so instead of quietly preparing another.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::issue_lifecycle::{
    approve_and_execute_issue_transition, prepare_resolve_issue, reject_issue_prepared_intent,
    IssueFlowError,
};
use pmc_application::risk_lifecycle::{
    approve_and_execute_record_risk_occurrence, prepare_close_risk, prepare_record_risk_occurrence,
    reject_risk_prepared_intent, RiskFlowError,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::evidence::{
    CreateEvidenceReference, EvidenceFingerprint, FingerprintAlgorithm,
    OperationContext as EvidenceContext, VaultRelativePath,
};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, EvidenceReferenceId, IdempotencyId, IssueId,
    RiskId,
};
use pmc_domain::issues::{CreateIssue, IssueDetails, IssueOperationContext, IssueTitle};
use pmc_domain::provenance::Provenance;
use pmc_domain::risks::{CreateRisk, RiskDetails, RiskOperationContext, RiskTitle};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    EvidenceVerification, HumanJudgment, HumanJudgmentDisposition, IntegrityDigest,
    IssueResolutionType, IssueState, RiskState, SupportDisposition, WorkManagementOperation,
    WorkManagementRationale,
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
            "pmc-synthetic-risk-issue-facade-{nonce}-{sequence}.sqlite3"
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

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn risk_context(key: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(format!("risk-idem-{key}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("risk-corr-{key}")).unwrap(),
    }
}

fn issue_context(key: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(format!("issue-idem-{key}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("issue-corr-{key}")).unwrap(),
    }
}

fn rationale(value: &str) -> WorkManagementRationale {
    WorkManagementRationale::parse(value).unwrap()
}

fn risk_refusal(error: RiskFlowError) -> pmc_domain::error::DomainError {
    match error {
        RiskFlowError::Ledger(LedgerTransactionError::Operation(domain))
        | RiskFlowError::Domain(domain) => domain,
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

fn issue_refusal(error: IssueFlowError) -> pmc_domain::error::DomainError {
    match error {
        IssueFlowError::Ledger(LedgerTransactionError::Operation(domain))
        | IssueFlowError::Domain(domain) => domain,
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

/// One Open Risk.
fn seeded_risk(ledger: &SyntheticLedger) -> (SqliteProductLedger, RiskId) {
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = RiskId::parse("risk-1").unwrap();
    writer
        .create_risk(
            CreateRisk {
                id: id.clone(),
                title: RiskTitle::parse("Synthetic facade risk").unwrap(),
                details: RiskDetails::parse("Public-safe facade fixture.").unwrap(),
                classification: DataClassification::Internal,
                context: risk_context("create"),
            },
            AuditEventId::parse("risk-audit-create").unwrap(),
            at(100),
        )
        .unwrap();
    (writer, id)
}

/// One Open Issue and one Verified Evidence reference.
fn seeded_issue(ledger: &SyntheticLedger) -> (SqliteProductLedger, IssueId) {
    let mut writer = SqliteProductLedger::open(&ledger.0).unwrap();
    let id = IssueId::parse("issue-1").unwrap();
    writer
        .create_issue(
            CreateIssue {
                id: id.clone(),
                title: IssueTitle::parse("Synthetic facade issue").unwrap(),
                details: IssueDetails::parse("Public-safe facade fixture.").unwrap(),
                classification: DataClassification::Internal,
                recurrence_of: None,
                context: issue_context("create"),
            },
            AuditEventId::parse("issue-audit-create").unwrap(),
            at(100),
        )
        .unwrap();
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: EvidenceReferenceId::parse("evidence-1").unwrap(),
                vault_path: VaultRelativePath::parse("Research/issue.md").unwrap(),
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
                    idempotency_id: IdempotencyId::parse("issue-idem-evidence").unwrap(),
                    correlation_id: CorrelationId::parse("issue-corr-evidence").unwrap(),
                },
            },
            AuditEventId::parse("issue-audit-evidence").unwrap(),
            at(60),
        )
        .unwrap();
    (writer, id)
}

// ------------------------------------------------------------------- Risk

#[test]
fn the_same_client_request_returns_the_first_preview_and_mints_no_second_identity() {
    let ledger = SyntheticLedger::new();
    let (mut writer, risk_id) = seeded_risk(&ledger);
    let mut ids = OpaqueIdSource::new();

    let first = prepare_record_risk_occurrence(
        &mut writer,
        risk_id.clone(),
        AggregateVersion::initial(),
        risk_context("prepare-occurrence"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    // A later instant, a fresh id source: a rebuilt preview would differ in
    // both the Issue identity and the expiry the digest binds.
    let mut other_ids = OpaqueIdSource::new();
    let second = prepare_record_risk_occurrence(
        &mut writer,
        risk_id,
        AggregateVersion::initial(),
        risk_context("prepare-occurrence"),
        &mut other_ids,
        at(9_000),
    )
    .unwrap();
    assert_eq!(first, second);
    let WorkManagementOperation::RecordRiskOccurrence { issue_id, .. } = second.operation() else {
        panic!("an occurrence preview names the Issue it will create");
    };
    assert_eq!(
        writer
            .load_risk_h2a_runtime_snapshot()
            .unwrap()
            .prepared()
            .len(),
        1,
        "a retry prepares nothing new"
    );
    assert!(issue_id.as_str().starts_with("issue-"));
}

#[test]
fn an_occurrence_preview_approves_into_a_risk_transition_and_the_issue_it_named() {
    let ledger = SyntheticLedger::new();
    let (mut writer, risk_id) = seeded_risk(&ledger);
    let mut ids = OpaqueIdSource::new();

    let prepared = prepare_record_risk_occurrence(
        &mut writer,
        risk_id,
        AggregateVersion::initial(),
        risk_context("prepare-occurrence"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let WorkManagementOperation::RecordRiskOccurrence { issue_id, .. } = prepared.operation()
    else {
        panic!("an occurrence preview names the Issue it will create");
    };
    let expected_issue = issue_id.clone();

    let outcome = approve_and_execute_record_risk_occurrence(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        risk_context("approve-occurrence"),
        &mut ids,
        at(2_000),
    )
    .unwrap();
    assert_eq!(outcome.risk.state(), RiskState::Occurred);
    assert_eq!(outcome.issue.id(), &expected_issue);
    assert_eq!(outcome.issue.state(), IssueState::Open);
    assert_eq!(outcome.audit_events.len(), 3);
}

#[test]
fn a_risk_preview_can_be_refused_durably_and_the_risk_is_untouched() {
    let ledger = SyntheticLedger::new();
    let (mut writer, risk_id) = seeded_risk(&ledger);
    let mut ids = OpaqueIdSource::new();

    let prepared = prepare_close_risk(
        &mut writer,
        risk_id,
        AggregateVersion::initial(),
        rationale("The synthetic risk no longer applies."),
        risk_context("prepare-close"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let outcome = reject_risk_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        risk_context("reject-close"),
        &mut ids,
        at(2_000),
    )
    .unwrap();
    assert_eq!(outcome.prepared_intent_id(), prepared.id());
    assert!(!outcome.expired_at_rejection());

    let snapshot = writer.load_risk_persistence_snapshot().unwrap();
    assert_eq!(snapshot.risks()[0].state(), RiskState::Open);
    assert_eq!(snapshot.risks()[0].version(), AggregateVersion::initial());
}

#[test]
fn a_client_request_whose_preview_was_consumed_is_told_so_rather_than_prepared_again() {
    let ledger = SyntheticLedger::new();
    let (mut writer, risk_id) = seeded_risk(&ledger);
    let mut ids = OpaqueIdSource::new();

    let prepared = prepare_close_risk(
        &mut writer,
        risk_id.clone(),
        AggregateVersion::initial(),
        rationale("The synthetic risk no longer applies."),
        risk_context("prepare-close"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    reject_risk_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        risk_context("reject-close"),
        &mut ids,
        at(2_000),
    )
    .unwrap();

    let again = prepare_close_risk(
        &mut writer,
        risk_id,
        AggregateVersion::initial(),
        rationale("The synthetic risk no longer applies."),
        risk_context("prepare-close"),
        &mut ids,
        at(3_000),
    );
    assert!(
        matches!(again, Err(RiskFlowError::PreviewAlreadyConsumed)),
        "a consumed preview is reported, never silently replaced: {again:?}"
    );
}

// ------------------------------------------------------------------ Issue

#[test]
fn resolving_an_issue_binds_the_ledgers_evidence_then_approves() {
    let ledger = SyntheticLedger::new();
    let (mut writer, issue_id) = seeded_issue(&ledger);
    let mut ids = OpaqueIdSource::new();

    let prepared = prepare_resolve_issue(
        &mut writer,
        issue_id.clone(),
        AggregateVersion::initial(),
        IssueResolutionType::Resolved,
        rationale("The synthetic issue is fixed."),
        vec![EvidenceReferenceId::parse("evidence-1").unwrap()],
        None,
        issue_context("prepare-resolve"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let support = prepared
        .preview()
        .support()
        .expect("a resolve preview carries its support witness");
    assert_eq!(support.evidence().len(), 1);

    let outcome = approve_and_execute_issue_transition(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        issue_context("approve-resolve"),
        &mut ids,
        at(2_000),
    )
    .unwrap();
    assert_eq!(outcome.record.state(), IssueState::Resolved);
    assert_eq!(outcome.record.id(), &issue_id);
    assert_eq!(
        outcome.record.resolution_evidence(),
        &[EvidenceReferenceId::parse("evidence-1").unwrap()]
    );
}

#[test]
fn an_evidence_reference_the_ledger_does_not_hold_is_refused_at_prepare() {
    let ledger = SyntheticLedger::new();
    let (mut writer, issue_id) = seeded_issue(&ledger);
    let mut ids = OpaqueIdSource::new();

    let refusal = issue_refusal(
        prepare_resolve_issue(
            &mut writer,
            issue_id,
            AggregateVersion::initial(),
            IssueResolutionType::Resolved,
            rationale("The synthetic issue is fixed."),
            vec![EvidenceReferenceId::parse("evidence-missing").unwrap()],
            None,
            issue_context("prepare-resolve-missing"),
            &mut ids,
            at(1_000),
        )
        .expect_err("Evidence the Ledger does not hold cannot support a preview"),
    );
    assert_ne!(refusal.code(), ErrorCode::PlatformInternal);
    assert!(writer
        .load_issue_h2a_runtime_snapshot()
        .unwrap()
        .prepared()
        .is_empty());
}

#[test]
fn an_issue_preview_can_be_refused_durably_and_the_issue_stays_open() {
    let ledger = SyntheticLedger::new();
    let (mut writer, issue_id) = seeded_issue(&ledger);
    let mut ids = OpaqueIdSource::new();

    let prepared = prepare_resolve_issue(
        &mut writer,
        issue_id.clone(),
        AggregateVersion::initial(),
        IssueResolutionType::Resolved,
        rationale("The synthetic issue is fixed."),
        vec![EvidenceReferenceId::parse("evidence-1").unwrap()],
        None,
        issue_context("prepare-resolve"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let outcome = reject_issue_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        issue_context("reject-resolve"),
        &mut ids,
        at(2_000),
    )
    .unwrap();
    assert_eq!(outcome.prepared_intent_id(), prepared.id());

    let snapshot = writer.load_issue_h2a_runtime_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    let record = snapshot
        .records()
        .iter()
        .find(|record| record.id() == &issue_id)
        .unwrap();
    assert_eq!(record.state(), IssueState::Open);
    assert_eq!(record.version(), AggregateVersion::initial());
}

#[test]
fn the_same_issue_client_request_returns_the_first_preview() {
    let ledger = SyntheticLedger::new();
    let (mut writer, issue_id) = seeded_issue(&ledger);
    let mut ids = OpaqueIdSource::new();

    let first = prepare_resolve_issue(
        &mut writer,
        issue_id.clone(),
        AggregateVersion::initial(),
        IssueResolutionType::Resolved,
        rationale("The synthetic issue is fixed."),
        vec![EvidenceReferenceId::parse("evidence-1").unwrap()],
        None,
        issue_context("prepare-resolve"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let mut other_ids = OpaqueIdSource::new();
    let second = prepare_resolve_issue(
        &mut writer,
        issue_id,
        AggregateVersion::initial(),
        IssueResolutionType::Resolved,
        rationale("The synthetic issue is fixed."),
        vec![EvidenceReferenceId::parse("evidence-1").unwrap()],
        None,
        issue_context("prepare-resolve"),
        &mut other_ids,
        at(9_000),
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        writer
            .load_issue_h2a_runtime_snapshot()
            .unwrap()
            .prepared()
            .len(),
        1
    );
}

/// Adds an observed-but-unpinned Evidence reference: read and hashed, no
/// fingerprint to compare against. Partly verified.
fn add_unpinned_evidence(writer: &mut SqliteProductLedger, id: &str) {
    writer
        .create_evidence_reference(
            CreateEvidenceReference {
                id: EvidenceReferenceId::parse(id).unwrap(),
                vault_path: VaultRelativePath::parse(format!("Research/{id}.md")).unwrap(),
                fingerprint: None,
                verification: EvidenceVerification::ObservedUnpinned {
                    observed_at: at(70),
                    integrity_digest: IntegrityDigest::parse("b".repeat(64)).unwrap(),
                },
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: EvidenceContext {
                    idempotency_id: IdempotencyId::parse(format!("issue-idem-{id}")).unwrap(),
                    correlation_id: CorrelationId::parse(format!("issue-corr-{id}")).unwrap(),
                },
            },
            AuditEventId::parse(format!("issue-audit-{id}")).unwrap(),
            at(70),
        )
        .unwrap();
}

fn judgment(rationale: &str, classification: DataClassification) -> HumanJudgment {
    HumanJudgment::new(
        HumanJudgmentDisposition::ProceedWithDocumentedRationale,
        rationale,
        classification,
    )
    .unwrap()
}

#[test]
fn a_judgment_moves_unpinned_issue_evidence_forward_as_verification_pending() {
    let ledger = SyntheticLedger::new();
    let (mut writer, issue_id) = seeded_issue(&ledger);
    add_unpinned_evidence(&mut writer, "evidence-unpinned");
    let mut ids = OpaqueIdSource::new();
    let evidence = vec![EvidenceReferenceId::parse("evidence-unpinned").unwrap()];

    let refused = prepare_resolve_issue(
        &mut writer,
        issue_id.clone(),
        AggregateVersion::initial(),
        IssueResolutionType::Resolved,
        rationale("The synthetic issue is fixed."),
        evidence.clone(),
        None,
        issue_context("prepare-resolve-bare"),
        &mut ids,
        at(1_000),
    );
    assert!(refused.is_err(), "unpinned Evidence alone does not resolve");

    let written = judgment(
        "The file was read; pinning waits for the owner.",
        DataClassification::Confidential,
    );
    let prepared = prepare_resolve_issue(
        &mut writer,
        issue_id,
        AggregateVersion::initial(),
        IssueResolutionType::Resolved,
        rationale("The synthetic issue is fixed."),
        evidence,
        Some(written.clone()),
        issue_context("prepare-resolve-judged"),
        &mut ids,
        at(1_000),
    )
    .unwrap();
    let support = prepared.preview().support().unwrap();
    assert_eq!(
        support.disposition(),
        SupportDisposition::VerificationPending
    );
    assert_eq!(support.judgments(), &[written]);
    assert_eq!(prepared.classification(), DataClassification::Confidential);

    let outcome = approve_and_execute_issue_transition(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        issue_context("approve-resolve-judged"),
        &mut ids,
        at(2_000),
    )
    .unwrap();
    assert_eq!(outcome.record.state(), IssueState::Resolved);
}

/// The facade answers a retry from the stored preview before the domain
/// ever sees the new payload, so it must compare that payload itself: a
/// changed Judgment or changed Evidence under the same client request id is
/// a conflict, and only an exact retry returns the first preview.
#[test]
fn the_same_issue_client_request_with_a_different_judgment_or_evidence_is_a_conflict() {
    let ledger = SyntheticLedger::new();
    let (mut writer, issue_id) = seeded_issue(&ledger);
    add_unpinned_evidence(&mut writer, "evidence-unpinned");
    add_unpinned_evidence(&mut writer, "evidence-other");
    let mut ids = OpaqueIdSource::new();
    let evidence = vec![EvidenceReferenceId::parse("evidence-unpinned").unwrap()];
    let mut attempt =
        |ids: &mut OpaqueIdSource, evidence: Vec<EvidenceReferenceId>, j: Option<HumanJudgment>| {
            prepare_resolve_issue(
                &mut writer,
                issue_id.clone(),
                AggregateVersion::initial(),
                IssueResolutionType::Resolved,
                rationale("The synthetic issue is fixed."),
                evidence,
                j,
                issue_context("prepare-resolve-retry"),
                ids,
                at(1_000),
            )
        };

    let first = attempt(
        &mut ids,
        evidence.clone(),
        Some(judgment("Reason A", DataClassification::Internal)),
    )
    .unwrap();
    let exact = attempt(
        &mut OpaqueIdSource::new(),
        evidence.clone(),
        Some(judgment("Reason A", DataClassification::Internal)),
    )
    .unwrap();
    assert_eq!(first, exact);

    for (label, evidence, j) in [
        (
            "rationale",
            evidence.clone(),
            Some(judgment("Reason B", DataClassification::Internal)),
        ),
        (
            "classification",
            evidence.clone(),
            Some(judgment("Reason A", DataClassification::Confidential)),
        ),
        ("dropped judgment", evidence.clone(), None),
        (
            "evidence",
            vec![EvidenceReferenceId::parse("evidence-other").unwrap()],
            Some(judgment("Reason A", DataClassification::Internal)),
        ),
    ] {
        let refusal = issue_refusal(
            attempt(&mut OpaqueIdSource::new(), evidence, j)
                .expect_err("a changed payload is not the same request"),
        );
        assert_eq!(
            refusal.code(),
            ErrorCode::DomainIdempotencyConflict,
            "changed {label}"
        );
    }
    assert_eq!(
        writer
            .load_issue_h2a_runtime_snapshot()
            .unwrap()
            .prepared()
            .len(),
        1,
        "no conflicting retry prepared anything"
    );
}

#[test]
fn a_risk_that_moved_on_refuses_a_stale_version_at_prepare() {
    let ledger = SyntheticLedger::new();
    let (mut writer, risk_id) = seeded_risk(&ledger);
    let mut ids = OpaqueIdSource::new();

    let refusal = risk_refusal(
        prepare_close_risk(
            &mut writer,
            risk_id,
            AggregateVersion::new(7).unwrap(),
            rationale("The synthetic risk no longer applies."),
            risk_context("prepare-close-stale"),
            &mut ids,
            at(1_000),
        )
        .expect_err("a version the Risk never had cannot be prepared against"),
    );
    assert_ne!(refusal.code(), ErrorCode::PlatformInternal);
}
