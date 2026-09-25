//! Public-safe, deterministic domain contract fixtures.
//!
//! This module is test/training data only.  It deliberately uses the same
//! typed public command surface that an application caller uses; it does not
//! seed or reach into adapter state, and it is not a production bootstrap.

#![allow(clippy::result_large_err)]

use pmc_domain::audit::{AuditActor, AuditEventIdSource};
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::{
    CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome,
    OperationContext as DeliveryContext, RecordName,
};
use pmc_domain::error::DomainError;
use pmc_domain::execution::{
    ApprovalAuthorizationPort, ExecutionIdSource, RecoveryEvidence, RecoveryEvidencePort,
    RemovalPolicyPort,
};
use pmc_domain::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, KpiId,
    KpiObservationId, MilestoneId, PortfolioId, ProductId, ProjectId, RecoveryEvidenceId,
    RelationshipId, RoadmapId, StakeholderId,
};
use pmc_domain::portfolio::{
    CreateKpiDefinition, CreateKpiObservation, CreatePortfolio, CreateProduct, CreateRoadmap,
    LongText, OperationContext as PortfolioContext, ShortText,
};
use pmc_domain::provenance::{Provenance, ProvenanceReference};
use pmc_domain::relationships::{
    CreateStakeholder, LinkInitiativeProject, LinkPortfolioInitiative, LinkPortfolioProduct,
    LinkProductKpi, LinkProductRoadmap, LinkProjectProduct, LinkStakeholderRelationship,
    OperationContext as RelationshipContext, StakeholderKind, StakeholderName,
    StakeholderRelationshipPurpose, StakeholderSubject,
};
use pmc_domain::time::{Clock, UtcTimestamp};

use pmc_ledger::InMemoryProductLedger;

#[derive(Clone, Copy, Debug)]
pub struct SyntheticClock;
impl Clock for SyntheticClock {
    fn now(&self) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(1_912_464_000_000)
    }
}

#[derive(Clone, Debug)]
pub struct SyntheticAuditIds {
    namespace: &'static str,
    next: u64,
}
impl SyntheticAuditIds {
    fn new(namespace: &'static str) -> Self {
        Self { namespace, next: 0 }
    }
}
impl AuditEventIdSource for SyntheticAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.next += 1;
        AuditEventId::parse(format!("synthetic-{}-audit-{}", self.namespace, self.next))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SyntheticRemovalPolicy;
impl RemovalPolicyPort for SyntheticRemovalPolicy {
    fn allow_relationship_removal(&self, _: &RelationshipId, _: DataClassification) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SyntheticApprovalAuthorization;
impl ApprovalAuthorizationPort for SyntheticApprovalAuthorization {
    fn authorize_relationship_removal(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

#[derive(Clone, Debug, Default)]
pub struct SyntheticExecutionIds(u64);
impl ExecutionIdSource for SyntheticExecutionIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<pmc_domain::identity::PreparedIntentId, pmc_domain::DomainValueError> {
        self.0 += 1;
        pmc_domain::identity::PreparedIntentId::parse(format!("synthetic-prepared-{}", self.0))
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<pmc_domain::identity::ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.0 += 1;
        pmc_domain::identity::ApprovalReceiptId::parse(format!("synthetic-receipt-{}", self.0))
    }
}

pub type SyntheticLedger = InMemoryProductLedger<
    SyntheticClock,
    SyntheticAuditIds,
    SyntheticExecutionIds,
    SyntheticRemovalPolicy,
    SyntheticApprovalAuthorization,
>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixtureHandles {
    pub portfolio: PortfolioId,
    pub product: ProductId,
    pub roadmap: RoadmapId,
    pub kpi: KpiId,
    pub observation: KpiObservationId,
    pub initiative: InitiativeId,
    pub project: ProjectId,
    pub milestone: MilestoneId,
    pub stakeholder: StakeholderId,
    pub portfolio_product: RelationshipId,
    pub portfolio_initiative: RelationshipId,
    pub product_roadmap: RelationshipId,
    pub product_kpi: RelationshipId,
    pub initiative_project: RelationshipId,
    pub project_product: RelationshipId,
    pub stakeholder_responsibility: RelationshipId,
    pub stakeholder_dependency: RelationshipId,
    pub expected_version: AggregateVersion,
}

#[derive(Clone, Debug)]
pub struct SyntheticRecoveryEvidence(pub RecoveryEvidence);
impl RecoveryEvidencePort for SyntheticRecoveryEvidence {
    fn recovery_evidence(&self, id: &RelationshipId) -> Option<RecoveryEvidence> {
        if self.0.relationship_id() == id {
            Some(self.0.clone())
        } else {
            None
        }
    }
}

pub struct SyntheticPmc003Fixture {
    ledger: SyntheticLedger,
    handles: FixtureHandles,
}

impl SyntheticPmc003Fixture {
    pub fn build() -> Result<Self, DomainError> {
        let mut ledger = InMemoryProductLedger::new_with_execution_authorities(
            SyntheticClock,
            SyntheticAuditIds::new("portfolio"),
            SyntheticAuditIds::new("delivery"),
            SyntheticAuditIds::new("relationship"),
            SyntheticExecutionIds::default(),
            SyntheticRemovalPolicy,
            SyntheticApprovalAuthorization,
        );
        let handles = FixtureHandles {
            portfolio: id("synthetic-portfolio-alpha"),
            product: id("synthetic-product-alpha"),
            roadmap: id("synthetic-roadmap-alpha"),
            kpi: id("synthetic-kpi-alpha"),
            observation: id("synthetic-kpi-observation-alpha"),
            initiative: id("synthetic-initiative-alpha"),
            project: id("synthetic-project-alpha"),
            milestone: id("synthetic-milestone-alpha"),
            stakeholder: id("synthetic-stakeholder-alpha"),
            portfolio_product: rel("synthetic-rel-portfolio-product"),
            portfolio_initiative: rel("synthetic-rel-portfolio-initiative"),
            product_roadmap: rel("synthetic-rel-product-roadmap"),
            product_kpi: rel("synthetic-rel-product-kpi"),
            initiative_project: rel("synthetic-rel-initiative-project"),
            project_product: rel("synthetic-rel-project-product"),
            stakeholder_responsibility: rel("synthetic-rel-stakeholder-responsibility"),
            stakeholder_dependency: rel("synthetic-rel-stakeholder-dependency"),
            expected_version: AggregateVersion::initial(),
        };
        let provenance = provenance();
        ledger.create_portfolio(CreatePortfolio {
            id: handles.portfolio.clone(),
            name: short("Synthetic Alpha Portfolio"),
            details: long("Synthetic public-safe training portfolio"),
            classification: Some(DataClassification::Public),
            provenance: provenance.clone(),
            context: portfolio_context("portfolio"),
        })?;
        ledger.create_product(CreateProduct {
            id: handles.product.clone(),
            name: short("Synthetic Product Alpha"),
            details: long("Synthetic public-safe product record"),
            classification: Some(DataClassification::Internal),
            provenance: provenance.clone(),
            context: portfolio_context("product"),
        })?;
        ledger.create_roadmap(CreateRoadmap {
            id: handles.roadmap.clone(),
            name: short("Synthetic Roadmap Alpha"),
            details: long("Synthetic public-safe roadmap"),
            classification: Some(DataClassification::Internal),
            provenance: provenance.clone(),
            context: portfolio_context("roadmap"),
        })?;
        ledger.create_kpi_definition(CreateKpiDefinition {
            id: handles.kpi.clone(),
            name: short("Synthetic KPI Alpha"),
            definition: long("Synthetic definition for training only"),
            owner: short("Synthetic owner"),
            target: short("Synthetic target"),
            cadence: short("Weekly"),
            source: long("Synthetic source"),
            classification: Some(DataClassification::Internal),
            provenance: provenance.clone(),
            context: portfolio_context("kpi"),
        })?;
        ledger.create_kpi_observation(CreateKpiObservation {
            id: handles.observation.clone(),
            kpi_id: handles.kpi.clone(),
            value: short("42"),
            observed_at: SyntheticClock.now(),
            source: long("Synthetic observation source"),
            classification: Some(DataClassification::Confidential),
            provenance: provenance.clone(),
            context: portfolio_context("observation"),
        })?;
        ledger.create_initiative(CreateInitiative {
            context: delivery_context("initiative"),
            id: handles.initiative.clone(),
            name: record_name("Synthetic Initiative Alpha"),
            defined_outcome: outcome("Synthetic outcome"),
            classification: Some(DataClassification::Internal),
            provenance: provenance.clone(),
        })?;
        ledger.create_project(CreateProject {
            context: delivery_context("project"),
            id: handles.project.clone(),
            name: record_name("Synthetic Project Alpha"),
            start_at: UtcTimestamp::from_unix_millis(1_912_464_000_000),
            end_at: UtcTimestamp::from_unix_millis(1_912_550_400_000),
            classification: Some(DataClassification::Internal),
            provenance: provenance.clone(),
        })?;
        ledger.create_milestone(CreateMilestone {
            context: delivery_context("milestone"),
            id: handles.milestone.clone(),
            project_id: handles.project.clone(),
            name: record_name("Synthetic Milestone Alpha"),
            verification_criteria: pmc_domain::delivery::VerificationCriteria::parse(
                "Synthetic criteria",
                &correlation("milestone"),
            )
            .unwrap_or_else(|_| unreachable!()),
            due_at: UtcTimestamp::from_unix_millis(1_912_507_200_000),
            classification: None,
            provenance: provenance.clone(),
        })?;
        ledger.create_stakeholder(CreateStakeholder {
            id: handles.stakeholder.clone(),
            name: stakeholder_name("Synthetic Stakeholder Alpha"),
            kind: StakeholderKind::Organization,
            classification: Some(DataClassification::Public),
            provenance,
            context: relationship_context("stakeholder"),
        })?;
        ledger.link_portfolio_product(LinkPortfolioProduct {
            id: handles.portfolio_product.clone(),
            portfolio_id: handles.portfolio.clone(),
            product_id: handles.product.clone(),
            expected_portfolio_version: v(),
            expected_product_version: v(),
            context: relationship_context("portfolio-product"),
        })?;
        ledger.link_portfolio_initiative(LinkPortfolioInitiative {
            id: handles.portfolio_initiative.clone(),
            portfolio_id: handles.portfolio.clone(),
            initiative_id: handles.initiative.clone(),
            expected_portfolio_version: v(),
            expected_initiative_version: v(),
            context: relationship_context("portfolio-initiative"),
        })?;
        ledger.link_product_roadmap(LinkProductRoadmap {
            id: handles.product_roadmap.clone(),
            product_id: handles.product.clone(),
            roadmap_id: handles.roadmap.clone(),
            expected_product_version: v(),
            expected_roadmap_version: v(),
            context: relationship_context("product-roadmap"),
        })?;
        ledger.link_product_kpi(LinkProductKpi {
            id: handles.product_kpi.clone(),
            product_id: handles.product.clone(),
            kpi_id: handles.kpi.clone(),
            expected_product_version: v(),
            expected_kpi_version: v(),
            context: relationship_context("product-kpi"),
        })?;
        ledger.link_initiative_project(LinkInitiativeProject {
            id: handles.initiative_project.clone(),
            initiative_id: handles.initiative.clone(),
            project_id: handles.project.clone(),
            expected_initiative_version: v(),
            expected_project_version: v(),
            context: relationship_context("initiative-project"),
        })?;
        ledger.link_project_product(LinkProjectProduct {
            id: handles.project_product.clone(),
            project_id: handles.project.clone(),
            product_id: handles.product.clone(),
            expected_project_version: v(),
            expected_product_version: v(),
            context: relationship_context("project-product"),
        })?;
        ledger.link_stakeholder_relationship(LinkStakeholderRelationship {
            id: handles.stakeholder_responsibility.clone(),
            stakeholder_id: handles.stakeholder.clone(),
            subject: StakeholderSubject::Product(handles.product.clone()),
            purpose: StakeholderRelationshipPurpose::Responsibility,
            expected_stakeholder_version: v(),
            expected_subject_version: v(),
            context: relationship_context("stakeholder-responsibility"),
        })?;
        ledger.link_stakeholder_relationship(LinkStakeholderRelationship {
            id: handles.stakeholder_dependency.clone(),
            stakeholder_id: handles.stakeholder.clone(),
            subject: StakeholderSubject::Project(handles.project.clone()),
            purpose: StakeholderRelationshipPurpose::Dependency,
            expected_stakeholder_version: v(),
            expected_subject_version: v(),
            context: relationship_context("stakeholder-dependency"),
        })?;
        Ok(Self { ledger, handles })
    }
    pub fn ledger(&self) -> &SyntheticLedger {
        &self.ledger
    }
    pub fn ledger_mut(&mut self) -> &mut SyntheticLedger {
        &mut self.ledger
    }
    pub fn handles(&self) -> &FixtureHandles {
        &self.handles
    }
    pub fn recovery_for(&self, relationship_id: &RelationshipId) -> SyntheticRecoveryEvidence {
        let evidence = RecoveryEvidence::new(
            recovery_id(),
            "synthetic-recovery-evidence",
            SyntheticClock.now(),
            relationship_id.clone(),
            true,
        )
        .unwrap_or_else(|_| unreachable!());
        SyntheticRecoveryEvidence(evidence)
    }
}

fn id<T>(value: &str) -> T
where
    T: FromSyntheticId,
{
    T::from_synthetic(value)
}
fn rel(value: &str) -> RelationshipId {
    id(value)
}
trait FromSyntheticId: Sized {
    fn from_synthetic(value: &str) -> Self;
}
macro_rules! synthetic_ids { ($($t:ty),+) => { $(impl FromSyntheticId for $t { fn from_synthetic(value: &str) -> Self { <$t>::parse(value).unwrap_or_else(|_| unreachable!()) } })+ }; }
synthetic_ids!(
    PortfolioId,
    ProductId,
    RoadmapId,
    KpiId,
    KpiObservationId,
    InitiativeId,
    ProjectId,
    MilestoneId,
    StakeholderId,
    RelationshipId,
    RecoveryEvidenceId
);
fn v() -> AggregateVersion {
    AggregateVersion::initial()
}
fn correlation(s: &str) -> CorrelationId {
    CorrelationId::parse(format!("synthetic-correlation-{s}")).unwrap_or_else(|_| unreachable!())
}
fn portfolio_context(s: &str) -> PortfolioContext {
    PortfolioContext {
        idempotency_id: IdempotencyId::parse(format!("synthetic-portfolio-idem-{s}"))
            .unwrap_or_else(|_| unreachable!()),
        correlation_id: correlation(s),
    }
}
fn delivery_context(s: &str) -> DeliveryContext {
    DeliveryContext {
        idempotency_id: IdempotencyId::parse(format!("synthetic-delivery-idem-{s}"))
            .unwrap_or_else(|_| unreachable!()),
        correlation_id: correlation(s),
    }
}
fn relationship_context(s: &str) -> RelationshipContext {
    RelationshipContext {
        idempotency_id: IdempotencyId::parse(format!("synthetic-relationship-idem-{s}"))
            .unwrap_or_else(|_| unreachable!()),
        correlation_id: correlation(s),
    }
}
fn provenance() -> Provenance {
    Provenance::SyntheticFixture(
        ProvenanceReference::parse("synthetic-pmc003-fixture").unwrap_or_else(|_| unreachable!()),
    )
}
fn short(s: &str) -> ShortText {
    ShortText::parse(s).unwrap_or_else(|_| unreachable!())
}
fn long(s: &str) -> LongText {
    LongText::parse(s).unwrap_or_else(|_| unreachable!())
}
fn record_name(s: &str) -> RecordName {
    RecordName::parse(s, &correlation("record-name")).unwrap_or_else(|_| unreachable!())
}
fn outcome(s: &str) -> DefinedOutcome {
    DefinedOutcome::parse(s, &correlation("outcome")).unwrap_or_else(|_| unreachable!())
}
fn stakeholder_name(s: &str) -> StakeholderName {
    StakeholderName::parse(s).unwrap_or_else(|_| unreachable!())
}
fn recovery_id() -> RecoveryEvidenceId {
    id("synthetic-recovery-alpha")
}
