//! The Stakeholder and Milestone read surface the Cockpit, People and
//! Product inspector composition needs.
//!
//! These were the facts the projection snapshot deliberately does not carry,
//! and their absence is what blocked S09 People and the Product-health
//! inspector. Every test here reads a real on-disk Ledger.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{
    CompositionSnapshotReadError, LedgerSnapshotForCompositionPort,
};
use pmc_domain::relationships::{
    RelationshipKind, StakeholderKind, StakeholderRelationshipPurpose,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, DecisionRequestState, EvidenceRole, EvidenceVerification, IssueState,
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::Connection;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("pmc-composition-read-{nonce}-{sequence}.sqlite3")))
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

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

/// Seeds two Stakeholders, a Project with a Milestone, one responsibility
/// and one dependency relationship, and -- for the Product-health inspector
/// -- a Product with a Project, an Initiative on that Project, a Roadmap, a
/// KPI, two Evidence references linked to it, and one deliberately malformed
/// three-endpoint relationship (`relationship-7`). Everything is synthetic
/// and public-safe.
fn seeded_ledger() -> SyntheticLedger {
    let ledger = SyntheticLedger::new();
    drop(SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open"));
    let connection = Connection::open(&ledger.0).expect("the fixture must open the database");
    connection
        .execute_batch(
            "
            INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES
              ('stakeholder-b','stakeholder',2,'confidential',0,0),
              ('stakeholder-a','stakeholder',1,'internal',0,0),
              ('project-1','project',1,'internal',0,0),
              ('milestone-1','milestone',3,'restricted',0,0),
              ('relationship-1','relationship',1,'internal',0,0),
              ('relationship-2','relationship',1,'confidential',0,0);
            INSERT INTO stakeholders (id,name,kind,provenance_kind,provenance_reference) VALUES
              ('stakeholder-b','Synthetic Organization','organization','synthetic_fixture','public-safe-fixture'),
              ('stakeholder-a','Synthetic Person','person','synthetic_fixture','public-safe-fixture');
            INSERT INTO projects (id,name,start_at,end_at,provenance_kind,provenance_reference) VALUES
              ('project-1','Synthetic Project',0,1000,'synthetic_fixture','public-safe-fixture');
            INSERT INTO milestones (id,project_id,name,verification_criteria,due_at,provenance_kind,provenance_reference) VALUES
              ('milestone-1','project-1','Synthetic Milestone','Synthetic criteria',5000,'synthetic_fixture','public-safe-fixture');
            INSERT INTO relationships (id,kind,purpose) VALUES
              ('relationship-1','stakeholder_subject','responsibility'),
              ('relationship-2','stakeholder_subject','dependency');
            INSERT INTO relationship_endpoints (relationship_id,ordinal,target_type,target_id,target_version,target_classification,parent_project_id) VALUES
              ('relationship-1',0,'stakeholder','stakeholder-a',1,'internal',NULL),
              ('relationship-1',1,'milestone','milestone-1',3,'restricted','project-1'),
              ('relationship-2',0,'stakeholder','stakeholder-b',2,'confidential',NULL),
              ('relationship-2',1,'project','project-1',1,'internal',NULL);
            INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES
              ('request-open','action_request',1,'internal',0,0),
              ('request-accepted','action_request',2,'internal',0,0),
              ('request-unowned','action_request',1,'confidential',0,0);
            INSERT INTO action_requests (id,title,details,intended_owner_id,response_due_at,intended_action_due_at,state,superseded_premise) VALUES
              ('request-open','Synthetic open request','Synthetic details','stakeholder-a',9000,NULL,'open',0),
              ('request-accepted','Synthetic accepted request','Synthetic details','stakeholder-a',NULL,12000,'accepted',0),
              ('request-unowned','Synthetic unowned request','Synthetic details',NULL,NULL,NULL,'open',0);
            INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES
              ('decision-request-1','decision_request',1,'internal',0,0),
              ('risk-1','risk',1,'internal',0,0),
              ('issue-fresh','issue',1,'internal',0,0),
              ('issue-recurrence','issue',2,'confidential',0,0);
            INSERT INTO decision_requests (id,subject,details,intended_owner_id,state) VALUES
              ('decision-request-1','Synthetic decision subject','Synthetic details','stakeholder-a','open');
            INSERT INTO risks (id,title,details,state,in_exception_queue,owner_id) VALUES
              ('risk-1','Synthetic risk','Synthetic details','open',0,'stakeholder-b');
            INSERT INTO issues (id,source_risk_id,recurrence_of_id,title,details,state) VALUES
              ('issue-fresh',NULL,NULL,'Synthetic fresh issue','Synthetic details','open'),
              ('issue-recurrence','risk-1','issue-fresh','Synthetic recurring issue','Synthetic details','open');
            INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES
              ('product-1','product',2,'internal',0,0),
              ('initiative-1','initiative',1,'internal',0,0),
              ('roadmap-1','roadmap',1,'confidential',0,0),
              ('kpi-1','kpi_definition',1,'internal',0,0),
              ('relationship-3','relationship',4,'internal',0,0),
              ('relationship-4','relationship',1,'internal',0,0),
              ('relationship-5','relationship',1,'confidential',0,0),
              ('relationship-6','relationship',1,'internal',0,0),
              ('relationship-7','relationship',1,'internal',0,0),
              ('evidence-1','evidence_reference',3,'restricted',0,7000),
              ('evidence-2','evidence_reference',1,'internal',0,8000);
            INSERT INTO products (id,name,details,provenance_kind,provenance_reference) VALUES
              ('product-1','Synthetic Product','Synthetic details','synthetic_fixture','public-safe-fixture');
            INSERT INTO initiatives (id,name,defined_outcome,provenance_kind,provenance_reference) VALUES
              ('initiative-1','Synthetic Initiative','Synthetic outcome','synthetic_fixture','public-safe-fixture');
            INSERT INTO roadmaps (id,name,details,provenance_kind,provenance_reference) VALUES
              ('roadmap-1','Synthetic Roadmap','Synthetic details','synthetic_fixture','public-safe-fixture');
            INSERT INTO kpi_definitions (id,name,definition,owner,target,cadence,source,provenance_kind,provenance_reference) VALUES
              ('kpi-1','Synthetic KPI','Synthetic definition','Synthetic owner','Synthetic target','weekly','Synthetic source','synthetic_fixture','public-safe-fixture');
            INSERT INTO relationships (id,kind,purpose) VALUES
              ('relationship-3','project_product',NULL),
              ('relationship-4','initiative_project',NULL),
              ('relationship-5','product_roadmap',NULL),
              ('relationship-6','product_kpi',NULL),
              ('relationship-7','product_kpi',NULL);
            INSERT INTO relationship_endpoints (relationship_id,ordinal,target_type,target_id,target_version,target_classification,parent_project_id) VALUES
              ('relationship-3',0,'project','project-1',1,'internal',NULL),
              ('relationship-3',1,'product','product-1',2,'internal',NULL),
              ('relationship-4',0,'initiative','initiative-1',1,'internal',NULL),
              ('relationship-4',1,'project','project-1',1,'internal',NULL),
              ('relationship-5',0,'product','product-1',2,'internal',NULL),
              ('relationship-5',1,'roadmap','roadmap-1',1,'confidential',NULL),
              ('relationship-6',0,'product','product-1',2,'internal',NULL),
              ('relationship-6',1,'kpi_definition','kpi-1',1,'internal',NULL),
              ('relationship-7',0,'product','product-1',2,'internal',NULL),
              ('relationship-7',1,'kpi_definition','kpi-1',1,'internal',NULL),
              ('relationship-7',2,'roadmap','roadmap-1',1,'confidential',NULL);
            INSERT INTO evidence_references (id,role,verification,integrity_digest,last_verified_at,vault_relative_path,fingerprint_algorithm,fingerprint_digest,provenance_kind,provenance_reference) VALUES
              ('evidence-1','action_completion','verified','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',7000,'evidence/synthetic-1.md',NULL,NULL,'synthetic_fixture','public-safe-fixture'),
              ('evidence-2',NULL,'unverified',NULL,NULL,'evidence/synthetic-2.md',NULL,NULL,'synthetic_fixture','public-safe-fixture');
            INSERT INTO evidence_links (evidence_id,target_type,target_id,classification,linked_at) VALUES
              ('evidence-1','product','product-1','internal',6000),
              ('evidence-1','milestone','milestone-1','restricted',6500),
              ('evidence-2','product','product-1','internal',6100);
            INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES
              ('observation-2','kpi_observation',2,'confidential',0,0),
              ('observation-1','kpi_observation',1,'internal',0,0);
            INSERT INTO kpi_observations (id,kpi_id,value,observed_at,source,provenance_kind,provenance_reference) VALUES
              ('observation-2','kpi-1','HIDDEN-VALUE-TWO',4000,'HIDDEN-SOURCE-TWO','synthetic_fixture','public-safe-fixture'),
              ('observation-1','kpi-1','HIDDEN-VALUE-ONE',3000,'HIDDEN-SOURCE-ONE','synthetic_fixture','public-safe-fixture');
            ",
        )
        .expect("the fixture must seed");
    ledger
}

#[test]
fn a_fresh_ledger_reads_an_empty_composition_snapshot() {
    // Empty is the honest answer for a database that holds nothing, not an
    // error, and not a reason for a route to fail.
    let ledger = SyntheticLedger::new();
    let reader = SqliteProductLedger::open(&ledger.0).expect("a fresh Ledger must open");

    let snapshot = reader
        .read_composition_snapshot(now())
        .expect("a fresh Ledger must yield a snapshot");

    assert!(snapshot.stakeholders.is_empty());
    assert!(snapshot.stakeholder_relationships.is_empty());
    assert!(snapshot.milestones.is_empty());
    assert!(snapshot.portfolio_relationships.is_empty());
    assert!(snapshot.products.is_empty());
    assert!(snapshot.initiatives.is_empty());
    assert!(snapshot.projects.is_empty());
    assert!(snapshot.roadmaps.is_empty());
    assert!(snapshot.kpi_definitions.is_empty());
    assert!(snapshot.kpi_observations.is_empty());
    assert!(snapshot.evidence_links.is_empty());
    assert!(snapshot.evidence_references.is_empty());
    assert!(snapshot.work_owners.is_empty());
    assert_eq!(snapshot.ledger_as_of_utc, now());
}

#[test]
fn stakeholders_carry_the_version_and_classification_the_registry_holds() {
    // Both come from `aggregate_registry`, which is where the Ledger keeps
    // them. A composition that invented either would assert authority it
    // does not have.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.stakeholders.len(), 2);
    let first = &snapshot.stakeholders[0];
    assert_eq!(first.id.as_str(), "stakeholder-a");
    assert_eq!(first.display_name, "Synthetic Person");
    assert_eq!(first.kind, StakeholderKind::Person);
    assert_eq!(first.classification, DataClassification::Internal);
    assert_eq!(first.version.get(), 1);

    let second = &snapshot.stakeholders[1];
    assert_eq!(second.kind, StakeholderKind::Organization);
    assert_eq!(second.classification, DataClassification::Confidential);
    assert_eq!(second.version.get(), 2);
}

#[test]
fn stakeholders_are_ordered_by_identifier_not_insertion_order() {
    // The fixture inserts `stakeholder-b` first on purpose. A read whose
    // order followed insertion would make repeated reads of unchanged state
    // incomparable.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let ids: Vec<&str> = snapshot
        .stakeholders
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(ids, vec!["stakeholder-a", "stakeholder-b"]);
}

#[test]
fn responsibility_and_dependency_are_read_as_distinct_purposes() {
    // Collapsing them would misstate accountability: being responsible for a
    // Milestone and depending on a Project are different relationships.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.stakeholder_relationships.len(), 2);
    let responsibility = snapshot
        .stakeholder_relationships
        .iter()
        .find(|record| record.purpose == StakeholderRelationshipPurpose::Responsibility)
        .expect("the responsibility relationship must be read");
    assert_eq!(responsibility.stakeholder_id.as_str(), "stakeholder-a");
    assert_eq!(responsibility.subject_type, "milestone");
    assert_eq!(responsibility.subject_id, "milestone-1");

    let dependency = snapshot
        .stakeholder_relationships
        .iter()
        .find(|record| record.purpose == StakeholderRelationshipPurpose::Dependency)
        .expect("the dependency relationship must be read");
    assert_eq!(dependency.stakeholder_id.as_str(), "stakeholder-b");
    assert_eq!(dependency.subject_type, "project");
}

#[test]
fn a_relationship_never_reports_the_stakeholder_as_its_own_subject() {
    // The join distinguishes the two endpoints by target type rather than by
    // ordinal, so it cannot mistake which end is the person.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    for record in &snapshot.stakeholder_relationships {
        assert_ne!(record.subject_type, "stakeholder");
        assert_ne!(record.subject_id, record.stakeholder_id.as_str());
    }
}

#[test]
fn milestones_carry_their_project_due_date_and_classification() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.milestones.len(), 1);
    let milestone = &snapshot.milestones[0];
    assert_eq!(milestone.id.as_str(), "milestone-1");
    assert_eq!(milestone.project_id.as_str(), "project-1");
    assert_eq!(milestone.name, "Synthetic Milestone");
    assert_eq!(milestone.due_at, UtcTimestamp::from_unix_millis(5_000));
    assert_eq!(milestone.classification, DataClassification::Restricted);
    assert_eq!(milestone.version.get(), 3);
}

#[test]
fn every_collection_reports_the_same_ledger_revision() {
    // All three are read inside one transaction, so they cannot disagree
    // about which instant they describe.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.ledger_revision, reader.revision().unwrap());
    assert_eq!(snapshot.schema_version, reader.schema_version());
}

#[test]
fn reading_the_composition_snapshot_mutates_nothing() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");
    let before = reader.revision().unwrap();

    let _ = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(
        reader.revision().unwrap(),
        before,
        "a query must not advance the Ledger revision"
    );
}

#[test]
fn repeated_reads_of_unchanged_state_are_identical() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let first = reader.read_composition_snapshot(now()).unwrap();
    let second = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(first, second);
}

#[test]
fn the_snapshot_survives_reopening_the_ledger() {
    let ledger = seeded_ledger();
    let first = {
        let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");
        reader.read_composition_snapshot(now()).unwrap()
    };

    let reopened = SqliteProductLedger::open(&ledger.0).expect("the Ledger must reopen");
    let second = reopened.read_composition_snapshot(now()).unwrap();

    assert_eq!(first, second);
}

#[test]
fn an_unreadable_error_is_not_confused_with_an_empty_result() {
    // `Unavailable` and an empty snapshot are different facts. This pins the
    // error type so a future change cannot quietly turn a failure into an
    // empty People directory.
    assert_ne!(
        CompositionSnapshotReadError::Unavailable,
        CompositionSnapshotReadError::ReadFailed
    );
}

#[test]
fn action_requests_report_their_state_rather_than_being_pre_filtered() {
    // "Outstanding" is a composition judgement, not a read-surface one. The
    // People routes want only `Open`; a Work Queue legitimately wants more.
    // A read that pre-filtered would force every consumer to accept one
    // caller's definition or read the table again.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.action_requests.len(), 3);
    let states: Vec<ActionRequestState> = snapshot
        .action_requests
        .iter()
        .map(|record| record.state)
        .collect();
    assert!(states.contains(&ActionRequestState::Open));
    assert!(
        states.contains(&ActionRequestState::Accepted),
        "an accepted request must still be readable, so a consumer can tell it is no longer outstanding"
    );
}

#[test]
fn a_request_with_no_intended_owner_survives_the_read() {
    // That absence is the fact behind `ActionRequestMissingIntendedOwner`.
    // Dropping the row, or inventing an owner for it, would erase the very
    // thing the attention reason exists to report.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let unowned = snapshot
        .action_requests
        .iter()
        .find(|record| record.id == "request-unowned")
        .expect("an unowned request must still be read");
    assert_eq!(unowned.intended_owner_id, None);
    assert_eq!(unowned.state, ActionRequestState::Open);
}

#[test]
fn action_requests_carry_their_owner_due_date_and_classification() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let open = snapshot
        .action_requests
        .iter()
        .find(|record| record.id == "request-open")
        .expect("the open request must be read");
    assert_eq!(
        open.intended_owner_id.as_ref().map(|id| id.as_str()),
        Some("stakeholder-a")
    );
    assert_eq!(
        open.response_due_at,
        Some(UtcTimestamp::from_unix_millis(9_000))
    );
    assert_eq!(open.classification, DataClassification::Internal);
    assert_eq!(open.version.get(), 1);

    let unowned = snapshot
        .action_requests
        .iter()
        .find(|record| record.id == "request-unowned")
        .expect("the unowned request must be read");
    assert_eq!(
        unowned.classification,
        DataClassification::Confidential,
        "each request keeps its own classification rather than the collection's"
    );
}

#[test]
fn a_request_carries_its_promised_action_date_apart_from_its_response_date() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let accepted = snapshot
        .action_requests
        .iter()
        .find(|record| record.id == "request-accepted")
        .expect("the accepted request must be read");
    assert_eq!(accepted.response_due_at, None);
    assert_eq!(
        accepted.intended_action_due_at,
        Some(UtcTimestamp::from_unix_millis(12_000))
    );
    let open = snapshot
        .action_requests
        .iter()
        .find(|record| record.id == "request-open")
        .expect("the open request must be read");
    assert_eq!(open.intended_action_due_at, None);
}

#[test]
fn a_risk_is_read_with_its_title_version_and_classification() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.risks.len(), 1);
    let risk = &snapshot.risks[0];
    assert_eq!(risk.id, "risk-1");
    assert_eq!(risk.title, "Synthetic risk");
    assert_eq!(risk.classification, DataClassification::Internal);
    assert_eq!(risk.version.get(), 1);
}

#[test]
fn a_decision_request_is_read_as_its_own_kind_of_ask() {
    // Asking someone to decide something and asking someone to do something
    // are different asks with different lifecycles. A queue that merged them
    // would tell the reader neither.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.decision_requests.len(), 1);
    let request = &snapshot.decision_requests[0];
    assert_eq!(request.id, "decision-request-1");
    assert_eq!(request.subject, "Synthetic decision subject");
    assert_eq!(request.state, DecisionRequestState::Open);
    assert_eq!(
        request.intended_owner_id.as_ref().map(|id| id.as_str()),
        Some("stakeholder-a")
    );
    // Distinct collections, so a Decision Request can never be counted as an
    // Action Request.
    assert!(snapshot
        .action_requests
        .iter()
        .all(|other| other.id != request.id));
}

#[test]
fn an_issue_keeps_both_of_its_origins() {
    // An Issue raised from a Risk, and an Issue that recurs an earlier one,
    // are materially different from a fresh Issue. `IssueRecurrence`
    // attention exists because of the second, so dropping either origin
    // would erase what that reason reports.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.issues.len(), 2);
    let fresh = snapshot
        .issues
        .iter()
        .find(|issue| issue.id == "issue-fresh")
        .expect("the fresh issue must be read");
    assert_eq!(fresh.source_risk_id, None);
    assert_eq!(fresh.recurrence_of_id, None);
    assert_eq!(fresh.state, IssueState::Open);

    let recurrence = snapshot
        .issues
        .iter()
        .find(|issue| issue.id == "issue-recurrence")
        .expect("the recurring issue must be read");
    assert_eq!(recurrence.source_risk_id.as_deref(), Some("risk-1"));
    assert_eq!(recurrence.recurrence_of_id.as_deref(), Some("issue-fresh"));
    assert_eq!(recurrence.classification, DataClassification::Confidential);
    assert_eq!(recurrence.version.get(), 2);
}

#[test]
fn the_five_work_queue_kinds_stay_in_separate_collections() {
    // The Work Queue requires Requests, Actions, Decisions, Risks and Issues
    // to remain distinct. Separate collections make merging them a deliberate
    // act rather than an accident of a shared query.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert_eq!(snapshot.action_requests.len(), 3);
    assert_eq!(snapshot.decision_requests.len(), 1);
    assert_eq!(snapshot.issues.len(), 2);
}

// ---------------------------------------------------------------------------
// The Portfolio hierarchy, for the S02 detail half and the O01 inspector.
// ---------------------------------------------------------------------------

#[test]
fn portfolio_relationships_carry_typed_endpoints_ordered_by_type_and_no_parent() {
    // The hierarchy is rows in `relationships`, not parent columns. Each
    // record names its two endpoints by type and identifier and nothing
    // else: which of the two is the "parent" is not a fact the Ledger holds.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let kinds: Vec<RelationshipKind> = snapshot
        .portfolio_relationships
        .iter()
        .map(|record| record.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            RelationshipKind::ProjectProduct,
            RelationshipKind::InitiativeProject,
            RelationshipKind::ProductRoadmap,
            RelationshipKind::ProductKpi,
        ]
    );

    let project_product = &snapshot.portfolio_relationships[0];
    assert_eq!(project_product.id, "relationship-3");
    // Ordered by type: "product" < "project".
    assert_eq!(project_product.endpoints[0].target_type, "product");
    assert_eq!(project_product.endpoints[0].target_id, "product-1");
    assert_eq!(project_product.endpoints[1].target_type, "project");
    assert_eq!(project_product.endpoints[1].target_id, "project-1");
    assert_eq!(project_product.endpoint_id("project"), Some("project-1"));
    assert_eq!(project_product.endpoint_id("stakeholder"), None);
    // Version and classification come from the registry, not the endpoints'
    // link-time copies.
    assert_eq!(project_product.version.get(), 4);
    assert_eq!(project_product.classification, DataClassification::Internal);
}

#[test]
fn a_malformed_relationship_produces_no_record_rather_than_a_guessed_pair() {
    // `relationship-7` has three endpoints. Picking any two would attribute
    // a relationship the Ledger does not hold, so the whole row is dropped --
    // the same choice the Stakeholder read makes for malformed rows.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert!(
        !snapshot
            .portfolio_relationships
            .iter()
            .any(|record| record.id == "relationship-7"),
        "a three-endpoint relationship reached the read surface as a pair"
    );
    // And it did not take its well-formed neighbours with it.
    assert_eq!(snapshot.portfolio_relationships.len(), 4);
}

#[test]
fn stakeholder_relationships_are_not_repeated_as_portfolio_relationships() {
    // The two reads partition `relationships` by kind. A stakeholder-subject
    // row appearing in both would count a person's accountability as
    // Portfolio structure.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert!(snapshot
        .portfolio_relationships
        .iter()
        .all(|record| record.kind != RelationshipKind::StakeholderSubject));
    assert_eq!(snapshot.stakeholder_relationships.len(), 2);
}

#[test]
fn named_portfolio_records_carry_the_registry_version_and_classification() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let product = &snapshot.products[0];
    assert_eq!(product.id.as_str(), "product-1");
    assert_eq!(product.name, "Synthetic Product");
    assert_eq!(product.version.get(), 2);

    let initiative = &snapshot.initiatives[0];
    assert_eq!(initiative.id.as_str(), "initiative-1");
    assert_eq!(initiative.name, "Synthetic Initiative");
    assert_eq!(initiative.classification, DataClassification::Internal);

    let project = &snapshot.projects[0];
    assert_eq!(project.id.as_str(), "project-1");
    assert_eq!(project.name, "Synthetic Project");
    assert_eq!(project.start_at, UtcTimestamp::from_unix_millis(0));
    assert_eq!(project.end_at, UtcTimestamp::from_unix_millis(1000));

    let roadmap = &snapshot.roadmaps[0];
    assert_eq!(roadmap.name, "Synthetic Roadmap");
    assert_eq!(roadmap.classification, DataClassification::Confidential);

    let kpi = &snapshot.kpi_definitions[0];
    assert_eq!(kpi.id.as_str(), "kpi-1");
    assert_eq!(kpi.name, "Synthetic KPI");
    assert_eq!(kpi.version.get(), 1);
}

// ---------------------------------------------------------------------------
// Evidence, for the O01 Evidence tab.
// ---------------------------------------------------------------------------

#[test]
fn evidence_links_are_read_for_every_target_type_the_ledger_holds() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let to_product: Vec<&str> = snapshot
        .evidence_links
        .iter()
        .filter(|link| link.target_type == "product" && link.target_id == "product-1")
        .map(|link| link.evidence_id.as_str())
        .collect();
    assert_eq!(to_product, ["evidence-1", "evidence-2"]);

    let to_milestone = snapshot
        .evidence_links
        .iter()
        .find(|link| link.target_type == "milestone")
        .expect("the milestone link must be read too");
    assert_eq!(to_milestone.evidence_id.as_str(), "evidence-1");
    assert_eq!(to_milestone.linked_at, UtcTimestamp::from_unix_millis(6500));
}

#[test]
fn a_link_carries_its_link_time_classification_separately_from_the_evidence_own() {
    // Three classifications, three facts. `evidence-1` is Restricted in the
    // registry, but its link to `product-1` was recorded as Internal. A read
    // that substituted one for the other would either under-classify the
    // Evidence or over-classify the link.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let link = snapshot
        .evidence_links
        .iter()
        .find(|link| link.evidence_id.as_str() == "evidence-1" && link.target_type == "product")
        .unwrap();
    assert_eq!(link.classification_at_link, DataClassification::Internal);

    let reference = snapshot
        .evidence_references
        .iter()
        .find(|reference| reference.id.as_str() == "evidence-1")
        .unwrap();
    assert_eq!(reference.classification, DataClassification::Restricted);
}

#[test]
fn a_verified_evidence_reference_carries_its_timestamp_and_digest() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let reference = &snapshot.evidence_references[0];
    assert_eq!(reference.id.as_str(), "evidence-1");
    assert_eq!(reference.role, Some(EvidenceRole::ActionCompletion));
    assert_eq!(reference.version.get(), 3);
    assert_eq!(reference.updated_at, UtcTimestamp::from_unix_millis(7000));
    match &reference.verification {
        EvidenceVerification::Verified {
            verified_at,
            integrity_digest,
        } => {
            assert_eq!(*verified_at, UtcTimestamp::from_unix_millis(7000));
            assert!(!integrity_digest.as_str().is_empty());
        }
        other => panic!("expected Verified, read {other:?}"),
    }
}

#[test]
fn an_evidence_reference_without_a_role_is_read_as_none_not_as_a_failure() {
    // `role` became nullable in schema v18. A read that assumed it never was
    // would fail the whole snapshot on the first such row, and every route
    // that reads the snapshot with it.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader
        .read_composition_snapshot(now())
        .expect("a NULL role must not fail the snapshot");

    let reference = snapshot
        .evidence_references
        .iter()
        .find(|reference| reference.id.as_str() == "evidence-2")
        .unwrap();
    assert_eq!(reference.role, None);
    assert_eq!(reference.verification, EvidenceVerification::Unverified);
}

#[test]
fn the_vault_path_never_enters_the_read_surface() {
    // The record type has no path field, so this is enforced by the type;
    // the assertion guards the *rendering* of the record too, so a Debug
    // dump of a snapshot in a log cannot become the place a path leaks from.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let rendered = format!("{:?}", snapshot.evidence_references);
    assert!(!rendered.contains("synthetic-1.md"));
    assert!(!rendered.contains("evidence/"));
}

// ---------------------------------------------------------------------------
// KPI observations: when, and for which KPI -- never what was measured
// (DG3 S01 amendment 2026-09-15, §8.2).
// ---------------------------------------------------------------------------

#[test]
fn kpi_observations_carry_when_and_for_which_kpi_ordered_by_identifier() {
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let observations = &snapshot.kpi_observations;
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].id.as_str(), "observation-1");
    assert_eq!(observations[0].kpi_id.as_str(), "kpi-1");
    assert_eq!(
        observations[0].observed_at,
        UtcTimestamp::from_unix_millis(3000)
    );
    assert_eq!(observations[0].version.get(), 1);
    assert_eq!(observations[0].classification, DataClassification::Internal);
    assert_eq!(observations[1].id.as_str(), "observation-2");
    assert_eq!(
        observations[1].observed_at,
        UtcTimestamp::from_unix_millis(4000)
    );
    assert_eq!(observations[1].version.get(), 2);
    assert_eq!(
        observations[1].classification,
        DataClassification::Confidential
    );
}

#[test]
fn a_measured_value_or_its_source_never_enters_the_read_surface() {
    // The record type has no value or source field, so the type enforces
    // this; rendering the whole snapshot guards against a later field that
    // carries either under another name. KPI values stay out of every
    // reviewed read surface, and the Lens needs only that an
    // observation exists and when it was made.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let rendered = format!("{:?}", reader.read_composition_snapshot(now()).unwrap());

    for hidden in [
        "HIDDEN-VALUE-ONE",
        "HIDDEN-VALUE-TWO",
        "HIDDEN-SOURCE-ONE",
        "HIDDEN-SOURCE-TWO",
    ] {
        assert!(!rendered.contains(hidden), "{hidden} reached the snapshot");
    }
}

// ---------------------------------------------------------------------------
// Work ownership, for the People tab of the Product-health inspector.
// ---------------------------------------------------------------------------

#[test]
fn work_owners_are_read_as_one_collection_across_every_owner_bearing_kind() {
    // Five tables spell "owner" three ways. One collection, one vocabulary,
    // ordered by kind then identifier. The fixture seeds an owned Risk, two
    // owned Action Requests and an unowned one, and an unowned Decision
    // Request; the unowned rows must be absent, not present with a blank.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let rows: Vec<(&str, &str, &str)> = snapshot
        .work_owners
        .iter()
        .map(|record| {
            (
                record.target_type.as_str(),
                record.target_id.as_str(),
                record.owner_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("action_request", "request-accepted", "stakeholder-a"),
            ("action_request", "request-open", "stakeholder-a"),
            ("decision_request", "decision-request-1", "stakeholder-a"),
            ("risk", "risk-1", "stakeholder-b"),
        ]
    );
    assert!(
        !rows.iter().any(|(_, id, _)| *id == "request-unowned"),
        "an unowned request was read as owned"
    );
}

#[test]
fn issues_never_appear_among_work_owners_because_they_have_no_owner_column() {
    // Not a filter: the `issues` table has no owner at all. A People tab that
    // listed Issues under anyone would be inventing accountability.
    let ledger = seeded_ledger();
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    assert!(snapshot
        .work_owners
        .iter()
        .all(|record| record.target_type != "issue"));
    assert_eq!(
        snapshot.issues.len(),
        2,
        "the Issues themselves are still read"
    );
}

#[test]
fn pinned_is_read_from_the_pin_columns_not_from_the_verification_state() {
    // `evidence-1` is `verified` but has no pin -- the shape a reference
    // observed before v43 can have -- and must read as unpinned. `evidence-2`
    // gains a pin and reads as pinned whatever its state says.
    let ledger = seeded_ledger();
    let connection = Connection::open(&ledger.0).expect("the fixture must open the database");
    connection
        .execute_batch(
            "UPDATE evidence_references SET fingerprint_algorithm='sha256',fingerprint_digest='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE id='evidence-2';
             UPDATE evidence_references SET verification='observed_unpinned',integrity_digest='cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',last_verified_at=7100 WHERE id='evidence-1';",
        )
        .expect("the fixture must accept a pin and the fifth state");
    drop(connection);
    let reader = SqliteProductLedger::open(&ledger.0).expect("the Ledger must open");

    let snapshot = reader.read_composition_snapshot(now()).unwrap();

    let by_id = |id: &str| {
        snapshot
            .evidence_references
            .iter()
            .find(|reference| reference.id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} must be read"))
    };
    let unpinned = by_id("evidence-1");
    assert!(!unpinned.pinned);
    match &unpinned.verification {
        EvidenceVerification::ObservedUnpinned {
            observed_at,
            integrity_digest,
        } => {
            assert_eq!(*observed_at, UtcTimestamp::from_unix_millis(7100));
            assert_eq!(integrity_digest.as_str(), "c".repeat(64));
        }
        other => panic!("expected ObservedUnpinned, read {other:?}"),
    }
    let pinned = by_id("evidence-2");
    assert!(pinned.pinned);
    assert_eq!(pinned.verification, EvidenceVerification::Unverified);
    let rendered = format!("{:?}", snapshot.evidence_references);
    assert!(
        !rendered.contains("synthetic-1.md") && !rendered.contains("synthetic-2.md"),
        "the pin fact must not bring the Vault path with it"
    );
}
