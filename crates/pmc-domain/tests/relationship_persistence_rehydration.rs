#![allow(clippy::result_large_err)]

//! Stakeholder and relationship persistence rehydration contract.
//!
//! This file deliberately names the persistence seam before its implementation
//! exists.  It is the compact, public-safe contract for ordinary stakeholder
//! and typed-relationship recovery; H2b removal state and project-milestone
//! authority are intentionally outside this contract.

use pmc_domain::audit::{AuditEffectScope, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, KpiId, PortfolioId,
    ProductId, ProjectId, RelationshipId, RoadmapId, StakeholderId,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::*;
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_908_072_600_000)
    }
}

#[derive(Default)]
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("rehydration-audit-{}", self.0))
    }
}

fn idempotency(value: &str, correlation: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(format!("relationship-rehydrate-{value}")).unwrap(),
        correlation_id: CorrelationId::parse(correlation).unwrap(),
    }
}

fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("relationship-rehydrate-v1").unwrap())
}

fn stakeholder_id() -> StakeholderId {
    StakeholderId::parse("stakeholder-synthetic-council").unwrap()
}
fn portfolio_id() -> PortfolioId {
    PortfolioId::parse("portfolio-synthetic-alpha").unwrap()
}
fn product_id() -> ProductId {
    ProductId::parse("product-synthetic-orbit").unwrap()
}
fn initiative_id() -> InitiativeId {
    InitiativeId::parse("initiative-synthetic-orbit").unwrap()
}
fn project_id() -> ProjectId {
    ProjectId::parse("project-synthetic-orbit").unwrap()
}

fn catalog() -> InMemoryEndpointCatalog {
    InMemoryEndpointCatalog::new([
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            portfolio_id(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Product(ProductSnapshot::new(
            product_id(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )),
        EndpointSnapshot::Initiative(InitiativeSnapshot::new(
            initiative_id(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Roadmap(RoadmapSnapshot::new(
            RoadmapId::parse("roadmap-synthetic-orbit").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Kpi(KpiSnapshot::new(
            KpiId::parse("kpi-synthetic-adoption").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Confidential,
        )),
        EndpointSnapshot::Project(ProjectSnapshot::new(
            project_id(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Milestone(MilestoneSnapshot::new(
            pmc_domain::identity::MilestoneId::parse("milestone-synthetic-orbit").unwrap(),
            project_id(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
    ])
}

type Service = InMemoryRelationshipService<FixedClock, InMemoryEndpointCatalog, AuditIds>;

fn new_service() -> Service {
    InMemoryRelationshipService::new(FixedClock, catalog(), AuditIds::default())
}

fn create_stakeholder() -> CreateStakeholder {
    CreateStakeholder {
        id: stakeholder_id(),
        name: StakeholderName::parse("Synthetic Product Council").unwrap(),
        kind: StakeholderKind::Organization,
        classification: Some(DataClassification::Public),
        provenance: provenance(),
        context: idempotency("stakeholder-create", "correlation-stakeholder-create"),
    }
}

fn assert_valid(snapshot: &RelationshipPersistenceSnapshot) {
    RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders().to_vec(),
        snapshot.relationships().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
}

#[test]
fn ordinary_relationship_snapshot_rehydrates_and_replays_exact_history() {
    let mut original = new_service();
    let created = original.create_stakeholder(create_stakeholder()).unwrap();
    let stakeholder_version = created.value().version();

    // Every ordinary removable family is represented.  ProjectMilestone is
    // deliberately absent because its authority is a separate validation seam.
    let links = [
        original
            .link_portfolio_product(LinkPortfolioProduct {
                id: RelationshipId::parse("relationship-synthetic-portfolio-product").unwrap(),
                portfolio_id: portfolio_id(),
                product_id: product_id(),
                expected_portfolio_version: AggregateVersion::initial(),
                expected_product_version: AggregateVersion::initial(),
                context: idempotency("portfolio-product", "correlation-portfolio-product"),
            })
            .unwrap(),
        original
            .link_portfolio_initiative(LinkPortfolioInitiative {
                id: RelationshipId::parse("relationship-synthetic-portfolio-initiative").unwrap(),
                portfolio_id: portfolio_id(),
                initiative_id: initiative_id(),
                expected_portfolio_version: AggregateVersion::initial(),
                expected_initiative_version: AggregateVersion::initial(),
                context: idempotency("portfolio-initiative", "correlation-portfolio-initiative"),
            })
            .unwrap(),
        original
            .link_product_roadmap(LinkProductRoadmap {
                id: RelationshipId::parse("relationship-synthetic-product-roadmap").unwrap(),
                product_id: product_id(),
                roadmap_id: RoadmapId::parse("roadmap-synthetic-orbit").unwrap(),
                expected_product_version: AggregateVersion::initial(),
                expected_roadmap_version: AggregateVersion::initial(),
                context: idempotency("product-roadmap", "correlation-product-roadmap"),
            })
            .unwrap(),
        original
            .link_product_kpi(LinkProductKpi {
                id: RelationshipId::parse("relationship-synthetic-product-kpi").unwrap(),
                product_id: product_id(),
                kpi_id: KpiId::parse("kpi-synthetic-adoption").unwrap(),
                expected_product_version: AggregateVersion::initial(),
                expected_kpi_version: AggregateVersion::initial(),
                context: idempotency("product-kpi", "correlation-product-kpi"),
            })
            .unwrap(),
        original
            .link_initiative_project(LinkInitiativeProject {
                id: RelationshipId::parse("relationship-synthetic-initiative-project").unwrap(),
                initiative_id: initiative_id(),
                project_id: project_id(),
                expected_initiative_version: AggregateVersion::initial(),
                expected_project_version: AggregateVersion::initial(),
                context: idempotency("initiative-project", "correlation-initiative-project"),
            })
            .unwrap(),
        original
            .link_project_product(LinkProjectProduct {
                id: RelationshipId::parse("relationship-synthetic-project-product").unwrap(),
                project_id: project_id(),
                product_id: product_id(),
                expected_project_version: AggregateVersion::initial(),
                expected_product_version: AggregateVersion::initial(),
                context: idempotency("project-product", "correlation-project-product"),
            })
            .unwrap(),
        original
            .link_stakeholder_relationship(LinkStakeholderRelationship {
                id: RelationshipId::parse("relationship-synthetic-stakeholder-product").unwrap(),
                stakeholder_id: stakeholder_id(),
                subject: StakeholderSubject::Product(product_id()),
                purpose: StakeholderRelationshipPurpose::Responsibility,
                expected_stakeholder_version: stakeholder_version,
                expected_subject_version: AggregateVersion::initial(),
                context: idempotency("stakeholder-product", "correlation-stakeholder-product"),
            })
            .unwrap(),
    ];
    assert_eq!(links.len(), 7);

    // A stakeholder classification change is an atomic fan-out: the
    // stakeholder and every derived relationship mutation are replayable.
    let updated = original
        .update_stakeholder(UpdateStakeholderDetails {
            id: stakeholder_id(),
            expected_version: stakeholder_version,
            name: StakeholderName::parse("Synthetic Product Council Updated").unwrap(),
            classification: Some(DataClassification::Restricted),
            context: idempotency("stakeholder-update", "correlation-stakeholder-update"),
        })
        .unwrap();
    assert_eq!(
        updated.value().classification(),
        DataClassification::Restricted
    );
    assert!(updated.outcome().audit_event_ids().len() >= 2);
    assert_eq!(updated.outcome().effect_scope(), AuditEffectScope::Complete);

    let snapshot = original.persistence_snapshot().unwrap();
    for (index, capsule) in snapshot.replay().iter().enumerate() {
        assert_eq!(capsule.operation_ordinal(), (index + 1) as u64);
        assert!(!capsule.idempotency_id().to_string().is_empty());
        assert!(!capsule.correlation_id().to_string().is_empty());
        assert!(!capsule.audit_event_ids().is_empty());
        // These accessors force the persisted representation to retain the
        // typed historical command/result, rather than only a generic payload.
        let _typed_command = capsule.command();
        let _typed_result = capsule.result();
    }
    let mut forged_no_classification_request = snapshot.replay().to_vec();
    let original_update = forged_no_classification_request.last().unwrap().clone();
    *forged_no_classification_request.last_mut().unwrap() = RelationshipReplayCapsule::new(
        original_update.idempotency_id().clone(),
        original_update.correlation_id().clone(),
        original_update.operation_ordinal(),
        RelationshipPersistenceCommand::UpdateStakeholder {
            id: stakeholder_id(),
            expected_version: stakeholder_version,
            name: StakeholderName::parse("Synthetic Product Council Updated").unwrap(),
            classification: None,
        },
        original_update.result().clone(),
        original_update.audit_event_ids().to_vec(),
    );
    assert!(RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders().to_vec(),
        snapshot.relationships().to_vec(),
        forged_no_classification_request,
        snapshot.audits().to_vec(),
    )
    .is_err());
    assert_valid(&snapshot);
    let mut restored = InMemoryRelationshipService::rehydrate(
        FixedClock,
        catalog(),
        AuditIds::default(),
        RelationshipPersistenceSnapshot::validate(
            snapshot.stakeholders().to_vec(),
            snapshot.relationships().to_vec(),
            snapshot.replay().to_vec(),
            snapshot.audits().to_vec(),
        )
        .unwrap(),
    )
    .unwrap();
    let audit_count = restored.audit_events().len();
    let replay = restored
        .update_stakeholder(UpdateStakeholderDetails {
            id: stakeholder_id(),
            expected_version: stakeholder_version,
            name: StakeholderName::parse("Synthetic Product Council Updated").unwrap(),
            classification: Some(DataClassification::Restricted),
            context: idempotency("stakeholder-update", "different-correlation"),
        })
        .unwrap();
    assert_eq!(replay, updated);
    assert_eq!(restored.audit_events().len(), audit_count);
    assert_eq!(restored.relationship_count(), original.relationship_count());
    for relationship in snapshot.relationships() {
        assert_eq!(
            restored.inspect_relationship(relationship.id()).unwrap(),
            Some(relationship.clone())
        );
    }
}

#[test]
fn family_global_idempotency_replays_without_new_effects_after_rehydrate() {
    let mut original = new_service();
    original.create_stakeholder(create_stakeholder()).unwrap();
    let first = original
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("relationship-synthetic-idempotent").unwrap(),
            stakeholder_id: stakeholder_id(),
            subject: StakeholderSubject::Product(product_id()),
            purpose: StakeholderRelationshipPurpose::Dependency,
            expected_stakeholder_version: AggregateVersion::initial(),
            expected_subject_version: AggregateVersion::initial(),
            context: idempotency("idempotent-first", "correlation-idempotent-first"),
        })
        .unwrap();
    let snapshot = original.persistence_snapshot().unwrap();
    let mut restored = InMemoryRelationshipService::rehydrate(
        FixedClock,
        catalog(),
        AuditIds::default(),
        RelationshipPersistenceSnapshot::validate(
            snapshot.stakeholders().to_vec(),
            snapshot.relationships().to_vec(),
            snapshot.replay().to_vec(),
            snapshot.audits().to_vec(),
        )
        .unwrap(),
    )
    .unwrap();
    let before = restored.audit_events().len();
    let exact = restored
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("relationship-synthetic-idempotent").unwrap(),
            stakeholder_id: stakeholder_id(),
            subject: StakeholderSubject::Product(product_id()),
            purpose: StakeholderRelationshipPurpose::Dependency,
            expected_stakeholder_version: AggregateVersion::initial(),
            expected_subject_version: AggregateVersion::initial(),
            context: idempotency("idempotent-first", "correlation-replay"),
        })
        .unwrap();
    assert_eq!(exact, first);
    assert_eq!(restored.audit_events().len(), before);
}

#[test]
fn validated_snapshot_rejects_duplicate_identity_semantics_and_history_corruption() {
    let mut service = new_service();
    service.create_stakeholder(create_stakeholder()).unwrap();
    service
        .link_stakeholder_relationship(LinkStakeholderRelationship {
            id: RelationshipId::parse("relationship-synthetic-corruption").unwrap(),
            stakeholder_id: stakeholder_id(),
            subject: StakeholderSubject::Product(product_id()),
            purpose: StakeholderRelationshipPurpose::Responsibility,
            expected_stakeholder_version: AggregateVersion::initial(),
            expected_subject_version: AggregateVersion::initial(),
            context: idempotency("corruption-link", "correlation-corruption-link"),
        })
        .unwrap();
    let snapshot = service.persistence_snapshot().unwrap();

    let mut duplicate_stakeholders = snapshot.stakeholders().to_vec();
    duplicate_stakeholders.push(duplicate_stakeholders[0].clone());
    assert!(RelationshipPersistenceSnapshot::validate(
        duplicate_stakeholders,
        snapshot.relationships().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .is_err());

    let mut duplicate_relationships = snapshot.relationships().to_vec();
    duplicate_relationships.push(duplicate_relationships[0].clone());
    assert!(RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders().to_vec(),
        duplicate_relationships,
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .is_err());

    let mut missing_audit = snapshot.audits().to_vec();
    missing_audit.pop();
    assert!(RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders().to_vec(),
        snapshot.relationships().to_vec(),
        snapshot.replay().to_vec(),
        missing_audit,
    )
    .is_err());

    let mut reordered_audits = snapshot.audits().to_vec();
    reordered_audits.swap(0, 1);
    assert!(RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders().to_vec(),
        snapshot.relationships().to_vec(),
        snapshot.replay().to_vec(),
        reordered_audits,
    )
    .is_err());

    let validated = RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders().to_vec(),
        snapshot.relationships().to_vec(),
        snapshot.replay().to_vec(),
        snapshot.audits().to_vec(),
    )
    .unwrap();
    let changed_catalog =
        InMemoryEndpointCatalog::new([EndpointSnapshot::Product(ProductSnapshot::new(
            product_id(),
            AggregateVersion::initial(),
            DataClassification::Restricted,
        ))]);
    assert!(InMemoryRelationshipService::rehydrate(
        FixedClock,
        changed_catalog,
        AuditIds::default(),
        validated,
    )
    .is_err());
}
