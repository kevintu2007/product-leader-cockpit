//! The shared record-entry flow (slice 6A): one client request, one record,
//! however many attempts; and the H1 Risk response update through the host's
//! flow.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::desktop_runtime::OpaqueIdSource;
use pmc_application::record_entry::{enter_risk, RecordEntryError, RiskEntry};
use pmc_application::risk_lifecycle::update_risk_response;
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{AggregateVersion, CorrelationId, IdempotencyId};
use pmc_domain::risks::{RiskDetails, RiskOperationContext, RiskTitle, UpdateRiskResponse};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::RiskResponseType;
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-record-entry-{nonce}-{sequence}.sqlite3"))
}

fn context(request: &str, correlation: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(request).unwrap_or_else(|_| panic!("id")),
        correlation_id: CorrelationId::parse(correlation).unwrap_or_else(|_| panic!("id")),
    }
}

fn entry() -> RiskEntry {
    RiskEntry {
        title: RiskTitle::parse("Synthetic risk").unwrap_or_else(|_| panic!("text")),
        details: RiskDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
        classification: DataClassification::Internal,
    }
}

#[test]
fn one_client_request_creates_one_risk_across_attempts_and_launches() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(7);
    let first = enter_risk(
        &mut ledger,
        entry(),
        context("sheet-1", "corr-1"),
        &mut ids,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(first.record.id().as_str().starts_with("risk-"));

    // The retry: another launch (another nonce), another correlation, later.
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let mut later_ids = OpaqueIdSource::with_nonce(9);
    let again = enter_risk(
        &mut ledger,
        entry(),
        context("sheet-1", "corr-2"),
        &mut later_ids,
        UtcTimestamp::from_unix_millis(900),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.record, first.record);
    assert_eq!(
        ledger
            .load_risk_persistence_snapshot()
            .unwrap_or_else(|e| panic!("{e:?}"))
            .risks()
            .len(),
        1
    );

    // The same request with changed fields is the sheet's mistake: refused,
    // and still one Risk.
    let mut changed = entry();
    changed.title = RiskTitle::parse("Another title").unwrap_or_else(|_| panic!("text"));
    let refused = enter_risk(
        &mut ledger,
        changed,
        context("sheet-1", "corr-3"),
        &mut later_ids,
        UtcTimestamp::from_unix_millis(950),
    );
    match refused {
        Err(error) => assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict),
        Ok(_) => panic!("a changed command under the same request id must be refused"),
    }
}

#[test]
fn a_request_id_spent_on_another_kind_is_reported_as_reused() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(1);
    ledger
        .reserve_or_get_record_id(
            &IdempotencyId::parse("sheet-x").unwrap_or_else(|_| panic!("id")),
            pmc_ledger::sqlite::ReservationRequest {
                kind: pmc_ledger::sqlite::ReservedEntityKind::Issue,
                operation: "create_issue",
            },
            UtcTimestamp::from_unix_millis(1),
            || ids.next_issue_id(),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    let refused = enter_risk(
        &mut ledger,
        entry(),
        context("sheet-x", "corr"),
        &mut ids,
        UtcTimestamp::from_unix_millis(2),
    );
    assert!(matches!(refused, Err(RecordEntryError::RequestReused)));
}

#[test]
fn the_host_flow_updates_a_risk_response_and_replays_it() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut ids = OpaqueIdSource::with_nonce(3);
    let created = enter_risk(
        &mut ledger,
        entry(),
        context("sheet-r", "corr-r"),
        &mut ids,
        UtcTimestamp::from_unix_millis(100),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let command = UpdateRiskResponse {
        risk_id: created.record.id().clone(),
        expected_version: AggregateVersion::initial(),
        response: RiskResponseType::Mitigate,
        owner: None,
        rationale: None,
        residual_exposure: None,
        next_review_at: None,
        context: context("sheet-u", "corr-u"),
    };
    let updated = update_risk_response(
        &mut ledger,
        command.clone(),
        &mut ids,
        UtcTimestamp::from_unix_millis(200),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(updated.record.version().get(), 2);
    assert_eq!(updated.record.response(), Some(RiskResponseType::Mitigate));
    let replayed = update_risk_response(
        &mut ledger,
        command,
        &mut ids,
        UtcTimestamp::from_unix_millis(300),
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(replayed, updated);
}
