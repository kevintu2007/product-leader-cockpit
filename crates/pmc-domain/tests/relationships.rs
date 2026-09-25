#![allow(clippy::result_large_err)]

use pmc_domain::audit::{AuditActor, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::error::ErrorCode;
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, KpiId, MilestoneId,
    PortfolioId, ProductId, ProjectId, RelationshipId, RoadmapId, StakeholderId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_908_072_600_000)
    }
}
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("relationship-audit-{}", self.0))
    }
}

fn context(name: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(format!("idem-{name}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("correlation-{name}")).unwrap(),
    }
}
fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("synthetic-relationships-v1").unwrap())
}
fn sid() -> StakeholderId {
    StakeholderId::parse("stakeholder-orbit").unwrap()
}
fn pid() -> PortfolioId {
    PortfolioId::parse("portfolio-alpha").unwrap()
}
fn product() -> ProductId {
    ProductId::parse("product-orbit").unwrap()
}
fn portfolio() -> PortfolioSnapshot {
    PortfolioSnapshot::new(
        pid(),
        AggregateVersion::initial(),
        DataClassification::Public,
    )
}
fn product_snapshot() -> ProductSnapshot {
    ProductSnapshot::new(
        product(),
        AggregateVersion::initial(),
        DataClassification::Internal,
    )
}
fn catalog() -> InMemoryEndpointCatalog {
    InMemoryEndpointCatalog::new([
        EndpointSnapshot::Portfolio(portfolio()),
        EndpointSnapshot::Product(product_snapshot()),
        EndpointSnapshot::Initiative(InitiativeSnapshot::new(
            InitiativeId::parse("initiative-orbit").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Roadmap(RoadmapSnapshot::new(
            RoadmapId::parse("roadmap-orbit").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Kpi(KpiSnapshot::new(
            KpiId::parse("kpi-adoption").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Confidential,
        )),
        EndpointSnapshot::Project(ProjectSnapshot::new(
            ProjectId::parse("project-orbit").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Milestone(MilestoneSnapshot::new(
            MilestoneId::parse("milestone-orbit").unwrap(),
            ProjectId::parse("project-orbit").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
    ])
}
fn service() -> InMemoryRelationshipService<FixedClock, InMemoryEndpointCatalog, AuditIds> {
    InMemoryRelationshipService::new(FixedClock, catalog(), AuditIds(0))
}

#[derive(Clone)]
struct MutableCatalog {
    portfolio: PortfolioSnapshot,
    product: Rc<RefCell<ProductSnapshot>>,
}

impl EndpointCatalog for MutableCatalog {
    fn portfolio(&self, id: &PortfolioId) -> Option<PortfolioSnapshot> {
        (self.portfolio.id() == id).then(|| self.portfolio.clone())
    }
    fn product(&self, id: &ProductId) -> Option<ProductSnapshot> {
        let product = self.product.borrow();
        (product.id() == id).then(|| product.clone())
    }
    fn initiative(&self, _id: &InitiativeId) -> Option<InitiativeSnapshot> {
        None
    }
    fn roadmap(&self, _id: &RoadmapId) -> Option<RoadmapSnapshot> {
        None
    }
    fn kpi(&self, _id: &KpiId) -> Option<KpiSnapshot> {
        None
    }
    fn project(&self, _id: &ProjectId) -> Option<ProjectSnapshot> {
        None
    }
    fn milestone(&self, _id: &MilestoneId) -> Option<MilestoneSnapshot> {
        None
    }
}
fn stakeholder() -> CreateStakeholder {
    CreateStakeholder {
        id: sid(),
        name: StakeholderName::parse("Synthetic Product Council").unwrap(),
        kind: StakeholderKind::Organization,
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: context("stakeholder-create"),
    }
}

type RelationshipService =
    InMemoryRelationshipService<FixedClock, InMemoryEndpointCatalog, AuditIds>;

#[derive(Clone, Copy)]
enum CatalogFamily {
    PortfolioProduct,
    PortfolioInitiative,
    ProductRoadmap,
    ProductKpi,
    InitiativeProject,
    ProjectProduct,
}

fn link_catalog_family(
    service: &mut RelationshipService,
    family: CatalogFamily,
    suffix: &str,
    expected_first: AggregateVersion,
) -> Result<MutationOutcome<RelationshipRecord>, pmc_domain::error::DomainError> {
    let relationship_id = RelationshipId::parse(format!("rel-{suffix}")).unwrap();
    let operation_context = context(suffix);
    match family {
        CatalogFamily::PortfolioProduct => service.link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id,
            portfolio_id: pid(),
            product_id: product(),
            expected_portfolio_version: expected_first,
            expected_product_version: AggregateVersion::initial(),
            context: operation_context,
        }),
        CatalogFamily::PortfolioInitiative => {
            service.link_portfolio_initiative(LinkPortfolioInitiative {
                id: relationship_id,
                portfolio_id: pid(),
                initiative_id: InitiativeId::parse("initiative-orbit").unwrap(),
                expected_portfolio_version: expected_first,
                expected_initiative_version: AggregateVersion::initial(),
                context: operation_context,
            })
        }
        CatalogFamily::ProductRoadmap => service.link_product_roadmap(LinkProductRoadmap {
            id: relationship_id,
            product_id: product(),
            roadmap_id: RoadmapId::parse("roadmap-orbit").unwrap(),
            expected_product_version: expected_first,
            expected_roadmap_version: AggregateVersion::initial(),
            context: operation_context,
        }),
        CatalogFamily::ProductKpi => service.link_product_kpi(LinkProductKpi {
            id: relationship_id,
            product_id: product(),
            kpi_id: KpiId::parse("kpi-adoption").unwrap(),
            expected_product_version: expected_first,
            expected_kpi_version: AggregateVersion::initial(),
            context: operation_context,
        }),
        CatalogFamily::InitiativeProject => {
            service.link_initiative_project(LinkInitiativeProject {
                id: relationship_id,
                initiative_id: InitiativeId::parse("initiative-orbit").unwrap(),
                project_id: ProjectId::parse("project-orbit").unwrap(),
                expected_initiative_version: expected_first,
                expected_project_version: AggregateVersion::initial(),
                context: operation_context,
            })
        }
        CatalogFamily::ProjectProduct => service.link_project_product(LinkProjectProduct {
            id: relationship_id,
            project_id: ProjectId::parse("project-orbit").unwrap(),
            product_id: product(),
            expected_project_version: expected_first,
            expected_product_version: AggregateVersion::initial(),
            context: operation_context,
        }),
    }
}

fn assert_catalog_family_contract(family: CatalogFamily, name: &str) {
    let mut service = service();
    let first = link_catalog_family(
        &mut service,
        family,
        &format!("{name}-first"),
        AggregateVersion::initial(),
    )
    .unwrap();
    assert_eq!(first.outcome().audit_event_ids().len(), 1);
    assert_eq!(
        first.outcome().effect_scope(),
        pmc_domain::audit::AuditEffectScope::Complete
    );
    let audit_count = service.audit_events().len();
    let duplicate = link_catalog_family(
        &mut service,
        family,
        &format!("{name}-duplicate"),
        AggregateVersion::initial(),
    )
    .unwrap();
    assert_eq!(duplicate.value().id(), first.value().id());
    assert_eq!(
        duplicate.outcome().effect_scope(),
        pmc_domain::audit::AuditEffectScope::None
    );
    assert_eq!(service.audit_events().len(), audit_count);
    let stale = link_catalog_family(
        &mut service,
        family,
        &format!("{name}-stale"),
        AggregateVersion::new(2).unwrap(),
    );
    assert_eq!(stale.unwrap_err().code(), ErrorCode::DomainConflict);
    assert_eq!(service.audit_events().len(), audit_count);
    assert_eq!(service.relationship_count(), 1);
}

#[test]
fn portfolio_product_relationship_contract() {
    assert_catalog_family_contract(CatalogFamily::PortfolioProduct, "portfolio-product");
}

#[test]
fn portfolio_initiative_relationship_contract() {
    assert_catalog_family_contract(CatalogFamily::PortfolioInitiative, "portfolio-initiative");
}

#[test]
fn product_roadmap_relationship_contract() {
    assert_catalog_family_contract(CatalogFamily::ProductRoadmap, "product-roadmap");
}

#[test]
fn product_kpi_relationship_contract() {
    assert_catalog_family_contract(CatalogFamily::ProductKpi, "product-kpi");
}

#[test]
fn initiative_project_relationship_contract() {
    assert_catalog_family_contract(CatalogFamily::InitiativeProject, "initiative-project");
}

#[test]
fn project_product_relationship_contract() {
    assert_catalog_family_contract(CatalogFamily::ProjectProduct, "project-product");
}

#[test]
fn stakeholder_is_typed_and_update_reclassifies_relationships_atomically() {
    let mut service = service();
    let created = service
        .create_stakeholder(stakeholder())
        .unwrap()
        .into_value();
    let linked = service
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("rel-stakeholder-product").unwrap(),
            stakeholder_id: sid(),
            subject: StakeholderSubject::Product(product()),
            purpose: StakeholderRelationshipPurpose::Responsibility,
            expected_stakeholder_version: created.version(),
            expected_subject_version: AggregateVersion::initial(),
            context: context("stakeholder-product"),
        })
        .unwrap()
        .into_value();
    assert_eq!(linked.classification(), DataClassification::Internal);
    let before_audits = service.audit_events().len();
    let updated = service
        .update_stakeholder(UpdateStakeholderDetails {
            id: sid(),
            expected_version: created.version(),
            name: StakeholderName::parse("Synthetic Council").unwrap(),
            classification: Some(DataClassification::Restricted),
            context: context("stakeholder-update"),
        })
        .unwrap();
    assert_eq!(
        updated.value().classification(),
        DataClassification::Restricted
    );
    let inspected = service.inspect_relationship(linked.id()).unwrap().unwrap();
    assert_eq!(inspected.classification(), DataClassification::Restricted);
    assert_eq!(inspected.version().get(), 2);
    assert_eq!(service.audit_events().len(), before_audits + 2);
    assert!(service
        .audit_events()
        .iter()
        .all(|event| event.actor() == AuditActor::HeadOfProducts));
}

#[test]
fn typed_catalog_links_require_expected_versions_and_replay_semantic_duplicates() {
    let mut service = service();
    let command = LinkPortfolioProduct {
        id: RelationshipId::parse("rel-first").unwrap(),
        portfolio_id: pid(),
        product_id: product(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("link-first"),
    };
    let first = service
        .link_portfolio_product(command)
        .unwrap()
        .into_value();
    let duplicate = service
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("rel-second").unwrap(),
            portfolio_id: pid(),
            product_id: product(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("link-second"),
        })
        .unwrap();
    assert_eq!(duplicate.value().id(), first.id());
    assert_eq!(
        duplicate.outcome().effect_scope(),
        pmc_domain::audit::AuditEffectScope::None
    );
    assert_eq!(service.relationship_count(), 1);
    let stale = service.link_portfolio_product(LinkPortfolioProduct {
        id: RelationshipId::parse("rel-stale").unwrap(),
        portfolio_id: pid(),
        product_id: product(),
        expected_portfolio_version: AggregateVersion::new(2).unwrap(),
        expected_product_version: AggregateVersion::initial(),
        context: context("link-stale"),
    });
    assert_eq!(stale.unwrap_err().code(), ErrorCode::DomainConflict);
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn semantic_duplicate_normalizes_to_current_trusted_classification() {
    let shared_product = Rc::new(RefCell::new(product_snapshot()));
    let catalog = MutableCatalog {
        portfolio: portfolio(),
        product: Rc::clone(&shared_product),
    };
    let mut service = InMemoryRelationshipService::new(FixedClock, catalog, AuditIds(0));
    let original_context = context("dynamic-first");
    let original = service
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("rel-dynamic-first").unwrap(),
            portfolio_id: pid(),
            product_id: product(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: original_context.clone(),
        })
        .unwrap();
    assert_eq!(
        original.value().classification(),
        DataClassification::Internal
    );

    *shared_product.borrow_mut() = ProductSnapshot::new(
        product(),
        AggregateVersion::new(2).unwrap(),
        DataClassification::Restricted,
    );

    let exact_replay = service
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("rel-dynamic-first").unwrap(),
            portfolio_id: pid(),
            product_id: product(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: original_context,
        })
        .unwrap();
    assert_eq!(
        exact_replay.value().classification(),
        DataClassification::Internal
    );

    let semantic_duplicate = service
        .link_portfolio_product(LinkPortfolioProduct {
            id: RelationshipId::parse("rel-dynamic-second").unwrap(),
            portfolio_id: pid(),
            product_id: product(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::new(2).unwrap(),
            context: context("dynamic-second"),
        })
        .unwrap();
    assert_eq!(semantic_duplicate.value().id(), original.value().id());
    assert_eq!(
        semantic_duplicate.value().classification(),
        DataClassification::Restricted
    );
    assert_eq!(
        semantic_duplicate.outcome().effect_scope(),
        pmc_domain::audit::AuditEffectScope::None
    );
    assert_eq!(service.audit_events().len(), 1);
}

#[test]
fn same_idempotency_changed_payload_conflicts_before_endpoint_validation() {
    let mut service = service();
    let first = LinkPortfolioProduct {
        id: RelationshipId::parse("rel-idem").unwrap(),
        portfolio_id: pid(),
        product_id: product(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("same-idem"),
    };
    service.link_portfolio_product(first.clone()).unwrap();
    let changed = service.link_portfolio_product(LinkPortfolioProduct {
        id: first.id,
        portfolio_id: PortfolioId::parse("missing-portfolio").unwrap(),
        product_id: product(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: first.context,
    });
    assert_eq!(
        changed.unwrap_err().code(),
        ErrorCode::DomainIdempotencyConflict
    );
}

#[test]
fn unclassified_endpoint_is_denied_without_effect_and_milestone_does_not_create_second_authority() {
    let mut endpoints = vec![
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            pid(),
            AggregateVersion::initial(),
            DataClassification::Unclassified,
        )),
        EndpointSnapshot::Product(product_snapshot()),
    ];
    let mut denied_service = InMemoryRelationshipService::new(
        FixedClock,
        InMemoryEndpointCatalog::new(endpoints.drain(..)),
        AuditIds(0),
    );
    let denied = denied_service.link_portfolio_product(LinkPortfolioProduct {
        id: RelationshipId::parse("rel-unclassified").unwrap(),
        portfolio_id: pid(),
        product_id: product(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("unclassified"),
    });
    assert_eq!(denied.unwrap_err().code(), ErrorCode::SecurityPolicyDenied);
    assert_eq!(denied_service.relationship_count(), 0);

    let service = service();
    let checked = service
        .validate_project_milestone(ValidateProjectMilestone {
            project_id: ProjectId::parse("project-orbit").unwrap(),
            milestone_id: MilestoneId::parse("milestone-orbit").unwrap(),
            expected_project_version: AggregateVersion::initial(),
            expected_milestone_version: AggregateVersion::initial(),
            context: context("milestone-check"),
        })
        .unwrap();
    assert_eq!(checked.project_id().as_str(), "project-orbit");
    assert_eq!(service.relationship_count(), 0);
    assert!(service.audit_events().is_empty());
}

#[test]
fn all_named_link_families_have_typed_boundaries() {
    let mut service = service();
    service
        .link_portfolio_initiative(LinkPortfolioInitiative {
            id: RelationshipId::parse("rel-pi").unwrap(),
            portfolio_id: pid(),
            initiative_id: InitiativeId::parse("initiative-orbit").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_initiative_version: AggregateVersion::initial(),
            context: context("pi"),
        })
        .unwrap();
    service
        .link_product_roadmap(LinkProductRoadmap {
            id: RelationshipId::parse("rel-pr").unwrap(),
            product_id: product(),
            roadmap_id: RoadmapId::parse("roadmap-orbit").unwrap(),
            expected_product_version: AggregateVersion::initial(),
            expected_roadmap_version: AggregateVersion::initial(),
            context: context("pr"),
        })
        .unwrap();
    service
        .link_product_kpi(LinkProductKpi {
            id: RelationshipId::parse("rel-pk").unwrap(),
            product_id: product(),
            kpi_id: KpiId::parse("kpi-adoption").unwrap(),
            expected_product_version: AggregateVersion::initial(),
            expected_kpi_version: AggregateVersion::initial(),
            context: context("pk"),
        })
        .unwrap();
    service
        .link_initiative_project(LinkInitiativeProject {
            id: RelationshipId::parse("rel-ip").unwrap(),
            initiative_id: InitiativeId::parse("initiative-orbit").unwrap(),
            project_id: ProjectId::parse("project-orbit").unwrap(),
            expected_initiative_version: AggregateVersion::initial(),
            expected_project_version: AggregateVersion::initial(),
            context: context("ip"),
        })
        .unwrap();
    service
        .link_project_product(LinkProjectProduct {
            id: RelationshipId::parse("rel-pp").unwrap(),
            project_id: ProjectId::parse("project-orbit").unwrap(),
            product_id: product(),
            expected_project_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: context("pp"),
        })
        .unwrap();
    assert_eq!(service.relationship_count(), 5);
}

fn assert_stakeholder_purpose_contract(purpose: StakeholderRelationshipPurpose, name: &str) {
    let mut service = service();
    let stakeholder = service
        .create_stakeholder(stakeholder())
        .unwrap()
        .into_value();
    let link = |suffix: &str, expected_stakeholder_version| LinkStakeholderRelationship {
        id: RelationshipId::parse(format!("rel-{name}-{suffix}")).unwrap(),
        stakeholder_id: sid(),
        subject: StakeholderSubject::Product(product()),
        purpose,
        expected_stakeholder_version,
        expected_subject_version: AggregateVersion::initial(),
        context: context(&format!("{name}-{suffix}")),
    };
    let first = service
        .link_stakeholder_relationship(link("first", stakeholder.version()))
        .unwrap();
    assert_eq!(first.outcome().audit_event_ids().len(), 1);
    let audit_count = service.audit_events().len();
    let duplicate = service
        .link_stakeholder_relationship(link("duplicate", stakeholder.version()))
        .unwrap();
    assert_eq!(duplicate.value().id(), first.value().id());
    assert_eq!(
        duplicate.outcome().effect_scope(),
        pmc_domain::audit::AuditEffectScope::None
    );
    assert_eq!(service.audit_events().len(), audit_count);
    let stale = service.link_stakeholder_relationship(link(
        "stale",
        AggregateVersion::new(stakeholder.version().get() + 1).unwrap(),
    ));
    assert_eq!(stale.unwrap_err().code(), ErrorCode::DomainConflict);
    assert_eq!(service.audit_events().len(), audit_count);
}

#[test]
fn stakeholder_responsibility_relationship_contract() {
    assert_stakeholder_purpose_contract(
        StakeholderRelationshipPurpose::Responsibility,
        "responsibility",
    );
}

#[test]
fn stakeholder_dependency_relationship_contract() {
    assert_stakeholder_purpose_contract(StakeholderRelationshipPurpose::Dependency, "dependency");
}

#[test]
fn stakeholder_name_only_update_does_not_reclassify_relationship() {
    let mut service = service();
    let stakeholder = service
        .create_stakeholder(stakeholder())
        .unwrap()
        .into_value();
    let relationship = service
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("rel-name-only").unwrap(),
            stakeholder_id: sid(),
            subject: StakeholderSubject::Product(product()),
            purpose: StakeholderRelationshipPurpose::Responsibility,
            expected_stakeholder_version: stakeholder.version(),
            expected_subject_version: AggregateVersion::initial(),
            context: context("name-only-link"),
        })
        .unwrap()
        .into_value();
    let before_audits = service.audit_events().len();
    service
        .update_stakeholder(UpdateStakeholderDetails {
            id: sid(),
            expected_version: stakeholder.version(),
            name: StakeholderName::parse("Synthetic Renamed Council").unwrap(),
            classification: None,
            context: context("name-only-update"),
        })
        .unwrap();
    let inspected = service
        .inspect_relationship(relationship.id())
        .unwrap()
        .unwrap();
    assert_eq!(inspected.version(), relationship.version());
    assert_eq!(inspected.classification(), relationship.classification());
    assert_eq!(service.audit_events().len(), before_audits + 1);
}

#[test]
fn unclassified_stakeholder_lockdown_remains_visible_and_cannot_be_lowered_ordinary() {
    let mut service = service();
    let stakeholder = service
        .create_stakeholder(stakeholder())
        .unwrap()
        .into_value();
    let relationship = service
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("rel-lockdown").unwrap(),
            stakeholder_id: sid(),
            subject: StakeholderSubject::Product(product()),
            purpose: StakeholderRelationshipPurpose::Dependency,
            expected_stakeholder_version: stakeholder.version(),
            expected_subject_version: AggregateVersion::initial(),
            context: context("lockdown-link"),
        })
        .unwrap()
        .into_value();
    let locked = service
        .update_stakeholder(UpdateStakeholderDetails {
            id: sid(),
            expected_version: stakeholder.version(),
            name: StakeholderName::parse("Synthetic Product Council").unwrap(),
            classification: Some(DataClassification::Unclassified),
            context: context("lockdown-update"),
        })
        .unwrap()
        .into_value();
    let inspected = service
        .inspect_relationship(relationship.id())
        .unwrap()
        .unwrap();
    assert_eq!(inspected.classification(), DataClassification::Unclassified);
    assert_eq!(inspected.version().get(), 2);
    let ordinary_recovery = service.update_stakeholder(UpdateStakeholderDetails {
        id: sid(),
        expected_version: locked.version(),
        name: StakeholderName::parse("Synthetic Product Council").unwrap(),
        classification: Some(DataClassification::Restricted),
        context: context("lockdown-lower"),
    });
    assert_eq!(
        ordinary_recovery.unwrap_err().code(),
        ErrorCode::SecurityPolicyDenied
    );
}

#[test]
fn stakeholder_create_and_reclassification_failures_roll_back_all_effects() {
    let mut create_service = service();
    create_service.inject_next_commit_failure();
    assert_eq!(
        create_service
            .create_stakeholder(stakeholder())
            .unwrap_err()
            .code(),
        ErrorCode::PlatformInternal
    );
    assert!(create_service.inspect_stakeholder(&sid()).is_none());
    assert!(create_service.audit_events().is_empty());

    let mut update_service = service();
    let stakeholder = update_service
        .create_stakeholder(stakeholder())
        .unwrap()
        .into_value();
    let relationship = update_service
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("rel-rollback-cascade").unwrap(),
            stakeholder_id: sid(),
            subject: StakeholderSubject::Product(product()),
            purpose: StakeholderRelationshipPurpose::Responsibility,
            expected_stakeholder_version: stakeholder.version(),
            expected_subject_version: AggregateVersion::initial(),
            context: context("rollback-cascade-link"),
        })
        .unwrap()
        .into_value();
    let audit_count = update_service.audit_events().len();
    update_service.inject_next_commit_failure();
    let failed = update_service.update_stakeholder(UpdateStakeholderDetails {
        id: sid(),
        expected_version: stakeholder.version(),
        name: StakeholderName::parse("Synthetic Product Council").unwrap(),
        classification: Some(DataClassification::Restricted),
        context: context("rollback-cascade-update"),
    });
    assert_eq!(failed.unwrap_err().code(), ErrorCode::PlatformInternal);
    assert_eq!(
        update_service
            .inspect_stakeholder(&sid())
            .unwrap()
            .version(),
        stakeholder.version()
    );
    assert_eq!(
        update_service
            .inspect_relationship(relationship.id())
            .unwrap()
            .unwrap()
            .version(),
        relationship.version()
    );
    assert_eq!(update_service.audit_events().len(), audit_count);
}

#[test]
fn project_milestone_validation_rechecks_version_parentage_and_classification() {
    let service = service();
    let base = ValidateProjectMilestone {
        project_id: ProjectId::parse("project-orbit").unwrap(),
        milestone_id: MilestoneId::parse("milestone-orbit").unwrap(),
        expected_project_version: AggregateVersion::initial(),
        expected_milestone_version: AggregateVersion::initial(),
        context: context("milestone-current"),
    };
    service.validate_project_milestone(base.clone()).unwrap();
    let mut stale = base.clone();
    stale.expected_project_version = AggregateVersion::new(2).unwrap();
    assert_eq!(
        service
            .validate_project_milestone(stale)
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );

    let wrong_parent_catalog = InMemoryEndpointCatalog::new([
        EndpointSnapshot::Project(ProjectSnapshot::new(
            base.project_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Milestone(MilestoneSnapshot::new(
            base.milestone_id.clone(),
            ProjectId::parse("project-other").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
    ]);
    let wrong_parent =
        InMemoryRelationshipService::new(FixedClock, wrong_parent_catalog, AuditIds(0));
    assert_eq!(
        wrong_parent
            .validate_project_milestone(base.clone())
            .unwrap_err()
            .code(),
        ErrorCode::DomainConflict
    );

    let unknown_catalog = InMemoryEndpointCatalog::new([
        EndpointSnapshot::Project(ProjectSnapshot::new(
            base.project_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Milestone(MilestoneSnapshot::new(
            base.milestone_id.clone(),
            base.project_id.clone(),
            AggregateVersion::initial(),
            DataClassification::Unclassified,
        )),
    ]);
    let unknown = InMemoryRelationshipService::new(FixedClock, unknown_catalog, AuditIds(0));
    assert_eq!(
        unknown.validate_project_milestone(base).unwrap_err().code(),
        ErrorCode::SecurityPolicyDenied
    );
}

#[test]
fn injected_failure_rolls_back_relationship_and_audit() {
    let mut service = service();
    service.inject_next_commit_failure();
    let result = service.link_portfolio_product(LinkPortfolioProduct {
        id: RelationshipId::parse("rel-failure").unwrap(),
        portfolio_id: pid(),
        product_id: product(),
        expected_portfolio_version: AggregateVersion::initial(),
        expected_product_version: AggregateVersion::initial(),
        context: context("failure"),
    });
    assert_eq!(result.unwrap_err().code(), ErrorCode::PlatformInternal);
    assert_eq!(service.relationship_count(), 0);
    assert!(service.audit_events().is_empty());
}
