//! Durable record-id reservations (schema v47; DG3 record-entry amendment
//! §3.6, §4): one `clientRequestId`, one record id, however many attempts.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{AuditEventId, CorrelationId, IdempotencyId, RiskId};
use pmc_domain::risks::{CreateRisk, RiskDetails, RiskOperationContext, RiskTitle};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    ReservationError, ReservationRequest, ReservedEntityKind, SqliteProductLedger,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-reservation-{nonce}-{sequence}.sqlite3"))
}

const CREATE_RISK: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Risk,
    operation: "create_risk",
};

fn idem(value: &str) -> IdempotencyId {
    IdempotencyId::parse(value).unwrap_or_else(|_| panic!("id"))
}

fn context(request: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: idem(request),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

#[test]
fn a_retry_reads_back_the_id_the_first_attempt_reserved() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let before = ledger.revision().unwrap_or_else(|e| panic!("{e:?}"));
    let mut minted = 0;
    let first = ledger
        .reserve_or_get_record_id(&idem("sheet-1"), CREATE_RISK, at(100), || {
            minted += 1;
            RiskId::parse(format!("risk-minted-{minted}"))
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(first.id().as_str(), "risk-minted-1");
    assert!(!first.replayed());
    // A durable change: the revision moved (product owner 2026-09-22).
    assert_eq!(
        ledger.revision().unwrap_or_else(|e| panic!("{e:?}")),
        before + 1
    );

    // The retry — here after reopening, as after a crash — never mints.
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let again = ledger
        .reserve_or_get_record_id::<RiskId>(&idem("sheet-1"), CREATE_RISK, at(200), || {
            panic!("a reserved id must not be minted again")
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(again.id(), first.id());
    assert!(again.replayed());
    assert_eq!(
        ledger.revision().unwrap_or_else(|e| panic!("{e:?}")),
        before + 1
    );
}

#[test]
fn the_same_request_id_for_another_kind_or_operation_is_a_conflict() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    ledger
        .reserve_or_get_record_id(&idem("sheet-2"), CREATE_RISK, at(100), || {
            RiskId::parse("risk-a")
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let other_kind = ledger.reserve_or_get_record_id(
        &idem("sheet-2"),
        ReservationRequest {
            kind: ReservedEntityKind::Issue,
            operation: "create_issue",
        },
        at(100),
        || pmc_domain::identity::IssueId::parse("issue-a"),
    );
    assert!(matches!(other_kind, Err(ReservationError::Conflict)));
    let other_operation = ledger.reserve_or_get_record_id(
        &idem("sheet-2"),
        ReservationRequest {
            kind: ReservedEntityKind::Risk,
            operation: "something_else",
        },
        at(100),
        || RiskId::parse("risk-b"),
    );
    assert!(matches!(other_operation, Err(ReservationError::Conflict)));
}

#[test]
fn a_request_id_already_spent_by_a_command_cannot_be_reserved() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    ledger
        .create_risk(
            CreateRisk {
                id: RiskId::parse("risk-plain").unwrap_or_else(|_| panic!("id")),
                title: RiskTitle::parse("Synthetic").unwrap_or_else(|_| panic!("text")),
                details: RiskDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
                classification: DataClassification::Internal,
                context: context("spent"),
            },
            AuditEventId::parse("audit-spent").unwrap_or_else(|_| panic!("id")),
            at(100),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    let reserved = ledger.reserve_or_get_record_id(&idem("spent"), CREATE_RISK, at(200), || {
        RiskId::parse("risk-late")
    });
    assert!(matches!(reserved, Err(ReservationError::Conflict)));
}

#[test]
fn an_id_already_in_use_is_minted_again_rather_than_exposed() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    ledger
        .create_risk(
            CreateRisk {
                id: RiskId::parse("risk-taken").unwrap_or_else(|_| panic!("id")),
                title: RiskTitle::parse("Synthetic").unwrap_or_else(|_| panic!("text")),
                details: RiskDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text")),
                classification: DataClassification::Internal,
                context: context("taken"),
            },
            AuditEventId::parse("audit-taken").unwrap_or_else(|_| panic!("id")),
            at(100),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut attempts = 0;
    let reserved = ledger
        .reserve_or_get_record_id(&idem("sheet-3"), CREATE_RISK, at(200), || {
            attempts += 1;
            RiskId::parse(if attempts == 1 {
                "risk-taken"
            } else {
                "risk-fresh"
            })
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(reserved.id().as_str(), "risk-fresh");
    assert_eq!(attempts, 2);
}

#[test]
fn a_create_through_its_reservation_names_the_reserved_risk_and_replays() {
    let path = ledger_path();
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let reserved = ledger
        .reserve_or_get_record_id(&idem("sheet-4"), CREATE_RISK, at(100), || {
            RiskId::parse("risk-reserved")
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let title = RiskTitle::parse("Synthetic").unwrap_or_else(|_| panic!("text"));
    let details = RiskDetails::parse("Synthetic only.").unwrap_or_else(|_| panic!("text"));
    let created = ledger
        .create_risk_from_reservation(
            &reserved,
            title.clone(),
            details.clone(),
            DataClassification::Internal,
            context("sheet-4"),
            AuditEventId::parse("audit-4").unwrap_or_else(|_| panic!("id")),
            at(150),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(created.record.id().as_str(), "risk-reserved");

    // The retry: the same reservation, the same command, the same outcome.
    drop(ledger);
    let mut ledger = SqliteProductLedger::open(&path).unwrap_or_else(|e| panic!("{e:?}"));
    let again = ledger
        .reserve_or_get_record_id::<RiskId>(&idem("sheet-4"), CREATE_RISK, at(300), || {
            panic!("must not mint")
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let replayed = ledger
        .create_risk_from_reservation(
            &again,
            title.clone(),
            details.clone(),
            DataClassification::Internal,
            context("sheet-4"),
            AuditEventId::parse("audit-ignored").unwrap_or_else(|_| panic!("id")),
            at(999),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(replayed, created);

    // A reservation used with another request's context is refused.
    let mismatched = ledger.create_risk_from_reservation(
        &again,
        title.clone(),
        details.clone(),
        DataClassification::Internal,
        context("sheet-other"),
        AuditEventId::parse("audit-5").unwrap_or_else(|_| panic!("id")),
        at(400),
    );
    assert!(mismatched.is_err());

    // A reservation from another Ledger file proves nothing here.
    let mut other = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let foreign = other
        .reserve_or_get_record_id(&idem("sheet-5"), CREATE_RISK, at(100), || {
            RiskId::parse("risk-foreign")
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
    let forged = ledger.create_risk_from_reservation(
        &foreign,
        title,
        details,
        DataClassification::Internal,
        context("sheet-5"),
        AuditEventId::parse("audit-6").unwrap_or_else(|_| panic!("id")),
        at(500),
    );
    assert!(forged.is_err());
    assert!(ledger
        .load_risk_persistence_snapshot()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .risks()
        .iter()
        .all(|risk| risk.id().as_str() != "risk-foreign"));
}
