//! The Product detail and Product-health inspector composition (S02 detail
//! half and O01), per the accepted DG3 O01 amendment.
//!
//! The amendment exists because the frozen domain anchors work to people, not
//! to Products: no relationship kind pairs a Product with a Decision, Action,
//! Risk or Issue. So this surface shows three things it can show truthfully
//! -- the Product's structure, the Evidence linked to it, and the people
//! accountable for it -- and reaches work only through those people. Every
//! carried item is presented as carried by the person, never as the
//! Product's own. That framing is the whole point and is enforced by the
//! shape: there is no field on [`ProductDetailBody`] for "the Product's
//! Actions", so no adapter can populate one.
//!
//! `發生` is what is currently the case: Evidence whose verification has not
//! held, and attention on the work accountable people carry. It is never a
//! change over time, because nothing here reads history. `影響` is a human
//! judgment and is reported as unassessed. `下一步` is what each item's
//! lifecycle admits, which is strictly weaker than what may be done.
//!
//! Classification folds upward across everything shown, and the record that
//! forced the fold is named. A Stakeholder's name is classified data, so the
//! People tab is usually what forces it; a reader is told that rather than
//! left to guess.

use pmc_domain::attention::Freshness;
use pmc_domain::classification::DataClassification;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceRole, EvidenceVerification};

use crate::attention_ranking::rank_attention;
use crate::cockpit_aggregation::{health_reasons, ReasonSource, UnattributableReason};
use crate::people_composition::RelationshipPurpose;
use crate::route_composition::{
    AccountablePerson, CarriedWorkItem, ClassificationFold, ComposedEntity, ComposedEntityKind,
    CompositionResult, EvidenceEntry, FieldProvenance, ImpactJudgment, OwnerModule,
    ProductDetailBody, RouteState, Sourced, StructureEntry,
};
use crate::work_queue_composition::WorkItemFacts;

/// The Product itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductFacts {
    pub id: String,
    pub name: String,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
}

/// One structural child. `via` follows [`StructureEntry::via`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureFacts {
    pub kind: ComposedEntityKind,
    pub id: String,
    pub label: String,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
    pub via: Option<String>,
}

/// One Evidence reference linked to the Product.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceFacts {
    pub id: String,
    pub role: Option<EvidenceRole>,
    pub verification: EvidenceVerification,
    /// Whether the reference carries a pinned fingerprint. A separate fact
    /// from `verification`: it says what the next observation *can* find.
    pub pinned: bool,
    /// The Evidence record's current classification.
    pub classification: DataClassification,
    pub classification_at_link: DataClassification,
    pub linked_at: UtcTimestamp,
    pub revision: u64,
    pub as_of: UtcTimestamp,
}

/// One accountable person and what they carry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonFacts {
    pub id: String,
    pub display_name: String,
    pub purpose: RelationshipPurpose,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
    pub other_products_accountable_for: usize,
    /// The work this person owns, as the Work Queue would present it. Reused
    /// rather than re-derived so labels, states, intents and attention are
    /// the same here as in the queue.
    pub carried: Vec<WorkItemFacts>,
}

/// Everything the adapter supplies for one Product.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductDetailFacts {
    pub product: ProductFacts,
    pub structure: Vec<StructureFacts>,
    pub evidence: Vec<EvidenceFacts>,
    pub people: Vec<PersonFacts>,
}

fn entity(
    kind: ComposedEntityKind,
    id: &str,
    owner: OwnerModule,
    classification: DataClassification,
    revision: u64,
    as_of: UtcTimestamp,
) -> ComposedEntity {
    ComposedEntity {
        kind,
        id: id.to_owned(),
        owner,
        revision,
        as_of,
        classification,
        freshness: Freshness::Fresh,
        degraded: false,
        attention: Vec::new(),
        // Composition offers no write. Legal intents are carried per item on
        // the People tab, where they belong to the item, not to the Product.
        lifecycle_legal_intents: Vec::new(),
    }
}

const fn structure_owner(kind: ComposedEntityKind) -> OwnerModule {
    match kind {
        ComposedEntityKind::Kpi => OwnerModule::Kpi,
        // Projects, Milestones and Roadmaps are Delivery's; Initiatives and
        // the Product itself are Portfolio's.
        ComposedEntityKind::Project
        | ComposedEntityKind::Milestone
        | ComposedEntityKind::Roadmap => OwnerModule::Delivery,
        _ => OwnerModule::Portfolio,
    }
}

/// The effective classification of the whole inspector, and the child that
/// forced it above the Product's own, if any.
///
/// Folds the Product with every structural child, every Evidence record and
/// link, every person, and every item they carry. `Unclassified` is absorbing
/// in [`DataClassification::combine`], so one unclassified child makes the
/// whole surface unclassified -- correct, fail-closed, and exactly why the
/// forcing child is named.
#[must_use]
pub fn effective_classification(
    facts: &ProductDetailFacts,
) -> (DataClassification, Option<ClassificationFold>) {
    let mut effective = facts.product.classification;
    let mut forced_by: Option<ClassificationFold> = None;

    let mut fold = |kind: ComposedEntityKind, id: &str, classification: DataClassification| {
        let next = effective.combine(classification);
        if next != effective {
            forced_by = Some(ClassificationFold {
                forced_by_kind: kind,
                forced_by_id: id.to_owned(),
                classification: next,
            });
            effective = next;
        }
    };

    for child in &facts.structure {
        fold(child.kind, &child.id, child.classification);
    }
    for evidence in &facts.evidence {
        fold(
            ComposedEntityKind::Evidence,
            &evidence.id,
            evidence.classification,
        );
        fold(
            ComposedEntityKind::Evidence,
            &evidence.id,
            evidence.classification_at_link,
        );
    }
    for person in &facts.people {
        fold(
            ComposedEntityKind::Stakeholder,
            &person.id,
            person.classification,
        );
        for item in &person.carried {
            fold(kind_of_work(item), &item.id, item.classification);
        }
    }
    (effective, forced_by)
}

const fn kind_of_work(item: &WorkItemFacts) -> ComposedEntityKind {
    match item.kind {
        crate::work_queue_composition::WorkItemKind::ActionRequest => {
            ComposedEntityKind::ActionRequest
        }
        crate::work_queue_composition::WorkItemKind::Action => ComposedEntityKind::Action,
        crate::work_queue_composition::WorkItemKind::DecisionRequest => {
            ComposedEntityKind::DecisionRequest
        }
        crate::work_queue_composition::WorkItemKind::Risk => ComposedEntityKind::Risk,
        crate::work_queue_composition::WorkItemKind::Issue => ComposedEntityKind::Issue,
    }
}

const fn verification_condition(verification: &EvidenceVerification) -> Option<&'static str> {
    match verification {
        EvidenceVerification::Verified { .. } => None,
        EvidenceVerification::DegradedLastVerified { .. } => {
            Some("its last verification is degraded")
        }
        EvidenceVerification::ObservedUnpinned { .. } => {
            Some("it carries no pinned fingerprint, so nothing was verified")
        }
        EvidenceVerification::Unverified => Some("it has not been verified"),
        EvidenceVerification::IntegrityMismatch => Some("its integrity does not match"),
    }
}

/// `發生`: the current conditions, each attributed to what raised it.
///
/// Two sources, and only two. Evidence linked to the Product whose
/// verification has not held is attributed to the Evidence record. Attention
/// on work that accountable people carry is attributed to the work record,
/// through [`health_reasons`] so the attribution rule lives in one place.
/// Nothing here is attributed to the Product, and nothing here compares with
/// an earlier state.
pub fn current_conditions(
    facts: &ProductDetailFacts,
) -> Result<Vec<Sourced<String>>, UnattributableReason> {
    let mut reasons: Vec<Sourced<String>> = facts
        .evidence
        .iter()
        .filter_map(|evidence| {
            verification_condition(&evidence.verification).map(|condition| {
                Sourced::new(
                    format!("Evidence {} is linked, but {}", evidence.id, condition),
                    FieldProvenance::new(
                        OwnerModule::Evidence,
                        evidence.id.clone(),
                        "verification",
                        evidence.revision,
                        evidence.as_of,
                    ),
                )
            })
        })
        .collect();

    // Gather every carried flag onto one carrier so the shared attribution
    // rule sees them all, together with the source each came from.
    let mut carrier = entity(
        ComposedEntityKind::Product,
        &facts.product.id,
        OwnerModule::Portfolio,
        facts.product.classification,
        facts.product.revision,
        facts.product.as_of,
    );
    let mut sources = Vec::new();
    let mut rankable = Vec::new();
    for person in &facts.people {
        for item in &person.carried {
            for flag in &item.attention {
                sources.push(ReasonSource {
                    target: flag.flag.target.clone(),
                    revision: item.revision,
                    as_of: item.as_of,
                });
                rankable.push(flag.clone());
            }
        }
    }
    carrier.attention = rank_attention(&rankable);
    reasons.extend(health_reasons(&carrier, &sources)?);
    Ok(reasons)
}

/// Composes the Product detail and inspector.
///
/// Returns an error rather than a body if any carried flag cannot be
/// attributed to its source; the adapter supplies both from the same records,
/// so that is a bug to surface, not a state to render.
pub fn compose_product_detail(
    facts: &ProductDetailFacts,
    ledger_revision: u64,
) -> Result<CompositionResult<ProductDetailBody>, UnattributableReason> {
    let (classification, classification_forced_by) = effective_classification(facts);
    let health = current_conditions(facts)?;

    let product = entity(
        ComposedEntityKind::Product,
        &facts.product.id,
        OwnerModule::Portfolio,
        classification,
        facts.product.revision,
        facts.product.as_of,
    );

    let structure = facts
        .structure
        .iter()
        .map(|child| StructureEntry {
            entity: entity(
                child.kind,
                &child.id,
                structure_owner(child.kind),
                child.classification,
                child.revision,
                child.as_of,
            ),
            label: child.label.clone(),
            via: child.via.clone(),
        })
        .collect();

    let evidence = facts
        .evidence
        .iter()
        .map(|record| EvidenceEntry {
            entity: entity(
                ComposedEntityKind::Evidence,
                &record.id,
                OwnerModule::Evidence,
                record.classification,
                record.revision,
                record.as_of,
            ),
            role: record.role,
            verification: record.verification.clone(),
            pinned: record.pinned,
            classification_at_link: record.classification_at_link,
            linked_at: record.linked_at,
        })
        .collect();

    let people = facts
        .people
        .iter()
        .map(|person| AccountablePerson {
            entity: entity(
                ComposedEntityKind::Stakeholder,
                &person.id,
                OwnerModule::Stakeholders,
                person.classification,
                person.revision,
                person.as_of,
            ),
            display_name: person.display_name.clone(),
            purpose: person.purpose,
            other_products_accountable_for: person.other_products_accountable_for,
            carried: person
                .carried
                .iter()
                .map(|item| CarriedWorkItem {
                    kind: item.kind,
                    id: item.id.clone(),
                    label: item.label.clone(),
                    state_label: item.state_label,
                    lifecycle_legal_intents: item.lifecycle_legal_intents.clone(),
                    classification: item.classification,
                    revision: item.revision,
                    as_of: item.as_of,
                    attention: rank_attention(&item.attention),
                })
                .collect(),
        })
        .collect();

    Ok(CompositionResult {
        state: RouteState::Success,
        as_of: facts.product.as_of,
        ledger_revision,
        body: ProductDetailBody {
            product,
            product_label: facts.product.name.clone(),
            classification_forced_by,
            structure,
            evidence,
            people,
            health_reasons: health,
            impact: ImpactJudgment::Unassessed,
        },
    })
}

/// What a health reason is about, so a surface can word it: the reason's
/// stable code, the kind of record that raised it, and that record's label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthReasonSubject {
    /// A verification kind for an Evidence condition; an attention reason
    /// for a carried-work condition.
    pub reason_code: &'static str,
    /// `evidence`, or the Work Queue kind of the carried item.
    pub subject_kind: &'static str,
    pub subject_label: String,
}

/// Reads back, from the same composed body, what one health reason is about.
///
/// Matched on the provenance the reason already carries -- the source field
/// and record -- and, for carried work, on the flag whose explanation
/// produced the sentence; the sentence itself is never parsed. `None` when
/// nothing in the body matches, in which case a surface shows the sentence as
/// it is.
#[must_use]
pub fn health_reason_subject(
    body: &ProductDetailBody,
    reason: &Sourced<String>,
) -> Option<HealthReasonSubject> {
    let provenance = reason.provenance();
    let record = provenance.source_record_id();
    if provenance.source_field() == "verification" {
        return body
            .evidence
            .iter()
            .find(|entry| entry.entity.id == record)
            .map(|entry| HealthReasonSubject {
                reason_code: entry.verification.kind_as_persisted(),
                subject_kind: "evidence",
                subject_label: entry.entity.id.clone(),
            });
    }
    body.people
        .iter()
        .flat_map(|person| person.carried.iter())
        .find_map(|item| {
            item.attention
                .iter()
                .find(|ranked| {
                    ranked.flag.explanation == reason.value().as_str()
                        && crate::attention_ranking::canonical_id_of(&ranked.flag.target) == record
                })
                .map(|ranked| HealthReasonSubject {
                    reason_code: ranked.flag.reason.as_str(),
                    subject_kind: item.kind.as_str(),
                    subject_label: item.label.clone(),
                })
        })
}

/// How much flagged work the people accountable for a Product carry: the
/// number of distinct items with at least one flag, and the most restrictive
/// classification among them (`None` when there are none).
///
/// A count is a disclosure too -- a Restricted item counted beside an
/// Internal label would say more than the label -- so a surface that shows
/// the count must fold this classification into the one it shows.
#[must_use]
pub fn flagged_carried_work(facts: &ProductDetailFacts) -> (usize, Option<DataClassification>) {
    let mut seen: Vec<(crate::work_queue_composition::WorkItemKind, &str)> = Vec::new();
    let mut classification: Option<DataClassification> = None;
    for item in facts.people.iter().flat_map(|person| person.carried.iter()) {
        if item.attention.is_empty() || seen.contains(&(item.kind, item.id.as_str())) {
            continue;
        }
        seen.push((item.kind, item.id.as_str()));
        classification = Some(classification.map_or(item.classification, |folded| {
            folded.combine(item.classification)
        }));
    }
    (seen.len(), classification)
}
