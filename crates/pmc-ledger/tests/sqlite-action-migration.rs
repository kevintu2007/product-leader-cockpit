use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_ledger::sqlite::{
    LedgerOpenError, SqliteProductLedger, APPLICATION_ID, CURRENT_SCHEMA_VERSION,
};
use rusqlite::Connection;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticDatabase(PathBuf);

impl SyntheticDatabase {
    fn new(label: &str) -> Self {
        let n = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-action-migration-{label}-{t}-{n}.sqlite3"
        )))
    }
}

impl Drop for SyntheticDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(PathBuf::from(format!("{}{}", self.0.display(), suffix)));
        }
    }
}

fn artifacts(path: &Path) -> Vec<Option<Vec<u8>>> {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
    .into_iter()
    .map(|p| fs::read(p).ok())
    .collect()
}

fn insert_synthetic_action_fixture(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES
             ('synthetic-owner','stakeholder',0,'internal',0,0),
             ('synthetic-request','action_request',0,'internal',0,0),
             ('synthetic-action','action',0,'internal',0,0);
             INSERT INTO stakeholders(id,name,kind,provenance_kind,provenance_reference) VALUES
             ('synthetic-owner','Synthetic owner','person','synthetic_fixture','action-migration-test');
             INSERT INTO action_requests(id,title,details,intended_owner_id,response_due_at,intended_action_due_at,state,terminal_rationale,linked_action_id,source_decision_id,superseded_premise)
             VALUES('synthetic-request','Synthetic request','Synthetic details','synthetic-owner',NULL,NULL,'open',NULL,NULL,NULL,0);
             INSERT INTO actions(id,source_request_id,title,details,owner_id,due_at,state,commitment_classification,support_id,transition_reason,source_decision_id,superseded_premise)
             VALUES('synthetic-action','synthetic-request','Synthetic action','Synthetic details','synthetic-owner',0,'open','internal',NULL,NULL,NULL,0);",
        )
        .expect("synthetic action fixture must be insertable");
}

fn insert_synthetic_prepared_intent(connection: &Connection, id: &str, digest: &str) {
    connection
        .execute(
            "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES(?1,1,'complete_action',?2,'internal','allowed','not_cancellable_after_submit','head_of_products',100,0)",
            (id, digest),
        )
        .expect("synthetic prepared intent must be insertable");
}

fn insert_h3_terminal(
    connection: &Connection,
    idempotency_id: &str,
    operation: &str,
    correlation_id: &str,
    ordinal: i64,
    h3_cause: &str,
) {
    connection
        .execute(
            "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,terminal_h3_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES(?1,?2,?3,?4,'terminal','not_applicable','h3_denied',?5,'DOMAIN_CONFLICT','action.h3_denied',?3,0)",
            (idempotency_id, operation, correlation_id, ordinal, h3_cause),
        )
        .expect("H3 terminal parent must insert before its deferred child");
    match operation {
        "prepare_complete" => connection
            .execute(
                "INSERT INTO action_command_prepare_completes(idempotency_id,action_id,expected_version) VALUES(?1,'synthetic-action',1)",
                [idempotency_id],
            )
            .expect("complete command child must insert"),
        "prepare_cancel" => connection
            .execute(
                "INSERT INTO action_command_prepare_cancels(idempotency_id,action_id,expected_version,reason) VALUES(?1,'synthetic-action',1,'Synthetic cancellation')",
                [idempotency_id],
            )
            .expect("cancel command child must insert"),
        "prepare_reopen" => connection
            .execute(
                "INSERT INTO action_command_prepare_reopens(idempotency_id,action_id,expected_version,mode,reason) VALUES(?1,'synthetic-action',1,'restart_cancelled','Synthetic restart')",
                [idempotency_id],
            )
            .expect("reopen command child must insert"),
        _ => panic!("fixture only accepts H3 prepare operations"),
    };
}

#[test]
fn fresh_v2_inventory_is_action_only_and_has_no_generic_work_management_tables() {
    let db = SyntheticDatabase::new("inventory");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).expect("ledger must be inspectable");
    let names = c
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type='table' AND name LIKE 'work_management_%'",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        names.is_empty(),
        "v2 must not add generic Work Management tables"
    );
    let action_count: i64 = c
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE 'action_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        action_count >= 14,
        "typed Action replay/command tables must exist"
    );
}

#[test]
fn malformed_operation_and_typed_command_pair_is_rejected_by_foreign_keys() {
    let db = SyntheticDatabase::new("pair");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).expect("ledger must be inspectable");
    c.pragma_update(None, "foreign_keys", true).unwrap();
    c.execute_batch("BEGIN;
        INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition)
        VALUES('pair-1','start_action','corr-1',0,'action','not_applicable');
        INSERT INTO action_command_prepare_accepts(idempotency_id,request_id,expected_version)
        VALUES('pair-1','request-1',1);
        COMMIT;").expect_err("operation/child kind mismatch must fail closed");
}

#[test]
fn action_replay_parent_requires_a_matching_typed_child_at_commit() {
    let db = SyntheticDatabase::new("missing-child");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    c.execute_batch("BEGIN;
        INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition)
        VALUES('missing-child','start_action','missing-child-correlation',0,'action','synthetic-action','not_applicable');")
        .unwrap();
    assert!(
        c.execute_batch("COMMIT;").is_err(),
        "parent without its typed command child must fail at commit"
    );
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn h3_terminal_rows_allow_not_applicable_without_a_prepared_intent() {
    let db = SyntheticDatabase::new("h3-terminal");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    c.execute_batch("BEGIN;").unwrap();
    insert_h3_terminal(
        &c,
        "h3-complete",
        "prepare_complete",
        "h3-corr-complete",
        0,
        "missing_completion_evidence",
    );
    insert_h3_terminal(
        &c,
        "h3-cancel",
        "prepare_cancel",
        "h3-corr-cancel",
        1,
        "classification_unresolved",
    );
    insert_h3_terminal(
        &c,
        "h3-reopen",
        "prepare_reopen",
        "h3-corr-reopen",
        2,
        "evidence_unavailable",
    );
    c.execute_batch("COMMIT;")
        .expect("complete H3 fixture must commit");
    let count: i64 = c
        .query_row(
            "SELECT count(*) FROM action_replay_operations WHERE prepared_disposition='not_applicable' AND prepared_intent_id IS NULL AND result_kind='terminal'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);

    c.execute_batch("BEGIN;").unwrap();
    let h2_not_applicable = c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('h2-invalid','execute_action','h2-invalid-correlation',3,'terminal','not_applicable','prepared_intent_not_found','DOMAIN_NOT_FOUND','action.prepared_intent_not_found','h2-invalid-correlation',0)",
        [],
    );
    assert!(
        h2_not_applicable.is_err(),
        "H2 terminal rows must retain a consumed or retained prepared intent"
    );
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn terminal_topology_rejects_non_h2_h3_and_invalid_h3_shapes() {
    let db = SyntheticDatabase::new("terminal-topology");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);

    c.execute_batch("BEGIN;
        INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable)
        VALUES('terminal-invalid-kind','start_action','terminal-invalid-kind-correlation',0,'terminal','not_applicable','infrastructure_failure','PLATFORM_INTERNAL','action.infrastructure_failure','terminal-invalid-kind-correlation',0);
        COMMIT;").expect_err("start_action must never have a terminal replay result");
    c.execute_batch("ROLLBACK;").unwrap();

    c.execute_batch("BEGIN;
        INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable)
        VALUES('h3-invalid-shape','prepare_cancel','h3-invalid-shape-correlation',0,'terminal','retained','h3_denied','DOMAIN_CONFLICT','action.h3_denied','h3-invalid-shape-correlation',0);
        COMMIT;").expect_err("H3 terminal must be not_applicable with no prepared intent");
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn execute_parent_and_typed_child_must_share_the_exact_prepared_intent() {
    let db = SyntheticDatabase::new("execute-binding");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    insert_synthetic_prepared_intent(&c, "prepared-a", "digest-a");
    insert_synthetic_prepared_intent(&c, "prepared-b", "digest-b");

    c.execute_batch("BEGIN;
        INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable)
        VALUES('execute-binding','execute_action','execute-binding-correlation',0,'terminal','retained','prepared-a','prepared_intent_changed','DOMAIN_CONFLICT','action.prepared_intent_changed','execute-binding-correlation',0);
        INSERT INTO action_command_execute_actions(idempotency_id,kind,prepared_id,actor,acknowledged_digest)
        VALUES('execute-binding','complete','prepared-b','head_of_products','digest-b');
        COMMIT;").expect_err("mismatched parent and typed child prepared intents must fail closed");
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn action_replay_ordinals_accept_zero_but_reject_a_gap() {
    let db = SyntheticDatabase::new("ordinal");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    c.execute_batch("BEGIN;").unwrap();
    insert_h3_terminal(
        &c,
        "ordinal-zero",
        "prepare_complete",
        "ordinal-corr-zero",
        0,
        "missing_completion_evidence",
    );
    c.execute_batch("COMMIT;")
        .expect("ordinal zero must be valid");

    c.execute_batch("BEGIN;").unwrap();
    let gap = c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,terminal_h3_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('ordinal-gap','prepare_cancel','ordinal-corr-gap',2,'terminal','not_applicable','h3_denied','classification_unresolved','DOMAIN_CONFLICT','action.h3_denied','ordinal-corr-gap',0)",
        [],
    );
    assert!(gap.is_err(), "a non-contiguous ordinal must fail closed");
    c.execute_batch("ROLLBACK;").unwrap();
    let count: i64 = c
        .query_row(
            "SELECT count(*) FROM action_replay_operations WHERE idempotency_id='ordinal-gap'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn h2_terminal_disposition_matches_the_exact_domain_cause() {
    let db = SyntheticDatabase::new("h2-disposition");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    insert_synthetic_prepared_intent(&c, "prepared-changed", "digest-changed");
    insert_synthetic_prepared_intent(&c, "prepared-expired", "digest-expired");

    c.execute_batch("BEGIN;").unwrap();
    let changed_consumed = c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('changed-consumed','execute_action','changed-consumed-correlation',0,'terminal','consumed_and_discarded','prepared-changed','prepared_intent_changed','DOMAIN_CONFLICT','action.prepared_intent_changed','changed-consumed-correlation',0)",
        [],
    );
    assert!(
        changed_consumed.is_err(),
        "prepared_intent_changed must retain the prepared intent"
    );
    c.execute_batch("ROLLBACK;").unwrap();

    c.execute_batch("BEGIN;").unwrap();
    let expired_retained = c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('expired-retained','execute_action','expired-retained-correlation',0,'terminal','retained','prepared-expired','expired','DOMAIN_CONFLICT','action.expired','expired-retained-correlation',0)",
        [],
    );
    assert!(
        expired_retained.is_err(),
        "every H2 cause other than prepared_intent_changed must discard"
    );
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn h2_consumed_terminal_requires_exact_discard_marker_at_commit() {
    let db = SyntheticDatabase::new("h2-discard-marker");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    insert_synthetic_prepared_intent(&c, "prepared-missing-marker", "digest-missing-marker");

    c.execute_batch("BEGIN;").unwrap();
    c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('missing-marker','execute_action','missing-marker-correlation',0,'terminal','consumed_and_discarded','prepared-missing-marker','expired','DOMAIN_CONFLICT','action.expired','missing-marker-correlation',0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_command_execute_actions(idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES('missing-marker','complete','prepared-missing-marker','head_of_products','digest-missing-marker')",
        [],
    )
    .unwrap();
    assert!(
        c.execute_batch("COMMIT;").is_err(),
        "a consumed H2 terminal without its exact discard marker must fail at commit"
    );
    c.execute_batch("ROLLBACK;").unwrap();

    c.execute_batch("BEGIN;").unwrap();
    c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('marked-discard','execute_action','marked-discard-correlation',0,'terminal','consumed_and_discarded','prepared-missing-marker','expired','DOMAIN_CONFLICT','action.expired','marked-discard-correlation',0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_command_execute_actions(idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES('marked-discard','complete','prepared-missing-marker','head_of_products','digest-missing-marker')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_discarded_prepared_intents(prepared_intent_id,idempotency_id) VALUES('prepared-missing-marker','marked-discard')",
        [],
    )
    .unwrap();
    c.execute_batch("COMMIT;")
        .expect("the exact discard marker must satisfy the deferred topology");
}

#[test]
fn retained_h2_terminal_does_not_require_a_discard_marker() {
    let db = SyntheticDatabase::new("h2-retained");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    insert_synthetic_prepared_intent(&c, "prepared-retained", "digest-retained");
    c.execute_batch("BEGIN;").unwrap();
    c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('retained-terminal','execute_action','retained-terminal-correlation',0,'terminal','retained','prepared-retained','prepared_intent_changed','DOMAIN_CONFLICT','action.prepared_intent_changed','retained-terminal-correlation',0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_command_execute_actions(idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES('retained-terminal','complete','prepared-retained','head_of_products','digest-retained')",
        [],
    )
    .unwrap();
    c.execute_batch("COMMIT;")
        .expect("retained H2 terminal must not require a discard marker");
}

#[test]
fn retained_h2_terminal_rejects_a_discard_marker() {
    let db = SyntheticDatabase::new("retained-marker-rejected");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    insert_synthetic_prepared_intent(&c, "prepared-retained-marker", "digest-retained-marker");

    c.execute_batch("BEGIN;").unwrap();
    c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('retained-marker','execute_action','retained-marker-correlation',0,'terminal','retained','prepared-retained-marker','prepared_intent_changed','DOMAIN_CONFLICT','action.prepared_intent_changed','retained-marker-correlation',0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_command_execute_actions(idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES('retained-marker','complete','prepared-retained-marker','head_of_products','digest-retained-marker')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_discarded_prepared_intents(prepared_intent_id,idempotency_id) VALUES('prepared-retained-marker','retained-marker')",
        [],
    )
    .unwrap();
    assert!(
        c.execute_batch("COMMIT;").is_err(),
        "a discard marker must reference a consumed-and-discarded terminal"
    );
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn consumed_h2_terminal_cannot_be_changed_to_retained_with_a_marker() {
    let db = SyntheticDatabase::new("consumed-marker-retained-update");
    drop(SqliteProductLedger::open(&db.0).expect("fresh synthetic ledger must initialize"));
    let c = Connection::open(&db.0).unwrap();
    c.pragma_update(None, "foreign_keys", true).unwrap();
    insert_synthetic_action_fixture(&c);
    insert_synthetic_prepared_intent(&c, "prepared-consumed-marker", "digest-consumed-marker");

    c.execute_batch("BEGIN;").unwrap();
    c.execute(
        "INSERT INTO action_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,prepared_intent_id,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES('consumed-marker','execute_action','consumed-marker-correlation',0,'terminal','consumed_and_discarded','prepared-consumed-marker','expired','DOMAIN_CONFLICT','action.expired','consumed-marker-correlation',0)",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_command_execute_actions(idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES('consumed-marker','complete','prepared-consumed-marker','head_of_products','digest-consumed-marker')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO action_discarded_prepared_intents(prepared_intent_id,idempotency_id) VALUES('prepared-consumed-marker','consumed-marker')",
        [],
    )
    .unwrap();
    c.execute_batch("COMMIT;").unwrap();

    c.execute_batch("BEGIN;").unwrap();
    c.execute(
        "UPDATE action_replay_operations SET prepared_disposition='retained',terminal_cause='prepared_intent_changed',error_message_key='action.prepared_intent_changed' WHERE idempotency_id='consumed-marker'",
        [],
    )
    .unwrap();
    assert!(
        c.execute_batch("COMMIT;").is_err(),
        "a terminal with a discard marker must not become retained"
    );
    c.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn future_schema_and_tampered_current_registry_fail_closed_without_mutation() {
    // One past the current version, derived rather than written down: a
    // literal here silently stops being "future" on the next schema bump.
    let future_version = CURRENT_SCHEMA_VERSION + 1;
    let future = SyntheticDatabase::new("future");
    let c = Connection::open(&future.0).unwrap();
    c.execute_batch(&format!("PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={future_version}; CREATE TABLE ledger_metadata(singleton INTEGER PRIMARY KEY,schema_version INTEGER NOT NULL,ledger_revision INTEGER NOT NULL) STRICT; INSERT INTO ledger_metadata VALUES(1,{future_version},0);")).unwrap();
    drop(c);
    let before = artifacts(&future.0);
    assert!(matches!(
        SqliteProductLedger::open(&future.0),
        Err(LedgerOpenError::FutureSchema { found }) if found == future_version
    ));
    assert_eq!(artifacts(&future.0), before);

    let tampered = SyntheticDatabase::new("tampered");
    drop(SqliteProductLedger::open(&tampered.0).unwrap());
    let c = Connection::open(&tampered.0).unwrap();
    c.execute(
        "UPDATE schema_migrations SET checksum=?1 WHERE version=2",
        ["sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"],
    )
    .unwrap();
    drop(c);
    let before = artifacts(&tampered.0);
    assert!(matches!(
        SqliteProductLedger::open(&tampered.0),
        Err(LedgerOpenError::InvalidMetadata)
    ));
    assert_eq!(artifacts(&tampered.0), before);
}
