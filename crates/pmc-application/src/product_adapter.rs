//! The production Product-detail adapter (S02 detail, O01).
//!
//! Maps the two real Ledger snapshots into [`ProductDetailFacts`]. The
//! composition rules -- upward classification folding, attribution of every
//! condition to the record that raised it, work only via accountability --
//! live in [`crate::product_composition`] and are not restated here. This
//! module decides only *which* records belong to one Product, and it does so
//! by walking edges the Ledger actually holds:
//!
//! | Facts | Edge walked |
//! |---|---|
//! | Projects | `project_product` relationship |
//! | Milestones | `milestones.project_id` of those Projects |
//! | Roadmaps, KPIs | `product_roadmap`, `product_kpi` |
//! | Initiatives | `initiative_project` of those Projects, with the Project named as `via` |
//! | Evidence | `evidence_links` targeting the Product |
//! | People | `stakeholder_subject` relationships whose subject is the Product |
//! | Carried work | `work_owners` rows whose owner is one of those People |
//!
//! Nothing is inferred. In particular, no work record is attributed to the
//! Product through a shared Stakeholder, shared Evidence, or free text: a
//! carried item is shown under the person, and the person is shown under
//! the Product, and that is the whole chain.
//!
//! Carried items are built by the Work Queue adapter and filtered, so their
//! labels, states, intents and attention are exactly what the queue shows.
//! Issues are never carried: the Ledger records no owner for an Issue.

use std::collections::{HashMap, HashSet};

use pmc_domain::attention::AttentionThresholds;
use pmc_domain::composition_source::LedgerCompositionSnapshot;
use pmc_domain::projection_source::LedgerProjectionSnapshot;
use pmc_domain::relationships::RelationshipKind;

use crate::people_adapter::purpose_of;
use crate::product_composition::{
    EvidenceFacts, PersonFacts, ProductDetailFacts, ProductFacts, StructureFacts,
};
use crate::route_composition::ComposedEntityKind;
use crate::work_queue_adapter::work_item_facts_from_snapshots;

/// Builds the facts for one Product, or `None` when the Ledger holds no such
/// Product -- a different answer from a Product that has nothing attached.
#[must_use]
pub fn product_detail_facts_from_snapshots(
    product_id: &str,
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
    thresholds: AttentionThresholds,
) -> Option<ProductDetailFacts> {
    let as_of = composition.ledger_as_of_utc;
    let record = composition
        .products
        .iter()
        .find(|product| product.id.as_str() == product_id)?;
    let product = ProductFacts {
        id: product_id.to_owned(),
        name: record.name.clone(),
        classification: record.classification,
        revision: record.version.get(),
        as_of,
    };

    // ---- Structure -------------------------------------------------------
    let related = |kind: RelationshipKind, this_type: &str, other_type: &str| -> Vec<String> {
        composition
            .portfolio_relationships
            .iter()
            .filter(|relationship| relationship.kind == kind)
            .filter(|relationship| relationship.endpoint_id(this_type) == Some(product_id))
            .filter_map(|relationship| relationship.endpoint_id(other_type))
            .map(str::to_owned)
            .collect()
    };

    let project_ids = related(RelationshipKind::ProjectProduct, "product", "project");
    let project_set: HashSet<&str> = project_ids.iter().map(String::as_str).collect();

    let mut structure = Vec::new();
    for project in &composition.projects {
        if project_set.contains(project.id.as_str()) {
            structure.push(StructureFacts {
                kind: ComposedEntityKind::Project,
                id: project.id.as_str().to_owned(),
                label: project.name.clone(),
                classification: project.classification,
                revision: project.version.get(),
                as_of,
                via: None,
            });
        }
    }
    for milestone in &composition.milestones {
        if project_set.contains(milestone.project_id.as_str()) {
            structure.push(StructureFacts {
                kind: ComposedEntityKind::Milestone,
                id: milestone.id.as_str().to_owned(),
                label: milestone.name.clone(),
                classification: milestone.classification,
                revision: milestone.version.get(),
                as_of,
                via: None,
            });
        }
    }
    let roadmap_ids: HashSet<String> =
        related(RelationshipKind::ProductRoadmap, "product", "roadmap")
            .into_iter()
            .collect();
    for roadmap in &composition.roadmaps {
        if roadmap_ids.contains(roadmap.id.as_str()) {
            structure.push(StructureFacts {
                kind: ComposedEntityKind::Roadmap,
                id: roadmap.id.as_str().to_owned(),
                label: roadmap.name.clone(),
                classification: roadmap.classification,
                revision: roadmap.version.get(),
                as_of,
                via: None,
            });
        }
    }
    let kpi_ids: HashSet<String> =
        related(RelationshipKind::ProductKpi, "product", "kpi_definition")
            .into_iter()
            .collect();
    for kpi in &composition.kpi_definitions {
        if kpi_ids.contains(kpi.id.as_str()) {
            structure.push(StructureFacts {
                kind: ComposedEntityKind::Kpi,
                id: kpi.id.as_str().to_owned(),
                label: kpi.name.clone(),
                classification: kpi.classification,
                revision: kpi.version.get(),
                as_of,
                via: None,
            });
        }
    }
    // Initiatives: reached through a Project, and named as such. The first
    // Project that reaches an Initiative is the one recorded; a second path
    // does not make it a second Initiative.
    let mut initiative_via: HashMap<&str, String> = HashMap::new();
    for relationship in &composition.portfolio_relationships {
        if relationship.kind != RelationshipKind::InitiativeProject {
            continue;
        }
        let (Some(project), Some(initiative)) = (
            relationship.endpoint_id("project"),
            relationship.endpoint_id("initiative"),
        ) else {
            continue;
        };
        if project_set.contains(project) {
            initiative_via
                .entry(initiative)
                .or_insert_with(|| format!("project:{project}"));
        }
    }
    for initiative in &composition.initiatives {
        if let Some(via) = initiative_via.get(initiative.id.as_str()) {
            structure.push(StructureFacts {
                kind: ComposedEntityKind::Initiative,
                id: initiative.id.as_str().to_owned(),
                label: initiative.name.clone(),
                classification: initiative.classification,
                revision: initiative.version.get(),
                as_of,
                via: Some(via.clone()),
            });
        }
    }

    // ---- Evidence --------------------------------------------------------
    let evidence = composition
        .evidence_links
        .iter()
        .filter(|link| link.target_type == "product" && link.target_id == product_id)
        .filter_map(|link| {
            // The link's foreign key guarantees the reference exists; a miss
            // here would be a corrupt Ledger, and is not rendered as Evidence
            // with invented fields.
            let reference = composition
                .evidence_references
                .iter()
                .find(|reference| reference.id == link.evidence_id)?;
            Some(EvidenceFacts {
                id: reference.id.as_str().to_owned(),
                role: reference.role,
                verification: reference.verification.clone(),
                pinned: reference.pinned,
                classification: reference.classification,
                classification_at_link: link.classification_at_link,
                linked_at: link.linked_at,
                revision: reference.version.get(),
                as_of,
            })
        })
        .collect();

    // ---- People ----------------------------------------------------------
    let all_work = work_item_facts_from_snapshots(projection, composition, thresholds);
    let mut people = Vec::new();
    for relationship in &composition.stakeholder_relationships {
        if relationship.subject_type != "product" || relationship.subject_id != product_id {
            continue;
        }
        let Some(stakeholder) = composition
            .stakeholders
            .iter()
            .find(|stakeholder| stakeholder.id == relationship.stakeholder_id)
        else {
            continue;
        };
        let other_products_accountable_for = composition
            .stakeholder_relationships
            .iter()
            .filter(|other| other.stakeholder_id == relationship.stakeholder_id)
            .filter(|other| other.subject_type == "product" && other.subject_id != product_id)
            .map(|other| other.subject_id.as_str())
            .collect::<HashSet<_>>()
            .len();
        let owned: HashSet<(&str, &str)> = composition
            .work_owners
            .iter()
            .filter(|owner| owner.owner_id == relationship.stakeholder_id)
            .map(|owner| (owner.target_type.as_str(), owner.target_id.as_str()))
            .collect();
        let carried = all_work
            .iter()
            .filter(|item| owned.contains(&(item.kind.as_str(), item.id.as_str())))
            .cloned()
            .collect();
        people.push(PersonFacts {
            id: stakeholder.id.as_str().to_owned(),
            display_name: stakeholder.display_name.clone(),
            purpose: purpose_of(relationship.purpose),
            classification: stakeholder.classification,
            revision: stakeholder.version.get(),
            as_of,
            other_products_accountable_for,
            carried,
        });
    }

    Some(ProductDetailFacts {
        product,
        structure,
        evidence,
        people,
    })
}
