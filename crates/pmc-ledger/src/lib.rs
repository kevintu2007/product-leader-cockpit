#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::result_large_err)]

//! Transactional, in-memory composition of the `pmc-domain` services.

pub mod sqlite;

use std::collections::{BTreeSet, HashMap};

use pmc_domain::audit::{AuditActor, AuditEvent, AuditEventIdSource};
use pmc_domain::delivery::{
    CreateInitiative, CreateMilestone, CreateProject, InMemoryDeliveryService, Initiative,
    Milestone, Project, UpdateInitiative, UpdateMilestone, UpdateProject,
};
use pmc_domain::error::{DomainError, ErrorCode, MessageKey};
use pmc_domain::execution::{
    ApprovalAuthorizationPort, ApproveAndExecuteRemoveRelationship, ExecutionIdSource,
    PrepareRemoveRelationship, PreparedIntent, RecoveryEvidencePort, RemovalOutcome,
    RemovalPolicyPort,
};
use pmc_domain::identity::*;
use pmc_domain::portfolio::{
    CreateKpiDefinition, CreateKpiObservation, CreatePortfolio, CreateProduct, CreateRoadmap,
    InMemoryPortfolioService, KpiDefinitionRecord, KpiObservationRecord,
    MutationOutcome as PortfolioMutationOutcome, PortfolioRecord, ProductRecord, RoadmapRecord,
    UpdateKpiDefinitionDetails, UpdateKpiObservationDetails, UpdatePortfolioDetails,
    UpdateProductDetails, UpdateRoadmapDetails,
};
use pmc_domain::relationships::{
    CreateStakeholder, InMemoryEndpointCatalog, InMemoryRelationshipService, LinkInitiativeProject,
    LinkPortfolioInitiative, LinkPortfolioProduct, LinkProductKpi, LinkProductRoadmap,
    LinkProjectProduct, LinkStakeholderRelationship, MutationOutcome, OperationContext,
    ProjectMilestoneValidation, RelationshipRecord, StakeholderRecord, UpdateStakeholderDetails,
    ValidateProjectMilestone,
};
use pmc_domain::time::Clock;
use pmc_domain::{
    actions::*,
    decisions::*,
    issues::*,
    risks::*,
    work_management::{ApprovalConfirmation, WorkManagementApproval, WorkManagementPreparedIntent},
    work_management_runtime::WorkManagementRuntimeComposition,
};

type RelationshipService<C, A, I, P, Q> =
    InMemoryRelationshipService<C, InMemoryEndpointCatalog, A, I, P, Q>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServiceNamespace {
    Portfolio,
    Delivery,
    Relationship,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct ServiceIdempotencyEntry {
    namespace: ServiceNamespace,
    rejection: Option<DomainError>,
    h2b_command: Option<H2bCommandIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct H2bCommandIdentity {
    prepared_id: PreparedIntentId,
    actor: AuditActor,
    confirmation: String,
    acknowledged_payload_digest: pmc_domain::execution::PayloadDigest,
}

impl From<&ApproveAndExecuteRemoveRelationship> for H2bCommandIdentity {
    fn from(command: &ApproveAndExecuteRemoveRelationship) -> Self {
        Self {
            prepared_id: command.prepared_id.clone(),
            actor: command.actor,
            confirmation: command.confirmation.clone(),
            acknowledged_payload_digest: command.acknowledged_payload_digest.clone(),
        }
    }
}

/// Atomic in-memory Product Ledger composition.  It intentionally provides no
/// durable persistence or generic command/transaction surface.
#[derive(Clone)]
pub struct InMemoryProductLedger<
    C,
    A,
    I = pmc_domain::execution::SequentialExecutionIdSource,
    P = pmc_domain::execution::DenyRelationshipRemoval,
    Q = pmc_domain::execution::DenyApprovalAuthorization,
> where
    C: Clock + Clone,
    A: AuditEventIdSource + Clone,
    I: ExecutionIdSource + Clone,
    P: RemovalPolicyPort + Clone,
    Q: ApprovalAuthorizationPort + Clone,
{
    portfolio: InMemoryPortfolioService<C, A>,
    delivery: InMemoryDeliveryService<C, A>,
    relationships: RelationshipService<C, A, I, P, Q>,
    fail_next_commit: bool,
    idempotency_namespaces: HashMap<IdempotencyId, ServiceIdempotencyEntry>,
    audit_events: Vec<AuditEvent>,
}

impl<C, A> InMemoryProductLedger<C, A>
where
    C: Clock + Clone,
    A: AuditEventIdSource + Clone,
{
    pub fn new(
        clock: C,
        portfolio_audit_ids: A,
        delivery_audit_ids: A,
        relationship_audit_ids: A,
    ) -> Self {
        let portfolio = InMemoryPortfolioService::new(clock.clone(), portfolio_audit_ids);
        let delivery = InMemoryDeliveryService::new(clock.clone(), delivery_audit_ids);
        let relationships = InMemoryRelationshipService::new(
            clock,
            InMemoryEndpointCatalog::new([]),
            relationship_audit_ids,
        );
        Self {
            portfolio,
            delivery,
            relationships,
            fail_next_commit: false,
            idempotency_namespaces: HashMap::new(),
            audit_events: Vec::new(),
        }
    }
}

impl<C, A, I, P, Q> InMemoryProductLedger<C, A, I, P, Q>
where
    C: Clock + Clone,
    A: AuditEventIdSource + Clone,
    I: ExecutionIdSource + Clone,
    P: RemovalPolicyPort + Clone,
    Q: ApprovalAuthorizationPort + Clone,
{
    pub fn new_with_execution_authorities(
        clock: C,
        portfolio_audit_ids: A,
        delivery_audit_ids: A,
        relationship_audit_ids: A,
        execution_ids: I,
        removal_policy: P,
        approval_authorization: Q,
    ) -> Self {
        let portfolio = InMemoryPortfolioService::new(clock.clone(), portfolio_audit_ids);
        let delivery = InMemoryDeliveryService::new(clock.clone(), delivery_audit_ids);
        let relationships = InMemoryRelationshipService::with_execution_authorities(
            clock,
            InMemoryEndpointCatalog::new([]),
            relationship_audit_ids,
            execution_ids,
            removal_policy,
            approval_authorization,
        );
        Self {
            portfolio,
            delivery,
            relationships,
            fail_next_commit: false,
            idempotency_namespaces: HashMap::new(),
            audit_events: Vec::new(),
        }
    }

    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }

    fn commit_error(correlation: &CorrelationId) -> DomainError {
        let key = match MessageKey::parse("ledger.commit_failed") {
            Ok(value) => value,
            Err(_) => unreachable!("static message key is valid"),
        };
        DomainError::new(ErrorCode::PlatformInternal, key, correlation.clone(), true)
    }

    fn refresh_catalog(&mut self) {
        let mut endpoints = self.portfolio.endpoint_snapshots();
        endpoints.extend(self.delivery.endpoint_snapshots());
        self.relationships
            .replace_endpoint_catalog(InMemoryEndpointCatalog::new(endpoints));
    }

    fn idempotency_error(correlation: &CorrelationId) -> DomainError {
        let key = match MessageKey::parse("ledger.idempotency_conflict") {
            Ok(value) => value,
            Err(_) => unreachable!("static message key is valid"),
        };
        DomainError::new(
            ErrorCode::DomainIdempotencyConflict,
            key,
            correlation.clone(),
            false,
        )
    }

    fn service_audits(&self, namespace: ServiceNamespace) -> &[AuditEvent] {
        match namespace {
            ServiceNamespace::Portfolio => self.portfolio.audit_events(),
            ServiceNamespace::Delivery => self.delivery.audit_events(),
            ServiceNamespace::Relationship => self.relationships.audit_events(),
        }
    }

    fn stage<T, F>(
        &mut self,
        correlation: &CorrelationId,
        idempotency_id: IdempotencyId,
        namespace: ServiceNamespace,
        command: F,
    ) -> Result<T, DomainError>
    where
        F: FnOnce(&mut Self) -> Result<T, DomainError>,
    {
        self.stage_with_h2b_identity(correlation, idempotency_id, namespace, None, command)
    }

    fn stage_with_h2b_identity<T, F>(
        &mut self,
        correlation: &CorrelationId,
        idempotency_id: IdempotencyId,
        namespace: ServiceNamespace,
        h2b_command: Option<H2bCommandIdentity>,
        command: F,
    ) -> Result<T, DomainError>
    where
        F: FnOnce(&mut Self) -> Result<T, DomainError>,
    {
        if let Some(previous) = self.idempotency_namespaces.get(&idempotency_id) {
            if previous.namespace != namespace {
                return Err(Self::idempotency_error(correlation));
            }
            if (previous.h2b_command.is_some() || h2b_command.is_some())
                && previous.h2b_command != h2b_command
            {
                return Err(Self::idempotency_error(correlation));
            }
            if let Some(rejection) = &previous.rejection {
                if !rejection.retryable() {
                    return Err(rejection.clone());
                }
            }
            self.refresh_catalog();
            return command(self);
        }
        let fail = self.fail_next_commit;
        let mut staged = self.clone();
        staged.fail_next_commit = false;
        staged.refresh_catalog();
        let old_audit_count = staged.service_audits(namespace).len();
        let result = command(&mut staged);
        staged.refresh_catalog();
        let new_events = staged.service_audits(namespace)[old_audit_count..].to_vec();
        if result.is_err() && new_events.is_empty() {
            return result;
        }
        let mut seen = staged
            .audit_events
            .iter()
            .map(|event| event.id().clone())
            .collect::<BTreeSet<_>>();
        for event in &new_events {
            if !seen.insert(event.id().clone()) {
                return Err(Self::commit_error(correlation));
            }
        }
        staged.audit_events.extend(new_events);
        staged.idempotency_namespaces.insert(
            idempotency_id,
            ServiceIdempotencyEntry {
                namespace,
                rejection: result.as_ref().err().cloned(),
                h2b_command,
            },
        );
        if fail {
            self.fail_next_commit = false;
            return Err(Self::commit_error(correlation));
        }
        *self = staged;
        result
    }

    pub fn create_portfolio(
        &mut self,
        c: CreatePortfolio,
    ) -> Result<PortfolioMutationOutcome<PortfolioRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.create_portfolio(c),
        )
    }
    pub fn update_portfolio_details(
        &mut self,
        c: UpdatePortfolioDetails,
    ) -> Result<PortfolioMutationOutcome<PortfolioRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.update_portfolio_details(c),
        )
    }
    pub fn create_product(
        &mut self,
        c: CreateProduct,
    ) -> Result<PortfolioMutationOutcome<ProductRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.create_product(c),
        )
    }
    pub fn update_product_details(
        &mut self,
        c: UpdateProductDetails,
    ) -> Result<PortfolioMutationOutcome<ProductRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.update_product_details(c),
        )
    }
    pub fn create_roadmap(
        &mut self,
        c: CreateRoadmap,
    ) -> Result<PortfolioMutationOutcome<RoadmapRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.create_roadmap(c),
        )
    }
    pub fn update_roadmap_details(
        &mut self,
        c: UpdateRoadmapDetails,
    ) -> Result<PortfolioMutationOutcome<RoadmapRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.update_roadmap_details(c),
        )
    }
    pub fn create_kpi_definition(
        &mut self,
        c: CreateKpiDefinition,
    ) -> Result<PortfolioMutationOutcome<KpiDefinitionRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.create_kpi_definition(c),
        )
    }
    pub fn update_kpi_definition_details(
        &mut self,
        c: UpdateKpiDefinitionDetails,
    ) -> Result<PortfolioMutationOutcome<KpiDefinitionRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.update_kpi_definition_details(c),
        )
    }
    pub fn create_kpi_observation(
        &mut self,
        c: CreateKpiObservation,
    ) -> Result<PortfolioMutationOutcome<KpiObservationRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.create_kpi_observation(c),
        )
    }
    pub fn update_kpi_observation_details(
        &mut self,
        c: UpdateKpiObservationDetails,
    ) -> Result<PortfolioMutationOutcome<KpiObservationRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Portfolio,
            |s| s.portfolio.update_kpi_observation_details(c),
        )
    }

    pub fn create_initiative(&mut self, c: CreateInitiative) -> Result<Initiative, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Delivery,
            |s| s.delivery.create_initiative(c),
        )
    }
    pub fn update_initiative(&mut self, c: UpdateInitiative) -> Result<Initiative, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Delivery,
            |s| s.delivery.update_initiative(c),
        )
    }
    pub fn create_project(&mut self, c: CreateProject) -> Result<Project, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Delivery,
            |s| s.delivery.create_project(c),
        )
    }
    pub fn update_project(&mut self, c: UpdateProject) -> Result<Project, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Delivery,
            |s| s.delivery.update_project(c),
        )
    }
    pub fn create_milestone(&mut self, c: CreateMilestone) -> Result<Milestone, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Delivery,
            |s| s.delivery.create_milestone(c),
        )
    }
    pub fn update_milestone(&mut self, c: UpdateMilestone) -> Result<Milestone, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Delivery,
            |s| s.delivery.update_milestone(c),
        )
    }

    pub fn create_stakeholder(
        &mut self,
        c: CreateStakeholder,
    ) -> Result<MutationOutcome<StakeholderRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.create_stakeholder(c),
        )
    }
    pub fn update_stakeholder(
        &mut self,
        c: UpdateStakeholderDetails,
    ) -> Result<MutationOutcome<StakeholderRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.update_stakeholder(c),
        )
    }
    pub fn link_portfolio_product(
        &mut self,
        c: LinkPortfolioProduct,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_portfolio_product(c),
        )
    }
    pub fn link_portfolio_initiative(
        &mut self,
        c: LinkPortfolioInitiative,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_portfolio_initiative(c),
        )
    }
    pub fn link_product_roadmap(
        &mut self,
        c: LinkProductRoadmap,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_product_roadmap(c),
        )
    }
    pub fn link_product_kpi(
        &mut self,
        c: LinkProductKpi,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_product_kpi(c),
        )
    }
    pub fn link_initiative_project(
        &mut self,
        c: LinkInitiativeProject,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_initiative_project(c),
        )
    }
    pub fn link_project_product(
        &mut self,
        c: LinkProjectProduct,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_project_product(c),
        )
    }
    pub fn validate_project_milestone(
        &mut self,
        c: ValidateProjectMilestone,
    ) -> Result<ProjectMilestoneValidation, DomainError> {
        self.refresh_catalog();
        self.relationships.validate_project_milestone(c)
    }
    pub fn link_stakeholder_relationship(
        &mut self,
        c: LinkStakeholderRelationship,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.link_stakeholder_relationship(c),
        )
    }
    pub fn prepare_remove_relationship<PV: RecoveryEvidencePort>(
        &mut self,
        c: PrepareRemoveRelationship,
        p: &PV,
    ) -> Result<PreparedIntent, DomainError> {
        let id = c.context.correlation_id.clone();
        self.stage(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.prepare_remove_relationship(c, p),
        )
    }
    pub fn cancel_remove_relationship(
        &mut self,
        id: &PreparedIntentId,
        actor: AuditActor,
        c: OperationContext,
    ) -> Result<(), DomainError> {
        let correlation = c.correlation_id.clone();
        let id = id.clone();
        self.stage(
            &correlation,
            c.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            |s| s.relationships.cancel_remove_relationship(&id, actor, c),
        )
    }
    pub fn approve_and_execute_remove_relationship<PV: RecoveryEvidencePort>(
        &mut self,
        c: ApproveAndExecuteRemoveRelationship,
        p: &PV,
    ) -> Result<RemovalOutcome, DomainError> {
        let id = c.context.correlation_id.clone();
        let h2b_command = H2bCommandIdentity::from(&c);
        self.stage_with_h2b_identity(
            &id,
            c.context.idempotency_id.clone(),
            ServiceNamespace::Relationship,
            Some(h2b_command),
            |s| {
                s.relationships
                    .approve_and_execute_remove_relationship(c, p)
            },
        )
    }

    pub fn inspect_portfolio(&self, id: &PortfolioId) -> Option<PortfolioRecord> {
        self.portfolio.inspect_portfolio(id)
    }
    pub fn inspect_product(&self, id: &ProductId) -> Option<ProductRecord> {
        self.portfolio.inspect_product(id)
    }
    pub fn inspect_roadmap(&self, id: &RoadmapId) -> Option<RoadmapRecord> {
        self.portfolio.inspect_roadmap(id)
    }
    pub fn inspect_kpi_definition(&self, id: &KpiId) -> Option<KpiDefinitionRecord> {
        self.portfolio.inspect_kpi_definition(id)
    }
    pub fn inspect_kpi_observation(&self, id: &KpiObservationId) -> Option<KpiObservationRecord> {
        self.portfolio.inspect_kpi_observation(id)
    }
    pub fn inspect_initiative(&self, id: &InitiativeId) -> Option<Initiative> {
        self.delivery.inspect_initiative(id).cloned()
    }
    pub fn inspect_project(&self, id: &ProjectId) -> Option<Project> {
        self.delivery.inspect_project(id).cloned()
    }
    pub fn inspect_milestone(&self, id: &MilestoneId) -> Option<Milestone> {
        self.delivery.inspect_milestone(id).cloned()
    }
    pub fn inspect_stakeholder(&self, id: &StakeholderId) -> Option<StakeholderRecord> {
        self.relationships.inspect_stakeholder(id).cloned()
    }
    pub fn inspect_relationship(
        &self,
        id: &RelationshipId,
    ) -> Result<Option<RelationshipRecord>, DomainError> {
        self.relationships.inspect_relationship(id)
    }
    pub fn portfolios(&self) -> Vec<PortfolioRecord> {
        self.portfolio.portfolios()
    }
    pub fn products(&self) -> Vec<ProductRecord> {
        self.portfolio.products()
    }
    pub fn roadmaps(&self) -> Vec<RoadmapRecord> {
        self.portfolio.roadmaps()
    }
    pub fn kpi_definitions(&self) -> Vec<KpiDefinitionRecord> {
        self.portfolio.kpi_definitions()
    }
    pub fn kpi_observations(&self) -> Vec<KpiObservationRecord> {
        self.portfolio.kpi_observations()
    }
    pub fn initiatives(&self) -> Vec<Initiative> {
        self.delivery.initiatives()
    }
    pub fn projects(&self) -> Vec<Project> {
        self.delivery.projects()
    }
    pub fn milestones(&self) -> Vec<Milestone> {
        self.delivery.milestones()
    }
    pub fn stakeholders(&self) -> Vec<StakeholderRecord> {
        self.relationships.stakeholders()
    }
    pub fn relationships(&self) -> Result<Vec<RelationshipRecord>, DomainError> {
        self.relationships.relationships()
    }
    pub fn audit_events(&self) -> Vec<AuditEvent> {
        self.audit_events.clone()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkNamespace {
    Action,
    Decision,
    Risk,
    Issue,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkOperation {
    CreateActionRequestDraft,
    SubmitActionRequest,
    DeclineActionRequest,
    WithdrawActionRequest,
    StartAction,
    LinkActionCompletionEvidence,
    CreateDecisionRequestDraft,
    SubmitDecisionRequest,
    WithdrawDecisionRequest,
    CreateRisk,
    UpdateRiskResponse,
    CreateIssue,
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum WorkIdentity {
    Ordinary(WorkOperation),
    PrepareAcceptActionRequest(PrepareAcceptActionRequest),
    ExecuteAcceptActionRequest(ApproveAndExecuteAcceptActionRequest),
    PrepareCompleteAction(PrepareCompleteAction),
    PrepareCancelAction(PrepareCancelAction),
    PrepareReopenAction(PrepareReopenAction),
    ExecuteCompleteAction(ApproveAndExecuteCompleteAction),
    ExecuteCancelAction(ApproveAndExecuteCancelAction),
    ExecuteReopenAction(ApproveAndExecuteReopenAction),
    PrepareResolveDecisionRequest(PrepareResolveDecisionRequest),
    PrepareSupersedeDecision(PrepareSupersedeDecision),
    ExecuteResolveDecisionRequest(ApproveAndExecuteResolveDecisionRequest),
    ExecuteSupersedeDecision(ApproveAndExecuteSupersedeDecision),
    PrepareRiskOccurrence(PrepareRecordRiskOccurrence),
    PrepareCloseRisk(PrepareCloseRisk),
    ExecuteRiskOccurrence(ApproveAndExecuteRecordRiskOccurrence),
    ExecuteCloseRisk(ApproveAndExecuteCloseRisk),
    PrepareResolveIssue(PrepareResolveIssue),
    PrepareCloseIssue(PrepareCloseIssue),
    PrepareReopenIssue(PrepareReopenIssue),
    ExecuteResolveIssue(ApproveAndExecuteIssueTransition),
    ExecuteCloseIssue(ApproveAndExecuteIssueTransition),
    ExecuteReopenIssue(ApproveAndExecuteIssueTransition),
}
impl From<WorkOperation> for WorkIdentity {
    fn from(value: WorkOperation) -> Self {
        Self::Ordinary(value)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkIdempotencyEntry {
    namespace: WorkNamespace,
    identity: WorkIdentity,
    terminal: bool,
    rejection: Option<DomainError>,
}

/// Synthetic-only, transactional in-memory composition of the four Work
/// Management lifecycle services. A new value is empty and makes no durability
/// or reopen claim.
pub struct InMemoryWorkManagementLedger<
    C,
    AI,
    DI,
    RI,
    II,
    Z,
    AP,
    DP,
    RP,
    IP,
    AE,
    DE,
    RE,
    RC,
    IE,
    IC,
> where
    C: Clock,
    AI: ActionServiceIdSource,
    DI: DecisionServiceIdSource,
    RI: RiskServiceIdSource,
    II: IssueServiceIdSource,
    Z: pmc_domain::work_management::ApprovalAuthorizationPort,
    AP: ActionExecutionPolicyPort,
    DP: DecisionExecutionPolicyPort,
    RP: RiskExecutionPolicyPort,
    IP: IssueExecutionPolicyPort,
    AE: ActionEvidenceAuthorityPort,
    DE: DecisionEvidenceAuthorityPort,
    RE: RiskEvidenceAuthorityPort,
    RC: RiskClassificationAuthorityPort,
    IE: IssueEvidenceAuthorityPort,
    IC: IssueClassificationAuthorityPort,
{
    actions: InMemoryActionService<C, AI, Z, AP, AE>,
    decisions: InMemoryDecisionService<C, DI, Z, DP, DE>,
    risk_issue_runtime: WorkManagementRuntimeComposition<C, RI, II, Z, RP, RE, RC, IP, IE, IC>,
    fail_next_commit: bool,
    idempotency_namespaces: HashMap<IdempotencyId, WorkIdempotencyEntry>,
    prepared_idempotency: HashMap<PreparedIntentId, IdempotencyId>,
    audit_events: Vec<AuditEvent>,
}

impl<C, AI, DI, RI, II, Z, AP, DP, RP, IP, AE, DE, RE, RC, IE, IC>
    InMemoryWorkManagementLedger<C, AI, DI, RI, II, Z, AP, DP, RP, IP, AE, DE, RE, RC, IE, IC>
where
    C: Clock + Clone,
    AI: ActionServiceIdSource + Clone,
    DI: DecisionServiceIdSource + Clone,
    RI: RiskServiceIdSource + Clone,
    II: IssueServiceIdSource + Clone,
    Z: pmc_domain::work_management::ApprovalAuthorizationPort + Clone,
    AP: ActionExecutionPolicyPort + Clone,
    DP: DecisionExecutionPolicyPort + Clone,
    RP: RiskExecutionPolicyPort + Clone,
    IP: IssueExecutionPolicyPort + Clone,
    AE: ActionEvidenceAuthorityPort + Clone,
    DE: DecisionEvidenceAuthorityPort + Clone,
    RE: RiskEvidenceAuthorityPort + Clone,
    RC: RiskClassificationAuthorityPort + Clone,
    IE: IssueEvidenceAuthorityPort + Clone,
    IC: IssueClassificationAuthorityPort + Clone,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: C,
        action_ids: AI,
        decision_ids: DI,
        risk_ids: RI,
        issue_ids: II,
        authorization: Z,
        action_policy: AP,
        decision_policy: DP,
        risk_policy: RP,
        issue_policy: IP,
        action_evidence: AE,
        decision_evidence: DE,
        risk_evidence: RE,
        risk_classification: RC,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Self {
        let risk_issue_runtime = WorkManagementRuntimeComposition::new(
            clock.clone(),
            risk_ids,
            authorization.clone(),
            risk_policy,
            risk_evidence,
            risk_classification,
            clock.clone(),
            issue_ids,
            authorization.clone(),
            issue_policy,
            issue_evidence,
            issue_classification,
        );
        Self {
            actions: InMemoryActionService::new(
                clock.clone(),
                action_ids,
                authorization.clone(),
                action_policy,
                action_evidence,
            ),
            decisions: InMemoryDecisionService::new(
                clock.clone(),
                decision_ids,
                authorization.clone(),
                decision_policy,
                decision_evidence,
            ),
            risk_issue_runtime,
            fail_next_commit: false,
            idempotency_namespaces: HashMap::new(),
            prepared_idempotency: HashMap::new(),
            audit_events: Vec::new(),
        }
    }

    pub fn inject_next_work_management_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }

    fn work_commit_error(correlation: &CorrelationId) -> DomainError {
        DomainError::new(
            ErrorCode::PlatformInternal,
            MessageKey::parse("ledger.commit_failed")
                .unwrap_or_else(|_| unreachable!("static message key")),
            correlation.clone(),
            true,
        )
    }

    fn work_idempotency_error(correlation: &CorrelationId) -> DomainError {
        DomainError::new(
            ErrorCode::DomainIdempotencyConflict,
            MessageKey::parse("ledger.idempotency_conflict")
                .unwrap_or_else(|_| unreachable!("static message key")),
            correlation.clone(),
            false,
        )
    }

    fn same_safe_error(left: &DomainError, right: &DomainError) -> bool {
        left.code() == right.code()
            && left.message_key() == right.message_key()
            && left.params() == right.params()
            && left.retryable() == right.retryable()
            && left.extensions() == right.extensions()
            && left.private_detail_ref() == right.private_detail_ref()
    }

    fn identity_idempotency() -> IdempotencyId {
        IdempotencyId::parse("ledger-business-identity")
            .unwrap_or_else(|_| unreachable!("static identity id"))
    }

    fn identity_correlation() -> CorrelationId {
        CorrelationId::parse("ledger-business-correlation")
            .unwrap_or_else(|_| unreachable!("static correlation id"))
    }

    fn identity_approval(approval: &WorkManagementApproval) -> WorkManagementApproval {
        WorkManagementApproval::new(
            approval.prepared_id().clone(),
            approval.actor(),
            approval.acknowledged_payload_digest().clone(),
            Self::identity_idempotency(),
            Some(ApprovalConfirmation::Confirmed),
        )
        .unwrap_or_else(|_| unreachable!("existing typed approval remains valid"))
    }

    fn isolated_copy(&self) -> Self {
        Self {
            actions: self.actions.isolated_copy(),
            decisions: self.decisions.isolated_copy(),
            risk_issue_runtime: self.risk_issue_runtime.isolated_stage(),
            fail_next_commit: false,
            idempotency_namespaces: self.idempotency_namespaces.clone(),
            prepared_idempotency: self.prepared_idempotency.clone(),
            audit_events: self.audit_events.clone(),
        }
    }

    fn work_audit_counts(&self) -> [usize; 4] {
        [
            self.actions.audit_events().len(),
            self.decisions.audit_events().len(),
            self.risk_issue_runtime.risk_audit_events().len(),
            self.risk_issue_runtime.issue_audit_events().len(),
        ]
    }

    fn work_audits_since(&self, counts: [usize; 4]) -> Vec<AuditEvent> {
        let mut events = Vec::new();
        events.extend_from_slice(&self.actions.audit_events()[counts[0]..]);
        events.extend_from_slice(&self.decisions.audit_events()[counts[1]..]);
        events.extend_from_slice(&self.risk_issue_runtime.risk_audit_events()[counts[2]..]);
        events.extend_from_slice(&self.risk_issue_runtime.issue_audit_events()[counts[3]..]);
        events.sort_by_key(|event| (event.occurred_at(), event.id().clone()));
        events
    }

    fn work_stage<T>(
        &mut self,
        correlation: &CorrelationId,
        idempotency: IdempotencyId,
        namespace: WorkNamespace,
        identity: impl Into<WorkIdentity>,
        command: impl FnOnce(&mut Self) -> Result<T, DomainError>,
    ) -> Result<T, DomainError> {
        let identity = identity.into();
        let previous = self.idempotency_namespaces.get(&idempotency).cloned();
        if let Some(previous) = &previous {
            if previous.namespace != namespace {
                return Err(Self::work_idempotency_error(correlation));
            }
            if previous.identity != identity {
                return Err(Self::work_idempotency_error(correlation));
            }
            if previous.terminal {
                if let Some(rejection) = &previous.rejection {
                    return Err(rejection.clone());
                }
                let mut replay = self.isolated_copy();
                let counts = replay.work_audit_counts();
                let result = command(&mut replay);
                if replay.work_audits_since(counts).is_empty() {
                    return result;
                }
                return Err(Self::work_idempotency_error(correlation));
            }
        }
        let fail = self.fail_next_commit;
        let mut staged = self.isolated_copy();
        let old_counts = staged.work_audit_counts();
        let result = command(&mut staged);
        let new_events = staged.work_audits_since(old_counts);
        if let (Some(previous), Err(error)) = (&previous, &result) {
            if previous
                .rejection
                .as_ref()
                .is_some_and(|stored| Self::same_safe_error(stored, error))
            {
                return Err(previous
                    .rejection
                    .clone()
                    .unwrap_or_else(|| unreachable!("matched rejection exists")));
            }
        }
        if result.is_err() && new_events.is_empty() {
            return result;
        }
        let mut seen = staged
            .audit_events
            .iter()
            .map(|event| event.id().clone())
            .collect::<BTreeSet<_>>();
        for event in &new_events {
            if !seen.insert(event.id().clone()) {
                return Err(Self::work_commit_error(correlation));
            }
        }
        staged.audit_events.extend(new_events);
        staged.idempotency_namespaces.insert(
            idempotency,
            WorkIdempotencyEntry {
                namespace,
                identity,
                terminal: result
                    .as_ref()
                    .map_or_else(|error| !error.retryable(), |_| true),
                rejection: result.as_ref().err().cloned(),
            },
        );
        if fail {
            self.fail_next_commit = false;
            return Err(Self::work_commit_error(correlation));
        }
        *self = staged;
        result
    }

    pub fn work_management_audit_events(&self) -> Vec<AuditEvent> {
        self.audit_events.clone()
    }
    pub fn action_audit_events(&self) -> Vec<AuditEvent> {
        self.actions.audit_events().to_vec()
    }
    pub fn decision_audit_events(&self) -> Vec<AuditEvent> {
        self.decisions.audit_events().to_vec()
    }
    pub fn risk_audit_events(&self) -> Vec<AuditEvent> {
        self.risk_issue_runtime.risk_audit_events().to_vec()
    }
    pub fn issue_audit_events(&self) -> Vec<AuditEvent> {
        self.risk_issue_runtime.issue_audit_events().to_vec()
    }
    pub fn derive_attention(
        &self,
        inputs: &pmc_domain::attention::AttentionInputs,
    ) -> pmc_domain::attention::AttentionResult {
        pmc_domain::attention::derive_attention(inputs)
    }
    pub fn action_request(&self, id: &ActionRequestId) -> Option<ActionRequestRecord> {
        self.actions.request(id).cloned()
    }
    pub fn action(&self, id: &ActionId) -> Option<ActionRecord> {
        self.actions.action(id).cloned()
    }
    pub fn decision_request(&self, id: &DecisionRequestId) -> Option<DecisionRequestRecord> {
        self.decisions.request(id).cloned()
    }
    pub fn decision(&self, id: &DecisionId) -> Option<DecisionRecord> {
        self.decisions.decision(id).cloned()
    }
    pub fn risk(&self, id: &RiskId) -> Option<RiskRecord> {
        self.risk_issue_runtime.risk(id).cloned()
    }
    pub fn issue(&self, id: &IssueId) -> Option<IssueRecord> {
        self.risk_issue_runtime.issue(id)
    }
    pub fn risk_issue_links(&self) -> Vec<RiskIssueLink> {
        self.risk_issue_runtime.risk_issue_links().to_vec()
    }
    pub fn risk_issue_link(&self, risk_id: &RiskId, issue_id: &IssueId) -> Option<RiskIssueLink> {
        self.risk_issue_runtime
            .risk_issue_link(risk_id, issue_id)
            .cloned()
    }
    pub fn action_requests_for_decision(&self, id: &DecisionId) -> Vec<ActionRequestRecord> {
        self.actions.action_requests_for_decision(id)
    }
    pub fn actions_for_decision(&self, id: &DecisionId) -> Vec<ActionRecord> {
        self.actions.actions_for_decision(id)
    }
    pub fn discard_action_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let removed = self.actions.discard_prepared_intent(id);
        self.finish_prepared_discard(id, removed);
        removed
    }
    pub fn discard_decision_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let removed = self.decisions.discard_prepared_intent(id);
        self.finish_prepared_discard(id, removed);
        removed
    }
    pub fn discard_risk_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let removed = self.risk_issue_runtime.discard_risk_prepared_intent(id);
        self.finish_prepared_discard(id, removed);
        removed
    }
    pub fn discard_issue_prepared_intent(&mut self, id: &PreparedIntentId) -> bool {
        let removed = self.risk_issue_runtime.discard_issue_prepared_intent(id);
        self.finish_prepared_discard(id, removed);
        removed
    }
    fn finish_prepared_discard(&mut self, id: &PreparedIntentId, removed: bool) {
        if removed {
            if let Some(key) = self.prepared_idempotency.remove(id) {
                self.idempotency_namespaces.remove(&key);
            }
        }
    }
    fn track_prepared(&mut self, key: IdempotencyId, prepared: &WorkManagementPreparedIntent) {
        self.prepared_idempotency.insert(prepared.id().clone(), key);
    }

    pub fn create_action_request_draft(
        &mut self,
        c: CreateActionRequestDraft,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkOperation::CreateActionRequestDraft,
            |s| s.actions.create_action_request_draft(c),
        )
    }
    pub fn submit_action_request(
        &mut self,
        c: SubmitActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkOperation::SubmitActionRequest,
            |s| s.actions.submit_action_request(c),
        )
    }
    pub fn decline_action_request(
        &mut self,
        c: DeclineActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkOperation::DeclineActionRequest,
            |s| s.actions.decline_action_request(c),
        )
    }
    pub fn withdraw_action_request(
        &mut self,
        c: WithdrawActionRequest,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkOperation::WithdrawActionRequest,
            |s| s.actions.withdraw_action_request(c),
        )
    }
    pub fn start_action(
        &mut self,
        c: StartAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkOperation::StartAction,
            |s| s.actions.start_action(c),
        )
    }
    pub fn link_action_completion_evidence(
        &mut self,
        c: LinkActionCompletionEvidence,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkOperation::LinkActionCompletionEvidence,
            |s| s.actions.link_action_completion_evidence(c),
        )
    }
    pub fn prepare_accept_action_request(
        &mut self,
        c: PrepareAcceptActionRequest,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::PrepareAcceptActionRequest(identity),
            |s| s.actions.prepare_accept_action_request(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn approve_and_execute_accept_action_request(
        &mut self,
        c: ApproveAndExecuteAcceptActionRequest,
    ) -> Result<AcceptedActionOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::ExecuteAcceptActionRequest(identity),
            |s| s.actions.approve_and_execute_accept_action_request(c),
        )
    }
    pub fn prepare_complete_action(
        &mut self,
        c: PrepareCompleteAction,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::PrepareCompleteAction(identity),
            |s| s.actions.prepare_complete_action(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn prepare_cancel_action(
        &mut self,
        c: PrepareCancelAction,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::PrepareCancelAction(identity),
            |s| s.actions.prepare_cancel_action(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn prepare_reopen_action(
        &mut self,
        c: PrepareReopenAction,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::PrepareReopenAction(identity),
            |s| s.actions.prepare_reopen_action(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn approve_and_execute_complete_action(
        &mut self,
        c: ApproveAndExecuteCompleteAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::ExecuteCompleteAction(identity),
            |s| s.actions.approve_and_execute_complete_action(c),
        )
    }
    pub fn approve_and_execute_cancel_action(
        &mut self,
        c: ApproveAndExecuteCancelAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::ExecuteCancelAction(identity),
            |s| s.actions.approve_and_execute_cancel_action(c),
        )
    }
    pub fn approve_and_execute_reopen_action(
        &mut self,
        c: ApproveAndExecuteReopenAction,
    ) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Action,
            WorkIdentity::ExecuteReopenAction(identity),
            |s| s.actions.approve_and_execute_reopen_action(c),
        )
    }

    pub fn create_decision_request_draft(
        &mut self,
        c: CreateDecisionRequestDraft,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkOperation::CreateDecisionRequestDraft,
            |s| s.decisions.create_decision_request_draft(c),
        )
    }
    pub fn submit_decision_request(
        &mut self,
        c: SubmitDecisionRequest,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkOperation::SubmitDecisionRequest,
            |s| s.decisions.submit_decision_request(c),
        )
    }
    pub fn withdraw_decision_request(
        &mut self,
        c: WithdrawDecisionRequest,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkOperation::WithdrawDecisionRequest,
            |s| s.decisions.withdraw_decision_request(c),
        )
    }
    pub fn prepare_resolve_decision_request(
        &mut self,
        c: PrepareResolveDecisionRequest,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkIdentity::PrepareResolveDecisionRequest(identity),
            |s| s.decisions.prepare_resolve_decision_request(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn prepare_supersede_decision(
        &mut self,
        c: PrepareSupersedeDecision,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkIdentity::PrepareSupersedeDecision(identity),
            |s| s.decisions.prepare_supersede_decision(c, &s.actions),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn approve_and_execute_resolve_decision_request(
        &mut self,
        c: ApproveAndExecuteResolveDecisionRequest,
    ) -> Result<ResolvedDecisionOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkIdentity::ExecuteResolveDecisionRequest(identity),
            |s| {
                s.decisions
                    .approve_and_execute_resolve_decision_request(c, &mut s.actions)
            },
        )
    }
    pub fn approve_and_execute_supersede_decision(
        &mut self,
        c: ApproveAndExecuteSupersedeDecision,
    ) -> Result<SupersededDecisionOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Decision,
            WorkIdentity::ExecuteSupersedeDecision(identity),
            |s| {
                s.decisions
                    .approve_and_execute_supersede_decision(c, &mut s.actions)
            },
        )
    }

    pub fn create_risk(
        &mut self,
        c: CreateRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Risk,
            WorkOperation::CreateRisk,
            |s| s.risk_issue_runtime.create_risk(c),
        )
    }
    pub fn update_risk_response(
        &mut self,
        c: UpdateRiskResponse,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Risk,
            WorkOperation::UpdateRiskResponse,
            |s| s.risk_issue_runtime.update_risk_response(c),
        )
    }
    pub fn prepare_record_risk_occurrence(
        &mut self,
        c: PrepareRecordRiskOccurrence,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Risk,
            WorkIdentity::PrepareRiskOccurrence(identity),
            |s| s.risk_issue_runtime.prepare_record_risk_occurrence(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn prepare_close_risk(
        &mut self,
        c: PrepareCloseRisk,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Risk,
            WorkIdentity::PrepareCloseRisk(identity),
            |s| s.risk_issue_runtime.prepare_close_risk(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn approve_and_execute_record_risk_occurrence(
        &mut self,
        c: ApproveAndExecuteRecordRiskOccurrence,
    ) -> Result<OccurredRiskOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Risk,
            WorkIdentity::ExecuteRiskOccurrence(identity),
            |s| {
                s.risk_issue_runtime
                    .approve_and_execute_record_risk_occurrence(c)
            },
        )
    }
    pub fn approve_and_execute_close_risk(
        &mut self,
        c: ApproveAndExecuteCloseRisk,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Risk,
            WorkIdentity::ExecuteCloseRisk(identity),
            |s| s.risk_issue_runtime.approve_and_execute_close_risk(c),
        )
    }
    pub fn risk_reenters_queue(
        &self,
        id: &RiskId,
        now: pmc_domain::time::UtcTimestamp,
        exposure_increased: bool,
        control_invalid: bool,
    ) -> Result<bool, DomainError> {
        self.risk_issue_runtime
            .risk_reenters_queue(id, now, exposure_increased, control_invalid)
    }

    pub fn create_issue(&mut self, c: CreateIssue) -> Result<IssueMutationOutcome, DomainError> {
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkOperation::CreateIssue,
            |s| s.risk_issue_runtime.create_issue(c),
        )
    }
    pub fn prepare_resolve_issue(
        &mut self,
        c: PrepareResolveIssue,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkIdentity::PrepareResolveIssue(identity),
            |s| s.risk_issue_runtime.prepare_resolve_issue(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn prepare_close_issue(
        &mut self,
        c: PrepareCloseIssue,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkIdentity::PrepareCloseIssue(identity),
            |s| s.risk_issue_runtime.prepare_close_issue(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn prepare_reopen_issue(
        &mut self,
        c: PrepareReopenIssue,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        let x = c.context.clone();
        let key = x.idempotency_id.clone();
        let prepared = self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkIdentity::PrepareReopenIssue(identity),
            |s| s.risk_issue_runtime.prepare_reopen_issue(c),
        )?;
        self.track_prepared(key, &prepared);
        Ok(prepared)
    }
    pub fn approve_and_execute_resolve_issue(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkIdentity::ExecuteResolveIssue(identity),
            |s| s.risk_issue_runtime.approve_and_execute_resolve_issue(c),
        )
    }
    pub fn approve_and_execute_close_issue(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkIdentity::ExecuteCloseIssue(identity),
            |s| s.risk_issue_runtime.approve_and_execute_close_issue(c),
        )
    }
    pub fn approve_and_execute_reopen_issue(
        &mut self,
        c: ApproveAndExecuteIssueTransition,
    ) -> Result<IssueMutationOutcome, DomainError> {
        let mut identity = c.clone();
        identity.context.correlation_id = Self::identity_correlation();
        identity.context.idempotency_id = Self::identity_idempotency();
        identity.approval = Self::identity_approval(&identity.approval);
        let x = c.context.clone();
        self.work_stage(
            &x.correlation_id,
            x.idempotency_id,
            WorkNamespace::Issue,
            WorkIdentity::ExecuteReopenIssue(identity),
            |s| s.risk_issue_runtime.approve_and_execute_reopen_issue(c),
        )
    }
}
