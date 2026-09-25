//! The production People (S09) adapter.
//!
//! Maps a real `LedgerCompositionSnapshot` into the shapes
//! [`crate::people_composition`] already composes and tests. The composition
//! rules -- classification folding upward, Responsibility staying distinct
//! from Dependency, S2 keeping every write intent -- live there and are not
//! restated here; this module only supplies real facts to them.
//!
//! Two decisions about naming, both of which refuse to invent:
//!
//! A relationship's subject is labelled with its name wherever the snapshot
//! carries one -- Milestones, Products, Initiatives, Projects, Roadmaps and
//! KPI definitions. A named subject's classification is folded into the
//! relationship's, because showing the name discloses the record. For any
//! other subject (a Portfolio, or a record the snapshot does not hold) the
//! label is the type and identifier, which is true and unhelpful rather than
//! helpful and invented.
//!
//! "Outstanding" means a request in the `Open` state. A draft has not been
//! submitted, an accepted request has become an Action and is a commitment
//! rather than a request, and declined and withdrawn are terminal. The read
//! surface deliberately reports every state so this judgement is made here,
//! visibly, rather than hidden in a query.

use std::collections::HashMap;

use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{ActionRequestReadRecord, LedgerCompositionSnapshot};
use pmc_domain::relationships::StakeholderRelationshipPurpose;
use pmc_domain::work_management::ActionRequestState;

use crate::people_composition::{
    DirectoryEntryFacts, OutstandingRequestFacts, RelationshipFacts, RelationshipPurpose,
    StakeholderFacts,
};

/// The composition-side purpose for a persisted one. Shared with the Product
/// inspector so the two surfaces cannot disagree about what a relationship
/// means.
#[must_use]
pub const fn purpose_of(purpose: StakeholderRelationshipPurpose) -> RelationshipPurpose {
    match purpose {
        StakeholderRelationshipPurpose::Responsibility => RelationshipPurpose::Responsibility,
        StakeholderRelationshipPurpose::Dependency => RelationshipPurpose::Dependency,
    }
}

/// A request is outstanding only while it is still open.
#[must_use]
pub const fn is_outstanding(state: ActionRequestState) -> bool {
    matches!(state, ActionRequestState::Open)
}

/// Every subject the snapshot names, keyed by its persisted type and id, with
/// the classification its name carries.
fn named_subjects(
    snapshot: &LedgerCompositionSnapshot,
) -> HashMap<(&str, &str), (&str, DataClassification)> {
    let mut named = HashMap::new();
    for record in &snapshot.milestones {
        named.insert(
            ("milestone", record.id.as_str()),
            (record.name.as_str(), record.classification),
        );
    }
    for record in &snapshot.products {
        named.insert(
            ("product", record.id.as_str()),
            (record.name.as_str(), record.classification),
        );
    }
    for record in &snapshot.initiatives {
        named.insert(
            ("initiative", record.id.as_str()),
            (record.name.as_str(), record.classification),
        );
    }
    for record in &snapshot.projects {
        named.insert(
            ("project", record.id.as_str()),
            (record.name.as_str(), record.classification),
        );
    }
    for record in &snapshot.roadmaps {
        named.insert(
            ("roadmap", record.id.as_str()),
            (record.name.as_str(), record.classification),
        );
    }
    for record in &snapshot.kpi_definitions {
        named.insert(
            ("kpi_definition", record.id.as_str()),
            (record.name.as_str(), record.classification),
        );
    }
    named
}

fn relationship_facts(
    snapshot: &LedgerCompositionSnapshot,
) -> HashMap<&str, Vec<RelationshipFacts>> {
    let named = named_subjects(snapshot);
    let mut grouped: HashMap<&str, Vec<RelationshipFacts>> = HashMap::new();
    for relationship in &snapshot.stakeholder_relationships {
        let subject = named
            .get(&(
                relationship.subject_type.as_str(),
                relationship.subject_id.as_str(),
            ))
            .copied();
        let (subject_label, classification) = subject.map_or_else(
            || {
                (
                    format!("{} {}", relationship.subject_type, relationship.subject_id),
                    relationship.classification,
                )
            },
            |(name, subject_classification)| {
                (
                    name.to_owned(),
                    relationship.classification.combine(subject_classification),
                )
            },
        );
        grouped
            .entry(relationship.stakeholder_id.as_str())
            .or_default()
            .push(RelationshipFacts {
                subject_id: relationship.subject_id.clone(),
                subject_label,
                purpose: purpose_of(relationship.purpose),
                classification,
                revision: relationship.version.get(),
                as_of: snapshot.ledger_as_of_utc,
            });
    }
    grouped
}
fn outstanding_facts(
    snapshot: &LedgerCompositionSnapshot,
) -> HashMap<&str, Vec<OutstandingRequestFacts>> {
    let mut grouped: HashMap<&str, Vec<OutstandingRequestFacts>> = HashMap::new();
    for request in &snapshot.action_requests {
        if !is_outstanding(request.state) {
            continue;
        }
        // A request with no intended owner belongs to nobody's page. It is
        // not silently attached to someone: the missing owner is itself the
        // problem, and the attention evaluator is what reports it.
        let Some(owner) = request.intended_owner_id.as_ref() else {
            continue;
        };
        grouped
            .entry(owner.as_str())
            .or_default()
            .push(request_facts(request, snapshot));
    }
    grouped
}

fn request_facts(
    request: &ActionRequestReadRecord,
    snapshot: &LedgerCompositionSnapshot,
) -> OutstandingRequestFacts {
    OutstandingRequestFacts {
        id: request.id.clone(),
        classification: request.classification,
        revision: request.version.get(),
        as_of: snapshot.ledger_as_of_utc,
    }
}

/// Builds every directory entry from a real snapshot, in the snapshot's own
/// stable order.
#[must_use]
pub fn directory_entries_from_snapshot(
    snapshot: &LedgerCompositionSnapshot,
) -> Vec<DirectoryEntryFacts> {
    let relationships = relationship_facts(snapshot);
    let requests = outstanding_facts(snapshot);
    snapshot
        .stakeholders
        .iter()
        .map(|stakeholder| {
            let id = stakeholder.id.as_str();
            DirectoryEntryFacts {
                stakeholder: StakeholderFacts {
                    id: id.to_owned(),
                    display_name: stakeholder.display_name.clone(),
                    classification: stakeholder.classification,
                    revision: stakeholder.version.get(),
                    as_of: snapshot.ledger_as_of_utc,
                },
                relationships: relationships.get(id).cloned().unwrap_or_default(),
                requests: requests.get(id).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

/// The facts for one Stakeholder, or `None` when the Ledger holds no such
/// person -- which is a different answer from a person who happens to have
/// nothing attached to them.
#[must_use]
pub fn stakeholder_detail_from_snapshot(
    snapshot: &LedgerCompositionSnapshot,
    stakeholder_id: &str,
) -> Option<DirectoryEntryFacts> {
    directory_entries_from_snapshot(snapshot)
        .into_iter()
        .find(|entry| entry.stakeholder.id == stakeholder_id)
}
