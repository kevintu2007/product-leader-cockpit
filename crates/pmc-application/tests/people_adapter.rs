//! The production People adapter (S09).

use pmc_application::people_adapter::{
    directory_entries_from_snapshot, is_outstanding, stakeholder_detail_from_snapshot,
};
use pmc_application::people_composition::RelationshipPurpose;
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{
    ActionRequestReadRecord, LedgerCompositionSnapshot, MilestoneReadRecord, ProductReadRecord,
    ProjectReadRecord, StakeholderReadRecord, StakeholderRelationshipReadRecord,
};
use pmc_domain::identity::{AggregateVersion, MilestoneId, ProductId, ProjectId, StakeholderId};
use pmc_domain::relationships::{StakeholderKind, StakeholderRelationshipPurpose};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::ActionRequestState;

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

fn version(value: u64) -> AggregateVersion {
    AggregateVersion::new(value).unwrap()
}

fn snapshot() -> LedgerCompositionSnapshot {
    LedgerCompositionSnapshot {
        schema_version: 42,
        ledger_revision: 7,
        ledger_as_of_utc: now(),
        stakeholders: vec![
            StakeholderReadRecord {
                id: StakeholderId::parse("stakeholder-a").unwrap(),
                display_name: "Synthetic Person".to_owned(),
                kind: StakeholderKind::Person,
                classification: DataClassification::Internal,
                version: version(1),
            },
            StakeholderReadRecord {
                id: StakeholderId::parse("stakeholder-b").unwrap(),
                display_name: "Synthetic Organization".to_owned(),
                kind: StakeholderKind::Organization,
                classification: DataClassification::Internal,
                version: version(2),
            },
        ],
        stakeholder_relationships: vec![
            StakeholderRelationshipReadRecord {
                stakeholder_id: StakeholderId::parse("stakeholder-a").unwrap(),
                subject_type: "milestone".to_owned(),
                subject_id: "milestone-1".to_owned(),
                purpose: StakeholderRelationshipPurpose::Responsibility,
                classification: DataClassification::Restricted,
                version: version(3),
            },
            StakeholderRelationshipReadRecord {
                stakeholder_id: StakeholderId::parse("stakeholder-b").unwrap(),
                subject_type: "project".to_owned(),
                subject_id: "project-1".to_owned(),
                purpose: StakeholderRelationshipPurpose::Dependency,
                classification: DataClassification::Internal,
                version: version(1),
            },
        ],
        milestones: vec![MilestoneReadRecord {
            id: MilestoneId::parse("milestone-1").unwrap(),
            project_id: ProjectId::parse("project-1").unwrap(),
            name: "Synthetic Milestone".to_owned(),
            due_at: UtcTimestamp::from_unix_millis(5_000),
            classification: DataClassification::Restricted,
            version: version(3),
        }],
        action_requests: vec![
            ActionRequestReadRecord {
                id: "request-open".to_owned(),
                title: "Synthetic open request".to_owned(),
                intended_owner_id: Some(StakeholderId::parse("stakeholder-a").unwrap()),
                state: ActionRequestState::Open,
                response_due_at: Some(UtcTimestamp::from_unix_millis(9_000)),
                intended_action_due_at: None,
                classification: DataClassification::Internal,
                version: version(1),
            },
            ActionRequestReadRecord {
                id: "request-accepted".to_owned(),
                title: "Synthetic accepted request".to_owned(),
                intended_owner_id: Some(StakeholderId::parse("stakeholder-a").unwrap()),
                state: ActionRequestState::Accepted,
                response_due_at: None,
                intended_action_due_at: None,
                classification: DataClassification::Internal,
                version: version(2),
            },
            ActionRequestReadRecord {
                id: "request-unowned".to_owned(),
                title: "Synthetic unowned request".to_owned(),
                intended_owner_id: None,
                state: ActionRequestState::Open,
                response_due_at: None,
                intended_action_due_at: None,
                classification: DataClassification::Confidential,
                version: version(1),
            },
        ],
        // The People adapter reads neither, but the snapshot carries them for
        // the Work Queue, so the fixture must be a whole snapshot rather than
        // a convenient subset of one.
        decision_requests: Vec::new(),
        risks: Vec::new(),
        issues: Vec::new(),
        portfolio_relationships: Vec::new(),
        products: Vec::new(),
        initiatives: Vec::new(),
        projects: Vec::new(),
        roadmaps: Vec::new(),
        kpi_definitions: Vec::new(),
        evidence_links: Vec::new(),
        evidence_references: Vec::new(),
        kpi_observations: Vec::new(),
        work_owners: Vec::new(),
    }
}

#[test]
fn only_an_open_request_counts_as_outstanding() {
    // A draft has not been submitted; an accepted request has become an
    // Action and is a commitment rather than a request; declined and
    // withdrawn are terminal.
    assert!(is_outstanding(ActionRequestState::Open));
    for settled in [
        ActionRequestState::Draft,
        ActionRequestState::Accepted,
        ActionRequestState::Declined,
        ActionRequestState::Withdrawn,
    ] {
        assert!(!is_outstanding(settled), "{settled:?} is not outstanding");
    }
}

#[test]
fn an_accepted_request_never_appears_on_a_persons_outstanding_list() {
    // The DG1 request/commitment distinction, applied to People: once
    // accepted it is an Action, and showing it as an outstanding request
    // would make a commitment read as something still awaiting a response.
    let entries = directory_entries_from_snapshot(&snapshot());

    let person = &entries[0];
    assert_eq!(person.requests.len(), 1);
    assert_eq!(person.requests[0].id, "request-open");
}

#[test]
fn a_request_with_no_owner_is_not_attached_to_anybody() {
    // Its missing owner is the problem, and the attention evaluator reports
    // it. Attaching it to someone would invent the accountability the record
    // is missing.
    let entries = directory_entries_from_snapshot(&snapshot());

    for entry in &entries {
        for request in &entry.requests {
            assert_ne!(request.id, "request-unowned");
        }
    }
}

#[test]
fn a_milestone_subject_is_labelled_with_its_real_name() {
    let entries = directory_entries_from_snapshot(&snapshot());

    let person = &entries[0];
    assert_eq!(person.relationships.len(), 1);
    assert_eq!(person.relationships[0].subject_label, "Synthetic Milestone");
    assert_eq!(
        person.relationships[0].purpose,
        RelationshipPurpose::Responsibility
    );
}

#[test]
fn a_subject_with_no_readable_name_is_labelled_truthfully_rather_than_invented() {
    // This fixture holds no Project record. A label that made one up would
    // read as authoritative and be wrong; type-and-identifier is true and
    // unhelpful, which is the better failure.
    let entries = directory_entries_from_snapshot(&snapshot());

    let organization = &entries[1];
    assert_eq!(
        organization.relationships[0].subject_label,
        "project project-1"
    );
    assert_eq!(
        organization.relationships[0].purpose,
        RelationshipPurpose::Dependency
    );
}

#[test]
fn a_named_subject_is_labelled_by_name_and_its_classification_is_folded_in() {
    // Showing a Project's name discloses the Project, so a Restricted Project
    // makes the relationship Restricted even when the relationship record
    // itself is Internal -- and with it the person's entry.
    let mut source = snapshot();
    source.projects.push(ProjectReadRecord {
        id: ProjectId::parse("project-1").unwrap(),
        name: "Synthetic Project".to_owned(),
        start_at: UtcTimestamp::from_unix_millis(0),
        end_at: UtcTimestamp::from_unix_millis(10_000),
        classification: DataClassification::Restricted,
        version: version(4),
    });

    let entries = directory_entries_from_snapshot(&source);

    let organization = &entries[1];
    assert_eq!(
        organization.relationships[0].subject_label,
        "Synthetic Project"
    );
    assert_eq!(
        organization.relationships[0].classification,
        DataClassification::Restricted
    );
}

#[test]
fn a_record_of_another_type_with_the_same_identifier_lends_no_name() {
    // Identifiers are unique per type, not across types.
    let mut source = snapshot();
    source.products.push(ProductReadRecord {
        id: ProductId::parse("project-1").unwrap(),
        name: "Synthetic Product".to_owned(),
        classification: DataClassification::Restricted,
        version: version(1),
    });

    let entries = directory_entries_from_snapshot(&source);

    assert_eq!(
        entries[1].relationships[0].subject_label,
        "project project-1"
    );
    assert_eq!(
        entries[1].relationships[0].classification,
        DataClassification::Internal
    );
}

#[test]
fn every_fact_carries_the_revision_and_as_of_the_snapshot_holds() {
    let snapshot = snapshot();
    let entries = directory_entries_from_snapshot(&snapshot);

    let person = &entries[0];
    assert_eq!(person.stakeholder.revision, 1);
    assert_eq!(person.stakeholder.as_of, now());
    assert_eq!(person.relationships[0].revision, 3);
    assert_eq!(person.requests[0].revision, 1);
    assert_eq!(person.requests[0].as_of, now());
}

#[test]
fn a_person_with_nothing_attached_still_appears_in_the_directory() {
    // Absence of relationships is not absence of the person. Dropping them
    // would make the directory silently incomplete.
    let mut source = snapshot();
    source.stakeholder_relationships.clear();
    source.action_requests.clear();

    let entries = directory_entries_from_snapshot(&source);

    assert_eq!(entries.len(), 2);
    assert!(entries[0].relationships.is_empty());
    assert!(entries[0].requests.is_empty());
}

#[test]
fn an_unknown_stakeholder_reads_as_absent_not_as_empty() {
    // "No such person" and "a person with nothing attached" are different
    // answers, and a detail route must be able to tell them apart.
    let source = snapshot();

    assert!(stakeholder_detail_from_snapshot(&source, "stakeholder-a").is_some());
    assert!(stakeholder_detail_from_snapshot(&source, "stakeholder-missing").is_none());
}
