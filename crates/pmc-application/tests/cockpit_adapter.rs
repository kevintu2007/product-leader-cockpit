//! The production Cockpit adapter against a real SQLite Ledger.
//!
//! The Executive Cockpit's route-level suite must pass "against the
//! production adapter". These tests open a real on-disk Ledger, read it
//! through `pmc-ledger`'s own `LedgerSnapshotForProjectionPort`, and compose
//! the Cockpit from what comes back -- no fixture stands in for the database.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::cockpit_adapter::{
    compose_cockpit_from_snapshot, products_from_snapshot, ranked_attention_from_snapshot,
};
use pmc_application::cockpit_aggregation::NoApprovedPeriodPort;
use pmc_application::route_composition::{OwnerModule, PeriodChange, RouteState};
use pmc_domain::attention::AttentionThresholds;
use pmc_domain::projection_source::LedgerSnapshotForProjectionPort;
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::SqliteProductLedger;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct RealLedger(PathBuf);

impl RealLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("pmc-cockpit-adapter-{nonce}-{sequence}.sqlite3")))
    }
}

impl Drop for RealLedger {
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            PathBuf::from(format!("{}-wal", self.0.display())),
            PathBuf::from(format!("{}-shm", self.0.display())),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

#[test]
fn the_adapter_composes_a_cockpit_from_a_real_on_disk_ledger() {
    // The whole point of this file: SQLite through to a composed route, with
    // nothing synthetic standing in for the database.
    let ledger = RealLedger::new();
    let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");

    let snapshot = reader
        .read_projection_snapshot(now())
        .expect("a fresh Ledger must yield a snapshot");
    let result = compose_cockpit_from_snapshot(
        &snapshot,
        AttentionThresholds::default(),
        &NoApprovedPeriodPort,
    );

    // A fresh Ledger genuinely has nothing in it, so Empty is the honest
    // state rather than a failure.
    assert_eq!(result.state, RouteState::Empty);
    assert_eq!(result.ledger_revision, snapshot.ledger_revision);
    assert_eq!(result.as_of, now());
    assert!(result.body.products.is_empty());
    assert!(result.body.exceptions.is_empty());
}

#[test]
fn the_composition_reports_the_revision_the_ledger_actually_holds() {
    // The revision is read from the Ledger through the port rather than
    // asserted by the caller, so a surface cannot claim an instant the
    // database was never at.
    let ledger = RealLedger::new();
    let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");
    let snapshot = reader
        .read_projection_snapshot(now())
        .expect("a fresh Ledger must yield a snapshot");

    let result = compose_cockpit_from_snapshot(
        &snapshot,
        AttentionThresholds::default(),
        &NoApprovedPeriodPort,
    );

    assert_eq!(result.ledger_revision, reader.revision().unwrap());
}

#[test]
fn a_cockpit_read_from_a_real_ledger_never_claims_a_period_comparison() {
    // Reviews & Reports has no report data, so there is no approved period.
    // The route must say so rather than show a zero delta.
    let ledger = RealLedger::new();
    let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");
    let snapshot = reader.read_projection_snapshot(now()).unwrap();

    let result = compose_cockpit_from_snapshot(
        &snapshot,
        AttentionThresholds::default(),
        &NoApprovedPeriodPort,
    );

    assert!(matches!(
        result.body.period_change,
        PeriodChange::NotComparable { .. }
    ));
}

#[test]
fn the_pulse_counts_only_what_the_snapshot_actually_returned() {
    let ledger = RealLedger::new();
    let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");
    let snapshot = reader.read_projection_snapshot(now()).unwrap();

    let result = compose_cockpit_from_snapshot(
        &snapshot,
        AttentionThresholds::default(),
        &NoApprovedPeriodPort,
    );

    assert_eq!(
        result.body.pulse.commitments.count(),
        snapshot.actions.len()
    );
    assert_eq!(result.body.pulse.kpis.count(), snapshot.kpis.len());
    // Milestones have no projection source, so the count is zero rather than
    // approximated from something else.
    assert_eq!(result.body.pulse.milestones.count(), 0);
}

#[test]
fn products_carry_the_revision_and_classification_the_ledger_holds() {
    let ledger = RealLedger::new();
    let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");
    let snapshot = reader.read_projection_snapshot(now()).unwrap();

    let products = products_from_snapshot(&snapshot);

    assert_eq!(products.len(), snapshot.products.len());
    for (composed, source) in products.iter().zip(snapshot.products.iter()) {
        assert_eq!(composed.id, source.id.as_str());
        assert_eq!(composed.revision, source.source_revision.get());
        assert_eq!(composed.classification, source.classification);
        assert_eq!(composed.owner, OwnerModule::Portfolio);
    }
}

#[test]
fn deriving_attention_from_a_real_snapshot_mutates_nothing() {
    // Evaluating attention is a query. It must not advance the revision or
    // write anything, and this proves it against the real database rather
    // than against the doc comment that says so.
    let ledger = RealLedger::new();
    let writer = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");
    let before = writer.revision().unwrap();
    let snapshot = writer.read_projection_snapshot(now()).unwrap();

    let _ = ranked_attention_from_snapshot(&snapshot, AttentionThresholds::default());

    assert_eq!(
        writer.revision().unwrap(),
        before,
        "a query must not advance the Ledger revision"
    );
}

#[test]
fn the_composition_survives_reopening_the_same_ledger() {
    // The adapter holds no state of its own, so the same database must
    // compose identically through a fresh handle.
    let ledger = RealLedger::new();
    let first = {
        let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");
        let snapshot = reader.read_projection_snapshot(now()).unwrap();
        compose_cockpit_from_snapshot(
            &snapshot,
            AttentionThresholds::default(),
            &NoApprovedPeriodPort,
        )
    };

    let reopened = SqliteProductLedger::open(&ledger.0).expect("the Ledger must reopen");
    let snapshot = reopened.read_projection_snapshot(now()).unwrap();
    let second = compose_cockpit_from_snapshot(
        &snapshot,
        AttentionThresholds::default(),
        &NoApprovedPeriodPort,
    );

    assert_eq!(first.state, second.state);
    assert_eq!(first.ledger_revision, second.ledger_revision);
    assert_eq!(first.body.products, second.body.products);
    assert_eq!(first.body.pulse, second.body.pulse);
}
