//! The authority inventory an Operational Backup carries, and the pristine
//! predicate the S7 bootstrap rests on.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    classification::DataClassification,
    identity::{AuditEventId, CorrelationId, IdempotencyId, PortfolioId},
    portfolio::{CreatePortfolio, LongText, OperationContext, ShortText, UpdatePortfolioDetails},
    provenance::Provenance,
    time::UtcTimestamp,
};
use pmc_ledger::sqlite::{SqliteProductLedger, AUTHORITY_INVENTORY_HEADER};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

fn ledger_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("synthetic test clock must follow the Unix epoch")
        .as_nanos();
    let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "pmc-synthetic-authority-inventory-{nonce}-{sequence}.sqlite3"
    ))
}

fn context(suffix: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(format!("synthetic-inventory-{suffix}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("synthetic-inventory-corr-{suffix}")).unwrap(),
    }
}

fn create(ledger: &mut SqliteProductLedger, id: &str) {
    ledger
        .create_portfolio(
            CreatePortfolio {
                id: PortfolioId::parse(id).unwrap(),
                name: ShortText::parse("Synthetic Portfolio").unwrap(),
                details: LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                provenance: Provenance::UserEntered,
                context: context(id),
            },
            AuditEventId::parse(format!("synthetic-audit-{id}")).unwrap(),
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap();
}

#[test]
fn a_new_ledger_is_pristine_and_its_inventory_is_only_the_header() {
    let ledger = SqliteProductLedger::open(ledger_path()).unwrap();
    assert!(ledger.is_pristine_authority().unwrap());
    let inventory = ledger.authority_inventory().unwrap();
    assert_eq!(inventory.bytes, AUTHORITY_INVENTORY_HEADER.as_bytes());
    assert_eq!(inventory.record_count, 0);
    assert_eq!(inventory.sha256.len(), 64);
}

#[test]
fn one_record_ends_the_pristine_state_and_appears_in_the_inventory() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap();
    create(&mut ledger, "synthetic-portfolio-b");
    create(&mut ledger, "synthetic-portfolio-a");
    assert!(!ledger.is_pristine_authority().unwrap());
    let inventory = ledger.authority_inventory().unwrap();
    assert_eq!(inventory.record_count, 2);
    let text = String::from_utf8(inventory.bytes).unwrap();
    assert_eq!(
        text,
        format!(
            "{AUTHORITY_INVENTORY_HEADER}portfolio\tsynthetic-portfolio-a\t1\n\
             portfolio\tsynthetic-portfolio-b\t1\n"
        )
    );
}

#[test]
fn an_edit_changes_the_inventory_digest() {
    let mut ledger = SqliteProductLedger::open(ledger_path()).unwrap();
    create(&mut ledger, "synthetic-portfolio-c");
    let before = ledger.authority_inventory().unwrap();
    ledger
        .update_portfolio_details(
            UpdatePortfolioDetails {
                id: PortfolioId::parse("synthetic-portfolio-c").unwrap(),
                expected_version: pmc_domain::identity::AggregateVersion::new(1).unwrap(),
                name: ShortText::parse("Renamed").unwrap(),
                details: LongText::parse("Synthetic only.").unwrap(),
                classification: Some(DataClassification::Internal),
                context: context("edit-c"),
            },
            AuditEventId::parse("synthetic-audit-edit-c").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
    let after = ledger.authority_inventory().unwrap();
    assert_eq!(after.record_count, 1);
    assert_ne!(before.sha256, after.sha256);
    assert!(String::from_utf8(after.bytes)
        .unwrap()
        .ends_with("synthetic-portfolio-c\t2\n"));
}
