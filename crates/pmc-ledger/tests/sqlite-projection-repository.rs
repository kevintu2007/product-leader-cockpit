//! `LedgerSnapshotForProjectionPort`: the narrow, allowlisted bulk read a Product Vault projection generator uses.
//! Seeds rows directly via raw SQL (matching the schema in
//! `crates/pmc-ledger/src/sqlite/schema.rs`) rather than driving the full
//! typed command surface for every one of the six record types -- this
//! isolates the read/decode path under test, matching the same
//! raw-INSERT-for-a-fixture-dependency technique already used elsewhere in
//! this suite (see `sqlite-action-start.rs`'s stakeholder seed). The bundled
//! SQLite here enforces foreign keys by default even on a bare
//! `rusqlite::Connection::open`, so the seed batch turns that off for itself
//! rather than populating every referenced-but-irrelevant row (owners,
//! source requests, support witnesses); the `CHECK` constraints on each
//! table's own columns still hold and are what this test actually cares
//! about.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    identity::{ActionId, DecisionId, KpiId, ProductId, ProjectId, RiskId},
    projection_source::LedgerSnapshotForProjectionPort,
    time::UtcTimestamp,
    work_management::{ActionState, DecisionState, RiskState},
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::Connection;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-projection-{nonce}-{sequence}.sqlite3"
        )))
    }
}

impl Drop for SyntheticLedger {
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

#[test]
fn an_empty_ledger_yields_an_empty_snapshot_with_correct_envelope() {
    let ledger = SyntheticLedger::new();
    let writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let snapshot = writer
        .read_projection_snapshot(UtcTimestamp::from_unix_millis(5_000))
        .expect("an empty, freshly bootstrapped ledger must read a valid snapshot");

    assert_eq!(snapshot.schema_version, writer.schema_version());
    assert_eq!(snapshot.ledger_revision, writer.revision().unwrap());
    assert_eq!(
        snapshot.ledger_as_of_utc,
        UtcTimestamp::from_unix_millis(5_000)
    );
    assert!(snapshot.products.is_empty());
    assert!(snapshot.projects.is_empty());
    assert!(snapshot.actions.is_empty());
    assert!(snapshot.decisions.is_empty());
    assert!(snapshot.risks.is_empty());
    assert!(snapshot.kpis.is_empty());
}

/// Seeds exactly one row of each of the six projectable types, deliberately
/// inserted out of ID order (`-2` before `-1` where more than one row exists
/// per type) so a passing determinism assertion actually proves the `ORDER
/// BY <table>.id` clauses, not merely an insertion-order coincidence.
fn seed_one_of_each(ledger: &SyntheticLedger) {
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = OFF; \
             INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES \
                ('product-2','product',3,'internal',0,0), \
                ('product-1','product',1,'confidential',0,0), \
                ('project-1','project',2,'public',0,0), \
                ('action-1','action',4,'restricted',0,0), \
                ('decision-1','decision',1,'internal',0,0), \
                ('risk-1','risk',2,'internal',0,0), \
                ('kpi-1','kpi_definition',1,'public',0,0); \
             INSERT INTO products (id,name,details,provenance_kind,provenance_reference) VALUES \
                ('product-2','P2','details 2','user_entered',NULL), \
                ('product-1','P1','details 1','user_entered',NULL); \
             INSERT INTO projects (id,name,start_at,end_at,provenance_kind,provenance_reference) VALUES \
                ('project-1','Proj 1',10000,20000,'user_entered',NULL); \
             INSERT INTO actions (id,source_request_id,title,details,owner_id,due_at,state,commitment_classification,support_id,transition_reason,source_decision_id,superseded_premise) VALUES \
                ('action-1','request-1','Title','Details','stakeholder-owner-1',30000,'in_progress','restricted',NULL,NULL,'decision-1',0); \
             INSERT INTO decisions (id,source_request_id,statement,rationale,impact,owner_id,decided_at,state,support_id,supersedes_decision_id,superseded_by_decision_id) VALUES \
                ('decision-1',NULL,'Statement','Rationale','Impact','stakeholder-owner-1',40000,'effective','support-1',NULL,NULL); \
             INSERT INTO risks (id,title,details,state,response,owner_id,rationale,residual_exposure,next_review_at,in_exception_queue) VALUES \
                ('risk-1','Title','Details','occurred',NULL,NULL,NULL,NULL,50000,1); \
             INSERT INTO kpi_definitions (id,name,definition,owner,target,cadence,source,provenance_kind,provenance_reference) VALUES \
                ('kpi-1','K1','def','owner','target','monthly','source','user_entered',NULL);",
        )
        .unwrap();
}

#[test]
fn a_populated_ledger_decodes_every_allowlisted_field_in_deterministic_id_order() {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_one_of_each(&ledger);
    let writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let snapshot = writer
        .read_projection_snapshot(UtcTimestamp::from_unix_millis(9_999))
        .expect("a populated ledger with schema-valid rows must decode");

    let product_ids: Vec<&str> = snapshot.products.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        product_ids,
        vec!["product-1", "product-2"],
        "products must be ordered by id"
    );
    let product_2 = snapshot
        .products
        .iter()
        .find(|p| p.id == ProductId::parse("product-2").unwrap())
        .unwrap();
    assert_eq!(
        product_2.classification,
        pmc_domain::classification::DataClassification::Internal
    );
    assert_eq!(product_2.source_revision.get(), 3);

    assert_eq!(snapshot.projects.len(), 1);
    let project = &snapshot.projects[0];
    assert_eq!(project.id, ProjectId::parse("project-1").unwrap());
    assert_eq!(project.start_at, UtcTimestamp::from_unix_millis(10_000));
    assert_eq!(project.end_at, UtcTimestamp::from_unix_millis(20_000));

    assert_eq!(snapshot.actions.len(), 1);
    let action = &snapshot.actions[0];
    assert_eq!(action.id, ActionId::parse("action-1").unwrap());
    assert_eq!(action.state, ActionState::InProgress);
    assert_eq!(action.due_at, UtcTimestamp::from_unix_millis(30_000));
    assert_eq!(action.source_request_id.as_str(), "request-1");
    assert_eq!(
        action.source_decision_id,
        Some(DecisionId::parse("decision-1").unwrap())
    );

    assert_eq!(snapshot.decisions.len(), 1);
    let decision = &snapshot.decisions[0];
    assert_eq!(decision.id, DecisionId::parse("decision-1").unwrap());
    assert_eq!(decision.state, DecisionState::Effective);
    assert_eq!(decision.decided_at, UtcTimestamp::from_unix_millis(40_000));
    assert_eq!(decision.supersedes_decision_id, None);
    assert_eq!(decision.superseded_by_decision_id, None);

    assert_eq!(snapshot.risks.len(), 1);
    let risk = &snapshot.risks[0];
    assert_eq!(risk.id, RiskId::parse("risk-1").unwrap());
    assert_eq!(risk.state, RiskState::Occurred);
    assert_eq!(
        risk.next_review_at,
        Some(UtcTimestamp::from_unix_millis(50_000))
    );

    assert_eq!(snapshot.kpis.len(), 1);
    let kpi = &snapshot.kpis[0];
    assert_eq!(kpi.id, KpiId::parse("kpi-1").unwrap());
    assert_eq!(kpi.source_revision.get(), 1);
}

#[test]
fn repeated_reads_of_an_unchanged_ledger_are_byte_for_byte_identical() {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).unwrap());
    seed_one_of_each(&ledger);
    let writer = SqliteProductLedger::open(&ledger.0).unwrap();

    let observed_at = UtcTimestamp::from_unix_millis(42);
    let first = writer.read_projection_snapshot(observed_at).unwrap();
    let second = writer.read_projection_snapshot(observed_at).unwrap();

    assert_eq!(first, second);
}
