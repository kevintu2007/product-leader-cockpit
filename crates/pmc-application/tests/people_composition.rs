//! The People route-composition pair.

use pmc_application::people_composition::{
    compose_people_directory, compose_stakeholder_detail, effective_classification,
    DirectoryEntryFacts, EvidenceAvailability, EvidencePort, OutstandingRequestFacts,
    RelationshipFacts, RelationshipPurpose, StakeholderFacts, UnavailableVaultPort,
};
use pmc_application::route_composition::{OwnerModule, PageState, RouteState};
use pmc_domain::classification::DataClassification;
use pmc_domain::time::UtcTimestamp;

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

fn stakeholder(id: &str, classification: DataClassification) -> StakeholderFacts {
    StakeholderFacts {
        id: id.to_owned(),
        display_name: format!("Name of {id}"),
        classification,
        revision: 7,
        as_of: now(),
    }
}

fn relationship(
    subject_id: &str,
    purpose: RelationshipPurpose,
    classification: DataClassification,
) -> RelationshipFacts {
    RelationshipFacts {
        subject_id: subject_id.to_owned(),
        subject_label: format!("Subject {subject_id}"),
        purpose,
        classification,
        revision: 3,
        as_of: now(),
    }
}

fn request(id: &str, classification: DataClassification) -> OutstandingRequestFacts {
    OutstandingRequestFacts {
        id: id.to_owned(),
        classification,
        revision: 2,
        as_of: now(),
    }
}

/// Reports everything verified, so degraded state cannot be an artefact of
/// the default port.
struct VerifiedVault;
impl EvidencePort for VerifiedVault {
    fn evidence_for(&self, _subject_id: &str) -> EvidenceAvailability {
        EvidenceAvailability::Verified
    }
}

#[test]
fn an_entry_is_presented_at_the_most_restrictive_thing_it_exposes() {
    // The aggregation leak: every field can be labelled correctly and the
    // entry still reveal more than any one of them, because putting a
    // person together with what they are accountable for is itself
    // disclosure.
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let relationships = vec![relationship(
        "project-1",
        RelationshipPurpose::Responsibility,
        DataClassification::Restricted,
    )];

    let effective = effective_classification(&person, &relationships, &[]);

    assert_eq!(
        effective,
        DataClassification::Restricted,
        "an Internal person exposing a Restricted responsibility must be presented as Restricted"
    );
}

#[test]
fn an_outstanding_request_also_raises_the_effective_classification() {
    let person = stakeholder("stakeholder-1", DataClassification::Public);
    let requests = vec![request(
        "action-request-1",
        DataClassification::Confidential,
    )];

    assert_eq!(
        effective_classification(&person, &[], &requests),
        DataClassification::Confidential
    );
}

#[test]
fn an_unclassified_fact_makes_the_whole_entry_unclassified() {
    // `Unclassified` is the most restrictive rank and is absorbing, so an
    // unknown classification fails closed rather than being treated as
    // harmless.
    let person = stakeholder("stakeholder-1", DataClassification::Public);
    let relationships = vec![relationship(
        "project-1",
        RelationshipPurpose::Dependency,
        DataClassification::Unclassified,
    )];

    assert_eq!(
        effective_classification(&person, &relationships, &[]),
        DataClassification::Unclassified
    );
}

#[test]
fn a_person_with_nothing_attached_keeps_their_own_classification() {
    let person = stakeholder("stakeholder-1", DataClassification::Internal);

    assert_eq!(
        effective_classification(&person, &[], &[]),
        DataClassification::Internal
    );
}

#[test]
fn the_directory_presents_each_entry_at_its_effective_classification() {
    let entries = vec![DirectoryEntryFacts {
        stakeholder: stakeholder("stakeholder-1", DataClassification::Internal),
        relationships: vec![relationship(
            "project-1",
            RelationshipPurpose::Responsibility,
            DataClassification::Restricted,
        )],
        requests: Vec::new(),
    }];

    let result = compose_people_directory(
        &entries,
        now(),
        7,
        PageState {
            offset: 0,
            limit: 10,
            total: 1,
        },
    );

    assert_eq!(
        result.body.stakeholders[0].classification,
        DataClassification::Restricted,
        "the directory must not present the record's own weaker label"
    );
}

#[test]
fn the_directory_orders_by_identifier_not_display_name() {
    // A display name is not stable. A directory whose order shifts when
    // someone is renamed is not one a reader can return to.
    let entries = vec![
        DirectoryEntryFacts {
            stakeholder: stakeholder("stakeholder-c", DataClassification::Internal),
            relationships: Vec::new(),
            requests: Vec::new(),
        },
        DirectoryEntryFacts {
            stakeholder: stakeholder("stakeholder-a", DataClassification::Internal),
            relationships: Vec::new(),
            requests: Vec::new(),
        },
        DirectoryEntryFacts {
            stakeholder: stakeholder("stakeholder-b", DataClassification::Internal),
            relationships: Vec::new(),
            requests: Vec::new(),
        },
    ];

    let result = compose_people_directory(
        &entries,
        now(),
        7,
        PageState {
            offset: 0,
            limit: 10,
            total: 3,
        },
    );

    let ids: Vec<&str> = result
        .body
        .stakeholders
        .iter()
        .map(|entity| entity.id.as_str())
        .collect();
    assert_eq!(ids, vec!["stakeholder-a", "stakeholder-b", "stakeholder-c"]);
}

#[test]
fn an_empty_directory_reports_empty() {
    let result = compose_people_directory(
        &[],
        now(),
        7,
        PageState {
            offset: 0,
            limit: 10,
            total: 0,
        },
    );

    assert_eq!(result.state, RouteState::Empty);
}

#[test]
fn responsibility_and_dependency_stay_distinguishable() {
    // Collapsing them would misstate accountability: being responsible for a
    // Milestone and depending on one are different relationships.
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let relationships = vec![
        relationship(
            "project-1",
            RelationshipPurpose::Responsibility,
            DataClassification::Internal,
        ),
        relationship(
            "project-2",
            RelationshipPurpose::Dependency,
            DataClassification::Internal,
        ),
    ];

    let result = compose_stakeholder_detail(&person, &relationships, &[], 7, &VerifiedVault);

    let texts: Vec<&str> = result
        .body
        .responsibilities
        .iter()
        .map(|entry| entry.value().as_str())
        .collect();
    assert!(texts[0].starts_with("responsibility for "));
    assert!(texts[1].starts_with("dependency for "));
}

#[test]
fn every_responsibility_keeps_the_source_it_came_from() {
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let relationships = vec![relationship(
        "project-1",
        RelationshipPurpose::Responsibility,
        DataClassification::Internal,
    )];

    let result = compose_stakeholder_detail(&person, &relationships, &[], 7, &VerifiedVault);

    let provenance = result.body.responsibilities[0].provenance();
    assert_eq!(provenance.owner(), OwnerModule::Stakeholders);
    assert_eq!(provenance.source_record_id(), "project-1");
    assert_eq!(provenance.source_field(), "stakeholder_relationship");
    assert_eq!(provenance.source_revision(), 3);
}

#[test]
fn an_unreachable_vault_reports_degraded_rather_than_no_evidence() {
    // "No Evidence" and "we could not look" are different facts. Rendering
    // them the same would let an unreachable Vault read as a clean record.
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let relationships = vec![relationship(
        "project-1",
        RelationshipPurpose::Responsibility,
        DataClassification::Internal,
    )];

    let result = compose_stakeholder_detail(&person, &relationships, &[], 7, &UnavailableVaultPort);

    assert_eq!(result.state, RouteState::Degraded);
    assert!(result.body.stakeholder.degraded);
}

#[test]
fn a_reachable_vault_leaves_the_route_undegraded() {
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let relationships = vec![relationship(
        "project-1",
        RelationshipPurpose::Responsibility,
        DataClassification::Internal,
    )];

    let result = compose_stakeholder_detail(&person, &relationships, &[], 7, &VerifiedVault);

    assert_eq!(result.state, RouteState::Success);
    assert!(!result.body.stakeholder.degraded);
}

#[test]
fn composition_offers_no_write_intent() {
    // S2 remains authoritative for Stakeholder records and every write
    // intent. A People surface that offered one would imply an authority
    // composition does not have.
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let requests = vec![request("action-request-1", DataClassification::Internal)];

    let result = compose_stakeholder_detail(&person, &[], &requests, 7, &VerifiedVault);

    assert!(result.body.stakeholder.lifecycle_legal_intents.is_empty());
    for entity in &result.body.outstanding_requests {
        assert!(entity.lifecycle_legal_intents.is_empty());
    }
}

#[test]
fn an_outstanding_request_is_owned_by_action_management_not_by_the_person() {
    // The request belongs to the Action Management module; People only shows
    // it. Attributing it to Stakeholders would put the write authority in
    // the wrong place.
    let person = stakeholder("stakeholder-1", DataClassification::Internal);
    let requests = vec![request("action-request-1", DataClassification::Internal)];

    let result = compose_stakeholder_detail(&person, &[], &requests, 7, &VerifiedVault);

    assert_eq!(
        result.body.outstanding_requests[0].owner,
        OwnerModule::ActionManagement
    );
}
