#![allow(clippy::result_large_err)]

use crate::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
    AuditEffectScope, AuditEvent, AuditEventCode, AuditEventIdSource, AuditExecutionOutcome,
    AuditModule, AuditPolicyOutcome, AuditTarget,
};
use crate::classification::DataClassification;
use crate::error::{DomainError, ErrorCode, MessageKey};
use crate::execution::{
    ApprovalAuthorizationPort, ApproveAndExecuteRemoveRelationship, CancellationPolicy,
    DenyApprovalAuthorization, DenyRelationshipRemoval, ExecutionIdSource, PayloadDigest,
    PrepareRemoveRelationship, PreparedIntent, RecoveryEvidencePort, RemovalEffect, RemovalOutcome,
    RemovalPolicyDecision, RemovalPolicyPort, RemoveRelationshipPreview,
    SequentialExecutionIdSource, REMOVE_RELATIONSHIP_INTENT_TYPE,
    REMOVE_RELATIONSHIP_INTENT_VERSION,
};
use crate::identity::{
    AggregateVersion, AuditEventId, CorrelationId, IdempotencyId, InitiativeId, KpiId, MilestoneId,
    PortfolioId, ProductId, ProjectId, RelationshipId, RoadmapId, StakeholderId,
};
use crate::provenance::Provenance;
use crate::time::{Clock, UtcTimestamp};
use crate::value::BoundedText;
use crate::{DomainValueError, ValueErrorKind};
use std::collections::{HashMap, HashSet};

pub type StakeholderName = BoundedText<128>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StakeholderKind {
    Person,
    Organization,
}
impl StakeholderKind {
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "person" => Ok(Self::Person),
            "organization" => Ok(Self::Organization),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderRecord {
    id: StakeholderId,
    name: StakeholderName,
    kind: StakeholderKind,
    classification: DataClassification,
    provenance: Provenance,
    version: AggregateVersion,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}
impl StakeholderRecord {
    #[doc(hidden)]
    pub fn rehydrate(
        value: StakeholderPersistenceRecord,
    ) -> Result<Self, RelationshipPersistenceError> {
        if value.created_at > value.updated_at {
            return Err(RelationshipPersistenceError::InvalidTimestampOrder);
        }
        Ok(Self {
            id: value.id,
            name: value.name,
            kind: value.kind,
            classification: value.classification,
            provenance: value.provenance,
            version: value.version,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
    pub fn id(&self) -> &StakeholderId {
        &self.id
    }
    pub fn name(&self) -> &StakeholderName {
        &self.name
    }
    pub const fn kind(&self) -> StakeholderKind {
        self.kind
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
    pub const fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }
    pub const fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

/// Adapter-only, domain-typed Stakeholder row. It contains no SQL or generic
/// serialized payload and is accepted only through [`StakeholderRecord::rehydrate`].
#[doc(hidden)]
pub struct StakeholderPersistenceRecord {
    pub id: StakeholderId,
    pub name: StakeholderName,
    pub kind: StakeholderKind,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateStakeholder {
    pub id: StakeholderId,
    pub name: StakeholderName,
    pub kind: StakeholderKind,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
    pub context: OperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateStakeholderDetails {
    pub id: StakeholderId,
    pub expected_version: AggregateVersion,
    pub name: StakeholderName,
    pub classification: Option<DataClassification>,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtomicOutcome {
    audit_event_ids: Vec<AuditEventId>,
    effect_scope: AuditEffectScope,
}
impl AtomicOutcome {
    pub fn audit_event_ids(&self) -> &[AuditEventId] {
        &self.audit_event_ids
    }
    pub const fn effect_scope(&self) -> AuditEffectScope {
        self.effect_scope
    }
    pub const fn execution_outcome(&self) -> AuditExecutionOutcome {
        AuditExecutionOutcome::Succeeded
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationOutcome<T> {
    value: T,
    outcome: AtomicOutcome,
}
impl<T> MutationOutcome<T> {
    /// Rebuilds one typed candidate outcome. This constructor does not grant
    /// authority by itself: the owning persistence snapshot validator checks
    /// audit cardinality, order, correlation, disposition, and command/result
    /// topology before rehydration can expose the value.
    #[doc(hidden)]
    pub fn from_persistence(
        value: T,
        audit_event_ids: Vec<AuditEventId>,
        effect_scope: AuditEffectScope,
    ) -> Self {
        Self {
            value,
            outcome: AtomicOutcome {
                audit_event_ids,
                effect_scope,
            },
        }
    }
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn into_value(self) -> T {
        self.value
    }
    pub const fn outcome(&self) -> &AtomicOutcome {
        &self.outcome
    }
}
impl<T: Clone> MutationOutcome<T> {
    pub fn cloned_value(&self) -> T {
        self.value.clone()
    }
}
impl<T: PartialEq> PartialEq<T> for MutationOutcome<T> {
    fn eq(&self, other: &T) -> bool {
        &self.value == other
    }
}

macro_rules! snapshot {
    ($name:ident, $id:ty) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            id: $id,
            version: AggregateVersion,
            classification: DataClassification,
        }
        impl $name {
            pub fn new(
                id: $id,
                version: AggregateVersion,
                classification: DataClassification,
            ) -> Self {
                Self {
                    id,
                    version,
                    classification,
                }
            }
            pub fn id(&self) -> &$id {
                &self.id
            }
            pub const fn version(&self) -> AggregateVersion {
                self.version
            }
            pub const fn classification(&self) -> DataClassification {
                self.classification
            }
        }
    };
}
snapshot!(PortfolioSnapshot, PortfolioId);
snapshot!(ProductSnapshot, ProductId);
snapshot!(InitiativeSnapshot, InitiativeId);
snapshot!(RoadmapSnapshot, RoadmapId);
snapshot!(KpiSnapshot, KpiId);
snapshot!(ProjectSnapshot, ProjectId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneSnapshot {
    id: MilestoneId,
    project_id: ProjectId,
    version: AggregateVersion,
    classification: DataClassification,
}
impl MilestoneSnapshot {
    pub fn new(
        id: MilestoneId,
        project_id: ProjectId,
        version: AggregateVersion,
        classification: DataClassification,
    ) -> Self {
        Self {
            id,
            project_id,
            version,
            classification,
        }
    }
    pub fn id(&self) -> &MilestoneId {
        &self.id
    }
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderSnapshot {
    id: StakeholderId,
    version: AggregateVersion,
    classification: DataClassification,
}
impl StakeholderSnapshot {
    pub fn new(
        id: StakeholderId,
        version: AggregateVersion,
        classification: DataClassification,
    ) -> Self {
        Self {
            id,
            version,
            classification,
        }
    }
    pub fn id(&self) -> &StakeholderId {
        &self.id
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EndpointSnapshot {
    Portfolio(PortfolioSnapshot),
    Product(ProductSnapshot),
    Initiative(InitiativeSnapshot),
    Roadmap(RoadmapSnapshot),
    Kpi(KpiSnapshot),
    Project(ProjectSnapshot),
    Milestone(MilestoneSnapshot),
    Stakeholder(StakeholderSnapshot),
}
impl EndpointSnapshot {
    /// Canonical, ordered encoding of every endpoint field exposed by a removal preview.
    pub(crate) fn removal_digest_fields(&self) -> [String; 5] {
        let (kind, id, version, classification, parent) = match self {
            Self::Portfolio(v) => (
                "portfolio",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
            Self::Product(v) => (
                "product",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
            Self::Initiative(v) => (
                "initiative",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
            Self::Roadmap(v) => (
                "roadmap",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
            Self::Kpi(v) => (
                "kpi",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
            Self::Project(v) => (
                "project",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
            Self::Milestone(v) => (
                "milestone",
                v.id().to_string(),
                v.version(),
                v.classification(),
                v.project_id().to_string(),
            ),
            Self::Stakeholder(v) => (
                "stakeholder",
                v.id().to_string(),
                v.version(),
                v.classification(),
                String::new(),
            ),
        };
        [
            kind.to_owned(),
            id,
            version.get().to_string(),
            classification.as_persisted().to_owned(),
            parent,
        ]
    }
    fn key(&self) -> EndpointKey {
        match self {
            Self::Portfolio(v) => EndpointKey::Portfolio(v.id.clone()),
            Self::Product(v) => EndpointKey::Product(v.id.clone()),
            Self::Initiative(v) => EndpointKey::Initiative(v.id.clone()),
            Self::Roadmap(v) => EndpointKey::Roadmap(v.id.clone()),
            Self::Kpi(v) => EndpointKey::Kpi(v.id.clone()),
            Self::Project(v) => EndpointKey::Project(v.id.clone()),
            Self::Milestone(v) => EndpointKey::Milestone(v.id.clone()),
            Self::Stakeholder(v) => EndpointKey::Stakeholder(v.id.clone()),
        }
    }
    fn version(&self) -> AggregateVersion {
        match self {
            Self::Portfolio(v) => v.version,
            Self::Product(v) => v.version,
            Self::Initiative(v) => v.version,
            Self::Roadmap(v) => v.version,
            Self::Kpi(v) => v.version,
            Self::Project(v) => v.version,
            Self::Milestone(v) => v.version,
            Self::Stakeholder(v) => v.version,
        }
    }
    fn classification(&self) -> DataClassification {
        match self {
            Self::Portfolio(v) => v.classification,
            Self::Product(v) => v.classification,
            Self::Initiative(v) => v.classification,
            Self::Roadmap(v) => v.classification,
            Self::Kpi(v) => v.classification,
            Self::Project(v) => v.classification,
            Self::Milestone(v) => v.classification,
            Self::Stakeholder(v) => v.classification,
        }
    }
}

/// Trusted read-only endpoint authority supplied by the Product Ledger adapter.
///
/// Implementations must return the current authoritative snapshot for exactly
/// the requested ID. They return `None` instead of fabricating, guessing, or
/// returning a stale endpoint. `Unclassified` means inheritance is unknown and
/// link creation therefore fails closed. Production implementations must source
/// versions and classifications from Product Ledger authority; public snapshot
/// constructors exist for adapters and synthetic tests, not command callers.
pub trait EndpointCatalog: Clone {
    fn portfolio(&self, id: &PortfolioId) -> Option<PortfolioSnapshot>;
    fn product(&self, id: &ProductId) -> Option<ProductSnapshot>;
    fn initiative(&self, id: &InitiativeId) -> Option<InitiativeSnapshot>;
    fn roadmap(&self, id: &RoadmapId) -> Option<RoadmapSnapshot>;
    fn kpi(&self, id: &KpiId) -> Option<KpiSnapshot>;
    fn project(&self, id: &ProjectId) -> Option<ProjectSnapshot>;
    fn milestone(&self, id: &MilestoneId) -> Option<MilestoneSnapshot>;
}

#[derive(Clone, Default)]
pub struct InMemoryEndpointCatalog {
    endpoints: HashMap<EndpointKey, EndpointSnapshot>,
}
impl InMemoryEndpointCatalog {
    pub fn new(endpoints: impl IntoIterator<Item = EndpointSnapshot>) -> Self {
        Self {
            endpoints: endpoints.into_iter().map(|v| (v.key(), v)).collect(),
        }
    }
}
impl EndpointCatalog for InMemoryEndpointCatalog {
    fn portfolio(&self, id: &PortfolioId) -> Option<PortfolioSnapshot> {
        match self.endpoints.get(&EndpointKey::Portfolio(id.clone())) {
            Some(EndpointSnapshot::Portfolio(v)) => Some(v.clone()),
            _ => None,
        }
    }
    fn product(&self, id: &ProductId) -> Option<ProductSnapshot> {
        match self.endpoints.get(&EndpointKey::Product(id.clone())) {
            Some(EndpointSnapshot::Product(v)) => Some(v.clone()),
            _ => None,
        }
    }
    fn initiative(&self, id: &InitiativeId) -> Option<InitiativeSnapshot> {
        match self.endpoints.get(&EndpointKey::Initiative(id.clone())) {
            Some(EndpointSnapshot::Initiative(v)) => Some(v.clone()),
            _ => None,
        }
    }
    fn roadmap(&self, id: &RoadmapId) -> Option<RoadmapSnapshot> {
        match self.endpoints.get(&EndpointKey::Roadmap(id.clone())) {
            Some(EndpointSnapshot::Roadmap(v)) => Some(v.clone()),
            _ => None,
        }
    }
    fn kpi(&self, id: &KpiId) -> Option<KpiSnapshot> {
        match self.endpoints.get(&EndpointKey::Kpi(id.clone())) {
            Some(EndpointSnapshot::Kpi(v)) => Some(v.clone()),
            _ => None,
        }
    }
    fn project(&self, id: &ProjectId) -> Option<ProjectSnapshot> {
        match self.endpoints.get(&EndpointKey::Project(id.clone())) {
            Some(EndpointSnapshot::Project(v)) => Some(v.clone()),
            _ => None,
        }
    }
    fn milestone(&self, id: &MilestoneId) -> Option<MilestoneSnapshot> {
        match self.endpoints.get(&EndpointKey::Milestone(id.clone())) {
            Some(EndpointSnapshot::Milestone(v)) => Some(v.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StakeholderRelationshipPurpose {
    Responsibility,
    Dependency,
}
impl StakeholderRelationshipPurpose {
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::Responsibility => "responsibility",
            Self::Dependency => "dependency",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "responsibility" => Ok(Self::Responsibility),
            "dependency" => Ok(Self::Dependency),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RelationshipKind {
    PortfolioProduct,
    PortfolioInitiative,
    ProductRoadmap,
    ProductKpi,
    InitiativeProject,
    ProjectProduct,
    StakeholderSubject,
}
impl RelationshipKind {
    const fn removable(self) -> bool {
        matches!(
            self,
            Self::PortfolioProduct
                | Self::PortfolioInitiative
                | Self::ProductRoadmap
                | Self::ProductKpi
                | Self::InitiativeProject
                | Self::ProjectProduct
                | Self::StakeholderSubject
        )
    }
    pub const fn as_persisted(self) -> &'static str {
        match self {
            Self::PortfolioProduct => "portfolio_product",
            Self::PortfolioInitiative => "portfolio_initiative",
            Self::ProductRoadmap => "product_roadmap",
            Self::ProductKpi => "product_kpi",
            Self::InitiativeProject => "initiative_project",
            Self::ProjectProduct => "project_product",
            Self::StakeholderSubject => "stakeholder_subject",
        }
    }

    pub fn from_persisted(value: &str) -> Result<Self, DomainValueError> {
        match value {
            "portfolio_product" => Ok(Self::PortfolioProduct),
            "portfolio_initiative" => Ok(Self::PortfolioInitiative),
            "product_roadmap" => Ok(Self::ProductRoadmap),
            "product_kpi" => Ok(Self::ProductKpi),
            "initiative_project" => Ok(Self::InitiativeProject),
            "project_product" => Ok(Self::ProjectProduct),
            "stakeholder_subject" => Ok(Self::StakeholderSubject),
            _ => Err(DomainValueError::new(ValueErrorKind::UnknownPersistedValue)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipRecord {
    id: RelationshipId,
    kind: RelationshipKind,
    endpoints: Vec<EndpointSnapshot>,
    purpose: Option<StakeholderRelationshipPurpose>,
    classification: DataClassification,
    version: AggregateVersion,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}
impl RelationshipRecord {
    #[doc(hidden)]
    pub fn rehydrate(
        value: RelationshipPersistenceRecord,
    ) -> Result<Self, RelationshipPersistenceError> {
        let calculated = value
            .endpoints
            .iter()
            .fold(DataClassification::Public, |current, endpoint| {
                current.combine(endpoint.classification())
            });
        if value.created_at > value.updated_at {
            return Err(RelationshipPersistenceError::InvalidTimestampOrder);
        }
        if value.endpoints.len() != 2
            || calculated != value.classification
            || value
                .endpoints
                .iter()
                .any(|endpoint| endpoint.classification() == DataClassification::Unclassified)
            || !valid_relationship_topology(value.kind, &value.endpoints, value.purpose)
        {
            return Err(RelationshipPersistenceError::EndpointMismatch);
        }
        Ok(Self {
            id: value.id,
            kind: value.kind,
            endpoints: value.endpoints,
            purpose: value.purpose,
            classification: value.classification,
            version: value.version,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
    pub fn id(&self) -> &RelationshipId {
        &self.id
    }
    pub const fn kind(&self) -> RelationshipKind {
        self.kind
    }
    pub fn endpoints(&self) -> &[EndpointSnapshot] {
        &self.endpoints
    }
    pub const fn purpose(&self) -> Option<StakeholderRelationshipPurpose> {
        self.purpose
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn version(&self) -> AggregateVersion {
        self.version
    }
    pub const fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }
    pub const fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }
}

/// Adapter-only, domain-typed Relationship row. Endpoint topology and
/// inherited classification are revalidated before it can become authority.
#[doc(hidden)]
pub struct RelationshipPersistenceRecord {
    pub id: RelationshipId,
    pub kind: RelationshipKind,
    pub endpoints: Vec<EndpointSnapshot>,
    pub purpose: Option<StakeholderRelationshipPurpose>,
    pub classification: DataClassification,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

macro_rules! link_command {
    ($name:ident { $($field:ident : $type:ty),+ $(,)? }, $($expected:ident),+) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            pub id: RelationshipId,
            $(pub $field: $type,)+
            $(pub $expected: AggregateVersion,)+
            pub context: OperationContext,
        }
    };
}
link_command!(
    LinkPortfolioProduct {
        portfolio_id: PortfolioId,
        product_id: ProductId
    },
    expected_portfolio_version,
    expected_product_version
);
link_command!(
    LinkPortfolioInitiative {
        portfolio_id: PortfolioId,
        initiative_id: InitiativeId
    },
    expected_portfolio_version,
    expected_initiative_version
);
link_command!(
    LinkProductRoadmap {
        product_id: ProductId,
        roadmap_id: RoadmapId
    },
    expected_product_version,
    expected_roadmap_version
);
link_command!(
    LinkProductKpi {
        product_id: ProductId,
        kpi_id: KpiId
    },
    expected_product_version,
    expected_kpi_version
);
link_command!(
    LinkInitiativeProject {
        initiative_id: InitiativeId,
        project_id: ProjectId
    },
    expected_initiative_version,
    expected_project_version
);
link_command!(
    LinkProjectProduct {
        project_id: ProjectId,
        product_id: ProductId
    },
    expected_project_version,
    expected_product_version
);
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidateProjectMilestone {
    pub project_id: ProjectId,
    pub milestone_id: MilestoneId,
    pub expected_project_version: AggregateVersion,
    pub expected_milestone_version: AggregateVersion,
    pub context: OperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkStakeholderRelationship {
    pub id: RelationshipId,
    pub stakeholder_id: StakeholderId,
    pub subject: StakeholderSubject,
    pub purpose: StakeholderRelationshipPurpose,
    pub expected_stakeholder_version: AggregateVersion,
    pub expected_subject_version: AggregateVersion,
    pub context: OperationContext,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StakeholderSubject {
    Portfolio(PortfolioId),
    Product(ProductId),
    Initiative(InitiativeId),
    Project(ProjectId),
    Roadmap(RoadmapId),
    Milestone(MilestoneId),
    Kpi(KpiId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectMilestoneValidation {
    project_id: ProjectId,
    milestone_id: MilestoneId,
    classification: DataClassification,
    outcome: AtomicOutcome,
}
impl ProjectMilestoneValidation {
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    pub fn milestone_id(&self) -> &MilestoneId {
        &self.milestone_id
    }
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }
    pub const fn outcome(&self) -> &AtomicOutcome {
        &self.outcome
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum EndpointKey {
    Portfolio(PortfolioId),
    Product(ProductId),
    Initiative(InitiativeId),
    Roadmap(RoadmapId),
    Kpi(KpiId),
    Project(ProjectId),
    Milestone(MilestoneId),
    Stakeholder(StakeholderId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum Identity {
    CreateStakeholder(
        StakeholderId,
        StakeholderName,
        StakeholderKind,
        Option<DataClassification>,
        Provenance,
    ),
    UpdateStakeholder(
        StakeholderId,
        AggregateVersion,
        StakeholderName,
        Option<DataClassification>,
    ),
    Link(
        RelationshipId,
        RelationshipKind,
        Vec<EndpointKey>,
        Option<StakeholderRelationshipPurpose>,
        Vec<AggregateVersion>,
    ),
    Remove(
        crate::identity::PreparedIntentId,
        PayloadDigest,
        PayloadDigest,
        AuditActor,
    ),
    PrepareRemove(RelationshipId),
    CancelRemove(crate::identity::PreparedIntentId, AuditActor),
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum Stored {
    Stakeholder(MutationOutcome<StakeholderRecord>),
    Relationship(MutationOutcome<RelationshipRecord>),
    Removal(RemovalOutcome),
    Rejection(DomainError),
    Prepared(PreparedIntent),
    Cancelled,
    Tombstone,
}

/// Domain-typed, row-shape-independent command retained for exact durable replay.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationshipPersistenceCommand {
    CreateStakeholder {
        id: StakeholderId,
        name: StakeholderName,
        kind: StakeholderKind,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateStakeholder {
        id: StakeholderId,
        expected_version: AggregateVersion,
        name: StakeholderName,
        classification: Option<DataClassification>,
    },
    Link {
        id: RelationshipId,
        kind: RelationshipKind,
        endpoints: Vec<EndpointSnapshot>,
        expected_versions: Vec<AggregateVersion>,
        purpose: Option<StakeholderRelationshipPurpose>,
    },
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationshipPersistenceResult {
    Stakeholder(MutationOutcome<StakeholderRecord>),
    Relationship(MutationOutcome<RelationshipRecord>),
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipReplayCapsule {
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    operation_ordinal: u64,
    command: RelationshipPersistenceCommand,
    result: RelationshipPersistenceResult,
    audit_event_ids: Vec<AuditEventId>,
}
impl RelationshipReplayCapsule {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        idempotency_id: IdempotencyId,
        correlation_id: CorrelationId,
        operation_ordinal: u64,
        command: RelationshipPersistenceCommand,
        result: RelationshipPersistenceResult,
        audit_event_ids: Vec<AuditEventId>,
    ) -> Self {
        Self {
            idempotency_id,
            correlation_id,
            operation_ordinal,
            command,
            result,
            audit_event_ids,
        }
    }
    pub fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }
    pub const fn operation_ordinal(&self) -> u64 {
        self.operation_ordinal
    }
    pub const fn command(&self) -> &RelationshipPersistenceCommand {
        &self.command
    }
    pub const fn result(&self) -> &RelationshipPersistenceResult {
        &self.result
    }
    pub fn audit_event_ids(&self) -> &[AuditEventId] {
        &self.audit_event_ids
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationshipPersistenceError {
    DuplicateIdentity,
    InvalidOperationOrder,
    InvalidCommandResult,
    AuditMismatch,
    EndpointMismatch,
    UnsupportedRemovalState,
    InvalidRemovalState,
    InvalidTimestampOrder,
}

/// Exact, domain-typed H2b command identity retained for durable replay.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationshipH2bPersistenceCommand {
    Prepare {
        relationship_id: RelationshipId,
    },
    Cancel {
        prepared_id: crate::identity::PreparedIntentId,
        actor: AuditActor,
    },
    Execute {
        prepared_id: crate::identity::PreparedIntentId,
        acknowledged_payload_digest: PayloadDigest,
        confirmation_digest: PayloadDigest,
        actor: AuditActor,
    },
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationshipH2bPersistenceResult {
    Prepared(PreparedIntent),
    Cancelled,
    Removal(RemovalOutcome),
    Rejection(DomainError),
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipH2bReplayCapsule {
    idempotency_id: IdempotencyId,
    correlation_id: CorrelationId,
    operation_ordinal: u64,
    command: RelationshipH2bPersistenceCommand,
    result: RelationshipH2bPersistenceResult,
    audit_event_ids: Vec<AuditEventId>,
}

impl RelationshipH2bReplayCapsule {
    pub fn new(
        idempotency_id: IdempotencyId,
        correlation_id: CorrelationId,
        operation_ordinal: u64,
        command: RelationshipH2bPersistenceCommand,
        result: RelationshipH2bPersistenceResult,
        audit_event_ids: Vec<AuditEventId>,
    ) -> Self {
        Self {
            idempotency_id,
            correlation_id,
            operation_ordinal,
            command,
            result,
            audit_event_ids,
        }
    }
    pub fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }

    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    pub const fn operation_ordinal(&self) -> u64 {
        self.operation_ordinal
    }

    pub const fn command(&self) -> &RelationshipH2bPersistenceCommand {
        &self.command
    }

    pub const fn result(&self) -> &RelationshipH2bPersistenceResult {
        &self.result
    }

    pub fn audit_event_ids(&self) -> &[AuditEventId] {
        &self.audit_event_ids
    }
}

/// Complete Relationship authority snapshot including destructive H2b state.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipH2bPersistenceSnapshot {
    stakeholders: Vec<StakeholderRecord>,
    relationships: Vec<RelationshipRecord>,
    ordinary_history_relationships: Vec<RelationshipRecord>,
    ordinary_replay: Vec<RelationshipReplayCapsule>,
    h2b_replay: Vec<RelationshipH2bReplayCapsule>,
    audits: Vec<AuditEvent>,
    pending: Vec<PreparedIntent>,
    completed: Vec<crate::identity::PreparedIntentId>,
    tombstoned_ordinary: Vec<IdempotencyId>,
    tombstoned_h2b: Vec<IdempotencyId>,
}

impl RelationshipH2bPersistenceSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn from_persistence(
        stakeholders: Vec<StakeholderRecord>,
        relationships: Vec<RelationshipRecord>,
        ordinary_history_relationships: Vec<RelationshipRecord>,
        ordinary_replay: Vec<RelationshipReplayCapsule>,
        h2b_replay: Vec<RelationshipH2bReplayCapsule>,
        audits: Vec<AuditEvent>,
        pending: Vec<PreparedIntent>,
        completed: Vec<crate::identity::PreparedIntentId>,
        tombstoned_ordinary: Vec<IdempotencyId>,
        tombstoned_h2b: Vec<IdempotencyId>,
    ) -> Result<Self, RelationshipPersistenceError> {
        Self {
            stakeholders,
            relationships,
            ordinary_history_relationships,
            ordinary_replay,
            h2b_replay,
            audits,
            pending,
            completed,
            tombstoned_ordinary,
            tombstoned_h2b,
        }
        .validate()
    }
    pub fn validate(self) -> Result<Self, RelationshipPersistenceError> {
        validate_h2b_snapshot(&self)?;
        Ok(self)
    }

    pub fn stakeholders(&self) -> &[StakeholderRecord] {
        &self.stakeholders
    }

    pub fn relationships(&self) -> &[RelationshipRecord] {
        &self.relationships
    }

    pub fn ordinary_history_relationships(&self) -> &[RelationshipRecord] {
        &self.ordinary_history_relationships
    }

    pub fn ordinary_replay(&self) -> &[RelationshipReplayCapsule] {
        &self.ordinary_replay
    }

    pub fn h2b_replay(&self) -> &[RelationshipH2bReplayCapsule] {
        &self.h2b_replay
    }

    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }

    pub fn pending(&self) -> &[PreparedIntent] {
        &self.pending
    }

    pub fn completed(&self) -> &[crate::identity::PreparedIntentId] {
        &self.completed
    }

    pub fn tombstoned_ordinary(&self) -> &[IdempotencyId] {
        &self.tombstoned_ordinary
    }

    pub fn tombstoned_h2b(&self) -> &[IdempotencyId] {
        &self.tombstoned_h2b
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationshipPersistenceSnapshot {
    stakeholders: Vec<StakeholderRecord>,
    relationships: Vec<RelationshipRecord>,
    replay: Vec<RelationshipReplayCapsule>,
    audits: Vec<AuditEvent>,
}
impl RelationshipPersistenceSnapshot {
    pub fn stakeholders(&self) -> &[StakeholderRecord] {
        &self.stakeholders
    }
    pub fn relationships(&self) -> &[RelationshipRecord] {
        &self.relationships
    }
    pub fn replay(&self) -> &[RelationshipReplayCapsule] {
        &self.replay
    }
    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }

    pub fn validate(
        mut stakeholders: Vec<StakeholderRecord>,
        mut relationships: Vec<RelationshipRecord>,
        mut replay: Vec<RelationshipReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, RelationshipPersistenceError> {
        stakeholders.sort_by(|a, b| a.id.cmp(&b.id));
        relationships.sort_by(|a, b| a.id.cmp(&b.id));
        if stakeholders.windows(2).any(|v| v[0].id == v[1].id)
            || relationships.windows(2).any(|v| v[0].id == v[1].id)
        {
            return Err(RelationshipPersistenceError::DuplicateIdentity);
        }
        replay.sort_by_key(|v| v.operation_ordinal);
        if replay
            .iter()
            .enumerate()
            .any(|(index, value)| value.operation_ordinal != index as u64 + 1)
            || replay
                .iter()
                .map(|v| &v.idempotency_id)
                .collect::<HashSet<_>>()
                .len()
                != replay.len()
        {
            return Err(RelationshipPersistenceError::InvalidOperationOrder);
        }
        let declared_audits = replay
            .iter()
            .flat_map(|v| v.audit_event_ids.iter())
            .collect::<Vec<_>>();
        let actual_audits = audits.iter().map(AuditEvent::id).collect::<Vec<_>>();
        if declared_audits != actual_audits
            || actual_audits.iter().collect::<HashSet<_>>().len() != actual_audits.len()
        {
            return Err(RelationshipPersistenceError::AuditMismatch);
        }
        for capsule in &replay {
            if capsule.audit_event_ids != result_audit_ids(&capsule.result)
                || !command_matches_result(&capsule.command, &capsule.result)
            {
                return Err(RelationshipPersistenceError::InvalidCommandResult);
            }
            for audit_id in &capsule.audit_event_ids {
                let audit = audits
                    .iter()
                    .find(|value| value.id() == audit_id)
                    .ok_or(RelationshipPersistenceError::AuditMismatch)?;
                if audit.correlation_id() != &capsule.correlation_id
                    || audit.actor() != AuditActor::HeadOfProducts
                    || audit.module() != AuditModule::Portfolio
                    || audit.policy_outcome() != AuditPolicyOutcome::NotRequired
                    || audit.approval_outcome() != AuditApprovalOutcome::NotRequired
                    || audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                    || audit.effect_scope() != AuditEffectScope::Complete
                    || audit.actual_effects().len() != 1
                    || audit.actual_effects()[0].as_str()
                        != "relationship.authoritative-record-changed"
                {
                    return Err(RelationshipPersistenceError::AuditMismatch);
                }
            }
        }
        for relationship in &relationships {
            let calculated = relationship
                .endpoints
                .iter()
                .fold(DataClassification::Public, |a, v| {
                    a.combine(v.classification())
                });
            if relationship.endpoints.len() != 2
                || calculated != relationship.classification
                || relationship
                    .endpoints
                    .iter()
                    .any(|endpoint| endpoint.classification() == DataClassification::Unclassified)
                || !valid_relationship_topology(
                    relationship.kind,
                    &relationship.endpoints,
                    relationship.purpose,
                )
            {
                return Err(RelationshipPersistenceError::EndpointMismatch);
            }
        }
        if relationships
            .iter()
            .map(|relationship| SemanticKey {
                kind: relationship.kind,
                endpoints: relationship
                    .endpoints
                    .iter()
                    .map(EndpointSnapshot::key)
                    .collect(),
                purpose: relationship.purpose,
            })
            .collect::<HashSet<_>>()
            .len()
            != relationships.len()
        {
            return Err(RelationshipPersistenceError::DuplicateIdentity);
        }
        let mut final_stakeholders = HashMap::new();
        let mut final_relationships: HashMap<RelationshipId, RelationshipRecord> = HashMap::new();
        for capsule in &replay {
            match (&capsule.command, &capsule.result) {
                (
                    RelationshipPersistenceCommand::CreateStakeholder { .. },
                    RelationshipPersistenceResult::Stakeholder(value),
                ) => {
                    if capsule.audit_event_ids.len() != 1
                        || audits
                            .iter()
                            .find(|audit| audit.id() == &capsule.audit_event_ids[0])
                            .is_none_or(|audit| {
                                audit.code().as_str() != "relationship.stakeholder.created"
                                    || audit.target()
                                        != &AuditTarget::Stakeholder(value.value.id.clone())
                                    || audit.occurred_at() != value.value.created_at
                            })
                    {
                        return Err(RelationshipPersistenceError::AuditMismatch);
                    }
                    final_stakeholders.insert(value.value.id.clone(), value.value.clone());
                }
                (
                    RelationshipPersistenceCommand::UpdateStakeholder { id, .. },
                    RelationshipPersistenceResult::Stakeholder(value),
                ) => {
                    let previous = final_stakeholders
                        .get(id)
                        .ok_or(RelationshipPersistenceError::InvalidCommandResult)?;
                    if previous.version.next() != Some(value.value.version)
                        || previous.classification.combine(value.value.classification)
                            != value.value.classification
                        || matches!(
                            &capsule.command,
                            RelationshipPersistenceCommand::UpdateStakeholder {
                                classification: None,
                                ..
                            }
                        ) && previous.classification != value.value.classification
                        || previous.kind != value.value.kind
                        || previous.provenance != value.value.provenance
                        || previous.created_at != value.value.created_at
                    {
                        return Err(RelationshipPersistenceError::InvalidCommandResult);
                    }
                    let mut affected_ids = if previous.classification != value.value.classification
                    {
                        final_relationships.values().filter(|relationship| {
                            relationship.endpoints.iter().any(|endpoint| matches!(endpoint, EndpointSnapshot::Stakeholder(snapshot) if snapshot.id() == id))
                        }).map(|relationship| relationship.id.clone()).collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    affected_ids.sort();
                    for relationship_id in &affected_ids {
                        let relationship = final_relationships
                            .get_mut(relationship_id)
                            .ok_or(RelationshipPersistenceError::InvalidCommandResult)?;
                        relationship.endpoints = relationship
                            .endpoints
                            .iter()
                            .map(|endpoint| match endpoint {
                                EndpointSnapshot::Stakeholder(snapshot) if snapshot.id() == id => {
                                    EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                                        id.clone(),
                                        value.value.version,
                                        value.value.classification,
                                    ))
                                }
                                other => other.clone(),
                            })
                            .collect();
                        relationship.classification = relationship
                            .endpoints
                            .iter()
                            .fold(DataClassification::Public, |current, endpoint| {
                                current.combine(endpoint.classification())
                            });
                        relationship.version = relationship
                            .version
                            .next()
                            .ok_or(RelationshipPersistenceError::InvalidCommandResult)?;
                        relationship.updated_at = value.value.updated_at;
                    }
                    if capsule.audit_event_ids.len() != affected_ids.len() + 1
                        || capsule.audit_event_ids.iter().enumerate().any(|(index, audit_id)| {
                            audits.iter().find(|audit| audit.id() == audit_id).is_none_or(|audit| {
                                audit.target() != &AuditTarget::Stakeholder(id.clone())
                                    || audit.code().as_str() != if index == 0 { "relationship.stakeholder.updated" } else { "relationship.stakeholder_relationship.reclassified" }
                                    || audit.occurred_at() != value.value.updated_at
                            })
                        })
                    {
                        return Err(RelationshipPersistenceError::AuditMismatch);
                    }
                    final_stakeholders.insert(id.clone(), value.value.clone());
                }
                (
                    RelationshipPersistenceCommand::Link { .. },
                    RelationshipPersistenceResult::Relationship(value),
                ) if value.outcome.effect_scope == AuditEffectScope::Complete => {
                    if capsule.audit_event_ids.len() != 1
                        || audits
                            .iter()
                            .find(|audit| audit.id() == &capsule.audit_event_ids[0])
                            .is_none_or(|audit| {
                                audit.code().as_str() != relationship_code(value.value.kind)
                                    || Some(audit.target())
                                        != audit_target(&value.value.endpoints).as_ref()
                                    || audit.occurred_at() != value.value.created_at
                            })
                    {
                        return Err(RelationshipPersistenceError::AuditMismatch);
                    }
                    final_relationships.insert(value.value.id.clone(), value.value.clone());
                }
                (
                    RelationshipPersistenceCommand::Link { .. },
                    RelationshipPersistenceResult::Relationship(value),
                ) if value.outcome.effect_scope == AuditEffectScope::None
                    && capsule.audit_event_ids.is_empty() => {}
                _ => return Err(RelationshipPersistenceError::InvalidCommandResult),
            }
        }
        if stakeholders.len() != final_stakeholders.len()
            || relationships.len() != final_relationships.len()
            || stakeholders
                .iter()
                .any(|v| final_stakeholders.get(&v.id) != Some(v))
            || relationships
                .iter()
                .any(|v| final_relationships.get(&v.id) != Some(v))
        {
            return Err(RelationshipPersistenceError::InvalidCommandResult);
        }
        Ok(Self {
            stakeholders,
            relationships,
            replay,
            audits,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct EphemeralApprovalReceipt {
    id: crate::identity::ApprovalReceiptId,
    prepared_id: crate::identity::PreparedIntentId,
    actor: AuditActor,
    acknowledged_payload_digest: PayloadDigest,
    consumed: bool,
}
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SemanticKey {
    kind: RelationshipKind,
    endpoints: Vec<EndpointKey>,
    purpose: Option<StakeholderRelationshipPurpose>,
}
#[derive(Clone)]
struct State {
    stakeholders: HashMap<StakeholderId, StakeholderRecord>,
    stakeholder_endpoints: HashMap<StakeholderId, EndpointSnapshot>,
    relationships: HashMap<RelationshipId, RelationshipRecord>,
    ordinary_latest_relationships: HashMap<RelationshipId, RelationshipRecord>,
    semantic: HashMap<SemanticKey, RelationshipId>,
    idempotency: HashMap<IdempotencyId, (Identity, Stored)>,
    ordinary_replay: HashMap<IdempotencyId, RelationshipReplayCapsule>,
    h2b_replay: HashMap<IdempotencyId, RelationshipH2bReplayCapsule>,
    next_operation_ordinal: u64,
    audits: Vec<AuditEvent>,
    prepared_removals: HashMap<crate::identity::PreparedIntentId, PreparedIntent>,
    completed_removals: HashSet<crate::identity::PreparedIntentId>,
}

#[derive(Clone)]
pub struct InMemoryRelationshipService<
    C,
    R,
    A,
    I = SequentialExecutionIdSource,
    P = DenyRelationshipRemoval,
    Q = DenyApprovalAuthorization,
> {
    clock: C,
    resolver: R,
    audit_ids: A,
    execution_ids: I,
    removal_policy: P,
    approval_authorization: Q,
    state: State,
    fail_next_commit: bool,
}
impl<C: Clock + Clone, R: EndpointCatalog, A: AuditEventIdSource>
    InMemoryRelationshipService<
        C,
        R,
        A,
        SequentialExecutionIdSource,
        DenyRelationshipRemoval,
        DenyApprovalAuthorization,
    >
{
    pub fn new(clock: C, resolver: R, audit_ids: A) -> Self {
        Self {
            clock,
            resolver,
            audit_ids,
            execution_ids: SequentialExecutionIdSource::default(),
            removal_policy: DenyRelationshipRemoval,
            approval_authorization: DenyApprovalAuthorization,
            fail_next_commit: false,
            state: State {
                stakeholders: HashMap::new(),
                stakeholder_endpoints: HashMap::new(),
                relationships: HashMap::new(),
                ordinary_latest_relationships: HashMap::new(),
                semantic: HashMap::new(),
                idempotency: HashMap::new(),
                ordinary_replay: HashMap::new(),
                h2b_replay: HashMap::new(),
                next_operation_ordinal: 1,
                audits: Vec::new(),
                prepared_removals: HashMap::new(),
                completed_removals: HashSet::new(),
            },
        }
    }

    #[doc(hidden)]
    pub fn rehydrate(
        clock: C,
        resolver: R,
        audit_ids: A,
        snapshot: RelationshipPersistenceSnapshot,
    ) -> Result<Self, RelationshipPersistenceError> {
        let mut service = Self::new(clock, resolver, audit_ids);
        for stakeholder in &snapshot.stakeholders {
            service
                .state
                .stakeholders
                .insert(stakeholder.id.clone(), stakeholder.clone());
            service.state.stakeholder_endpoints.insert(
                stakeholder.id.clone(),
                EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                    stakeholder.id.clone(),
                    stakeholder.version,
                    stakeholder.classification,
                )),
            );
        }
        for relationship in &snapshot.relationships {
            let current = service
                .current_endpoints(relationship, &inspection_correlation())
                .map_err(|_| RelationshipPersistenceError::EndpointMismatch)?;
            if current != relationship.endpoints {
                return Err(RelationshipPersistenceError::EndpointMismatch);
            }
            let semantic = SemanticKey {
                kind: relationship.kind,
                endpoints: relationship
                    .endpoints
                    .iter()
                    .map(EndpointSnapshot::key)
                    .collect(),
                purpose: relationship.purpose,
            };
            if service
                .state
                .semantic
                .insert(semantic, relationship.id.clone())
                .is_some()
            {
                return Err(RelationshipPersistenceError::DuplicateIdentity);
            }
            service
                .state
                .relationships
                .insert(relationship.id.clone(), relationship.clone());
        }
        for capsule in &snapshot.replay {
            let identity = persistence_identity(&capsule.command);
            let stored = match &capsule.result {
                RelationshipPersistenceResult::Stakeholder(value) => {
                    Stored::Stakeholder(value.clone())
                }
                RelationshipPersistenceResult::Relationship(value) => {
                    Stored::Relationship(value.clone())
                }
            };
            service
                .state
                .idempotency
                .insert(capsule.idempotency_id.clone(), (identity, stored));
            service
                .state
                .ordinary_replay
                .insert(capsule.idempotency_id.clone(), capsule.clone());
        }
        service.state.next_operation_ordinal = u64::try_from(snapshot.replay.len())
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(RelationshipPersistenceError::InvalidOperationOrder)?;
        service.state.audits = snapshot.audits;
        service.state.ordinary_latest_relationships = service.state.relationships.clone();
        Ok(service)
    }
}

impl<
        C: Clock + Clone,
        R: EndpointCatalog,
        A: AuditEventIdSource,
        I: ExecutionIdSource,
        P: RemovalPolicyPort,
        Q: ApprovalAuthorizationPort,
    > InMemoryRelationshipService<C, R, A, I, P, Q>
{
    pub fn with_execution_authorities(
        clock: C,
        resolver: R,
        audit_ids: A,
        execution_ids: I,
        removal_policy: P,
        approval_authorization: Q,
    ) -> Self {
        Self {
            clock,
            resolver,
            audit_ids,
            execution_ids,
            removal_policy,
            approval_authorization,
            fail_next_commit: false,
            state: State {
                stakeholders: HashMap::new(),
                stakeholder_endpoints: HashMap::new(),
                relationships: HashMap::new(),
                ordinary_latest_relationships: HashMap::new(),
                semantic: HashMap::new(),
                idempotency: HashMap::new(),
                ordinary_replay: HashMap::new(),
                h2b_replay: HashMap::new(),
                next_operation_ordinal: 1,
                audits: Vec::new(),
                prepared_removals: HashMap::new(),
                completed_removals: HashSet::new(),
            },
        }
    }
    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.state.audits
    }

    #[doc(hidden)]
    pub fn persistence_snapshot(
        &self,
    ) -> Result<RelationshipPersistenceSnapshot, RelationshipPersistenceError> {
        if !self.state.prepared_removals.is_empty()
            || !self.state.completed_removals.is_empty()
            || self.state.idempotency.values().any(|(identity, _)| {
                matches!(
                    identity,
                    Identity::Remove(..) | Identity::PrepareRemove(..) | Identity::CancelRemove(..)
                )
            })
        {
            return Err(RelationshipPersistenceError::UnsupportedRemovalState);
        }
        let mut stakeholders = self
            .state
            .stakeholders
            .values()
            .cloned()
            .collect::<Vec<_>>();
        stakeholders.sort_by(|a, b| a.id.cmp(&b.id));
        let mut relationships = self
            .state
            .relationships
            .values()
            .cloned()
            .collect::<Vec<_>>();
        relationships.sort_by(|a, b| a.id.cmp(&b.id));
        let mut replay = self
            .state
            .ordinary_replay
            .values()
            .cloned()
            .collect::<Vec<_>>();
        replay.sort_by_key(|value| value.operation_ordinal);
        Ok(RelationshipPersistenceSnapshot {
            stakeholders,
            relationships,
            replay,
            audits: self.state.audits.clone(),
        })
    }

    #[doc(hidden)]
    pub fn persistence_snapshot_with_h2b(
        &self,
    ) -> Result<RelationshipH2bPersistenceSnapshot, RelationshipPersistenceError> {
        let mut stakeholders = self
            .state
            .stakeholders
            .values()
            .cloned()
            .collect::<Vec<_>>();
        stakeholders.sort_by(|a, b| a.id.cmp(&b.id));
        let mut relationships = self
            .state
            .relationships
            .values()
            .cloned()
            .collect::<Vec<_>>();
        relationships.sort_by(|a, b| a.id.cmp(&b.id));
        let mut ordinary_history_relationships = self
            .state
            .ordinary_latest_relationships
            .values()
            .cloned()
            .collect::<Vec<_>>();
        ordinary_history_relationships.sort_by(|a, b| a.id.cmp(&b.id));
        let mut ordinary_replay = self
            .state
            .ordinary_replay
            .values()
            .cloned()
            .collect::<Vec<_>>();
        ordinary_replay.sort_by_key(|value| value.operation_ordinal);
        let mut h2b_replay = self.state.h2b_replay.values().cloned().collect::<Vec<_>>();
        h2b_replay.sort_by_key(|value| value.operation_ordinal);
        let mut pending = self
            .state
            .prepared_removals
            .values()
            .cloned()
            .collect::<Vec<_>>();
        pending.sort_by(|a, b| a.id().cmp(b.id()));
        let mut completed = self
            .state
            .completed_removals
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        completed.sort();
        let mut tombstoned_ordinary = self
            .state
            .ordinary_replay
            .keys()
            .filter(|id| {
                matches!(
                    self.state.idempotency.get(*id),
                    Some((_, Stored::Tombstone))
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        tombstoned_ordinary.sort();
        let mut tombstoned_h2b = self
            .state
            .h2b_replay
            .keys()
            .filter(|id| {
                matches!(
                    self.state.idempotency.get(*id),
                    Some((_, Stored::Tombstone))
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        tombstoned_h2b.sort();
        RelationshipH2bPersistenceSnapshot {
            stakeholders,
            relationships,
            ordinary_history_relationships,
            ordinary_replay,
            h2b_replay,
            audits: self.state.audits.clone(),
            pending,
            completed,
            tombstoned_ordinary,
            tombstoned_h2b,
        }
        .validate()
    }

    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn rehydrate_with_h2b(
        clock: C,
        resolver: R,
        audit_ids: A,
        execution_ids: I,
        removal_policy: P,
        approval_authorization: Q,
        snapshot: RelationshipH2bPersistenceSnapshot,
    ) -> Result<Self, RelationshipPersistenceError> {
        let snapshot = snapshot.validate()?;
        let mut service = Self::with_execution_authorities(
            clock,
            resolver,
            audit_ids,
            execution_ids,
            removal_policy,
            approval_authorization,
        );
        for stakeholder in &snapshot.stakeholders {
            service
                .state
                .stakeholders
                .insert(stakeholder.id.clone(), stakeholder.clone());
            service.state.stakeholder_endpoints.insert(
                stakeholder.id.clone(),
                EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                    stakeholder.id.clone(),
                    stakeholder.version,
                    stakeholder.classification,
                )),
            );
        }
        for relationship in &snapshot.relationships {
            let current = service
                .current_endpoints(relationship, &inspection_correlation())
                .map_err(|_| RelationshipPersistenceError::EndpointMismatch)?;
            if current != relationship.endpoints {
                return Err(RelationshipPersistenceError::EndpointMismatch);
            }
            let semantic = SemanticKey {
                kind: relationship.kind,
                endpoints: relationship
                    .endpoints
                    .iter()
                    .map(EndpointSnapshot::key)
                    .collect(),
                purpose: relationship.purpose,
            };
            if service
                .state
                .semantic
                .insert(semantic, relationship.id.clone())
                .is_some()
            {
                return Err(RelationshipPersistenceError::DuplicateIdentity);
            }
            service
                .state
                .relationships
                .insert(relationship.id.clone(), relationship.clone());
        }
        service.state.ordinary_latest_relationships = snapshot
            .ordinary_history_relationships
            .iter()
            .map(|value| (value.id.clone(), value.clone()))
            .collect();
        let tombstoned = snapshot.tombstoned_ordinary.iter().collect::<HashSet<_>>();
        for capsule in &snapshot.ordinary_replay {
            let identity = persistence_identity(&capsule.command);
            let stored = if tombstoned.contains(&capsule.idempotency_id) {
                Stored::Tombstone
            } else {
                match &capsule.result {
                    RelationshipPersistenceResult::Stakeholder(value) => {
                        Stored::Stakeholder(value.clone())
                    }
                    RelationshipPersistenceResult::Relationship(value) => {
                        Stored::Relationship(value.clone())
                    }
                }
            };
            service
                .state
                .idempotency
                .insert(capsule.idempotency_id.clone(), (identity, stored));
            service
                .state
                .ordinary_replay
                .insert(capsule.idempotency_id.clone(), capsule.clone());
        }
        for capsule in &snapshot.h2b_replay {
            let stored = if snapshot.tombstoned_h2b.contains(&capsule.idempotency_id) {
                Stored::Tombstone
            } else {
                h2b_persistence_stored(&capsule.result)
            };
            service.state.idempotency.insert(
                capsule.idempotency_id.clone(),
                (h2b_persistence_identity(&capsule.command), stored),
            );
            service
                .state
                .h2b_replay
                .insert(capsule.idempotency_id.clone(), capsule.clone());
        }
        service.state.prepared_removals = snapshot
            .pending
            .iter()
            .map(|value| (value.id().clone(), value.clone()))
            .collect();
        service.state.completed_removals = snapshot.completed.iter().cloned().collect();
        service.state.next_operation_ordinal =
            u64::try_from(snapshot.ordinary_replay.len() + snapshot.h2b_replay.len())
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(RelationshipPersistenceError::InvalidOperationOrder)?;
        service.state.audits = snapshot.audits;
        Ok(service)
    }
    pub fn relationship_count(&self) -> usize {
        self.state.relationships.len()
    }
    pub fn inspect_stakeholder(&self, id: &StakeholderId) -> Option<&StakeholderRecord> {
        self.state.stakeholders.get(id)
    }

    /// Return current authoritative relationship views in deterministic
    /// identifier order. Every stored endpoint is resolved through the
    /// current trusted catalog, and a missing endpoint fails closed rather
    /// than returning stale relationship data.
    #[must_use]
    pub fn stakeholders(&self) -> Vec<StakeholderRecord> {
        let mut values: Vec<_> = self.state.stakeholders.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use = "relationship collection queries should be handled"]
    pub fn relationships(&self) -> Result<Vec<RelationshipRecord>, DomainError> {
        let mut values: Vec<_> = self.state.relationships.values().cloned().collect();
        for value in &mut values {
            value.endpoints = self.current_endpoints(value, &inspection_correlation())?;
            value.classification = value
                .endpoints
                .iter()
                .fold(DataClassification::Public, |a, v| {
                    a.combine(v.classification())
                });
        }
        values.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(values)
    }
    pub fn inspect_relationship(
        &self,
        id: &RelationshipId,
    ) -> Result<Option<RelationshipRecord>, DomainError> {
        let Some(stored) = self.state.relationships.get(id).cloned() else {
            return Ok(None);
        };
        let current = self.current_endpoints(&stored, &inspection_correlation())?;
        let mut value = stored;
        value.endpoints = current;
        value.classification = value
            .endpoints
            .iter()
            .fold(DataClassification::Public, |a, v| {
                a.combine(v.classification())
            });
        Ok(Some(value))
    }
    pub fn create_stakeholder(
        &mut self,
        c: CreateStakeholder,
    ) -> Result<MutationOutcome<StakeholderRecord>, DomainError> {
        let persistence_command = RelationshipPersistenceCommand::CreateStakeholder {
            id: c.id.clone(),
            name: c.name.clone(),
            kind: c.kind,
            classification: c.classification,
            provenance: c.provenance.clone(),
        };
        let identity = Identity::CreateStakeholder(
            c.id.clone(),
            c.name.clone(),
            c.kind,
            c.classification,
            c.provenance.clone(),
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_stakeholder(value);
        }
        if self.state.stakeholders.contains_key(&c.id) {
            return Err(conflict(&c.context.correlation_id));
        }
        let now = self.clock.now();
        let record = StakeholderRecord {
            id: c.id.clone(),
            name: c.name,
            kind: c.kind,
            classification: c.classification.unwrap_or_default(),
            provenance: c.provenance,
            version: AggregateVersion::initial(),
            created_at: now,
            updated_at: now,
        };
        let mut next = self.state.clone();
        let audit = append_audit(
            &mut self.audit_ids,
            &mut next,
            &c.context,
            AuditTarget::Stakeholder(record.id.clone()),
            "relationship.stakeholder.created",
            now,
        )?;
        next.stakeholders.insert(c.id.clone(), record.clone());
        next.stakeholder_endpoints.insert(
            c.id.clone(),
            EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                c.id,
                record.version,
                record.classification,
            )),
        );
        let result = MutationOutcome {
            value: record,
            outcome: AtomicOutcome {
                audit_event_ids: vec![audit],
                effect_scope: AuditEffectScope::Complete,
            },
        };
        next.idempotency.insert(
            c.context.idempotency_id.clone(),
            (identity, Stored::Stakeholder(result.clone())),
        );
        retain_ordinary_replay(
            &mut next,
            &c.context,
            persistence_command,
            RelationshipPersistenceResult::Stakeholder(result.clone()),
        )?;
        self.commit(next, &c.context.correlation_id)?;
        Ok(result)
    }
    pub fn update_stakeholder(
        &mut self,
        c: UpdateStakeholderDetails,
    ) -> Result<MutationOutcome<StakeholderRecord>, DomainError> {
        let persistence_command = RelationshipPersistenceCommand::UpdateStakeholder {
            id: c.id.clone(),
            expected_version: c.expected_version,
            name: c.name.clone(),
            classification: c.classification,
        };
        let identity = Identity::UpdateStakeholder(
            c.id.clone(),
            c.expected_version,
            c.name.clone(),
            c.classification,
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_stakeholder(value);
        }
        let current = self
            .state
            .stakeholders
            .get(&c.id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        if current.version != c.expected_version {
            return Err(conflict(&c.context.correlation_id));
        }
        let classification = match c.classification {
            None => current.classification,
            Some(v) if current.classification.combine(v) == v => v,
            Some(_) => return Err(denied(&c.context.correlation_id)),
        };
        let now = self.clock.now();
        let version = current
            .version
            .next()
            .ok_or_else(|| internal(&c.context.correlation_id))?;
        let record = StakeholderRecord {
            id: current.id.clone(),
            name: c.name,
            kind: current.kind,
            classification,
            provenance: current.provenance.clone(),
            version,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.stakeholders.insert(c.id.clone(), record.clone());
        next.stakeholder_endpoints.insert(
            c.id.clone(),
            EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                c.id.clone(),
                version,
                classification,
            )),
        );
        let mut audit_ids = vec![append_audit(
            &mut self.audit_ids,
            &mut next,
            &c.context,
            AuditTarget::Stakeholder(record.id.clone()),
            "relationship.stakeholder.updated",
            now,
        )?];
        let mut affected = if classification != current.classification {
            next.relationships
                .iter()
                .filter_map(|(id, relationship)| {
                    relationship
                        .endpoints
                        .iter()
                        .any(|v| matches!(v, EndpointSnapshot::Stakeholder(s) if s.id() == &c.id))
                        .then_some(id.clone())
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        affected.sort();
        for relationship_id in affected {
            if let Some(relationship) = next.relationships.get_mut(&relationship_id) {
                relationship.endpoints = relationship
                    .endpoints
                    .iter()
                    .map(|v| match v {
                        EndpointSnapshot::Stakeholder(s) if s.id() == &c.id => {
                            EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                                c.id.clone(),
                                version,
                                classification,
                            ))
                        }
                        other => other.clone(),
                    })
                    .collect();
                relationship.classification = relationship
                    .endpoints
                    .iter()
                    .fold(DataClassification::Public, |a, v| {
                        a.combine(v.classification())
                    });
                relationship.version = relationship
                    .version
                    .next()
                    .ok_or_else(|| internal(&c.context.correlation_id))?;
                relationship.updated_at = now;
            }
            let id = append_audit(
                &mut self.audit_ids,
                &mut next,
                &c.context,
                AuditTarget::Stakeholder(c.id.clone()),
                "relationship.stakeholder_relationship.reclassified",
                now,
            )?;
            audit_ids.push(id);
        }
        let result = MutationOutcome {
            value: record,
            outcome: AtomicOutcome {
                audit_event_ids: audit_ids,
                effect_scope: AuditEffectScope::Complete,
            },
        };
        next.idempotency.insert(
            c.context.idempotency_id.clone(),
            (identity, Stored::Stakeholder(result.clone())),
        );
        retain_ordinary_replay(
            &mut next,
            &c.context,
            persistence_command,
            RelationshipPersistenceResult::Stakeholder(result.clone()),
        )?;
        self.commit(next, &c.context.correlation_id)?;
        Ok(result)
    }

    pub fn link_portfolio_product(
        &mut self,
        c: LinkPortfolioProduct,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::PortfolioProduct,
            vec![
                EndpointKey::Portfolio(c.portfolio_id.clone()),
                EndpointKey::Product(c.product_id.clone()),
            ],
            None,
            vec![c.expected_portfolio_version, c.expected_product_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let p = self
            .resolver
            .portfolio(&c.portfolio_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let q = self
            .resolver
            .product(&c.product_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        self.link(
            c.id,
            RelationshipKind::PortfolioProduct,
            vec![EndpointSnapshot::Portfolio(p), EndpointSnapshot::Product(q)],
            vec![c.expected_portfolio_version, c.expected_product_version],
            None,
            c.context,
        )
    }
    pub fn link_portfolio_initiative(
        &mut self,
        c: LinkPortfolioInitiative,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::PortfolioInitiative,
            vec![
                EndpointKey::Portfolio(c.portfolio_id.clone()),
                EndpointKey::Initiative(c.initiative_id.clone()),
            ],
            None,
            vec![c.expected_portfolio_version, c.expected_initiative_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let p = self
            .resolver
            .portfolio(&c.portfolio_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let q = self
            .resolver
            .initiative(&c.initiative_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        self.link(
            c.id,
            RelationshipKind::PortfolioInitiative,
            vec![
                EndpointSnapshot::Portfolio(p),
                EndpointSnapshot::Initiative(q),
            ],
            vec![c.expected_portfolio_version, c.expected_initiative_version],
            None,
            c.context,
        )
    }
    pub fn link_product_roadmap(
        &mut self,
        c: LinkProductRoadmap,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::ProductRoadmap,
            vec![
                EndpointKey::Product(c.product_id.clone()),
                EndpointKey::Roadmap(c.roadmap_id.clone()),
            ],
            None,
            vec![c.expected_product_version, c.expected_roadmap_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let p = self
            .resolver
            .product(&c.product_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let q = self
            .resolver
            .roadmap(&c.roadmap_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        self.link(
            c.id,
            RelationshipKind::ProductRoadmap,
            vec![EndpointSnapshot::Product(p), EndpointSnapshot::Roadmap(q)],
            vec![c.expected_product_version, c.expected_roadmap_version],
            None,
            c.context,
        )
    }
    pub fn link_product_kpi(
        &mut self,
        c: LinkProductKpi,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::ProductKpi,
            vec![
                EndpointKey::Product(c.product_id.clone()),
                EndpointKey::Kpi(c.kpi_id.clone()),
            ],
            None,
            vec![c.expected_product_version, c.expected_kpi_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let p = self
            .resolver
            .product(&c.product_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let q = self
            .resolver
            .kpi(&c.kpi_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        self.link(
            c.id,
            RelationshipKind::ProductKpi,
            vec![EndpointSnapshot::Product(p), EndpointSnapshot::Kpi(q)],
            vec![c.expected_product_version, c.expected_kpi_version],
            None,
            c.context,
        )
    }
    pub fn link_initiative_project(
        &mut self,
        c: LinkInitiativeProject,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::InitiativeProject,
            vec![
                EndpointKey::Initiative(c.initiative_id.clone()),
                EndpointKey::Project(c.project_id.clone()),
            ],
            None,
            vec![c.expected_initiative_version, c.expected_project_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let p = self
            .resolver
            .initiative(&c.initiative_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let q = self
            .resolver
            .project(&c.project_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        self.link(
            c.id,
            RelationshipKind::InitiativeProject,
            vec![
                EndpointSnapshot::Initiative(p),
                EndpointSnapshot::Project(q),
            ],
            vec![c.expected_initiative_version, c.expected_project_version],
            None,
            c.context,
        )
    }
    pub fn link_project_product(
        &mut self,
        c: LinkProjectProduct,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::ProjectProduct,
            vec![
                EndpointKey::Project(c.project_id.clone()),
                EndpointKey::Product(c.product_id.clone()),
            ],
            None,
            vec![c.expected_project_version, c.expected_product_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let p = self
            .resolver
            .project(&c.project_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let q = self
            .resolver
            .product(&c.product_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        self.link(
            c.id,
            RelationshipKind::ProjectProduct,
            vec![EndpointSnapshot::Project(p), EndpointSnapshot::Product(q)],
            vec![c.expected_project_version, c.expected_product_version],
            None,
            c.context,
        )
    }
    pub fn validate_project_milestone(
        &self,
        c: ValidateProjectMilestone,
    ) -> Result<ProjectMilestoneValidation, DomainError> {
        let p = self
            .resolver
            .project(&c.project_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let m = self
            .resolver
            .milestone(&c.milestone_id)
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        check_versions(
            &[
                EndpointSnapshot::Project(p.clone()),
                EndpointSnapshot::Milestone(m.clone()),
            ],
            &[c.expected_project_version, c.expected_milestone_version],
            &c.context.correlation_id,
        )?;
        if m.project_id() != p.id() {
            return Err(conflict(&c.context.correlation_id));
        }
        let classification = p.classification().combine(m.classification());
        if classification == DataClassification::Unclassified {
            return Err(denied(&c.context.correlation_id));
        }
        let result = ProjectMilestoneValidation {
            project_id: c.project_id,
            milestone_id: c.milestone_id,
            classification,
            outcome: AtomicOutcome {
                audit_event_ids: Vec::new(),
                effect_scope: AuditEffectScope::None,
            },
        };
        Ok(result)
    }
    pub fn link_stakeholder_relationship(
        &mut self,
        c: LinkStakeholderRelationship,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let subject_key = stakeholder_subject_key(&c.subject);
        let identity = Identity::Link(
            c.id.clone(),
            RelationshipKind::StakeholderSubject,
            vec![
                EndpointKey::Stakeholder(c.stakeholder_id.clone()),
                subject_key,
            ],
            Some(c.purpose),
            vec![c.expected_stakeholder_version, c.expected_subject_version],
        );
        if let Some(value) = self.replay(&c.context, &identity)? {
            return expect_relationship(value);
        }
        let s = self
            .state
            .stakeholder_endpoints
            .get(&c.stakeholder_id)
            .cloned()
            .ok_or_else(|| not_found(&c.context.correlation_id))?;
        let subject = self.subject(&c.subject, &c.context.correlation_id)?;
        self.link(
            c.id,
            RelationshipKind::StakeholderSubject,
            vec![s, subject],
            vec![c.expected_stakeholder_version, c.expected_subject_version],
            Some(c.purpose),
            c.context,
        )
    }

    /// Prepare the H2b relationship-removal intent.  Recovery evidence is an
    /// injected authority; the default unavailable authority therefore fails
    /// closed and never fabricates a recovery claim.
    pub fn prepare_remove_relationship<PV: RecoveryEvidencePort>(
        &mut self,
        request: PrepareRemoveRelationship,
        recovery: &PV,
    ) -> Result<PreparedIntent, DomainError> {
        let prepare_identity = Identity::PrepareRemove(request.relationship_id.clone());
        if let Some(stored) = self.replay(&request.context, &prepare_identity)? {
            return match stored {
                Stored::Prepared(value) => Ok(value),
                _ => Err(idempotency_conflict(&request.context.correlation_id)),
            };
        }
        let expires_at = UtcTimestamp::from_unix_millis(
            self.clock
                .now()
                .unix_millis()
                .checked_add(crate::execution::MAX_PREPARED_INTENT_TTL_MILLIS)
                .ok_or_else(|| internal(&request.context.correlation_id))?,
        );
        let record = self
            .state
            .relationships
            .get(&request.relationship_id)
            .cloned()
            .ok_or_else(|| not_found(&request.context.correlation_id))?;
        if !record.kind.removable() {
            return Err(denied(&request.context.correlation_id));
        }
        let endpoints = self.current_endpoints(&record, &request.context.correlation_id)?;
        let classification = endpoints
            .iter()
            .fold(record.classification, |value, endpoint| {
                value.combine(endpoint.classification())
            });
        if !self
            .removal_policy
            .allow_relationship_removal(&record.id, classification)
        {
            return Err(denied(&request.context.correlation_id));
        }
        let evidence = recovery
            .recovery_evidence(&record.id)
            .filter(|value| value.relationship_id() == record.id() && value.compatible());
        let Some(evidence) = evidence.filter(|value| value.verified_at().unix_millis() > 0) else {
            return Err(denied(&request.context.correlation_id));
        };
        if classification == DataClassification::Unclassified
            || expires_at.unix_millis() <= self.clock.now().unix_millis()
        {
            return Err(denied(&request.context.correlation_id));
        }
        let id = self
            .execution_ids
            .next_prepared_intent_id()
            .map_err(|_| internal(&request.context.correlation_id))?;
        if self.state.prepared_removals.contains_key(&id)
            || self
                .state
                .prepared_removals
                .values()
                .any(|value| value.relationship_id() == &record.id)
            || self.state.h2b_replay.values().any(|capsule| {
                matches!(
                    &capsule.result,
                    RelationshipH2bPersistenceResult::Prepared(value) if value.id() == &id
                )
            })
        {
            return Err(conflict(&request.context.correlation_id));
        }
        let confirmation_challenge = format!("REMOVE {id}");
        let preview = RemoveRelationshipPreview {
            intent_type: REMOVE_RELATIONSHIP_INTENT_TYPE,
            intent_version: REMOVE_RELATIONSHIP_INTENT_VERSION,
            prepared_id: id.clone(),
            relationship_id: record.id,
            relationship_version: record.version,
            endpoints,
            kind: record.kind,
            purpose: record.purpose,
            effects: vec![
                RemovalEffect::RemoveRelationshipRecord,
                RemovalEffect::RemoveSemanticRelationshipIndex,
                RemovalEffect::CreateIdempotencyTombstone,
            ],
            classification,
            policy_decision: RemovalPolicyDecision::Allowed,
            evidence,
            expires_at,
            cancellation_policy: CancellationPolicy::NotCancellableAfterSubmit,
            confirmation_challenge,
        };
        let prepared = PreparedIntent {
            payload_digest: PayloadDigest::for_preview(&preview),
            preview,
        };
        let mut next = self.state.clone();
        next.prepared_removals.insert(id, prepared.clone());
        next.idempotency.insert(
            request.context.idempotency_id.clone(),
            (prepare_identity, Stored::Prepared(prepared.clone())),
        );
        retain_h2b_replay(
            &mut next,
            &request.context,
            RelationshipH2bPersistenceCommand::Prepare {
                relationship_id: prepared.relationship_id().clone(),
            },
            RelationshipH2bPersistenceResult::Prepared(prepared.clone()),
            Vec::new(),
        )?;
        self.commit(next, &request.context.correlation_id)?;
        Ok(prepared)
    }

    pub fn discard_prepared_remove_relationship(
        &mut self,
        prepared_id: &crate::identity::PreparedIntentId,
        actor: AuditActor,
        context: OperationContext,
    ) -> Result<(), DomainError> {
        if actor != AuditActor::HeadOfProducts
            || !self
                .approval_authorization
                .authorize_relationship_removal(actor)
        {
            return Err(denied(&context.correlation_id));
        }
        let cancel_identity = Identity::CancelRemove(prepared_id.clone(), actor);
        if let Some(stored) = self.replay(&context, &cancel_identity)? {
            return match stored {
                Stored::Cancelled => Ok(()),
                _ => Err(idempotency_conflict(&context.correlation_id)),
            };
        }
        if self.state.completed_removals.contains(prepared_id) {
            return Err(too_late_to_cancel(&context.correlation_id));
        }
        let mut next = self.state.clone();
        let prepared = next
            .prepared_removals
            .remove(prepared_id)
            .ok_or_else(|| not_found(&context.correlation_id))?;
        let cancellation_audit = append_cancellation_audit(
            &mut self.audit_ids,
            &mut next,
            &context,
            prepared.relationship_id().clone(),
            self.clock.now(),
            actor,
        )?;
        next.idempotency.insert(
            context.idempotency_id.clone(),
            (cancel_identity, Stored::Cancelled),
        );
        retain_h2b_replay(
            &mut next,
            &context,
            RelationshipH2bPersistenceCommand::Cancel {
                prepared_id: prepared_id.clone(),
                actor,
            },
            RelationshipH2bPersistenceResult::Cancelled,
            vec![cancellation_audit],
        )?;
        self.commit(next, &context.correlation_id)
    }

    pub fn approve_and_execute_remove_relationship<PV: RecoveryEvidencePort>(
        &mut self,
        request: ApproveAndExecuteRemoveRelationship,
        recovery: &PV,
    ) -> Result<RemovalOutcome, DomainError> {
        let removal_identity = Identity::Remove(
            request.prepared_id.clone(),
            request.acknowledged_payload_digest.clone(),
            PayloadDigest::for_confirmation(&request.confirmation),
            request.actor,
        );
        if let Some(stored) = self.replay(&request.context, &removal_identity)? {
            return match stored {
                Stored::Removal(outcome) => Ok(outcome),
                Stored::Rejection(error) => Err(error),
                _ => Err(idempotency_conflict(&request.context.correlation_id)),
            };
        }
        if request.actor != AuditActor::HeadOfProducts
            || !self
                .approval_authorization
                .authorize_relationship_removal(request.actor)
        {
            if let Some(relationship_id) = self.prepared_relationship_id(&request.prepared_id) {
                return self.reject_removal(
                    &request.context,
                    removal_identity.clone(),
                    relationship_id,
                    request.actor,
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    authorization_denied(&request.context.correlation_id),
                );
            }
            return Err(authorization_denied(&request.context.correlation_id));
        }
        let prepared = match self
            .state
            .prepared_removals
            .get(&request.prepared_id)
            .cloned()
        {
            Some(value) => value,
            None => {
                let rejection = if self.state.completed_removals.contains(&request.prepared_id) {
                    conflict(&request.context.correlation_id)
                } else {
                    not_found(&request.context.correlation_id)
                };
                if let Some(relationship_id) = self.prepared_relationship_id(&request.prepared_id) {
                    return self.reject_removal(
                        &request.context,
                        removal_identity.clone(),
                        relationship_id,
                        request.actor,
                        "relationship.removal.approval_rejected",
                        AuditPolicyOutcome::Allowed,
                        AuditApprovalOutcome::Rejected,
                        AuditExecutionOutcome::NotAttempted,
                        rejection,
                    );
                }
                return Err(rejection);
            }
        };
        if prepared.confirmation_challenge() != request.confirmation {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                prepared.relationship_id().clone(),
                request.actor,
                "relationship.removal.approval_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Rejected,
                AuditExecutionOutcome::NotAttempted,
                confirmation_mismatch(&request.context.correlation_id),
            );
        }
        if prepared.expires_at().unix_millis() <= self.clock.now().unix_millis()
            || prepared.payload_digest() != &request.acknowledged_payload_digest
        {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                prepared.relationship_id().clone(),
                request.actor,
                "relationship.removal.approval_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Rejected,
                AuditExecutionOutcome::NotAttempted,
                preview_expired_or_changed(&request.context.correlation_id),
            );
        }
        let Some(current) = self
            .state
            .relationships
            .get(prepared.relationship_id())
            .cloned()
        else {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                prepared.relationship_id().clone(),
                request.actor,
                "relationship.removal.execution_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Failed,
                preview_expired_or_changed(&request.context.correlation_id),
            );
        };
        let current_endpoints =
            match self.current_endpoints(&current, &request.context.correlation_id) {
                Ok(value) => value,
                Err(_error) => {
                    return self.reject_removal(
                        &request.context,
                        removal_identity.clone(),
                        current.id.clone(),
                        request.actor,
                        "relationship.removal.execution_rejected",
                        AuditPolicyOutcome::Allowed,
                        AuditApprovalOutcome::Approved,
                        AuditExecutionOutcome::Failed,
                        preview_expired_or_changed(&request.context.correlation_id),
                    );
                }
            };
        let versions = current_endpoints
            .iter()
            .map(EndpointSnapshot::version)
            .collect::<Vec<_>>();
        let current_classification = current_endpoints
            .iter()
            .fold(current.classification, |value, endpoint| {
                value.combine(endpoint.classification())
            });
        let current_evidence = recovery.recovery_evidence(&current.id);
        // Revalidate current H3 policy prerequisites before comparing the
        // prepared snapshot. A prerequisite that is no longer valid is a
        // policy denial, not an invitation to prepare the same unsafe action
        // again. Only a *currently valid* evidence snapshot may be stale.
        if current_classification == DataClassification::Unclassified
            || !self
                .removal_policy
                .allow_relationship_removal(&current.id, current_classification)
            || !current_evidence.as_ref().is_some_and(|value| {
                value.relationship_id() == &current.id
                    && value.compatible()
                    && value.verified_at().unix_millis() > 0
            })
        {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                current.id.clone(),
                request.actor,
                "relationship.removal.policy_denied",
                AuditPolicyOutcome::Denied,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::NotAttempted,
                policy_denied(&request.context.correlation_id),
            );
        }
        let evidence_matches_prepared = current_evidence
            .as_ref()
            .is_some_and(|value| value == prepared.preview().evidence());
        if current.version != prepared.preview().relationship_version()
            || versions
                != prepared
                    .preview()
                    .endpoints()
                    .iter()
                    .map(EndpointSnapshot::version)
                    .collect::<Vec<_>>()
            || current_classification != prepared.preview().classification()
            || !evidence_matches_prepared
        {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                current.id.clone(),
                request.actor,
                "relationship.removal.execution_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Failed,
                preview_expired_or_changed(&request.context.correlation_id),
            );
        }
        let current_evidence =
            current_evidence.ok_or_else(|| conflict(&request.context.correlation_id))?;
        let semantic = SemanticKey {
            kind: current.kind,
            endpoints: current
                .endpoints
                .iter()
                .map(EndpointSnapshot::key)
                .collect(),
            purpose: current.purpose,
        };
        if self.state.semantic.get(&semantic) != Some(&current.id) {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                current.id.clone(),
                request.actor,
                "relationship.removal.execution_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Failed,
                preview_expired_or_changed(&request.context.correlation_id),
            );
        }
        let rebuilt_preview = RemoveRelationshipPreview {
            intent_type: REMOVE_RELATIONSHIP_INTENT_TYPE,
            intent_version: REMOVE_RELATIONSHIP_INTENT_VERSION,
            prepared_id: prepared.id().clone(),
            relationship_id: current.id.clone(),
            relationship_version: current.version,
            endpoints: current_endpoints,
            kind: current.kind,
            purpose: current.purpose,
            effects: vec![
                RemovalEffect::RemoveRelationshipRecord,
                RemovalEffect::RemoveSemanticRelationshipIndex,
                RemovalEffect::CreateIdempotencyTombstone,
            ],
            classification: current_classification,
            policy_decision: RemovalPolicyDecision::Allowed,
            evidence: current_evidence,
            expires_at: prepared.expires_at(),
            cancellation_policy: CancellationPolicy::NotCancellableAfterSubmit,
            confirmation_challenge: prepared.confirmation_challenge().to_owned(),
        };
        let recomputed_digest = PayloadDigest::for_preview(&rebuilt_preview);
        if rebuilt_preview != *prepared.preview()
            || recomputed_digest != *prepared.payload_digest()
            || recomputed_digest != request.acknowledged_payload_digest
        {
            return self.reject_removal(
                &request.context,
                removal_identity.clone(),
                current.id.clone(),
                request.actor,
                "relationship.removal.execution_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Failed,
                preview_expired_or_changed(&request.context.correlation_id),
            );
        }
        let mut next = self.state.clone();
        // Mint only after every live validation. This authority is deliberately
        // ephemeral: binding and consumption happen inside this transaction.
        let mut receipt = EphemeralApprovalReceipt {
            id: self
                .execution_ids
                .next_approval_receipt_id()
                .map_err(|_| internal(&request.context.correlation_id))?,
            prepared_id: request.prepared_id.clone(),
            actor: request.actor,
            acknowledged_payload_digest: request.acknowledged_payload_digest.clone(),
            consumed: false,
        };
        if receipt.prepared_id != *prepared.id()
            || receipt.actor != AuditActor::HeadOfProducts
            || receipt.acknowledged_payload_digest != recomputed_digest
            || receipt.consumed
        {
            return self.reject_removal(
                &request.context,
                removal_identity,
                current.id.clone(),
                request.actor,
                "relationship.removal.approval_rejected",
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Rejected,
                AuditExecutionOutcome::NotAttempted,
                authorization_denied(&request.context.correlation_id),
            );
        }
        receipt.consumed = true;
        debug_assert!(receipt.consumed && !receipt.id.to_string().is_empty());
        if next.relationships.remove(&current.id).is_none()
            || next.semantic.remove(&semantic).is_none()
        {
            return Err(conflict(&request.context.correlation_id));
        }
        for (_, stored) in next.idempotency.values_mut() {
            if matches!(stored, Stored::Relationship(value) if value.value().id() == &current.id)
                || matches!(stored, Stored::Prepared(value) if value.relationship_id() == &current.id)
            {
                *stored = Stored::Tombstone;
            }
        }
        let audit = append_removal_audit(
            &mut self.audit_ids,
            &mut next,
            &request.context,
            current.id.clone(),
            self.clock.now(),
            receipt.actor,
        )?;
        let outcome = RemovalOutcome {
            relationship_id: current.id.clone(),
            audit_event_ids: vec![audit],
        };
        next.idempotency.insert(
            request.context.idempotency_id.clone(),
            (removal_identity, Stored::Removal(outcome.clone())),
        );
        retain_h2b_replay(
            &mut next,
            &request.context,
            RelationshipH2bPersistenceCommand::Execute {
                prepared_id: request.prepared_id.clone(),
                acknowledged_payload_digest: request.acknowledged_payload_digest.clone(),
                confirmation_digest: PayloadDigest::for_confirmation(&request.confirmation),
                actor: request.actor,
            },
            RelationshipH2bPersistenceResult::Removal(outcome.clone()),
            outcome.audit_event_ids.clone(),
        )?;
        next.prepared_removals.remove(prepared.id());
        next.completed_removals.insert(prepared.id().clone());
        self.commit(next, &request.context.correlation_id)?;
        Ok(outcome)
    }

    pub fn cancel_remove_relationship(
        &mut self,
        prepared_id: &crate::identity::PreparedIntentId,
        actor: AuditActor,
        context: OperationContext,
    ) -> Result<(), DomainError> {
        self.discard_prepared_remove_relationship(prepared_id, actor, context)
    }

    fn subject(
        &self,
        s: &StakeholderSubject,
        id: &CorrelationId,
    ) -> Result<EndpointSnapshot, DomainError> {
        Ok(match s {
            StakeholderSubject::Portfolio(v) => EndpointSnapshot::Portfolio(
                self.resolver.portfolio(v).ok_or_else(|| not_found(id))?,
            ),
            StakeholderSubject::Product(v) => {
                EndpointSnapshot::Product(self.resolver.product(v).ok_or_else(|| not_found(id))?)
            }
            StakeholderSubject::Initiative(v) => EndpointSnapshot::Initiative(
                self.resolver.initiative(v).ok_or_else(|| not_found(id))?,
            ),
            StakeholderSubject::Project(v) => {
                EndpointSnapshot::Project(self.resolver.project(v).ok_or_else(|| not_found(id))?)
            }
            StakeholderSubject::Roadmap(v) => {
                EndpointSnapshot::Roadmap(self.resolver.roadmap(v).ok_or_else(|| not_found(id))?)
            }
            StakeholderSubject::Milestone(v) => EndpointSnapshot::Milestone(
                self.resolver.milestone(v).ok_or_else(|| not_found(id))?,
            ),
            StakeholderSubject::Kpi(v) => {
                EndpointSnapshot::Kpi(self.resolver.kpi(v).ok_or_else(|| not_found(id))?)
            }
        })
    }
    fn link(
        &mut self,
        id: RelationshipId,
        kind: RelationshipKind,
        endpoints: Vec<EndpointSnapshot>,
        expected: Vec<AggregateVersion>,
        purpose: Option<StakeholderRelationshipPurpose>,
        context: OperationContext,
    ) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
        let persistence_command = RelationshipPersistenceCommand::Link {
            id: id.clone(),
            kind,
            endpoints: endpoints.clone(),
            expected_versions: expected.clone(),
            purpose,
        };
        let keys = endpoints
            .iter()
            .map(EndpointSnapshot::key)
            .collect::<Vec<_>>();
        let identity = Identity::Link(id.clone(), kind, keys.clone(), purpose, expected.clone());
        if let Some(value) = self.replay(&context, &identity)? {
            return expect_relationship(value);
        }
        check_versions(&endpoints, &expected, &context.correlation_id)?;
        if endpoints
            .iter()
            .any(|v| v.classification() == DataClassification::Unclassified)
        {
            return Err(denied(&context.correlation_id));
        }
        let semantic = SemanticKey {
            kind,
            endpoints: keys,
            purpose,
        };
        if let Some(existing_id) = self.state.semantic.get(&semantic) {
            let existing = self
                .state
                .relationships
                .get(existing_id)
                .cloned()
                .ok_or_else(|| internal(&context.correlation_id))?;
            let current = self.current_endpoints(&existing, &context.correlation_id)?;
            let mut normalized = existing;
            normalized.endpoints = current;
            normalized.classification = normalized
                .endpoints
                .iter()
                .fold(DataClassification::Public, |a, v| {
                    a.combine(v.classification())
                });
            let mut next = self.state.clone();
            let result = MutationOutcome {
                value: normalized,
                outcome: AtomicOutcome {
                    audit_event_ids: Vec::new(),
                    effect_scope: AuditEffectScope::None,
                },
            };
            next.idempotency.insert(
                context.idempotency_id.clone(),
                (identity, Stored::Relationship(result.clone())),
            );
            retain_ordinary_replay(
                &mut next,
                &context,
                persistence_command,
                RelationshipPersistenceResult::Relationship(result.clone()),
            )?;
            self.commit(next, &context.correlation_id)?;
            return Ok(result);
        }
        // Relationship identifiers are authority identities, not recyclable
        // row keys. Keep a removed identifier reserved so an H2b tombstone
        // can never be confused with a later relationship during replay or
        // persistence reconstruction.
        if self.state.ordinary_latest_relationships.contains_key(&id) {
            return Err(conflict(&context.correlation_id));
        }
        let classification = endpoints.iter().fold(DataClassification::Public, |a, v| {
            a.combine(v.classification())
        });
        let now = self.clock.now();
        let mut next = self.state.clone();
        let target = audit_target(&endpoints).ok_or_else(|| internal(&context.correlation_id))?;
        let code = relationship_code(kind);
        let audit = append_audit(&mut self.audit_ids, &mut next, &context, target, code, now)?;
        let value = RelationshipRecord {
            id: id.clone(),
            kind,
            endpoints,
            purpose,
            classification,
            version: AggregateVersion::initial(),
            created_at: now,
            updated_at: now,
        };
        next.relationships.insert(id.clone(), value.clone());
        next.semantic.insert(semantic, id);
        let result = MutationOutcome {
            value,
            outcome: AtomicOutcome {
                audit_event_ids: vec![audit],
                effect_scope: AuditEffectScope::Complete,
            },
        };
        next.idempotency.insert(
            context.idempotency_id.clone(),
            (identity, Stored::Relationship(result.clone())),
        );
        retain_ordinary_replay(
            &mut next,
            &context,
            persistence_command,
            RelationshipPersistenceResult::Relationship(result.clone()),
        )?;
        self.commit(next, &context.correlation_id)?;
        Ok(result)
    }
    fn current_endpoints(
        &self,
        record: &RelationshipRecord,
        correlation_id: &CorrelationId,
    ) -> Result<Vec<EndpointSnapshot>, DomainError> {
        record
            .endpoints
            .iter()
            .map(|v| match v {
                EndpointSnapshot::Stakeholder(s) => {
                    self.state.stakeholder_endpoints.get(s.id()).cloned()
                }
                EndpointSnapshot::Portfolio(s) => self
                    .resolver
                    .portfolio(s.id())
                    .map(EndpointSnapshot::Portfolio),
                EndpointSnapshot::Product(s) => {
                    self.resolver.product(s.id()).map(EndpointSnapshot::Product)
                }
                EndpointSnapshot::Initiative(s) => self
                    .resolver
                    .initiative(s.id())
                    .map(EndpointSnapshot::Initiative),
                EndpointSnapshot::Roadmap(s) => {
                    self.resolver.roadmap(s.id()).map(EndpointSnapshot::Roadmap)
                }
                EndpointSnapshot::Kpi(s) => self.resolver.kpi(s.id()).map(EndpointSnapshot::Kpi),
                EndpointSnapshot::Project(s) => {
                    self.resolver.project(s.id()).map(EndpointSnapshot::Project)
                }
                EndpointSnapshot::Milestone(s) => self
                    .resolver
                    .milestone(s.id())
                    .map(EndpointSnapshot::Milestone),
            })
            .map(|value| value.ok_or_else(|| not_found(correlation_id)))
            .collect()
    }
    fn replay(
        &self,
        c: &OperationContext,
        identity: &Identity,
    ) -> Result<Option<Stored>, DomainError> {
        match self.state.idempotency.get(&c.idempotency_id) {
            None => Ok(None),
            Some((_, Stored::Tombstone)) => Err(idempotency_conflict(&c.correlation_id)),
            Some((saved, result)) if saved == identity => Ok(Some(result.clone())),
            Some(_) => Err(idempotency_conflict(&c.correlation_id)),
        }
    }
    fn commit(&mut self, state: State, id: &CorrelationId) -> Result<(), DomainError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(internal(id));
        }
        self.state = state;
        Ok(())
    }

    fn prepared_relationship_id(
        &self,
        prepared_id: &crate::identity::PreparedIntentId,
    ) -> Option<RelationshipId> {
        self.state
            .prepared_removals
            .get(prepared_id)
            .map(|value| value.relationship_id().clone())
            .or_else(|| {
                self.state.idempotency.values().find_map(|(identity, stored)| match stored {
                    Stored::Prepared(value) if value.id() == prepared_id => {
                        Some(value.relationship_id().clone())
                    }
                    Stored::Removal(value)
                        if matches!(identity, Identity::Remove(id, ..) if id == prepared_id) =>
                    {
                        Some(value.relationship_id.clone())
                    }
                    _ => None,
                })
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn reject_removal<T>(
        &mut self,
        context: &OperationContext,
        identity: Identity,
        relationship_id: RelationshipId,
        actor: AuditActor,
        code: &str,
        policy_outcome: AuditPolicyOutcome,
        approval_outcome: AuditApprovalOutcome,
        execution_outcome: AuditExecutionOutcome,
        rejection: DomainError,
    ) -> Result<T, DomainError> {
        let mut next = self.state.clone();
        let audit_id = append_removal_rejection_audit(
            &mut self.audit_ids,
            &mut next,
            context,
            relationship_id,
            self.clock.now(),
            actor,
            code,
            policy_outcome,
            approval_outcome,
            execution_outcome,
        )?;
        next.idempotency.insert(
            context.idempotency_id.clone(),
            (identity.clone(), Stored::Rejection(rejection.clone())),
        );
        let Identity::Remove(prepared_id, acknowledged_payload_digest, confirmation_digest, actor) =
            identity
        else {
            return Err(internal(&context.correlation_id));
        };
        retain_h2b_replay(
            &mut next,
            context,
            RelationshipH2bPersistenceCommand::Execute {
                prepared_id,
                acknowledged_payload_digest,
                confirmation_digest,
                actor,
            },
            RelationshipH2bPersistenceResult::Rejection(rejection.clone()),
            vec![audit_id],
        )?;
        self.commit(next, &context.correlation_id)?;
        Err(rejection)
    }
}

/// In-memory adapter seam used by the transactional ledger composition. The
/// catalog refresh only changes the trusted resolver used by future commands;
/// authoritative relationship records, audit history, idempotency and H2b
/// state remain untouched.
impl<
        C: Clock + Clone,
        A: AuditEventIdSource,
        I: ExecutionIdSource,
        P: RemovalPolicyPort,
        Q: ApprovalAuthorizationPort,
    > InMemoryRelationshipService<C, InMemoryEndpointCatalog, A, I, P, Q>
{
    pub fn replace_endpoint_catalog(&mut self, resolver: InMemoryEndpointCatalog) {
        self.resolver = resolver;
    }
}

fn expect_stakeholder(value: Stored) -> Result<MutationOutcome<StakeholderRecord>, DomainError> {
    match value {
        Stored::Stakeholder(v) => Ok(v),
        _ => Err(internal_from_stored()),
    }
}
fn expect_relationship(value: Stored) -> Result<MutationOutcome<RelationshipRecord>, DomainError> {
    match value {
        Stored::Relationship(v) => Ok(v),
        _ => Err(internal_from_stored()),
    }
}
fn stakeholder_subject_key(subject: &StakeholderSubject) -> EndpointKey {
    match subject {
        StakeholderSubject::Portfolio(value) => EndpointKey::Portfolio(value.clone()),
        StakeholderSubject::Product(value) => EndpointKey::Product(value.clone()),
        StakeholderSubject::Initiative(value) => EndpointKey::Initiative(value.clone()),
        StakeholderSubject::Project(value) => EndpointKey::Project(value.clone()),
        StakeholderSubject::Roadmap(value) => EndpointKey::Roadmap(value.clone()),
        StakeholderSubject::Milestone(value) => EndpointKey::Milestone(value.clone()),
        StakeholderSubject::Kpi(value) => EndpointKey::Kpi(value.clone()),
    }
}
fn internal_from_stored() -> DomainError {
    let correlation = match CorrelationId::parse("relationship-internal") {
        Ok(value) => value,
        Err(_) => unreachable!("static relationship correlation is valid"),
    };
    error(
        ErrorCode::PlatformInternal,
        "relationship.persistence_failed",
        &correlation,
    )
}
fn inspection_correlation() -> CorrelationId {
    match CorrelationId::parse("relationship-inspect") {
        Ok(value) => value,
        Err(_) => unreachable!("static inspection correlation is valid"),
    }
}
fn check_versions(
    values: &[EndpointSnapshot],
    expected: &[AggregateVersion],
    id: &CorrelationId,
) -> Result<(), DomainError> {
    if values.len() != expected.len() || values.iter().zip(expected).any(|(v, e)| v.version() != *e)
    {
        Err(conflict(id))
    } else {
        Ok(())
    }
}
fn audit_target(values: &[EndpointSnapshot]) -> Option<AuditTarget> {
    match values.first()? {
        EndpointSnapshot::Portfolio(v) => Some(AuditTarget::Portfolio(v.id.clone())),
        EndpointSnapshot::Product(v) => Some(AuditTarget::Product(v.id.clone())),
        EndpointSnapshot::Initiative(v) => Some(AuditTarget::Initiative(v.id.clone())),
        EndpointSnapshot::Project(v) => Some(AuditTarget::Project(v.id.clone())),
        EndpointSnapshot::Roadmap(v) => Some(AuditTarget::Roadmap(v.id.clone())),
        EndpointSnapshot::Milestone(v) => Some(AuditTarget::Milestone(v.id.clone())),
        EndpointSnapshot::Kpi(v) => Some(AuditTarget::Kpi(v.id.clone())),
        EndpointSnapshot::Stakeholder(v) => Some(AuditTarget::Stakeholder(v.id.clone())),
    }
}

fn result_audit_ids(result: &RelationshipPersistenceResult) -> Vec<AuditEventId> {
    match result {
        RelationshipPersistenceResult::Stakeholder(value) => value.outcome.audit_event_ids.clone(),
        RelationshipPersistenceResult::Relationship(value) => value.outcome.audit_event_ids.clone(),
    }
}

fn valid_relationship_topology(
    kind: RelationshipKind,
    endpoints: &[EndpointSnapshot],
    purpose: Option<StakeholderRelationshipPurpose>,
) -> bool {
    match (kind, endpoints, purpose) {
        (
            RelationshipKind::PortfolioProduct,
            [EndpointSnapshot::Portfolio(_), EndpointSnapshot::Product(_)],
            None,
        )
        | (
            RelationshipKind::PortfolioInitiative,
            [EndpointSnapshot::Portfolio(_), EndpointSnapshot::Initiative(_)],
            None,
        )
        | (
            RelationshipKind::ProductRoadmap,
            [EndpointSnapshot::Product(_), EndpointSnapshot::Roadmap(_)],
            None,
        )
        | (
            RelationshipKind::ProductKpi,
            [EndpointSnapshot::Product(_), EndpointSnapshot::Kpi(_)],
            None,
        )
        | (
            RelationshipKind::InitiativeProject,
            [EndpointSnapshot::Initiative(_), EndpointSnapshot::Project(_)],
            None,
        )
        | (
            RelationshipKind::ProjectProduct,
            [EndpointSnapshot::Project(_), EndpointSnapshot::Product(_)],
            None,
        ) => true,
        (
            RelationshipKind::StakeholderSubject,
            [EndpointSnapshot::Stakeholder(_), subject],
            Some(_),
        ) => !matches!(subject, EndpointSnapshot::Stakeholder(_)),
        _ => false,
    }
}

fn persistence_identity(command: &RelationshipPersistenceCommand) -> Identity {
    match command {
        RelationshipPersistenceCommand::CreateStakeholder {
            id,
            name,
            kind,
            classification,
            provenance,
        } => Identity::CreateStakeholder(
            id.clone(),
            name.clone(),
            *kind,
            *classification,
            provenance.clone(),
        ),
        RelationshipPersistenceCommand::UpdateStakeholder {
            id,
            expected_version,
            name,
            classification,
        } => Identity::UpdateStakeholder(
            id.clone(),
            *expected_version,
            name.clone(),
            *classification,
        ),
        RelationshipPersistenceCommand::Link {
            id,
            kind,
            endpoints,
            expected_versions,
            purpose,
        } => Identity::Link(
            id.clone(),
            *kind,
            endpoints.iter().map(EndpointSnapshot::key).collect(),
            *purpose,
            expected_versions.clone(),
        ),
    }
}

fn h2b_persistence_identity(command: &RelationshipH2bPersistenceCommand) -> Identity {
    match command {
        RelationshipH2bPersistenceCommand::Prepare { relationship_id } => {
            Identity::PrepareRemove(relationship_id.clone())
        }
        RelationshipH2bPersistenceCommand::Cancel { prepared_id, actor } => {
            Identity::CancelRemove(prepared_id.clone(), *actor)
        }
        RelationshipH2bPersistenceCommand::Execute {
            prepared_id,
            acknowledged_payload_digest,
            confirmation_digest,
            actor,
        } => Identity::Remove(
            prepared_id.clone(),
            acknowledged_payload_digest.clone(),
            confirmation_digest.clone(),
            *actor,
        ),
    }
}

fn h2b_persistence_stored(result: &RelationshipH2bPersistenceResult) -> Stored {
    match result {
        RelationshipH2bPersistenceResult::Prepared(value) => Stored::Prepared(value.clone()),
        RelationshipH2bPersistenceResult::Cancelled => Stored::Cancelled,
        RelationshipH2bPersistenceResult::Removal(value) => Stored::Removal(value.clone()),
        RelationshipH2bPersistenceResult::Rejection(value) => Stored::Rejection(value.clone()),
    }
}

fn relationship_before_operation(
    snapshot: &RelationshipH2bPersistenceSnapshot,
    relationship_id: &RelationshipId,
    operation_ordinal: u64,
) -> Result<Option<RelationshipRecord>, RelationshipPersistenceError> {
    enum HistoricalOperation<'a> {
        Ordinary(&'a RelationshipReplayCapsule),
        H2b(&'a RelationshipH2bReplayCapsule),
    }

    let mut history = snapshot
        .ordinary_replay
        .iter()
        .filter(|capsule| capsule.operation_ordinal < operation_ordinal)
        .map(|capsule| {
            (
                capsule.operation_ordinal,
                HistoricalOperation::Ordinary(capsule),
            )
        })
        .chain(
            snapshot
                .h2b_replay
                .iter()
                .filter(|capsule| capsule.operation_ordinal < operation_ordinal)
                .map(|capsule| (capsule.operation_ordinal, HistoricalOperation::H2b(capsule))),
        )
        .collect::<Vec<_>>();
    history.sort_by_key(|value| value.0);

    let mut current = None;
    for (_, operation) in history {
        match operation {
            HistoricalOperation::Ordinary(capsule) => match (&capsule.command, &capsule.result) {
                (
                    RelationshipPersistenceCommand::Link { .. },
                    RelationshipPersistenceResult::Relationship(value),
                ) if value.outcome.effect_scope == AuditEffectScope::Complete
                    && value.value.id == *relationship_id =>
                {
                    current = Some(value.value.clone());
                }
                (
                    RelationshipPersistenceCommand::UpdateStakeholder { id, .. },
                    RelationshipPersistenceResult::Stakeholder(value),
                ) => {
                    if let Some(relationship) = current.as_mut() {
                        if relationship.endpoints.iter().any(|endpoint| {
                            matches!(endpoint, EndpointSnapshot::Stakeholder(snapshot) if snapshot.id() == id)
                        }) {
                            relationship.endpoints = relationship
                                .endpoints
                                .iter()
                                .map(|endpoint| match endpoint {
                                    EndpointSnapshot::Stakeholder(snapshot)
                                        if snapshot.id() == id =>
                                    {
                                        EndpointSnapshot::Stakeholder(StakeholderSnapshot::new(
                                            id.clone(),
                                            value.value.version,
                                            value.value.classification,
                                        ))
                                    }
                                    other => other.clone(),
                                })
                                .collect();
                            relationship.classification = relationship.endpoints.iter().fold(
                                DataClassification::Public,
                                |classification, endpoint| {
                                    classification.combine(endpoint.classification())
                                },
                            );
                            relationship.version = relationship
                                .version
                                .next()
                                .ok_or(RelationshipPersistenceError::InvalidRemovalState)?;
                            relationship.updated_at = value.value.updated_at;
                        }
                    }
                }
                _ => {}
            },
            HistoricalOperation::H2b(capsule) => {
                if matches!(
                    &capsule.result,
                    RelationshipH2bPersistenceResult::Removal(outcome)
                        if outcome.relationship_id == *relationship_id
                ) {
                    current = None;
                }
            }
        }
    }
    Ok(current)
}

fn validate_prepared_removal(
    snapshot: &RelationshipH2bPersistenceSnapshot,
    capsule: &RelationshipH2bReplayCapsule,
    relationship_id: &RelationshipId,
    prepared: &PreparedIntent,
) -> Result<(), RelationshipPersistenceError> {
    let preview = prepared.preview();
    let relationship =
        relationship_before_operation(snapshot, relationship_id, capsule.operation_ordinal)?
            .ok_or(RelationshipPersistenceError::InvalidRemovalState)?;
    let expected_effects = [
        RemovalEffect::RemoveRelationshipRecord,
        RemovalEffect::RemoveSemanticRelationshipIndex,
        RemovalEffect::CreateIdempotencyTombstone,
    ];
    if prepared.relationship_id() != relationship_id
        || prepared.payload_digest() != &PayloadDigest::for_preview(preview)
        || preview.intent_type() != REMOVE_RELATIONSHIP_INTENT_TYPE
        || preview.intent_version() != REMOVE_RELATIONSHIP_INTENT_VERSION
        || preview.relationship_version() != relationship.version
        || preview.endpoints() != relationship.endpoints
        || preview.kind() != relationship.kind
        || preview.purpose() != relationship.purpose
        || preview.effects() != expected_effects
        || preview.classification() != relationship.classification
        || preview.classification() == DataClassification::Unclassified
        || preview.policy_decision() != RemovalPolicyDecision::Allowed
        || preview.cancellation_policy() != CancellationPolicy::NotCancellableAfterSubmit
        || preview.confirmation_challenge() != format!("REMOVE {}", prepared.id())
        || preview.evidence().relationship_id() != relationship_id
        || !preview.evidence().compatible()
        || preview.evidence().verified_at().unix_millis() <= 0
    {
        return Err(RelationshipPersistenceError::InvalidRemovalState);
    }
    Ok(())
}

fn validate_h2b_snapshot(
    snapshot: &RelationshipH2bPersistenceSnapshot,
) -> Result<(), RelationshipPersistenceError> {
    let ordinary_audit_ids = snapshot
        .ordinary_replay
        .iter()
        .flat_map(|value| value.audit_event_ids.iter())
        .collect::<HashSet<_>>();
    let ordinary_audits = snapshot
        .audits
        .iter()
        .filter(|value| ordinary_audit_ids.contains(value.id()))
        .cloned()
        .collect::<Vec<_>>();
    let mut normalized_ordinary = snapshot.ordinary_replay.clone();
    normalized_ordinary.sort_by_key(|value| value.operation_ordinal);
    for (index, capsule) in normalized_ordinary.iter_mut().enumerate() {
        capsule.operation_ordinal = index as u64 + 1;
    }
    RelationshipPersistenceSnapshot::validate(
        snapshot.stakeholders.clone(),
        snapshot.ordinary_history_relationships.clone(),
        normalized_ordinary,
        ordinary_audits,
    )?;

    let mut operations = snapshot
        .ordinary_replay
        .iter()
        .map(|value| {
            (
                value.operation_ordinal,
                &value.idempotency_id,
                value.audit_event_ids.as_slice(),
            )
        })
        .chain(snapshot.h2b_replay.iter().map(|value| {
            (
                value.operation_ordinal,
                &value.idempotency_id,
                value.audit_event_ids.as_slice(),
            )
        }))
        .collect::<Vec<_>>();
    operations.sort_by_key(|value| value.0);
    if operations
        .iter()
        .enumerate()
        .any(|(index, value)| value.0 != index as u64 + 1)
        || operations
            .iter()
            .map(|value| value.1)
            .collect::<HashSet<_>>()
            .len()
            != operations.len()
        || operations
            .iter()
            .flat_map(|value| value.2.iter())
            .ne(snapshot.audits.iter().map(AuditEvent::id))
        || snapshot
            .audits
            .iter()
            .map(AuditEvent::id)
            .collect::<HashSet<_>>()
            .len()
            != snapshot.audits.len()
    {
        return Err(RelationshipPersistenceError::InvalidOperationOrder);
    }

    let mut prepared_by_id = HashMap::new();
    let mut cancelled = HashSet::new();
    let mut removed = HashMap::new();
    let mut h2b_replay = snapshot.h2b_replay.iter().collect::<Vec<_>>();
    h2b_replay.sort_by_key(|capsule| capsule.operation_ordinal);
    for capsule in h2b_replay {
        match (&capsule.command, &capsule.result) {
            (
                RelationshipH2bPersistenceCommand::Prepare { relationship_id },
                RelationshipH2bPersistenceResult::Prepared(prepared),
            ) => {
                if validate_prepared_removal(snapshot, capsule, relationship_id, prepared).is_err()
                    || !capsule.audit_event_ids.is_empty()
                    || prepared_by_id
                        .insert(prepared.id().clone(), prepared.clone())
                        .is_some()
                {
                    return Err(RelationshipPersistenceError::InvalidRemovalState);
                }
            }
            (
                RelationshipH2bPersistenceCommand::Cancel { prepared_id, actor },
                RelationshipH2bPersistenceResult::Cancelled,
            ) => {
                if *actor != AuditActor::HeadOfProducts
                    || !prepared_by_id.contains_key(prepared_id)
                    || !cancelled.insert(prepared_id.clone())
                    || capsule.audit_event_ids.len() != 1
                {
                    return Err(RelationshipPersistenceError::InvalidRemovalState);
                }
            }
            (
                RelationshipH2bPersistenceCommand::Execute {
                    prepared_id,
                    acknowledged_payload_digest,
                    confirmation_digest,
                    actor,
                },
                RelationshipH2bPersistenceResult::Removal(outcome),
            ) => {
                let prepared = prepared_by_id
                    .get(prepared_id)
                    .ok_or(RelationshipPersistenceError::InvalidRemovalState)?;
                if *actor != AuditActor::HeadOfProducts
                    || cancelled.contains(prepared_id)
                    || acknowledged_payload_digest != prepared.payload_digest()
                    || confirmation_digest
                        != &PayloadDigest::for_confirmation(prepared.confirmation_challenge())
                    || outcome.relationship_id != *prepared.relationship_id()
                    || outcome.audit_event_ids != capsule.audit_event_ids
                    || capsule.audit_event_ids.len() != 1
                    || removed
                        .insert(prepared_id.clone(), outcome.relationship_id.clone())
                        .is_some()
                {
                    return Err(RelationshipPersistenceError::InvalidRemovalState);
                }
            }
            (
                RelationshipH2bPersistenceCommand::Execute { prepared_id, .. },
                RelationshipH2bPersistenceResult::Rejection(error),
            ) => {
                if !prepared_by_id.contains_key(prepared_id)
                    || error.correlation_id() != &capsule.correlation_id
                    || capsule.audit_event_ids.len() != 1
                {
                    return Err(RelationshipPersistenceError::InvalidRemovalState);
                }
            }
            _ => return Err(RelationshipPersistenceError::InvalidRemovalState),
        }
        for audit_id in &capsule.audit_event_ids {
            let audit = snapshot
                .audits
                .iter()
                .find(|value| value.id() == audit_id)
                .ok_or(RelationshipPersistenceError::AuditMismatch)?;
            let prepared = match &capsule.command {
                RelationshipH2bPersistenceCommand::Prepare { .. } => None,
                RelationshipH2bPersistenceCommand::Cancel { prepared_id, .. }
                | RelationshipH2bPersistenceCommand::Execute { prepared_id, .. } => {
                    prepared_by_id.get(prepared_id)
                }
            };
            let actor = match &capsule.command {
                RelationshipH2bPersistenceCommand::Prepare { .. } => AuditActor::HeadOfProducts,
                RelationshipH2bPersistenceCommand::Cancel { actor, .. }
                | RelationshipH2bPersistenceCommand::Execute { actor, .. } => *actor,
            };
            if audit.correlation_id() != &capsule.correlation_id
                || audit.module() != AuditModule::Execution
                || audit.actor() != actor
                || prepared.is_some_and(|value| {
                    audit.target() != &AuditTarget::Relationship(value.relationship_id().clone())
                })
            {
                return Err(RelationshipPersistenceError::AuditMismatch);
            }
            match &capsule.result {
                RelationshipH2bPersistenceResult::Cancelled
                    if audit.code().as_str()
                        == "relationship.removal.cancelled_before_approval"
                        && audit.policy_outcome() == AuditPolicyOutcome::Allowed
                        && audit.approval_outcome() == AuditApprovalOutcome::NotRequired
                        && audit.execution_outcome() == AuditExecutionOutcome::Cancelled
                        && audit.effect_scope() == AuditEffectScope::None => {}
                RelationshipH2bPersistenceResult::Removal(_)
                    if audit.code().as_str() == "relationship.removed"
                        && audit.policy_outcome() == AuditPolicyOutcome::Allowed
                        && audit.approval_outcome() == AuditApprovalOutcome::Approved
                        && audit.execution_outcome() == AuditExecutionOutcome::Succeeded
                        && audit.effect_scope() == AuditEffectScope::Complete
                        && audit.actual_effects().len() == 1
                        && audit.actual_effects()[0].as_str()
                            == "relationship.authoritative-record-removed" => {}
                RelationshipH2bPersistenceResult::Rejection(_)
                    if matches!(
                        audit.code().as_str(),
                        "relationship.removal.approval_rejected"
                            | "relationship.removal.execution_rejected"
                            | "relationship.removal.policy_denied"
                    ) && audit.effect_scope() == AuditEffectScope::None
                        && audit.actual_effects().is_empty() => {}
                _ => return Err(RelationshipPersistenceError::AuditMismatch),
            }
        }
    }

    let pending = snapshot
        .pending
        .iter()
        .map(|value| value.id())
        .collect::<HashSet<_>>();
    let expected_pending = prepared_by_id
        .keys()
        .filter(|id| !cancelled.contains(*id) && !removed.contains_key(*id))
        .collect::<HashSet<_>>();
    let completed = snapshot.completed.iter().collect::<HashSet<_>>();
    if pending != expected_pending
        || completed != removed.keys().collect::<HashSet<_>>()
        || snapshot.completed.len() != completed.len()
        || snapshot
            .pending
            .iter()
            .any(|value| prepared_by_id.get(value.id()) != Some(value))
    {
        return Err(RelationshipPersistenceError::InvalidRemovalState);
    }
    let removed_relationships = removed.values().collect::<HashSet<_>>();
    let expected_current = snapshot
        .ordinary_history_relationships
        .iter()
        .filter(|value| !removed_relationships.contains(&value.id))
        .collect::<Vec<_>>();
    if expected_current.len() != snapshot.relationships.len()
        || snapshot
            .relationships
            .iter()
            .any(|value| !expected_current.contains(&value))
    {
        return Err(RelationshipPersistenceError::InvalidRemovalState);
    }
    let expected_tombstones = snapshot
        .ordinary_replay
        .iter()
        .filter_map(|capsule| match &capsule.result {
            RelationshipPersistenceResult::Relationship(value)
                if removed_relationships.contains(&value.value.id) =>
            {
                Some(&capsule.idempotency_id)
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    if snapshot.tombstoned_ordinary.iter().collect::<HashSet<_>>() != expected_tombstones
        || snapshot.tombstoned_ordinary.len() != expected_tombstones.len()
    {
        return Err(RelationshipPersistenceError::InvalidRemovalState);
    }
    let expected_h2b_tombstones = snapshot
        .h2b_replay
        .iter()
        .filter_map(|capsule| match (&capsule.command, &capsule.result) {
            (
                RelationshipH2bPersistenceCommand::Prepare { .. },
                RelationshipH2bPersistenceResult::Prepared(prepared),
            ) if removed.contains_key(prepared.id()) => Some(&capsule.idempotency_id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    if snapshot.tombstoned_h2b.iter().collect::<HashSet<_>>() != expected_h2b_tombstones
        || snapshot.tombstoned_h2b.len() != expected_h2b_tombstones.len()
    {
        return Err(RelationshipPersistenceError::InvalidRemovalState);
    }
    Ok(())
}

fn command_matches_result(
    command: &RelationshipPersistenceCommand,
    result: &RelationshipPersistenceResult,
) -> bool {
    match (command, result) {
        (
            RelationshipPersistenceCommand::CreateStakeholder {
                id,
                name,
                kind,
                classification,
                provenance,
            },
            RelationshipPersistenceResult::Stakeholder(value),
        ) => {
            value.value.id == *id
                && value.value.name == *name
                && value.value.kind == *kind
                && value.value.classification == classification.unwrap_or_default()
                && value.value.provenance == *provenance
                && value.value.version == AggregateVersion::initial()
                && value.value.created_at == value.value.updated_at
        }
        (
            RelationshipPersistenceCommand::UpdateStakeholder {
                id,
                expected_version,
                name,
                classification,
            },
            RelationshipPersistenceResult::Stakeholder(value),
        ) => {
            value.value.id == *id
                && value.value.name == *name
                && value.value.version.get() == expected_version.get().saturating_add(1)
                && classification.is_none_or(|requested| value.value.classification == requested)
        }
        (
            RelationshipPersistenceCommand::Link {
                id,
                kind,
                endpoints,
                expected_versions,
                purpose,
            },
            RelationshipPersistenceResult::Relationship(value),
        ) => {
            (value.outcome.effect_scope == AuditEffectScope::None || value.value.id == *id)
                && value.value.kind == *kind
                && value.value.endpoints == *endpoints
                && value.value.purpose == *purpose
                && endpoints
                    .iter()
                    .map(EndpointSnapshot::version)
                    .eq(expected_versions.iter().copied())
                && value.value.version == AggregateVersion::initial()
                && value.value.created_at == value.value.updated_at
        }
        _ => false,
    }
}

fn retain_ordinary_replay(
    state: &mut State,
    context: &OperationContext,
    command: RelationshipPersistenceCommand,
    result: RelationshipPersistenceResult,
) -> Result<(), DomainError> {
    state.ordinary_latest_relationships = state.relationships.clone();
    let ordinal = state.next_operation_ordinal;
    state.next_operation_ordinal = ordinal
        .checked_add(1)
        .ok_or_else(|| internal(&context.correlation_id))?;
    state.ordinary_replay.insert(
        context.idempotency_id.clone(),
        RelationshipReplayCapsule {
            idempotency_id: context.idempotency_id.clone(),
            correlation_id: context.correlation_id.clone(),
            operation_ordinal: ordinal,
            audit_event_ids: result_audit_ids(&result),
            command,
            result,
        },
    );
    Ok(())
}

fn retain_h2b_replay(
    state: &mut State,
    context: &OperationContext,
    command: RelationshipH2bPersistenceCommand,
    result: RelationshipH2bPersistenceResult,
    audit_event_ids: Vec<AuditEventId>,
) -> Result<(), DomainError> {
    let ordinal = state.next_operation_ordinal;
    state.next_operation_ordinal = ordinal
        .checked_add(1)
        .ok_or_else(|| internal(&context.correlation_id))?;
    state.h2b_replay.insert(
        context.idempotency_id.clone(),
        RelationshipH2bReplayCapsule {
            idempotency_id: context.idempotency_id.clone(),
            correlation_id: context.correlation_id.clone(),
            operation_ordinal: ordinal,
            command,
            result,
            audit_event_ids,
        },
    );
    Ok(())
}
fn relationship_code(kind: RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::PortfolioProduct => "relationship.portfolio_product.linked",
        RelationshipKind::PortfolioInitiative => "relationship.portfolio_initiative.linked",
        RelationshipKind::ProductRoadmap => "relationship.product_roadmap.linked",
        RelationshipKind::ProductKpi => "relationship.product_kpi.linked",
        RelationshipKind::InitiativeProject => "relationship.initiative_project.linked",
        RelationshipKind::ProjectProduct => "relationship.project_product.linked",
        RelationshipKind::StakeholderSubject => "relationship.stakeholder_subject.linked",
    }
}
fn append_audit<A: AuditEventIdSource>(
    ids: &mut A,
    state: &mut State,
    c: &OperationContext,
    target: AuditTarget,
    code: &str,
    now: UtcTimestamp,
) -> Result<AuditEventId, DomainError> {
    let id = ids
        .next_audit_event_id()
        .map_err(|_| internal(&c.correlation_id))?;
    if state.audits.iter().any(|v| v.id() == &id) {
        return Err(conflict(&c.correlation_id));
    }
    let event_code = AuditEventCode::parse(code).map_err(|_| internal(&c.correlation_id))?;
    let effect = AuditEffectCode::parse("relationship.authoritative-record-changed")
        .map_err(|_| internal(&c.correlation_id))?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::NotRequired,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect],
    )
    .map_err(|_| internal(&c.correlation_id))?;
    state.audits.push(AuditEvent::new(
        id.clone(),
        now,
        AuditActor::HeadOfProducts,
        AuditAction::new(AuditModule::Portfolio, event_code, target),
        c.correlation_id.clone(),
        disposition,
    ));
    Ok(id)
}

fn append_removal_audit<A: AuditEventIdSource>(
    ids: &mut A,
    state: &mut State,
    c: &OperationContext,
    relationship_id: RelationshipId,
    now: UtcTimestamp,
    actor: AuditActor,
) -> Result<AuditEventId, DomainError> {
    let id = ids
        .next_audit_event_id()
        .map_err(|_| internal(&c.correlation_id))?;
    if state.audits.iter().any(|value| value.id() == &id) {
        return Err(conflict(&c.correlation_id));
    }
    let code =
        AuditEventCode::parse("relationship.removed").map_err(|_| internal(&c.correlation_id))?;
    let effect = AuditEffectCode::parse("relationship.authoritative-record-removed")
        .map_err(|_| internal(&c.correlation_id))?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect],
    )
    .map_err(|_| internal(&c.correlation_id))?;
    state.audits.push(AuditEvent::new(
        id.clone(),
        now,
        actor,
        AuditAction::new(
            AuditModule::Execution,
            code,
            AuditTarget::Relationship(relationship_id),
        ),
        c.correlation_id.clone(),
        disposition,
    ));
    Ok(id)
}
fn append_cancellation_audit<A: AuditEventIdSource>(
    ids: &mut A,
    state: &mut State,
    c: &OperationContext,
    relationship_id: RelationshipId,
    now: UtcTimestamp,
    actor: AuditActor,
) -> Result<AuditEventId, DomainError> {
    let id = ids
        .next_audit_event_id()
        .map_err(|_| internal(&c.correlation_id))?;
    if state.audits.iter().any(|value| value.id() == &id) {
        return Err(conflict(&c.correlation_id));
    }
    let code = AuditEventCode::parse("relationship.removal.cancelled_before_approval")
        .map_err(|_| internal(&c.correlation_id))?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Cancelled,
        AuditEffectScope::None,
        Vec::new(),
    )
    .map_err(|_| internal(&c.correlation_id))?;
    state.audits.push(AuditEvent::new(
        id.clone(),
        now,
        actor,
        AuditAction::new(
            AuditModule::Execution,
            code,
            AuditTarget::Relationship(relationship_id),
        ),
        c.correlation_id.clone(),
        disposition,
    ));
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn append_removal_rejection_audit<A: AuditEventIdSource>(
    ids: &mut A,
    state: &mut State,
    context: &OperationContext,
    relationship_id: RelationshipId,
    now: UtcTimestamp,
    actor: AuditActor,
    code: &str,
    policy_outcome: AuditPolicyOutcome,
    approval_outcome: AuditApprovalOutcome,
    execution_outcome: AuditExecutionOutcome,
) -> Result<AuditEventId, DomainError> {
    let id = ids
        .next_audit_event_id()
        .map_err(|_| internal(&context.correlation_id))?;
    if state.audits.iter().any(|value| value.id() == &id) {
        return Err(internal(&context.correlation_id));
    }
    let event_code = AuditEventCode::parse(code).map_err(|_| internal(&context.correlation_id))?;
    let disposition = AuditDisposition::new(
        policy_outcome,
        approval_outcome,
        execution_outcome,
        AuditEffectScope::None,
        Vec::new(),
    )
    .map_err(|_| internal(&context.correlation_id))?;
    state.audits.push(AuditEvent::new(
        id.clone(),
        now,
        actor,
        AuditAction::new(
            AuditModule::Execution,
            event_code,
            AuditTarget::Relationship(relationship_id),
        ),
        context.correlation_id.clone(),
        disposition,
    ));
    Ok(id)
}
fn error(code: ErrorCode, key: &str, id: &CorrelationId) -> DomainError {
    let key = match MessageKey::parse(key) {
        Ok(v) => v,
        Err(_) => unreachable!("static relationship message key"),
    };
    DomainError::new(
        code,
        key,
        id.clone(),
        matches!(code, ErrorCode::PlatformInternal),
    )
}
fn conflict(id: &CorrelationId) -> DomainError {
    error(ErrorCode::DomainConflict, "relationship.conflict", id)
}
fn too_late_to_cancel(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::DomainConflict,
        "relationship.removal.too_late_to_cancel",
        id,
    )
}
fn not_found(id: &CorrelationId) -> DomainError {
    error(ErrorCode::DomainNotFound, "relationship.not_found", id)
}
fn denied(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::SecurityPolicyDenied,
        "relationship.classification.unclassified_or_lowering_denied",
        id,
    )
}
fn preview_expired_or_changed(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::SecurityPreviewExpiredOrChanged,
        "relationship.removal.preview_expired_or_changed",
        id,
    )
}
fn authorization_denied(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.authorization_denied",
        id,
    )
}
fn policy_denied(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.policy_denied",
        id,
    )
}
fn confirmation_mismatch(id: &CorrelationId) -> DomainError {
    // DG0 has no separate confirmation error code. A mismatched named
    // confirmation is therefore a security-policy rejection with a precise,
    // operation-specific safe key and no echoed confirmation value.
    error(
        ErrorCode::SecurityPolicyDenied,
        "relationship.removal.confirmation_mismatch",
        id,
    )
}
fn idempotency_conflict(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::DomainIdempotencyConflict,
        "relationship.idempotency_conflict",
        id,
    )
}
fn internal(id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::PlatformInternal,
        "relationship.persistence_failed",
        id,
    )
}
