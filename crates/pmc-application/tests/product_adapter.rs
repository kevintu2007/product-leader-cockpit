//! The production Product-detail adapter (S02 detail, O01).
//!
//! These assert which records the adapter attaches to a Product and -- the
//! part that matters more -- which it does not. Every edge is one the
//! Ledger holds; nothing is reached through a shared person, shared Evidence
//! or free text.

use pmc_application::people_composition::RelationshipPurpose;
use pmc_application::product_adapter::product_detail_facts_from_snapshots;
use pmc_application::route_composition::ComposedEntityKind;
use pmc_application::work_queue_composition::WorkItemKind;
use pmc_domain::attention::{AttentionReason, AttentionThresholds};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{
    ActionRequestReadRecord, EvidenceLinkReadRecord, EvidenceReferenceReadRecord,
    InitiativeReadRecord, IssueReadRecord, KpiDefinitionReadRecord, LedgerCompositionSnapshot,
    MilestoneReadRecord, PortfolioRelationshipReadRecord, ProductReadRecord, ProjectReadRecord,
    RelationshipEndpointReadRecord, RoadmapReadRecord, StakeholderReadRecord,
    StakeholderRelationshipReadRecord, WorkOwnerReadRecord,
};
use pmc_domain::identity::{
    ActionId, ActionRequestId, AggregateVersion, EvidenceReferenceId, InitiativeId, KpiId,
    MilestoneId, ProductId, ProjectId, RoadmapId, StakeholderId,
};
use pmc_domain::projection_source::{ActionProjectionSource, LedgerProjectionSnapshot};
use pmc_domain::relationships::{
    RelationshipKind, StakeholderKind, StakeholderRelationshipPurpose,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, ActionState, EvidenceVerification, IssueState,
};

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn version(value: u64) -> AggregateVersion {
    AggregateVersion::new(value).unwrap()
}

fn endpoint(target_type: &str, target_id: &str) -> RelationshipEndpointReadRecord {
    RelationshipEndpointReadRecord {
        target_type: target_type.to_owned(),
        target_id: target_id.to_owned(),
    }
}

fn relationship(
    id: &str,
    kind: RelationshipKind,
    a: (&str, &str),
    b: (&str, &str),
) -> PortfolioRelationshipReadRecord {
    PortfolioRelationshipReadRecord {
        id: id.to_owned(),
        kind,
        endpoints: [endpoint(a.0, a.1), endpoint(b.0, b.1)],
        classification: DataClassification::Internal,
        version: version(1),
    }
}

fn accountable(
    stakeholder: &str,
    subject_type: &str,
    subject_id: &str,
) -> StakeholderRelationshipReadRecord {
    StakeholderRelationshipReadRecord {
        stakeholder_id: StakeholderId::parse(stakeholder).unwrap(),
        subject_type: subject_type.to_owned(),
        subject_id: subject_id.to_owned(),
        purpose: StakeholderRelationshipPurpose::Responsibility,
        classification: DataClassification::Internal,
        version: version(1),
    }
}

/// A Product with one Project (carrying a Milestone and reached by an
/// Initiative), a Roadmap, a KPI, two Evidence links -- one to the Product
/// and one to the Milestone -- and one accountable person who owns an
/// overdue Action, an open request, and is also accountable for a second
/// Product. Plus an unrelated Product that must attract nothing.
fn composition() -> LedgerCompositionSnapshot {
    LedgerCompositionSnapshot {
        schema_version: 42,
        ledger_revision: 7,
        ledger_as_of_utc: now(),
        stakeholders: vec![StakeholderReadRecord {
            id: StakeholderId::parse("stakeholder-a").unwrap(),
            display_name: "Synthetic Person".to_owned(),
            kind: StakeholderKind::Person,
            classification: DataClassification::Confidential,
            version: version(1),
        }],
        stakeholder_relationships: vec![
            accountable("stakeholder-a", "product", "product-1"),
            accountable("stakeholder-a", "product", "product-2"),
            accountable("stakeholder-a", "milestone", "milestone-1"),
        ],
        milestones: vec![MilestoneReadRecord {
            id: MilestoneId::parse("milestone-1").unwrap(),
            project_id: ProjectId::parse("project-1").unwrap(),
            name: "Synthetic Milestone".to_owned(),
            due_at: at(5_000),
            classification: DataClassification::Restricted,
            version: version(3),
        }],
        action_requests: vec![ActionRequestReadRecord {
            id: "request-open".to_owned(),
            title: "Synthetic open request".to_owned(),
            intended_owner_id: Some(StakeholderId::parse("stakeholder-a").unwrap()),
            state: ActionRequestState::Open,
            response_due_at: None,
            intended_action_due_at: None,
            classification: DataClassification::Internal,
            version: version(4),
        }],
        decision_requests: Vec::new(),
        risks: Vec::new(),
        issues: vec![IssueReadRecord {
            id: "issue-1".to_owned(),
            title: "Synthetic issue".to_owned(),
            state: IssueState::Open,
            source_risk_id: None,
            recurrence_of_id: None,
            classification: DataClassification::Internal,
            version: version(1),
        }],
        portfolio_relationships: vec![
            relationship(
                "relationship-3",
                RelationshipKind::ProjectProduct,
                ("product", "product-1"),
                ("project", "project-1"),
            ),
            relationship(
                "relationship-4",
                RelationshipKind::InitiativeProject,
                ("initiative", "initiative-1"),
                ("project", "project-1"),
            ),
            relationship(
                "relationship-5",
                RelationshipKind::ProductRoadmap,
                ("product", "product-1"),
                ("roadmap", "roadmap-1"),
            ),
            relationship(
                "relationship-6",
                RelationshipKind::ProductKpi,
                ("kpi_definition", "kpi-1"),
                ("product", "product-1"),
            ),
            // The other Product's Project: must not appear under product-1.
            relationship(
                "relationship-8",
                RelationshipKind::ProjectProduct,
                ("product", "product-2"),
                ("project", "project-2"),
            ),
            // project-2 also relates to product-3, which nobody is
            // accountable for. A Project can relate to more than one Product;
            // that is why structure is an association, not containment.
            relationship(
                "relationship-9",
                RelationshipKind::ProjectProduct,
                ("product", "product-3"),
                ("project", "project-2"),
            ),
        ],
        products: vec![
            ProductReadRecord {
                id: ProductId::parse("product-1").unwrap(),
                name: "Synthetic Product".to_owned(),
                classification: DataClassification::Internal,
                version: version(2),
            },
            ProductReadRecord {
                id: ProductId::parse("product-2").unwrap(),
                name: "Other Product".to_owned(),
                classification: DataClassification::Internal,
                version: version(1),
            },
            ProductReadRecord {
                id: ProductId::parse("product-3").unwrap(),
                name: "Unattended Product".to_owned(),
                classification: DataClassification::Internal,
                version: version(1),
            },
        ],
        initiatives: vec![InitiativeReadRecord {
            id: InitiativeId::parse("initiative-1").unwrap(),
            name: "Synthetic Initiative".to_owned(),
            classification: DataClassification::Internal,
            version: version(1),
        }],
        projects: vec![
            ProjectReadRecord {
                id: ProjectId::parse("project-1").unwrap(),
                name: "Synthetic Project".to_owned(),
                start_at: at(0),
                end_at: at(1_000),
                classification: DataClassification::Internal,
                version: version(1),
            },
            ProjectReadRecord {
                id: ProjectId::parse("project-2").unwrap(),
                name: "Other Project".to_owned(),
                start_at: at(0),
                end_at: at(1_000),
                classification: DataClassification::Internal,
                version: version(1),
            },
        ],
        roadmaps: vec![RoadmapReadRecord {
            id: RoadmapId::parse("roadmap-1").unwrap(),
            name: "Synthetic Roadmap".to_owned(),
            classification: DataClassification::Internal,
            version: version(1),
        }],
        kpi_definitions: vec![KpiDefinitionReadRecord {
            id: KpiId::parse("kpi-1").unwrap(),
            name: "Synthetic KPI".to_owned(),
            classification: DataClassification::Internal,
            version: version(1),
        }],
        evidence_links: vec![
            EvidenceLinkReadRecord {
                evidence_id: EvidenceReferenceId::parse("evidence-1").unwrap(),
                target_type: "product".to_owned(),
                target_id: "product-1".to_owned(),
                classification_at_link: DataClassification::Internal,
                linked_at: at(6_000),
            },
            EvidenceLinkReadRecord {
                evidence_id: EvidenceReferenceId::parse("evidence-2").unwrap(),
                target_type: "milestone".to_owned(),
                target_id: "milestone-1".to_owned(),
                classification_at_link: DataClassification::Internal,
                linked_at: at(6_500),
            },
        ],
        evidence_references: vec![
            EvidenceReferenceReadRecord {
                id: EvidenceReferenceId::parse("evidence-1").unwrap(),
                role: None,
                verification: EvidenceVerification::Unverified,
                pinned: false,
                classification: DataClassification::Restricted,
                version: version(3),
                updated_at: at(7_000),
            },
            EvidenceReferenceReadRecord {
                id: EvidenceReferenceId::parse("evidence-2").unwrap(),
                role: None,
                verification: EvidenceVerification::Unverified,
                pinned: false,
                classification: DataClassification::Internal,
                version: version(1),
                updated_at: at(8_000),
            },
        ],
        kpi_observations: Vec::new(),
        work_owners: vec![
            WorkOwnerReadRecord {
                target_type: "action".to_owned(),
                target_id: "action-1".to_owned(),
                owner_id: StakeholderId::parse("stakeholder-a").unwrap(),
            },
            WorkOwnerReadRecord {
                target_type: "action_request".to_owned(),
                target_id: "request-open".to_owned(),
                owner_id: StakeholderId::parse("stakeholder-a").unwrap(),
            },
        ],
    }
}

fn projection() -> LedgerProjectionSnapshot {
    LedgerProjectionSnapshot {
        schema_version: 42,
        ledger_revision: 7,
        ledger_as_of_utc: now(),
        products: Vec::new(),
        projects: Vec::new(),
        actions: vec![ActionProjectionSource {
            id: ActionId::parse("action-1").unwrap(),
            classification: DataClassification::Internal,
            source_revision: version(8),
            state: ActionState::Open,
            due_at: at(1_000),
            source_request_id: ActionRequestId::parse("request-open").unwrap(),
            source_decision_id: None,
        }],
        risks: Vec::new(),
        decisions: Vec::new(),
        kpis: Vec::new(),
    }
}

fn facts_for(product: &str) -> Option<pmc_application::product_composition::ProductDetailFacts> {
    product_detail_facts_from_snapshots(
        product,
        &projection(),
        &composition(),
        AttentionThresholds::default(),
    )
}

#[test]
fn an_unknown_product_is_none_rather_than_an_empty_inspector() {
    assert!(facts_for("product-9").is_none());
}

#[test]
fn the_product_carries_its_name_and_registry_revision() {
    let facts = facts_for("product-1").unwrap();
    assert_eq!(facts.product.name, "Synthetic Product");
    assert_eq!(facts.product.revision, 2);
}

#[test]
fn structure_is_reached_only_through_edges_the_ledger_holds() {
    let facts = facts_for("product-1").unwrap();
    let ids: Vec<(ComposedEntityKind, &str)> = facts
        .structure
        .iter()
        .map(|child| (child.kind, child.id.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            (ComposedEntityKind::Project, "project-1"),
            (ComposedEntityKind::Milestone, "milestone-1"),
            (ComposedEntityKind::Roadmap, "roadmap-1"),
            (ComposedEntityKind::Kpi, "kpi-1"),
            (ComposedEntityKind::Initiative, "initiative-1"),
        ]
    );
    // The other Product's Project is not here.
    assert!(!facts.structure.iter().any(|child| child.id == "project-2"));
}

#[test]
fn an_initiative_names_the_project_it_was_reached_through() {
    let facts = facts_for("product-1").unwrap();
    let initiative = facts
        .structure
        .iter()
        .find(|child| child.kind == ComposedEntityKind::Initiative)
        .unwrap();
    assert_eq!(initiative.via.as_deref(), Some("project:project-1"));
    // And everything reached directly names no path.
    assert!(facts
        .structure
        .iter()
        .filter(|child| child.kind != ComposedEntityKind::Initiative)
        .all(|child| child.via.is_none()));
}

#[test]
fn only_evidence_linked_to_the_product_itself_is_attached() {
    // evidence-2 is linked to the Milestone beneath the Product. It is not
    // the Product's Evidence, and attaching it would be inferring scope
    // through structure.
    let facts = facts_for("product-1").unwrap();
    let ids: Vec<&str> = facts
        .evidence
        .iter()
        .map(|record| record.id.as_str())
        .collect();
    assert_eq!(ids, ["evidence-1"]);
    assert_eq!(
        facts.evidence[0].classification,
        DataClassification::Restricted
    );
    assert_eq!(
        facts.evidence[0].classification_at_link,
        DataClassification::Internal
    );
}

#[test]
fn people_are_those_whose_relationship_subject_is_the_product() {
    let facts = facts_for("product-1").unwrap();
    assert_eq!(facts.people.len(), 1);
    let person = &facts.people[0];
    assert_eq!(person.id, "stakeholder-a");
    assert_eq!(person.display_name, "Synthetic Person");
    assert_eq!(person.purpose, RelationshipPurpose::Responsibility);
    assert_eq!(person.classification, DataClassification::Confidential);
}

#[test]
fn a_person_reports_how_many_other_products_they_are_accountable_for() {
    // stakeholder-a is also accountable for product-2. The Milestone
    // relationship is not a Product and does not count.
    let facts = facts_for("product-1").unwrap();
    assert_eq!(facts.people[0].other_products_accountable_for, 1);
}

#[test]
fn carried_work_is_exactly_what_the_owner_rows_say_the_person_owns() {
    let facts = facts_for("product-1").unwrap();
    let carried: Vec<(WorkItemKind, &str)> = facts.people[0]
        .carried
        .iter()
        .map(|item| (item.kind, item.id.as_str()))
        .collect();
    assert_eq!(
        carried,
        [
            (WorkItemKind::ActionRequest, "request-open"),
            (WorkItemKind::Action, "action-1"),
        ]
    );
}

#[test]
fn a_carried_action_keeps_the_attention_the_work_queue_would_show() {
    // Built by the Work Queue adapter and filtered, not re-derived: the
    // overdue flag here is the same flag the queue shows.
    let facts = facts_for("product-1").unwrap();
    let action = facts.people[0]
        .carried
        .iter()
        .find(|item| item.id == "action-1")
        .unwrap();
    assert!(action
        .attention
        .iter()
        .any(|item| item.flag.reason == AttentionReason::ActionOverdue));
    assert_eq!(action.revision, 8);
}

#[test]
fn an_issue_is_never_carried_because_no_issue_has_an_owner() {
    let facts = facts_for("product-1").unwrap();
    assert!(!facts.people[0]
        .carried
        .iter()
        .any(|item| item.kind == WorkItemKind::Issue));
}

#[test]
fn a_product_nobody_is_accountable_for_has_no_people_and_therefore_no_work() {
    // product-3 has a Project but no accountable person. Its inspector shows
    // structure and nothing carried -- stakeholder-a's overdue Action still
    // exists, but no edge the Ledger holds reaches it from this Product, and
    // sharing project-2 with product-2 is not such an edge.
    let facts = facts_for("product-3").unwrap();
    assert_eq!(facts.structure.len(), 1);
    assert_eq!(facts.structure[0].id, "project-2");
    assert!(facts.people.is_empty());
}

#[test]
fn a_project_shared_by_two_products_appears_under_both_without_either_owning_it() {
    // Association, not containment: the same Project is structure for
    // product-2 and product-3, and neither entry names a parent.
    let second = facts_for("product-2").unwrap();
    let third = facts_for("product-3").unwrap();
    assert_eq!(second.structure[0].id, "project-2");
    assert_eq!(third.structure[0].id, "project-2");
    assert!(second.structure[0].via.is_none());
    assert!(third.structure[0].via.is_none());
}
