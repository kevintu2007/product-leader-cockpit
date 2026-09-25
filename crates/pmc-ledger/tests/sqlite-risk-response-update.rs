//! `UpdateRiskResponse` persisted (schema v47): the response columns, the
//! version, the typed replay that outlives later changes, and a Risk that
//! carries a response still rehydrating through every loader.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, PreparedIntentId, RiskId,
    StakeholderId,
};
use pmc_domain::provenance::Provenance;
use pmc_domain::relationships::{CreateStakeholder, StakeholderKind, StakeholderName};
use pmc_domain::risks::{
    CreateRisk, InMemoryRiskService, RecordedRiskClassification, RiskDetails, RiskOperationContext,
    RiskTitle, UpdateRiskResponse,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::work_management::{
    RiskResponseType, RiskState, WorkManagementOperation, WorkManagementPreparedIntent,
    WorkManagementRationale,
};
use pmc_ledger::sqlite::{LedgerTransactionError, SqliteProductLedger};
use rusqlite::Connection;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-risk-response-{nonce}-{sequence}.sqlite3"))
}

fn context(request: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn owner() -> StakeholderId {
    StakeholderId::parse("owner-1").unwrap_or_else(|_| panic!("id"))
}

/// A Ledger with one Open Risk (version 1) and one Stakeholder to own it.
fn scene() -> (PathBuf, SqliteProductLedger, RiskId) {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    ledger
        .create_stakeholder(
            CreateStakeholder {
                id: owner(),
                name: StakeholderName::parse("Synthetic owner").unwrap_or_else(|_| panic!("text")),
                kind: StakeholderKind::Person,
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: pmc_domain::relationships::OperationContext {
                    idempotency_id: IdempotencyId::parse("owner-create")
                        .unwrap_or_else(|_| panic!("id")),
                    correlation_id: CorrelationId::parse("owner-corr")
                        .unwrap_or_else(|_| panic!("id")),
                },
            },
            AuditEventId::parse("audit-owner").unwrap_or_else(|_| panic!("id")),
            at(50),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    let risk_id = RiskId::parse("risk-1").unwrap_or_else(|_| panic!("id"));
    ledger
        .create_risk(
            CreateRisk {
                id: risk_id.clone(),
                title: RiskTitle::parse("Synthetic risk").unwrap_or_else(|_| panic!("text")),
                details: RiskDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
                classification: DataClassification::Internal,
                context: context("create"),
            },
            AuditEventId::parse("audit-create").unwrap_or_else(|_| panic!("id")),
            at(100),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    (path, ledger, risk_id)
}

fn accept(risk_id: &RiskId, version: u64, request: &str) -> UpdateRiskResponse {
    UpdateRiskResponse {
        risk_id: risk_id.clone(),
        expected_version: AggregateVersion::new(version).unwrap_or_else(|_| panic!("version")),
        response: RiskResponseType::Accept,
        owner: Some(owner()),
        rationale: Some(
            pmc_domain::risks::RiskRationale::parse("Accepted for now.")
                .unwrap_or_else(|_| panic!("text")),
        ),
        residual_exposure: Some(
            pmc_domain::risks::ResidualExposure::parse("Low").unwrap_or_else(|_| panic!("text")),
        ),
        next_review_at: Some(at(5_000)),
        context: context(request),
    }
}

fn mitigate(risk_id: &RiskId, version: u64, request: &str) -> UpdateRiskResponse {
    UpdateRiskResponse {
        risk_id: risk_id.clone(),
        expected_version: AggregateVersion::new(version).unwrap_or_else(|_| panic!("version")),
        response: RiskResponseType::Mitigate,
        owner: None,
        rationale: None,
        residual_exposure: None,
        next_review_at: None,
        context: context(request),
    }
}

fn row(path: &PathBuf, sql: &str) -> (Option<String>, Option<String>, i64, i64) {
    Connection::open(path)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .query_row(sql, [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn an_accepted_response_is_stored_versioned_audited_and_reloads() {
    let (path, mut ledger, risk_id) = scene();
    let revision = ledger.revision().unwrap_or_else(|e| panic!("{e:?}"));
    let outcome = ledger
        .update_risk_response(
            accept(&risk_id, 1, "accept-1"),
            AuditEventId::parse("audit-accept").unwrap_or_else(|_| panic!("id")),
            at(200),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(outcome.record.version().get(), 2);
    assert_eq!(outcome.record.response(), Some(RiskResponseType::Accept));
    assert_eq!(outcome.record.owner(), Some(&owner()));
    assert!(!outcome.record.in_exception_queue());
    assert_eq!(outcome.record.state(), RiskState::Open);
    assert_eq!(outcome.audit_events.len(), 1);
    assert_eq!(
        outcome.audit_events[0].code().as_str(),
        "risk.response_updated"
    );
    assert_eq!(
        ledger.revision().unwrap_or_else(|e| panic!("{e:?}")),
        revision + 1
    );
    let stored = row(
        &path,
        "SELECT response,owner_id,in_exception_queue,(SELECT version FROM aggregate_registry WHERE id='risk-1') FROM risks WHERE id='risk-1'",
    );
    assert_eq!(
        stored,
        (Some("accept".to_owned()), Some("owner-1".to_owned()), 0, 2)
    );

    // Both loaders rehydrate the responded Risk from its typed replay rows.
    drop(ledger);
    let ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let snapshot = ledger
        .load_risk_persistence_snapshot()
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(snapshot.risks(), &[outcome.record.clone()]);
    // The H2a runtime loader accepts it too.
    ledger
        .load_risk_h2a_runtime_snapshot()
        .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn every_response_type_is_accepted_and_only_accept_or_transfer_leave_the_queue() {
    let (_path, mut ledger, risk_id) = scene();
    let mut version = 1;
    for (index, response) in [
        RiskResponseType::Mitigate,
        RiskResponseType::Avoid,
        RiskResponseType::Transfer,
        RiskResponseType::Accept,
    ]
    .into_iter()
    .enumerate()
    {
        let mut command = accept(&risk_id, version, &format!("response-{index}"));
        command.response = response;
        let outcome = ledger
            .update_risk_response(
                command,
                AuditEventId::parse(format!("audit-response-{index}"))
                    .unwrap_or_else(|_| panic!("id")),
                at(200 + i64::try_from(index).unwrap_or(0)),
            )
            .unwrap_or_else(|e| panic!("{response:?}: {e:?}"));
        version += 1;
        assert_eq!(outcome.record.version().get(), version);
        assert_eq!(
            outcome.record.in_exception_queue(),
            !matches!(
                response,
                RiskResponseType::Accept | RiskResponseType::Transfer
            ),
            "{response:?}"
        );
    }
    // After four updates the Risk still rehydrates, at version 5.
    let snapshot = ledger
        .load_risk_persistence_snapshot()
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(snapshot.risks()[0].version().get(), 5);
}

#[test]
fn the_domain_and_the_ledger_refuse_the_same_commands() {
    let (_path, mut ledger, risk_id) = scene();
    let refused =
        |result: Result<_, LedgerTransactionError<pmc_domain::error::DomainError>>| match result {
            Err(LedgerTransactionError::Operation(error)) => error.code(),
            other => panic!("expected a domain refusal, got {other:?}"),
        };
    // Accept without every attribute.
    let mut incomplete = accept(&risk_id, 1, "incomplete");
    incomplete.owner = None;
    assert_eq!(
        refused(ledger.update_risk_response(
            incomplete.clone(),
            AuditEventId::parse("audit-x1").unwrap_or_else(|_| panic!("id")),
            at(200)
        )),
        ErrorCode::ValidationInvalidField
    );
    // The in-memory service says the same.
    let mut service = InMemoryRiskService::new(
        FixedClock(at(200)),
        Ids(0),
        Gate,
        Gate,
        pmc_domain::risks::AllowRiskEvidence,
        RecordedRiskClassification,
    );
    service
        .create_risk(CreateRisk {
            id: risk_id.clone(),
            title: RiskTitle::parse("Synthetic risk").unwrap_or_else(|_| panic!("text")),
            details: RiskDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
            classification: DataClassification::Internal,
            context: context("create"),
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(
        service.update_risk_response(incomplete).unwrap_err().code(),
        ErrorCode::ValidationInvalidField
    );
    // A stale version, an unknown Risk, an unknown owner.
    assert_eq!(
        refused(ledger.update_risk_response(
            accept(&risk_id, 2, "stale"),
            AuditEventId::parse("audit-x2").unwrap_or_else(|_| panic!("id")),
            at(200)
        )),
        ErrorCode::DomainConflict
    );
    assert_eq!(
        refused(ledger.update_risk_response(
            accept(
                &RiskId::parse("risk-none").unwrap_or_else(|_| panic!("id")),
                1,
                "none"
            ),
            AuditEventId::parse("audit-x3").unwrap_or_else(|_| panic!("id")),
            at(200)
        )),
        ErrorCode::DomainNotFound
    );
    let mut stranger = accept(&risk_id, 1, "stranger");
    stranger.owner = Some(StakeholderId::parse("owner-none").unwrap_or_else(|_| panic!("id")));
    assert_eq!(
        refused(ledger.update_risk_response(
            stranger,
            AuditEventId::parse("audit-x4").unwrap_or_else(|_| panic!("id")),
            at(200)
        )),
        ErrorCode::DomainNotFound
    );
    // Nothing above changed the Risk.
    assert_eq!(
        ledger
            .load_risk_persistence_snapshot()
            .unwrap_or_else(|e| panic!("{e:?}"))
            .risks()[0]
            .version()
            .get(),
        1
    );
}

#[test]
fn a_replay_returns_the_first_outcome_however_the_risk_moved_since() {
    let (path, mut ledger, risk_id) = scene();
    let first = ledger
        .update_risk_response(
            mitigate(&risk_id, 1, "first"),
            AuditEventId::parse("audit-first").unwrap_or_else(|_| panic!("id")),
            at(200),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    // The Risk moves on: a second update.
    ledger
        .update_risk_response(
            accept(&risk_id, 2, "second"),
            AuditEventId::parse("audit-second").unwrap_or_else(|_| panic!("id")),
            at(300),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    // The same first command again, even with a fresh correlation and audit
    // id, returns what the first attempt committed.
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut retry = mitigate(&risk_id, 1, "first");
    retry.context.correlation_id =
        CorrelationId::parse("corr-retry").unwrap_or_else(|_| panic!("id"));
    let replayed = ledger
        .update_risk_response(
            retry,
            AuditEventId::parse("audit-ignored").unwrap_or_else(|_| panic!("id")),
            at(999),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(replayed, first);
    assert_eq!(replayed.record.version().get(), 2);
    assert_eq!(
        replayed.audit_events[0].correlation_id().as_str(),
        "corr-first"
    );
    // The same id with a changed field is a conflict, as is an id another
    // command spent.
    let mut changed = mitigate(&risk_id, 1, "first");
    changed.response = RiskResponseType::Avoid;
    assert!(matches!(
        ledger.update_risk_response(changed, AuditEventId::parse("audit-c").unwrap_or_else(|_| panic!("id")), at(400)),
        Err(LedgerTransactionError::Operation(error)) if error.code() == ErrorCode::DomainIdempotencyConflict
    ));
    assert!(matches!(
        ledger.update_risk_response(accept(&risk_id, 3, "create"), AuditEventId::parse("audit-d").unwrap_or_else(|_| panic!("id")), at(400)),
        Err(LedgerTransactionError::Operation(error)) if error.code() == ErrorCode::DomainIdempotencyConflict
    ));
}

#[test]
fn a_responded_risk_can_still_be_closed_and_rehydrates_afterwards() {
    let (path, mut ledger, risk_id) = scene();
    ledger
        .update_risk_response(
            accept(&risk_id, 1, "accept"),
            AuditEventId::parse("audit-accept").unwrap_or_else(|_| panic!("id")),
            at(200),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    // The shipped H2a close, prepared and executed on version 2.
    let rationale =
        WorkManagementRationale::parse("Done with it.").unwrap_or_else(|_| panic!("text"));
    let version = AggregateVersion::new(2).unwrap_or_else(|_| panic!("version"));
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("prepared-close").unwrap_or_else(|_| panic!("id")),
        WorkManagementOperation::CloseRisk {
            risk_id: risk_id.clone(),
            risk_version: version,
            rationale: rationale.clone(),
        },
        DataClassification::Internal,
        None,
        at(300),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    ledger
        .prepare_close_risk(
            pmc_domain::risks::PrepareCloseRisk {
                risk_id: risk_id.clone(),
                expected_version: version,
                rationale,
                context: context("prepare-close"),
            },
            prepared.clone(),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    let approval = pmc_domain::work_management::WorkManagementApproval::new(
        prepared.id().clone(),
        pmc_domain::audit::AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("execute-close").unwrap_or_else(|_| panic!("id")),
        Some(pmc_domain::work_management::ApprovalConfirmation::Confirmed),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let closed = ledger
        .approve_and_execute_close_risk(
            pmc_domain::risks::ApproveAndExecuteCloseRisk {
                approval,
                context: context("execute-close"),
            },
            AuditEventId::parse("audit-close").unwrap_or_else(|_| panic!("id")),
            pmc_domain::identity::ApprovalReceiptId::parse("receipt-close")
                .unwrap_or_else(|_| panic!("id")),
            at(400),
            Gate,
            Gate,
            pmc_domain::risks::AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(closed.record.state(), RiskState::Closed);
    assert_eq!(closed.record.version().get(), 3);
    assert_eq!(closed.record.response(), Some(RiskResponseType::Accept));
    assert!(!closed.record.in_exception_queue());
    drop(ledger);
    let ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let snapshot = ledger
        .load_risk_persistence_snapshot()
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(snapshot.risks(), &[closed.record]);
    ledger
        .load_risk_h2a_runtime_snapshot()
        .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn a_risk_row_whose_response_has_no_replay_row_does_not_load() {
    let (path, ledger, _risk_id) = scene();
    drop(ledger);
    Connection::open(&path)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .execute_batch("UPDATE risks SET response='accept' WHERE id='risk-1';")
        .unwrap_or_else(|e| panic!("{e:?}"));
    let ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(ledger.load_risk_persistence_snapshot().is_err());
}

#[derive(Clone, Copy)]
struct FixedClock(UtcTimestamp);
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

struct Ids(u64);
impl pmc_domain::risks::RiskServiceIdSource for Ids {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<pmc_domain::identity::ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0 += 1;
        pmc_domain::identity::ApprovalReceiptId::parse(format!("receipt-{}", self.0))
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("audit-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct Gate;
impl pmc_domain::work_management::ApprovalAuthorizationPort for Gate {
    fn authorize(&self, actor: pmc_domain::audit::AuditActor) -> bool {
        actor == pmc_domain::audit::AuditActor::HeadOfProducts
    }
}
impl pmc_domain::risks::RiskExecutionPolicyPort for Gate {
    fn current_policy(
        &self,
        _: &WorkManagementOperation,
    ) -> pmc_domain::risks::RiskExecutionPolicy {
        pmc_domain::risks::RiskExecutionPolicy::Allowed
    }
}

#[test]
fn a_response_result_that_lost_its_command_or_was_renumbered_does_not_load() {
    for tamper in [
        "DELETE FROM risk_response_command_updates WHERE idempotency_id='accept-1'; DELETE FROM risk_response_replay_audits WHERE idempotency_id='accept-1';",
        "UPDATE risk_response_replay_operations SET result_version=99 WHERE idempotency_id='accept-1'; UPDATE aggregate_registry SET version=99 WHERE id='risk-1';",
        "UPDATE risk_response_command_updates SET response='mitigate' WHERE idempotency_id='accept-1';",
        "DELETE FROM ledger_idempotency_claims WHERE idempotency_id='accept-1';",
    ] {
        let (path, mut ledger, risk_id) = scene();
        ledger
            .update_risk_response(
                accept(&risk_id, 1, "accept-1"),
                AuditEventId::parse("audit-accept").unwrap_or_else(|_| panic!("id")),
                at(200),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        drop(ledger);
        Connection::open(&path)
            .unwrap_or_else(|e| panic!("{e:?}"))
            .execute_batch(&format!("PRAGMA foreign_keys=OFF; {tamper}"))
            .unwrap_or_else(|e| panic!("{tamper}: {e:?}"));
        let ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            ledger.load_risk_persistence_snapshot().is_err(),
            "{tamper} must not decode"
        );
        // The replay authority is gone with it.
        let mut ledger = ledger;
        assert!(ledger
            .update_risk_response(
                accept(&risk_id, 1, "accept-1"),
                AuditEventId::parse("audit-again").unwrap_or_else(|_| panic!("id")),
                at(300),
            )
            .is_err());
    }
}

#[test]
fn a_next_review_before_the_epoch_is_a_field_refusal() {
    let (_path, mut ledger, risk_id) = scene();
    let mut command = accept(&risk_id, 1, "epoch");
    command.next_review_at = Some(at(-1));
    match ledger.update_risk_response(
        command,
        AuditEventId::parse("audit-epoch").unwrap_or_else(|_| panic!("id")),
        at(200),
    ) {
        Err(LedgerTransactionError::Operation(error)) => {
            assert_eq!(error.code(), ErrorCode::ValidationInvalidField);
        }
        other => panic!("expected a field refusal, got {other:?}"),
    }
}
