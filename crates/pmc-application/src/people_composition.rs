//! The People route-composition pair.
//!
//! Composes `GetPeopleDirectory` and `GetStakeholderDetail`, and integrates
//! Evidence and degraded state through an owner port while preserving
//! field-level provenance. The Stakeholders owner module remains
//! authoritative for Stakeholder records and for every write intent; nothing
//! here writes, and nothing here offers a write.
//!
//! One property carries most of the weight. A People surface exposes what a
//! person is accountable for, and those subjects can be classified more
//! restrictively than the person's own record. Composition therefore folds
//! classification **upward**: an entry is presented at the most restrictive
//! classification of anything it exposes, never at its own. Presenting a
//! Restricted responsibility beneath an Internal directory entry would leak by
//! aggregation even though every individual field was labelled correctly.
//! This mirrors how the managed projection folds classification.
//!
//! The Responsibility and Dependency purposes stay distinct throughout.
//! Collapsing them into one "involvement" list would misstate accountability:
//! being responsible for a Milestone and depending on one are different
//! relationships to the same subject.

use pmc_domain::classification::DataClassification;
use pmc_domain::time::UtcTimestamp;

use crate::route_composition::{
    ComposedEntity, ComposedEntityKind, CompositionResult, FieldProvenance, OwnerModule, PageState,
    PeopleDirectoryBody, RouteState, Sourced, StakeholderDetailBody,
};

/// What the Evidence module can currently say about Evidence for one subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceAvailability {
    /// Evidence is linked and its last verification held.
    Verified,
    /// Linked, but verification has not been completed.
    VerificationPending,
    /// The Vault could not be consulted, so this is unknown rather than
    /// absent. The distinction matters: "no Evidence" and "we could not look"
    /// are different facts and must not render the same.
    Degraded { because: &'static str },
    /// Nothing is linked, and the Vault answered.
    NotLinked,
}

impl EvidenceAvailability {
    #[must_use]
    pub const fn is_degraded(&self) -> bool {
        matches!(self, Self::Degraded { .. })
    }
}

/// Supplies Evidence and degraded state.
///
/// The Evidence/Vault module owns these facts. The port keeps composition from reaching into
/// the Vault directly, and keeps "we could not look" reportable rather than
/// collapsing into "nothing found".
pub trait EvidencePort {
    fn evidence_for(&self, subject_id: &str) -> EvidenceAvailability;
}

/// Answers when no Vault is attached. Reports degraded rather than
/// `NotLinked`, because an absent Vault is not evidence of absent Evidence.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnavailableVaultPort;

impl EvidencePort for UnavailableVaultPort {
    fn evidence_for(&self, _subject_id: &str) -> EvidenceAvailability {
        EvidenceAvailability::Degraded {
            because: "no Product Vault is attached, so Evidence could not be consulted",
        }
    }
}

/// How a Stakeholder relates to a subject. Mirrors the domain's own
/// `StakeholderRelationshipPurpose`; the two are kept distinct here for the
/// same reason the domain keeps them distinct.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelationshipPurpose {
    Responsibility,
    Dependency,
}

impl RelationshipPurpose {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Responsibility => "responsibility",
            Self::Dependency => "dependency",
        }
    }
}

/// One Stakeholder as the Stakeholders owner module holds them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderFacts {
    pub id: String,
    pub display_name: String,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
}

/// One relationship between a Stakeholder and a subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipFacts {
    pub subject_id: String,
    pub subject_label: String,
    pub purpose: RelationshipPurpose,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
}

/// An Action Request naming this Stakeholder as its intended owner.
///
/// A request, deliberately not an Action: it is not yet a commitment, and the
/// People surface must not let one read as one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutstandingRequestFacts {
    pub id: String,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
}

/// The most restrictive classification across a Stakeholder and everything
/// the surface would expose about them.
///
/// This is the aggregation guard. Every individual field can be labelled
/// correctly and the entry still leak, because putting them together reveals
/// more than any one of them.
#[must_use]
pub fn effective_classification(
    stakeholder: &StakeholderFacts,
    relationships: &[RelationshipFacts],
    requests: &[OutstandingRequestFacts],
) -> DataClassification {
    let mut effective = stakeholder.classification;
    for relationship in relationships {
        effective = effective.combine(relationship.classification);
    }
    for request in requests {
        effective = effective.combine(request.classification);
    }
    effective
}

fn stakeholder_entity(
    stakeholder: &StakeholderFacts,
    classification: DataClassification,
) -> ComposedEntity {
    ComposedEntity {
        kind: ComposedEntityKind::Stakeholder,
        id: stakeholder.id.clone(),
        owner: OwnerModule::Stakeholders,
        revision: stakeholder.revision,
        as_of: stakeholder.as_of,
        classification,
        freshness: pmc_domain::attention::Freshness::Fresh,
        degraded: false,
        attention: Vec::new(),
        // The owner module owns every write intent. Composition never offers
        // one, so this stays empty here rather than being guessed from the
        // record's state.
        lifecycle_legal_intents: Vec::new(),
    }
}

/// One directory entry, already folded to its effective classification.
pub struct DirectoryEntryFacts {
    pub stakeholder: StakeholderFacts,
    pub relationships: Vec<RelationshipFacts>,
    pub requests: Vec<OutstandingRequestFacts>,
}

/// Composes the People directory.
///
/// Ordered by canonical identifier rather than by name: a display name is not
/// stable, and a directory whose order shifts when someone is renamed is not
/// a directory a reader can return to.
#[must_use]
pub fn compose_people_directory(
    entries: &[DirectoryEntryFacts],
    as_of: UtcTimestamp,
    ledger_revision: u64,
    page: PageState,
) -> CompositionResult<PeopleDirectoryBody> {
    let mut composed: Vec<ComposedEntity> = entries
        .iter()
        .map(|entry| {
            let classification =
                effective_classification(&entry.stakeholder, &entry.relationships, &entry.requests);
            stakeholder_entity(&entry.stakeholder, classification)
        })
        .collect();
    composed.sort_by(|left, right| left.id.cmp(&right.id));
    let stakeholders: Vec<ComposedEntity> = composed
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect();
    CompositionResult {
        state: if stakeholders.is_empty() {
            RouteState::Empty
        } else {
            RouteState::Success
        },
        as_of,
        ledger_revision,
        body: PeopleDirectoryBody { stakeholders, page },
    }
}

/// Composes one Stakeholder's detail, with Evidence state from the Evidence port.
///
/// Responsibilities and dependencies are returned as one ordered list of
/// `Sourced` values whose text names the purpose explicitly, so the two can
/// never be read as interchangeable.
#[must_use]
pub fn compose_stakeholder_detail(
    stakeholder: &StakeholderFacts,
    relationships: &[RelationshipFacts],
    requests: &[OutstandingRequestFacts],
    ledger_revision: u64,
    evidence: &dyn EvidencePort,
) -> CompositionResult<StakeholderDetailBody> {
    let classification = effective_classification(stakeholder, relationships, requests);
    let mut entity = stakeholder_entity(stakeholder, classification);

    let mut any_degraded = false;
    let responsibilities: Vec<Sourced<String>> = relationships
        .iter()
        .map(|relationship| {
            let availability = evidence.evidence_for(&relationship.subject_id);
            any_degraded |= availability.is_degraded();
            Sourced::new(
                format!(
                    "{} for {}",
                    relationship.purpose.as_str(),
                    relationship.subject_label
                ),
                FieldProvenance::new(
                    OwnerModule::Stakeholders,
                    relationship.subject_id.clone(),
                    "stakeholder_relationship",
                    relationship.revision,
                    relationship.as_of,
                ),
            )
        })
        .collect();

    entity.degraded = any_degraded;

    let outstanding_requests: Vec<ComposedEntity> = requests
        .iter()
        .map(|request| ComposedEntity {
            // An Action Request, and labelled as one. This was
            // `ComposedEntityKind::Product` until the kinds a route can
            // compose were widened; the People DTO never surfaced the kind,
            // which is the only reason the mislabel was invisible.
            kind: ComposedEntityKind::ActionRequest,
            id: request.id.clone(),
            owner: OwnerModule::ActionManagement,
            revision: request.revision,
            as_of: request.as_of,
            classification: request.classification,
            freshness: pmc_domain::attention::Freshness::Fresh,
            degraded: false,
            attention: Vec::new(),
            lifecycle_legal_intents: Vec::new(),
        })
        .collect();

    CompositionResult {
        state: if any_degraded {
            RouteState::Degraded
        } else {
            RouteState::Success
        },
        as_of: stakeholder.as_of,
        ledger_revision,
        body: StakeholderDetailBody {
            stakeholder: entity,
            responsibilities,
            outstanding_requests,
        },
    }
}
