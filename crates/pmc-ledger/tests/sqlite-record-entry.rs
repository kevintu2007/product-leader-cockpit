//! The Portfolio-family entry repository (slice 6B): a create rests on the
//! Ledger's own reservation and on nothing else, and the entry reads return
//! what a sheet edits, in the order a list shows it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{AuditEventId, CorrelationId, IdempotencyId, PortfolioId, ProductId};
use pmc_domain::portfolio::{LongText, OperationContext, ShortText};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    LedgerTransactionError, SqliteProductLedger, CREATE_PORTFOLIO, CREATE_PRODUCT,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pmc-record-entry-repo-{nonce}-{sequence}.sqlite3"))
}

fn idempotency(value: &str) -> IdempotencyId {
    IdempotencyId::parse(value).unwrap_or_else(|_| panic!("id"))
}

fn context(request: &str) -> OperationContext {
    OperationContext {
        idempotency_id: idempotency(request),
        correlation_id: CorrelationId::parse(format!("corr-{request}"))
            .unwrap_or_else(|_| panic!("id")),
    }
}

fn audit(n: u64) -> AuditEventId {
    AuditEventId::parse(format!("audit-{n}")).unwrap_or_else(|_| panic!("id"))
}

fn short(value: &str) -> ShortText {
    ShortText::parse(value).unwrap_or_else(|_| panic!("text"))
}

fn long(value: &str) -> LongText {
    LongText::parse(value).unwrap_or_else(|_| panic!("text"))
}

#[test]
fn a_create_needs_the_reservation_the_ledger_issued_for_that_request_and_kind() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut n = 0_u64;
    let reserved = ledger
        .reserve_or_get_record_id::<PortfolioId>(
            &idempotency("sheet-1"),
            CREATE_PORTFOLIO,
            UtcTimestamp::from_unix_millis(1),
            || {
                n += 1;
                PortfolioId::parse(format!("portfolio-{n}"))
            },
        )
        .unwrap_or_else(|e| panic!("{e:?}"));

    // The reservation is for "sheet-1"; a create under another request id
    // with the same proof is refused as a reused request.
    let mismatch = ledger.create_portfolio_from_reservation(
        &reserved,
        short("P"),
        long("Details."),
        Some(DataClassification::Internal),
        context("sheet-2"),
        audit(1),
        UtcTimestamp::from_unix_millis(2),
    );
    match mismatch {
        Err(LedgerTransactionError::Operation(error)) => {
            assert_eq!(error.code(), ErrorCode::DomainIdempotencyConflict);
        }
        other => panic!("a mismatched reservation must be refused: {other:?}"),
    }
    assert!(ledger
        .read_portfolio_entry(reserved.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_none());

    // With the matching request it lands, and a replay answers the same.
    let created = ledger
        .create_portfolio_from_reservation(
            &reserved,
            short("P"),
            long("Details."),
            Some(DataClassification::Internal),
            context("sheet-1"),
            audit(2),
            UtcTimestamp::from_unix_millis(3),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(&created.record.id, reserved.id());
    let read = ledger
        .read_portfolio_entry(reserved.id())
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap_or_else(|| panic!("created"));
    assert_eq!(read.name, "P");
    assert_eq!(read.details, "Details.");
    assert_eq!(read.classification, DataClassification::Internal);
    assert_eq!(read.version.get(), 1);
}

#[test]
fn the_lists_come_in_name_order_with_versions_and_a_missing_read_is_none() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap_or_else(|e| panic!("{e:?}"));
    let mut n = 0_u64;
    for (request, name) in [("a", "Zeta"), ("b", "Alpha"), ("c", "Mid")] {
        let reserved = ledger
            .reserve_or_get_record_id::<ProductId>(
                &idempotency(request),
                CREATE_PRODUCT,
                UtcTimestamp::from_unix_millis(1),
                || {
                    n += 1;
                    ProductId::parse(format!("product-{n}"))
                },
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        ledger
            .create_product_from_reservation(
                &reserved,
                short(name),
                long("Synthetic."),
                Some(DataClassification::Public),
                context(request),
                audit(n),
                UtcTimestamp::from_unix_millis(2),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
    }
    let names: Vec<String> = ledger
        .list_product_entries()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .into_iter()
        .map(|record| record.name)
        .collect();
    assert_eq!(names, ["Alpha", "Mid", "Zeta"]);
    assert!(ledger
        .list_portfolio_entries()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_empty());
    assert!(ledger
        .list_roadmap_entries()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_empty());
    assert!(ledger
        .list_kpi_definition_entries()
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_empty());
    assert!(ledger
        .read_product_entry(&ProductId::parse("product-none").unwrap_or_else(|_| panic!("id")))
        .unwrap_or_else(|e| panic!("{e:?}"))
        .is_none());
}
