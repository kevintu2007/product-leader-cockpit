use pmc_domain::audit::{AuditActor, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::{
    CreateInitiative, CreateProject, DefinedOutcome, InMemoryDeliveryService, OperationContext,
    RecordName,
};
use pmc_domain::execution::{
    ApprovalAuthorizationPort, ApproveAndExecuteRemoveRelationship, ExecutionIdSource,
    PrepareRemoveRelationship, RecoveryEvidence, RecoveryEvidencePort, RemovalPolicyPort,
};
use pmc_domain::identity::{
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, InitiativeId,
    PortfolioId, PreparedIntentId, ProductId, ProjectId, RecoveryEvidenceId, RelationshipId,
    StakeholderId,
};
use pmc_domain::portfolio::{
    CreatePortfolio, CreateProduct, InMemoryPortfolioService, LongText,
    OperationContext as PortfolioOperationContext, ShortText,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, EndpointSnapshot, InMemoryEndpointCatalog, InMemoryRelationshipService,
    LinkPortfolioProduct, PortfolioSnapshot, ProductSnapshot, StakeholderKind, StakeholderName,
};
use pmc_domain::time::{Clock, UtcTimestamp};
use pmc_domain::DomainValueError;

#[derive(Clone, Copy)]
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_908_072_600_000)
    }
}

#[derive(Clone, Default)]
struct AuditIds(u64);
impl AuditEventIdSource for AuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0 += 1;
        AuditEventId::parse(format!("seam-audit-{}", self.0))
    }
}

fn provenance() -> Provenance {
    Provenance::SyntheticFixture(ProvenanceReference::parse("synthetic-ledger-seams").unwrap())
}

fn portfolio_context(id: &str) -> PortfolioOperationContext {
    PortfolioOperationContext {
        idempotency_id: IdempotencyId::parse(format!("portfolio-idem-{id}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("portfolio-correlation-{id}")).unwrap(),
    }
}

fn delivery_context(id: &str) -> OperationContext {
    OperationContext {
        idempotency_id: IdempotencyId::parse(format!("delivery-idem-{id}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("delivery-correlation-{id}")).unwrap(),
    }
}

fn relationship_context(id: &str) -> pmc_domain::relationships::OperationContext {
    pmc_domain::relationships::OperationContext {
        idempotency_id: IdempotencyId::parse(format!("relationship-idem-{id}")).unwrap(),
        correlation_id: CorrelationId::parse(format!("relationship-correlation-{id}")).unwrap(),
    }
}

fn short(value: &str) -> ShortText {
    ShortText::parse(value).unwrap()
}
fn long(value: &str) -> LongText {
    LongText::parse(value).unwrap()
}
fn name(value: &str) -> RecordName {
    RecordName::parse(value, &CorrelationId::parse("name-correlation").unwrap()).unwrap()
}
fn outcome(value: &str) -> DefinedOutcome {
    DefinedOutcome::parse(value, &CorrelationId::parse("outcome-correlation").unwrap()).unwrap()
}

#[derive(Clone, Default)]
struct ExecutionIds(u64);
impl ExecutionIdSource for ExecutionIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.0 += 1;
        PreparedIntentId::parse(format!("seam-prepared-{}", self.0))
    }
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.0 += 1;
        ApprovalReceiptId::parse(format!("seam-receipt-{}", self.0))
    }
}

#[derive(Clone, Copy)]
struct AllowRemoval;
impl RemovalPolicyPort for AllowRemoval {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}

#[derive(Clone, Copy)]
struct AllowApproval;
impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize_relationship_removal(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

#[derive(Clone)]
struct SyntheticRecovery(RecoveryEvidence);
impl RecoveryEvidencePort for SyntheticRecovery {
    fn recovery_evidence(&self, _: &RelationshipId) -> Option<RecoveryEvidence> {
        Some(self.0.clone())
    }
}

type RelationshipService = InMemoryRelationshipService<
    FixedClock,
    InMemoryEndpointCatalog,
    AuditIds,
    ExecutionIds,
    AllowRemoval,
    AllowApproval,
>;

fn relationship_catalog() -> InMemoryEndpointCatalog {
    InMemoryEndpointCatalog::new([
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-alpha").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-alpha").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )),
    ])
}

fn relationship_service() -> (RelationshipService, RelationshipId) {
    let mut service = InMemoryRelationshipService::with_execution_authorities(
        FixedClock,
        relationship_catalog(),
        AuditIds::default(),
        ExecutionIds::default(),
        AllowRemoval,
        AllowApproval,
    );
    let relationship_id = RelationshipId::parse("relationship-alpha").unwrap();
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: relationship_id.clone(),
            portfolio_id: PortfolioId::parse("portfolio-alpha").unwrap(),
            product_id: ProductId::parse("product-alpha").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: relationship_context("link"),
        })
        .unwrap();
    (service, relationship_id)
}

fn recovery_for(id: &RelationshipId) -> SyntheticRecovery {
    SyntheticRecovery(
        RecoveryEvidence::new(
            RecoveryEvidenceId::parse("recovery-seam").unwrap(),
            "synthetic-recovery",
            UtcTimestamp::from_unix_millis(900),
            id.clone(),
            true,
        )
        .unwrap(),
    )
}

fn execute_request(
    prepared: &pmc_domain::execution::PreparedIntent,
    id: &str,
) -> ApproveAndExecuteRemoveRelationship {
    ApproveAndExecuteRemoveRelationship {
        prepared_id: prepared.id().clone(),
        actor: AuditActor::HeadOfProducts,
        confirmation: prepared.confirmation_challenge().into(),
        acknowledged_payload_digest: prepared.payload_digest().clone(),
        context: relationship_context(id),
    }
}

#[test]
fn portfolio_and_delivery_snapshots_are_cloned_and_id_stable() {
    let mut portfolio = InMemoryPortfolioService::new(FixedClock, AuditIds::default());
    for id in ["portfolio-z", "portfolio-a"] {
        portfolio
            .create_portfolio(CreatePortfolio {
                id: PortfolioId::parse(id).unwrap(),
                name: short(id),
                details: long("synthetic record"),
                classification: Some(DataClassification::Public),
                provenance: provenance(),
                context: portfolio_context(id),
            })
            .unwrap();
    }
    let listed = portfolio.portfolios();
    assert_eq!(listed[0].id.as_str(), "portfolio-a");
    assert_eq!(listed[1].id.as_str(), "portfolio-z");
    let snapshots = portfolio.endpoint_snapshots();
    assert!(matches!(
        snapshots[0],
        EndpointSnapshot::Portfolio(PortfolioSnapshot { .. })
    ));

    let staged = portfolio.clone();
    portfolio
        .create_product(CreateProduct {
            id: ProductId::parse("product-live").unwrap(),
            name: short("live"),
            details: long("synthetic record"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
            context: portfolio_context("product"),
        })
        .unwrap();
    assert_eq!(portfolio.products().len(), 1);
    assert!(staged.products().is_empty());

    let mut delivery = InMemoryDeliveryService::new(FixedClock, AuditIds::default());
    delivery
        .create_initiative(CreateInitiative {
            context: delivery_context("initiative"),
            id: InitiativeId::parse("initiative-a").unwrap(),
            name: name("Synthetic initiative"),
            defined_outcome: outcome("Validate outcome"),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    delivery
        .create_project(CreateProject {
            context: delivery_context("project"),
            id: ProjectId::parse("project-a").unwrap(),
            name: name("Synthetic project"),
            start_at: UtcTimestamp::from_unix_millis(100),
            end_at: UtcTimestamp::from_unix_millis(200),
            classification: Some(DataClassification::Public),
            provenance: provenance(),
        })
        .unwrap();
    assert_eq!(delivery.initiatives()[0].id().as_str(), "initiative-a");
    assert_eq!(delivery.projects()[0].id().as_str(), "project-a");
    assert_eq!(delivery.endpoint_snapshots().len(), 2);
}

#[test]
fn relationship_catalog_refresh_is_narrow_and_clone_isolated() {
    let catalog = InMemoryEndpointCatalog::new([]);
    let mut service = InMemoryRelationshipService::new(FixedClock, catalog, AuditIds::default());
    let replacement =
        InMemoryEndpointCatalog::new([EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-new").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        ))]);
    service.replace_endpoint_catalog(replacement.clone());
    let staged = service.clone();
    service.replace_endpoint_catalog(InMemoryEndpointCatalog::new([]));
    assert_eq!(service.relationship_count(), staged.relationship_count());
    assert!(
        staged.relationships().unwrap().is_empty(),
        "catalog refresh must not materialize relationship state"
    );
}

#[test]
fn relationship_clone_keeps_prepared_h2b_intent_independent() {
    let (mut original, relationship_id) = relationship_service();
    let evidence = recovery_for(&relationship_id);
    let prepared = original
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("prepare"),
            },
            &evidence,
        )
        .unwrap();
    let mut staged = original.clone();

    staged
        .cancel_remove_relationship(
            prepared.id(),
            AuditActor::HeadOfProducts,
            relationship_context("cancel-staged"),
        )
        .unwrap();
    assert_eq!(staged.relationship_count(), 1);
    assert_eq!(original.relationship_count(), 1);

    original
        .approve_and_execute_remove_relationship(
            execute_request(&prepared, "execute-original"),
            &evidence,
        )
        .unwrap();
    assert_eq!(original.relationship_count(), 0);
    assert_eq!(staged.relationship_count(), 1);

    // A failed staged commit must not consume the staged prepared intent.
    let (mut untouched, relationship_id) = relationship_service();
    let evidence = recovery_for(&relationship_id);
    let prepared = untouched
        .prepare_remove_relationship(
            PrepareRemoveRelationship {
                relationship_id: relationship_id.clone(),
                context: relationship_context("prepare-failure"),
            },
            &evidence,
        )
        .unwrap();
    let mut failed_stage = untouched.clone();
    failed_stage.inject_next_commit_failure();
    assert!(failed_stage
        .approve_and_execute_remove_relationship(
            execute_request(&prepared, "execute-failed-stage"),
            &evidence,
        )
        .is_err());
    assert_eq!(failed_stage.relationship_count(), 1);
    assert_eq!(untouched.relationship_count(), 1);
    assert_eq!(untouched.audit_events().len(), 1);
    // Both the original and failed staged service retain an independently
    // usable prepared intent after the failed staged commit.
    untouched
        .approve_and_execute_remove_relationship(
            execute_request(&prepared, "execute-untouched"),
            &evidence,
        )
        .unwrap();
    failed_stage
        .approve_and_execute_remove_relationship(
            execute_request(&prepared, "retry-failed-stage"),
            &evidence,
        )
        .unwrap();
    assert_eq!(failed_stage.relationship_count(), 0);
}

#[test]
fn relationship_and_stakeholder_collections_are_identifier_ordered() {
    let (mut service, first_relationship) = relationship_service();
    service.replace_endpoint_catalog(InMemoryEndpointCatalog::new([
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-alpha").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-alpha").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )),
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-beta").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-beta").unwrap(),
            AggregateVersion::initial(),
            DataClassification::Internal,
        )),
    ]));
    let second_relationship = RelationshipId::parse("relationship-zeta").unwrap();
    service
        .link_portfolio_product(LinkPortfolioProduct {
            id: second_relationship.clone(),
            portfolio_id: PortfolioId::parse("portfolio-beta").unwrap(),
            product_id: ProductId::parse("product-beta").unwrap(),
            expected_portfolio_version: AggregateVersion::initial(),
            expected_product_version: AggregateVersion::initial(),
            context: relationship_context("link-second"),
        })
        .unwrap();
    let relationships = service.relationships().unwrap();
    assert_eq!(relationships[0].id(), &first_relationship);
    assert_eq!(relationships[1].id(), &second_relationship);

    for id in ["stakeholder-zeta", "stakeholder-alpha"] {
        service
            .create_stakeholder(CreateStakeholder {
                id: StakeholderId::parse(id).unwrap(),
                name: StakeholderName::parse(id).unwrap(),
                kind: StakeholderKind::Person,
                classification: Some(DataClassification::Public),
                provenance: provenance(),
                context: relationship_context(id),
            })
            .unwrap();
    }
    let stakeholders = service.stakeholders();
    assert_eq!(stakeholders[0].id().as_str(), "stakeholder-alpha");
    assert_eq!(stakeholders[1].id().as_str(), "stakeholder-zeta");
}

#[test]
fn relationship_collection_resolves_current_catalog_and_fails_closed() {
    let (mut service, relationship_id) = relationship_service();
    service.replace_endpoint_catalog(InMemoryEndpointCatalog::new([
        EndpointSnapshot::Portfolio(PortfolioSnapshot::new(
            PortfolioId::parse("portfolio-alpha").unwrap(),
            AggregateVersion::new(2).unwrap(),
            DataClassification::Public,
        )),
        EndpointSnapshot::Product(ProductSnapshot::new(
            ProductId::parse("product-alpha").unwrap(),
            AggregateVersion::new(3).unwrap(),
            DataClassification::Confidential,
        )),
    ]));
    let inspected = service
        .inspect_relationship(&relationship_id)
        .unwrap()
        .unwrap();
    let listed = service.relationships().unwrap();
    assert_eq!(listed, vec![inspected]);
    assert_eq!(listed[0].classification(), DataClassification::Confidential);
    assert_eq!(
        match &listed[0].endpoints()[0] {
            EndpointSnapshot::Portfolio(value) => value.version(),
            _ => panic!("expected portfolio endpoint"),
        },
        AggregateVersion::new(2).unwrap()
    );
    assert_eq!(
        match &listed[0].endpoints()[1] {
            EndpointSnapshot::Product(value) => value.version(),
            _ => panic!("expected product endpoint"),
        },
        AggregateVersion::new(3).unwrap()
    );

    service.replace_endpoint_catalog(InMemoryEndpointCatalog::new([]));
    assert!(service.relationships().is_err());
}
