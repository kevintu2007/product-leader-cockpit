//! The Action Request facade against a real SQLite Ledger: the desktop's
//! first write flow, end to end below the IPC boundary.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_application::action_requests::{
    approve_and_execute_accept_action_request, decline_action_request,
    prepare_accept_action_request, reject_action_prepared_intent, start_action,
    ActionRequestFlowError,
};
use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_domain::actions::{
    ActionDetails, ActionOperationContext, ActionTitle, CreateActionRequestDraft,
    DeclineActionRequest, PrepareAcceptActionRequest, StartAction, SubmitActionRequest,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    ActionRequestId, AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, StakeholderId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, OperationContext as RelationshipContext, StakeholderKind, StakeholderName,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, ActionState, WorkManagementOperation, WorkManagementPayloadDigest,
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
            "pmc-synthetic-action-request-facade-{nonce}-{sequence}.sqlite3"
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
        idempotency_id: IdempotencyId::parse(format!("facade-idem-{key}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("facade-corr-{key}")).unwrap(),
    }
}

/// A Ledger with one owner and one submitted (Open, version 2) Action
/// Request, the state the seed leaves six of.
fn seeded_open_request(ledger: &SyntheticLedger) -> (SqliteProductLedger, ActionRequestId) {
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
                    idempotency_id: IdempotencyId::parse("facade-idem-owner").unwrap(),
                    correlation_id: CorrelationId::parse("facade-corr-owner").unwrap(),
                },
            },
            AuditEventId::parse("facade-audit-owner").unwrap(),
            UtcTimestamp::from_unix_millis(50),
        )
        .unwrap();
    let request_id = ActionRequestId::parse("request-1").unwrap();
    writer
        .create_action_request_draft(
            CreateActionRequestDraft {
                id: request_id.clone(),
                title: ActionTitle::parse("Synthetic facade request").unwrap(),
                details: ActionDetails::parse("Accept me through the facade.").unwrap(),
                intended_owner: Some(StakeholderId::parse("stakeholder-owner-1").unwrap()),
                response_due_at: Some(UtcTimestamp::from_unix_millis(200_000)),
                intended_action_due_at: Some(UtcTimestamp::from_unix_millis(300_000)),
                classification: DataClassification::Internal,
                context: context("create"),
            },
            AuditEventId::parse("facade-audit-create").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    writer
        .submit_action_request(
            SubmitActionRequest {
                request_id: request_id.clone(),
                expected_version: AggregateVersion::initial(),
                context: context("submit"),
            },
            AuditEventId::parse("facade-audit-submit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .unwrap();
    (writer, request_id)
}

#[test]
fn prepare_then_approve_accepts_the_request_and_creates_an_action_the_host_can_start() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xfacade);
    let now = UtcTimestamp::from_unix_millis(1_000);

    let prepared = prepare_accept_action_request(
        &mut writer,
        PrepareAcceptActionRequest {
            request_id: request_id.clone(),
            expected_version: AggregateVersion::new(2).unwrap(),
            context: context("prepare"),
        },
        &mut ids,
        now,
    )
    .expect("an Open request at its current version prepares");

    // The canonical preview: the domain built it, the Ledger persisted it.
    assert!(matches!(
        prepared.operation(),
        WorkManagementOperation::AcceptActionRequest { request_id: id, .. } if *id == request_id
    ));
    assert_eq!(
        prepared.preview().expires_at(),
        UtcTimestamp::from_unix_millis(1_000 + 300_000)
    );

    let outcome = approve_and_execute_accept_action_request(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("execute"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .expect("the acknowledged digest executes");
    assert_eq!(outcome.request.state(), ActionRequestState::Accepted);
    assert_eq!(outcome.action.state(), ActionState::Open);
    assert_eq!(outcome.audit_events.len(), 3);

    // An exact replay under the same idempotency id returns the original.
    let replay = approve_and_execute_accept_action_request(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("execute"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    )
    .expect("an exact replay returns the original outcome");
    assert_eq!(replay.action.id(), outcome.action.id());
    assert_eq!(replay.approval_receipt_id, outcome.approval_receipt_id);

    let started = start_action(
        &mut writer,
        StartAction {
            action_id: outcome.action.id().clone(),
            expected_version: outcome.action.version(),
            context: context("start"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(3_000),
    )
    .expect("an Open Action starts");
    assert_eq!(started.record.state(), ActionState::InProgress);
}

#[test]
fn a_wrong_acknowledged_digest_executes_nothing() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xd1);
    let prepared = prepare_accept_action_request(
        &mut writer,
        PrepareAcceptActionRequest {
            request_id: request_id.clone(),
            expected_version: AggregateVersion::new(2).unwrap(),
            context: context("prepare"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    let revision_before = writer.revision().unwrap();

    let wrong = WorkManagementPayloadDigest::from_persisted("f".repeat(64)).unwrap();
    let result = approve_and_execute_accept_action_request(
        &mut writer,
        prepared.id().clone(),
        wrong,
        context("execute-wrong"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2_000),
    );

    assert!(result.is_err(), "{result:?}");
    // The Ledger records the denial durably (an H3 terminal), so the
    // revision may advance; what must not happen is an accepted request.
    let snapshot = writer.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().is_empty(), "no Action may exist");
    let _ = revision_before;
}

#[test]
fn a_stale_expected_version_is_refused_by_the_domain_before_any_write() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0x5a1e);
    let revision_before = writer.revision().unwrap();

    let result = prepare_accept_action_request(
        &mut writer,
        PrepareAcceptActionRequest {
            request_id,
            expected_version: AggregateVersion::new(1).unwrap(),
            context: context("prepare-stale"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_000),
    );

    assert!(
        matches!(result, Err(ActionRequestFlowError::Domain(_))),
        "{result:?}"
    );
    assert_eq!(writer.revision().unwrap(), revision_before);
}

#[test]
fn declining_is_a_plain_h1_write_with_a_rationale() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xdec);

    let declined = decline_action_request(
        &mut writer,
        DeclineActionRequest {
            request_id,
            expected_version: AggregateVersion::new(2).unwrap(),
            rationale: ActionDetails::parse("Not this quarter.").unwrap(),
            context: context("decline"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .expect("an Open request declines");

    assert_eq!(declined.record.state(), ActionRequestState::Declined);
}

#[test]
fn a_ledger_refusal_surfaces_as_the_ledger_variant() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::with_nonce(0xbad);

    // Declining twice: the second is a stale version at the Ledger.
    decline_action_request(
        &mut writer,
        DeclineActionRequest {
            request_id: request_id.clone(),
            expected_version: AggregateVersion::new(2).unwrap(),
            rationale: ActionDetails::parse("Not this quarter.").unwrap(),
            context: context("decline-1"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    let result = decline_action_request(
        &mut writer,
        DeclineActionRequest {
            request_id,
            expected_version: AggregateVersion::new(2).unwrap(),
            rationale: ActionDetails::parse("Again.").unwrap(),
            context: context("decline-2"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_100),
    );

    assert!(
        matches!(
            result,
            Err(ActionRequestFlowError::Ledger(
                LedgerTransactionError::Operation(_)
            ))
        ),
        "{result:?}"
    );
}

#[test]
fn rejecting_a_prepared_accept_is_durable_replays_and_blocks_a_later_approval() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::new();
    let prepared = prepare_accept_action_request(
        &mut writer,
        PrepareAcceptActionRequest {
            request_id,
            expected_version: AggregateVersion::initial().next().unwrap(),
            context: context("prepare"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();

    let rejected = reject_action_prepared_intent(
        &mut writer,
        prepared.id().clone(),
        context("reject"),
        &mut ids,
        UtcTimestamp::from_unix_millis(1_100),
    )
    .unwrap();
    assert_eq!(rejected.prepared_intent_id(), prepared.id());
    assert_eq!(
        rejected.rejected_at(),
        UtcTimestamp::from_unix_millis(1_100)
    );
    assert!(!rejected.expired_at_rejection());
    assert_eq!(
        rejected.audit_event().code().as_str(),
        "action.prepared_rejected"
    );

    // Same context: the same durable outcome, nothing new written.
    assert_eq!(
        reject_action_prepared_intent(
            &mut writer,
            prepared.id().clone(),
            context("reject"),
            &mut ids,
            UtcTimestamp::from_unix_millis(9_000),
        )
        .unwrap(),
        rejected
    );

    let error = approve_and_execute_accept_action_request(
        &mut writer,
        prepared.id().clone(),
        prepared.payload_digest().clone(),
        context("approve-after-reject"),
        &mut ids,
        UtcTimestamp::from_unix_millis(1_200),
    )
    .unwrap_err();
    let ActionRequestFlowError::Ledger(LedgerTransactionError::Operation(domain)) = error else {
        panic!("approving a rejected preview must be a domain refusal");
    };
    assert_eq!(domain.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    let snapshot = writer.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.prepared().is_empty());
    assert!(snapshot.actions().is_empty());
}

#[test]
fn approving_with_a_digest_the_person_did_not_acknowledge_is_refused_without_effect() {
    let ledger = SyntheticLedger::new();
    let (mut writer, request_id) = seeded_open_request(&ledger);
    let mut ids = OpaqueIdSource::new();
    let prepared = prepare_accept_action_request(
        &mut writer,
        PrepareAcceptActionRequest {
            request_id,
            expected_version: AggregateVersion::initial().next().unwrap(),
            context: context("prepare-digest"),
        },
        &mut ids,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    let wrong = WorkManagementPayloadDigest::from_persisted("0".repeat(64)).unwrap();
    assert_ne!(&wrong, prepared.payload_digest());
    let error = approve_and_execute_accept_action_request(
        &mut writer,
        prepared.id().clone(),
        wrong,
        context("approve-wrong-digest"),
        &mut ids,
        UtcTimestamp::from_unix_millis(1_100),
    )
    .unwrap_err();
    let ActionRequestFlowError::Ledger(LedgerTransactionError::Operation(domain)) = error else {
        panic!("a digest mismatch must be a domain refusal");
    };
    assert_eq!(domain.code(), ErrorCode::SecurityPreviewExpiredOrChanged);
    let snapshot = writer.load_action_persistence_snapshot().unwrap();
    assert!(snapshot.actions().is_empty());
    assert_eq!(snapshot.requests()[0].state(), ActionRequestState::Open);
}
