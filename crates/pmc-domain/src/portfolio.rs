// DomainError is the repository-wide safe error contract; keep it at the public seam.
#![allow(clippy::result_large_err)]

use std::collections::{HashMap, HashSet};

use crate::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
    AuditEffectScope, AuditEvent, AuditEventCode, AuditEventIdSource, AuditExecutionOutcome,
    AuditModule, AuditPolicyOutcome, AuditTarget,
};
use crate::classification::DataClassification;
use crate::error::{DomainError, ErrorCode, MessageKey, SafeErrorExtension};
use crate::identity::{
    AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, KpiId,
    KpiObservationId, PortfolioId, PreparedIntentId, ProductId, RoadmapId,
};
use crate::provenance::Provenance;
use crate::relationships::{
    EndpointSnapshot, KpiSnapshot, PortfolioSnapshot, ProductSnapshot, RoadmapSnapshot,
};
use crate::time::{Clock, UtcTimestamp};
use crate::value::BoundedText;
use crate::work_management::{
    validate_and_mint_work_management_h2a_receipt, ApprovalAuthorizationPort,
    WorkManagementApproval, WorkManagementAuthoritativeSnapshot, WorkManagementCurrentPolicy,
    WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
};

const MAX_SHORT_TEXT: usize = 160;
const MAX_LONG_TEXT: usize = 2_000;

pub type ShortText = BoundedText<MAX_SHORT_TEXT>;
pub type LongText = BoundedText<MAX_LONG_TEXT>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationContext {
    pub idempotency_id: IdempotencyId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationOutcome<T> {
    pub record: T,
    pub audit_event: AuditEvent,
}

macro_rules! simple_record {
    ($record:ident, $id:ty) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $record {
            pub id: $id,
            pub name: ShortText,
            pub details: LongText,
            pub classification: DataClassification,
            pub provenance: Provenance,
            pub version: AggregateVersion,
        }
    };
}

simple_record!(PortfolioRecord, PortfolioId);
simple_record!(ProductRecord, ProductId);
simple_record!(RoadmapRecord, RoadmapId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiDefinitionRecord {
    pub id: KpiId,
    pub name: ShortText,
    pub definition: LongText,
    pub owner: ShortText,
    pub target: ShortText,
    pub cadence: ShortText,
    pub source: LongText,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiObservationRecord {
    pub id: KpiObservationId,
    pub kpi_id: KpiId,
    pub value: ShortText,
    pub observed_at: UtcTimestamp,
    pub source: LongText,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

macro_rules! simple_intents {
    ($create:ident, $update:ident, $id:ty) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $create {
            pub id: $id,
            pub name: ShortText,
            pub details: LongText,
            pub classification: Option<DataClassification>,
            pub provenance: Provenance,
            pub context: OperationContext,
        }

        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $update {
            pub id: $id,
            pub expected_version: AggregateVersion,
            pub name: ShortText,
            pub details: LongText,
            pub classification: Option<DataClassification>,
            pub context: OperationContext,
        }
    };
}

simple_intents!(CreatePortfolio, UpdatePortfolioDetails, PortfolioId);
simple_intents!(CreateProduct, UpdateProductDetails, ProductId);
simple_intents!(CreateRoadmap, UpdateRoadmapDetails, RoadmapId);

// H2a "Lower Data Classification" for the Portfolio record type.
// This is the first of the nine classified aggregate families to gain the
// operation named in the frozen DG0 table (`PrepareLowerDataClassification`
// -> `ApproveAndExecuteLowerDataClassification`); Product/Roadmap/Kpi/
// KpiObservation and the other aggregate families follow in later slices,
// each mechanically extending the same shape.
//
// `InMemoryPortfolioService::new(clock, audit_ids)` is unchanged -- every
// existing caller (including `pmc-ledger`) is unaffected. The extra H2a
// capability (a prepared-intent ID source and an approval-authorization
// port) is threaded in as method-level generics on the two new methods
// below instead of struct-level ones, unlike Risk/Decision/Issue/Action,
// which bake those into the service's own constructor. This is a
// deliberate, narrower deviation chosen specifically to avoid a breaking
// constructor change on a service other crates already construct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerPortfolioClassification {
    pub portfolio_id: PortfolioId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerPortfolioClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}

/// Mints the two identifiers a Portfolio classification-lowering H2a
/// round trip needs. Mirrors `RiskServiceIdSource` and its siblings on the
/// other aggregate families, kept as its own trait (rather than reusing
/// theirs) because it is threaded in per-call here, not baked into the
/// service's own constructor generics.
///
/// Shared by all five Portfolio-family record types (Portfolio itself,
/// Product, Roadmap, Kpi, KpiObservation) -- one prepared-intent ID space,
/// disambiguated by the `WorkManagementOperation` variant it wraps, exactly
/// like the shared `outcomes` idempotency store already disambiguates by
/// `CommandIdentity`.
pub trait PortfolioClassificationLoweringIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, crate::DomainValueError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerProductClassification {
    pub product_id: ProductId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerProductClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerRoadmapClassification {
    pub roadmap_id: RoadmapId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerRoadmapClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerKpiClassification {
    pub kpi_id: KpiId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerKpiClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareLowerKpiObservationClassification {
    pub observation_id: KpiObservationId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveAndExecuteLowerKpiObservationClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateKpiDefinition {
    pub id: KpiId,
    pub name: ShortText,
    pub definition: LongText,
    pub owner: ShortText,
    pub target: ShortText,
    pub cadence: ShortText,
    pub source: LongText,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateKpiDefinitionDetails {
    pub id: KpiId,
    pub expected_version: AggregateVersion,
    pub name: ShortText,
    pub definition: LongText,
    pub owner: ShortText,
    pub target: ShortText,
    pub cadence: ShortText,
    pub source: LongText,
    pub classification: Option<DataClassification>,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateKpiObservation {
    pub id: KpiObservationId,
    pub kpi_id: KpiId,
    pub value: ShortText,
    pub observed_at: UtcTimestamp,
    pub source: LongText,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateKpiObservationDetails {
    pub id: KpiObservationId,
    pub expected_version: AggregateVersion,
    pub value: ShortText,
    pub observed_at: UtcTimestamp,
    pub source: LongText,
    pub classification: Option<DataClassification>,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub enum CommandIdentity {
    CreatePortfolio {
        id: PortfolioId,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdatePortfolio {
        id: PortfolioId,
        expected_version: AggregateVersion,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
    },
    CreateProduct {
        id: ProductId,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateProduct {
        id: ProductId,
        expected_version: AggregateVersion,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
    },
    CreateRoadmap {
        id: RoadmapId,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateRoadmap {
        id: RoadmapId,
        expected_version: AggregateVersion,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
    },
    CreateKpi {
        id: KpiId,
        name: ShortText,
        definition: LongText,
        owner: ShortText,
        target: ShortText,
        cadence: ShortText,
        source: LongText,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateKpi {
        id: KpiId,
        expected_version: AggregateVersion,
        name: ShortText,
        definition: LongText,
        owner: ShortText,
        target: ShortText,
        cadence: ShortText,
        source: LongText,
        classification: Option<DataClassification>,
    },
    CreateObservation {
        id: KpiObservationId,
        kpi_id: KpiId,
        value: ShortText,
        observed_at: UtcTimestamp,
        source: LongText,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateObservation {
        id: KpiObservationId,
        expected_version: AggregateVersion,
        value: ShortText,
        observed_at: UtcTimestamp,
        source: LongText,
        classification: Option<DataClassification>,
    },
    PrepareLowerPortfolioClassification {
        portfolio_id: PortfolioId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerPortfolioClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
    PrepareLowerProductClassification {
        product_id: ProductId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerProductClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
    PrepareLowerRoadmapClassification {
        roadmap_id: RoadmapId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerRoadmapClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
    PrepareLowerKpiClassification {
        kpi_id: KpiId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerKpiClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
    PrepareLowerKpiObservationClassification {
        observation_id: KpiObservationId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerKpiObservationClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredResult {
    Portfolio(MutationOutcome<PortfolioRecord>),
    Product(MutationOutcome<ProductRecord>),
    Roadmap(MutationOutcome<RoadmapRecord>),
    Kpi(MutationOutcome<KpiDefinitionRecord>),
    Observation(MutationOutcome<KpiObservationRecord>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredOutcome {
    command: CommandIdentity,
    result: StoredResult,
    correlation_id: CorrelationId,
    audit_event_ids: Vec<AuditEventId>,
    operation_ordinal: u64,
    derived_kpi_observation_mutations: Vec<DerivedKpiObservationMutation>,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedKpiObservationMutation {
    observation_id: KpiObservationId,
    previous_version: AggregateVersion,
    resulting_version: AggregateVersion,
    previous_classification: DataClassification,
    resulting_classification: DataClassification,
    previous_updated_at: UtcTimestamp,
    resulting_updated_at: UtcTimestamp,
    audit_event_id: AuditEventId,
}

impl DerivedKpiObservationMutation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        observation_id: KpiObservationId,
        previous_version: AggregateVersion,
        resulting_version: AggregateVersion,
        previous_classification: DataClassification,
        resulting_classification: DataClassification,
        previous_updated_at: UtcTimestamp,
        resulting_updated_at: UtcTimestamp,
        audit_event_id: AuditEventId,
    ) -> Self {
        Self {
            observation_id,
            previous_version,
            resulting_version,
            previous_classification,
            resulting_classification,
            previous_updated_at,
            resulting_updated_at,
            audit_event_id,
        }
    }
    pub fn observation_id(&self) -> &KpiObservationId {
        &self.observation_id
    }
    pub const fn previous_version(&self) -> AggregateVersion {
        self.previous_version
    }
    pub const fn resulting_version(&self) -> AggregateVersion {
        self.resulting_version
    }
    pub const fn previous_classification(&self) -> DataClassification {
        self.previous_classification
    }
    pub const fn resulting_classification(&self) -> DataClassification {
        self.resulting_classification
    }
    pub const fn previous_updated_at(&self) -> UtcTimestamp {
        self.previous_updated_at
    }
    pub const fn resulting_updated_at(&self) -> UtcTimestamp {
        self.resulting_updated_at
    }
    pub fn audit_event_id(&self) -> &AuditEventId {
        &self.audit_event_id
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortfolioPersistenceResult {
    Portfolio {
        outcome: MutationOutcome<PortfolioRecord>,
    },
    Product {
        outcome: MutationOutcome<ProductRecord>,
    },
    Roadmap {
        outcome: MutationOutcome<RoadmapRecord>,
    },
    KpiDefinition {
        outcome: MutationOutcome<KpiDefinitionRecord>,
    },
    KpiObservation {
        outcome: MutationOutcome<KpiObservationRecord>,
    },
}

impl From<StoredResult> for PortfolioPersistenceResult {
    fn from(value: StoredResult) -> Self {
        match value {
            StoredResult::Portfolio(outcome) => Self::Portfolio { outcome },
            StoredResult::Product(outcome) => Self::Product { outcome },
            StoredResult::Roadmap(outcome) => Self::Roadmap { outcome },
            StoredResult::Kpi(outcome) => Self::KpiDefinition { outcome },
            StoredResult::Observation(outcome) => Self::KpiObservation { outcome },
        }
    }
}

impl From<PortfolioPersistenceResult> for StoredResult {
    fn from(value: PortfolioPersistenceResult) -> Self {
        match value {
            PortfolioPersistenceResult::Portfolio { outcome } => Self::Portfolio(outcome),
            PortfolioPersistenceResult::Product { outcome } => Self::Product(outcome),
            PortfolioPersistenceResult::Roadmap { outcome } => Self::Roadmap(outcome),
            PortfolioPersistenceResult::KpiDefinition { outcome } => Self::Kpi(outcome),
            PortfolioPersistenceResult::KpiObservation { outcome } => Self::Observation(outcome),
        }
    }
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct PortfolioReplayCapsule {
    idempotency_id: IdempotencyId,
    command: CommandIdentity,
    result: PortfolioPersistenceResult,
    correlation_id: CorrelationId,
    audit_event_ids: Vec<AuditEventId>,
    operation_ordinal: u64,
    derived_kpi_observation_mutations: Vec<DerivedKpiObservationMutation>,
}

impl PortfolioReplayCapsule {
    pub fn new(
        idempotency_id: IdempotencyId,
        command: CommandIdentity,
        result: PortfolioPersistenceResult,
        correlation_id: CorrelationId,
        audit_event_ids: Vec<AuditEventId>,
        operation_ordinal: u64,
        derived_kpi_observation_mutations: Vec<DerivedKpiObservationMutation>,
    ) -> Self {
        Self {
            idempotency_id,
            command,
            result,
            correlation_id,
            audit_event_ids,
            operation_ordinal,
            derived_kpi_observation_mutations,
        }
    }
    pub fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }
    pub fn command(&self) -> &CommandIdentity {
        &self.command
    }
    pub fn result(&self) -> &PortfolioPersistenceResult {
        &self.result
    }
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }
    pub fn audit_event_ids(&self) -> &[AuditEventId] {
        &self.audit_event_ids
    }
    pub const fn operation_ordinal(&self) -> u64 {
        self.operation_ordinal
    }
    pub fn derived_kpi_observation_mutations(&self) -> &[DerivedKpiObservationMutation] {
        &self.derived_kpi_observation_mutations
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortfolioRehydrationError {
    DuplicateRecord,
    MissingParent,
    DuplicateAuditEvent,
    DuplicateIdempotency,
    ResultMismatch,
    VersionLineageMismatch,
    ClassificationMismatch,
    AuditMismatch,
    OrphanAuditEvent,
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct PortfolioPersistenceSnapshot {
    portfolios: Vec<PortfolioRecord>,
    products: Vec<ProductRecord>,
    roadmaps: Vec<RoadmapRecord>,
    kpi_definitions: Vec<KpiDefinitionRecord>,
    kpi_observations: Vec<KpiObservationRecord>,
    replay: Vec<PortfolioReplayCapsule>,
    audits: Vec<AuditEvent>,
}

impl PortfolioPersistenceSnapshot {
    pub fn portfolios(&self) -> &[PortfolioRecord] {
        &self.portfolios
    }
    pub fn products(&self) -> &[ProductRecord] {
        &self.products
    }
    pub fn roadmaps(&self) -> &[RoadmapRecord] {
        &self.roadmaps
    }
    pub fn kpi_definitions(&self) -> &[KpiDefinitionRecord] {
        &self.kpi_definitions
    }
    pub fn kpi_observations(&self) -> &[KpiObservationRecord] {
        &self.kpi_observations
    }
    pub fn replay(&self) -> &[PortfolioReplayCapsule] {
        &self.replay
    }
    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }

    #[allow(clippy::too_many_arguments)]
    pub fn validate(
        portfolios: Vec<PortfolioRecord>,
        products: Vec<ProductRecord>,
        roadmaps: Vec<RoadmapRecord>,
        kpi_definitions: Vec<KpiDefinitionRecord>,
        kpi_observations: Vec<KpiObservationRecord>,
        replay: Vec<PortfolioReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, PortfolioRehydrationError> {
        let portfolio_map = unique_records(portfolios.iter().map(|v| (&v.id, v)))?;
        let product_map = unique_records(products.iter().map(|v| (&v.id, v)))?;
        let roadmap_map = unique_records(roadmaps.iter().map(|v| (&v.id, v)))?;
        let kpi_map = unique_records(kpi_definitions.iter().map(|v| (&v.id, v)))?;
        let observation_map = unique_records(kpi_observations.iter().map(|v| (&v.id, v)))?;
        if kpi_observations
            .iter()
            .any(|v| !kpi_map.contains_key(&v.kpi_id))
        {
            return Err(PortfolioRehydrationError::MissingParent);
        }
        let mut audit_ids = HashSet::new();
        if audits.iter().any(|v| !audit_ids.insert(v.id().clone())) {
            return Err(PortfolioRehydrationError::DuplicateAuditEvent);
        }
        let mut idem_ids = HashSet::new();
        if replay
            .iter()
            .any(|v| !idem_ids.insert(v.idempotency_id.clone()))
        {
            return Err(PortfolioRehydrationError::DuplicateIdempotency);
        }
        validate_portfolio_timeline(
            &replay,
            &audits,
            &portfolio_map,
            &product_map,
            &roadmap_map,
            &kpi_map,
            &observation_map,
        )?;
        Ok(Self {
            portfolios,
            products,
            roadmaps,
            kpi_definitions,
            kpi_observations,
            replay,
            audits,
        })
    }
}

fn unique_records<'a, K: Eq + std::hash::Hash, V>(
    values: impl IntoIterator<Item = (&'a K, &'a V)>,
) -> Result<HashMap<&'a K, &'a V>, PortfolioRehydrationError> {
    let mut result = HashMap::new();
    for (id, value) in values {
        if result.insert(id, value).is_some() {
            return Err(PortfolioRehydrationError::DuplicateRecord);
        }
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn validate_portfolio_timeline(
    replay: &[PortfolioReplayCapsule],
    audits: &[AuditEvent],
    expected_portfolios: &HashMap<&PortfolioId, &PortfolioRecord>,
    expected_products: &HashMap<&ProductId, &ProductRecord>,
    expected_roadmaps: &HashMap<&RoadmapId, &RoadmapRecord>,
    expected_kpis: &HashMap<&KpiId, &KpiDefinitionRecord>,
    expected_observations: &HashMap<&KpiObservationId, &KpiObservationRecord>,
) -> Result<(), PortfolioRehydrationError> {
    let audit_map: HashMap<_, _> = audits.iter().map(|v| (v.id(), v)).collect();
    let mut claimed = HashSet::new();
    let mut ordered: Vec<_> = replay.iter().collect();
    ordered.sort_by_key(|v| v.operation_ordinal);
    if ordered
        .iter()
        .enumerate()
        .any(|(i, v)| v.operation_ordinal != i as u64)
    {
        return Err(PortfolioRehydrationError::VersionLineageMismatch);
    }
    let mut portfolios = HashMap::new();
    let mut products = HashMap::new();
    let mut roadmaps = HashMap::new();
    let mut kpis = HashMap::new();
    let mut observations: HashMap<KpiObservationId, KpiObservationRecord> = HashMap::new();
    let mut audit_cursor = 0;
    for capsule in ordered {
        if capsule.audit_event_ids.is_empty()
            || capsule.audit_event_ids.len() != capsule.derived_kpi_observation_mutations.len() + 1
        {
            return Err(PortfolioRehydrationError::AuditMismatch);
        }
        for (offset, id) in capsule.audit_event_ids.iter().enumerate() {
            if !claimed.insert(id.clone())
                || audits.get(audit_cursor + offset).map(AuditEvent::id) != Some(id)
                || !audit_map.contains_key(id)
            {
                return Err(PortfolioRehydrationError::AuditMismatch);
            }
        }
        let audit_id = &capsule.audit_event_ids[0];
        let persisted_audit = audit_map
            .get(audit_id)
            .ok_or(PortfolioRehydrationError::AuditMismatch)?;
        audit_cursor += capsule.audit_event_ids.len();
        if !matches!(capsule.command, CommandIdentity::UpdateKpi { .. })
            && !capsule.derived_kpi_observation_mutations.is_empty()
        {
            return Err(PortfolioRehydrationError::ResultMismatch);
        }
        let (outcome_audit, target, code) = match (&capsule.command, &capsule.result) {
            (
                CommandIdentity::CreatePortfolio {
                    id,
                    name,
                    details,
                    classification: class,
                    provenance,
                },
                PortfolioPersistenceResult::Portfolio { outcome },
            ) => {
                let v = &outcome.record;
                if &v.id != id
                    || &v.name != name
                    || &v.details != details
                    || v.classification != class.unwrap_or_default()
                    || &v.provenance != provenance
                    || v.version != AggregateVersion::initial()
                    || portfolios.insert(v.id.clone(), v.clone()).is_some()
                {
                    return Err(PortfolioRehydrationError::ResultMismatch);
                }
                (
                    &outcome.audit_event,
                    AuditTarget::Portfolio(id.clone()),
                    "portfolio.created",
                )
            }
            (
                CommandIdentity::UpdatePortfolio {
                    id,
                    expected_version: expected,
                    name,
                    details,
                    classification: class,
                },
                PortfolioPersistenceResult::Portfolio { outcome },
            ) => {
                let previous = portfolios
                    .get(id)
                    .ok_or(PortfolioRehydrationError::VersionLineageMismatch)?;
                let v = &outcome.record;
                validate_simple_update(previous, v, *expected, name, details, *class)?;
                portfolios.insert(id.clone(), v.clone());
                (
                    &outcome.audit_event,
                    AuditTarget::Portfolio(id.clone()),
                    "portfolio.updated",
                )
            }
            (
                CommandIdentity::CreateProduct {
                    id,
                    name,
                    details,
                    classification: class,
                    provenance,
                },
                PortfolioPersistenceResult::Product { outcome },
            ) => {
                let v = &outcome.record;
                if &v.id != id
                    || &v.name != name
                    || &v.details != details
                    || v.classification != class.unwrap_or_default()
                    || &v.provenance != provenance
                    || v.version != AggregateVersion::initial()
                    || products.insert(v.id.clone(), v.clone()).is_some()
                {
                    return Err(PortfolioRehydrationError::ResultMismatch);
                }
                (
                    &outcome.audit_event,
                    AuditTarget::Product(id.clone()),
                    "product.created",
                )
            }
            (
                CommandIdentity::UpdateProduct {
                    id,
                    expected_version: expected,
                    name,
                    details,
                    classification: class,
                },
                PortfolioPersistenceResult::Product { outcome },
            ) => {
                let previous = products
                    .get(id)
                    .ok_or(PortfolioRehydrationError::VersionLineageMismatch)?;
                let v = &outcome.record;
                validate_simple_update(previous, v, *expected, name, details, *class)?;
                products.insert(id.clone(), v.clone());
                (
                    &outcome.audit_event,
                    AuditTarget::Product(id.clone()),
                    "product.updated",
                )
            }
            (
                CommandIdentity::CreateRoadmap {
                    id,
                    name,
                    details,
                    classification: class,
                    provenance,
                },
                PortfolioPersistenceResult::Roadmap { outcome },
            ) => {
                let v = &outcome.record;
                if &v.id != id
                    || &v.name != name
                    || &v.details != details
                    || v.classification != class.unwrap_or_default()
                    || &v.provenance != provenance
                    || v.version != AggregateVersion::initial()
                    || roadmaps.insert(v.id.clone(), v.clone()).is_some()
                {
                    return Err(PortfolioRehydrationError::ResultMismatch);
                }
                (
                    &outcome.audit_event,
                    AuditTarget::Roadmap(id.clone()),
                    "roadmap.created",
                )
            }
            (
                CommandIdentity::UpdateRoadmap {
                    id,
                    expected_version: expected,
                    name,
                    details,
                    classification: class,
                },
                PortfolioPersistenceResult::Roadmap { outcome },
            ) => {
                let previous = roadmaps
                    .get(id)
                    .ok_or(PortfolioRehydrationError::VersionLineageMismatch)?;
                let v = &outcome.record;
                validate_simple_update(previous, v, *expected, name, details, *class)?;
                roadmaps.insert(id.clone(), v.clone());
                (
                    &outcome.audit_event,
                    AuditTarget::Roadmap(id.clone()),
                    "roadmap.updated",
                )
            }
            (
                CommandIdentity::CreateKpi {
                    id,
                    name,
                    definition,
                    owner,
                    target,
                    cadence,
                    source,
                    classification: class,
                    provenance,
                },
                PortfolioPersistenceResult::KpiDefinition { outcome },
            ) => {
                let v = &outcome.record;
                if &v.id != id
                    || &v.name != name
                    || &v.definition != definition
                    || &v.owner != owner
                    || &v.target != target
                    || &v.cadence != cadence
                    || &v.source != source
                    || v.classification != class.unwrap_or_default()
                    || &v.provenance != provenance
                    || v.version != AggregateVersion::initial()
                    || kpis.insert(v.id.clone(), v.clone()).is_some()
                {
                    return Err(PortfolioRehydrationError::ResultMismatch);
                }
                (
                    &outcome.audit_event,
                    AuditTarget::Kpi(id.clone()),
                    "kpi.definition.created",
                )
            }
            (
                CommandIdentity::UpdateKpi {
                    id,
                    expected_version: expected,
                    name,
                    definition,
                    owner,
                    target,
                    cadence,
                    source,
                    classification: class,
                },
                PortfolioPersistenceResult::KpiDefinition { outcome },
            ) => {
                let previous = kpis
                    .get(id)
                    .ok_or(PortfolioRehydrationError::VersionLineageMismatch)?;
                let v = &outcome.record;
                if previous.version != *expected
                    || expected.next() != Some(v.version)
                    || &v.id != id
                    || &v.name != name
                    || &v.definition != definition
                    || &v.owner != owner
                    || &v.target != target
                    || &v.cadence != cadence
                    || &v.source != source
                    || v.provenance != previous.provenance
                    || previous.classification.combine(v.classification) != v.classification
                    || !class.map_or(v.classification == previous.classification, |c| {
                        c == v.classification
                    })
                {
                    return Err(PortfolioRehydrationError::ResultMismatch);
                }
                let mut affected_ids: Vec<_> = observations
                    .values()
                    .filter(|observation| {
                        observation.kpi_id == *id
                            && v.classification.combine(observation.classification)
                                != observation.classification
                    })
                    .map(|observation| observation.id.clone())
                    .collect();
                affected_ids.sort();
                if affected_ids.len() != capsule.derived_kpi_observation_mutations.len() {
                    return Err(PortfolioRehydrationError::ClassificationMismatch);
                }
                for (index, (observation_id, mutation)) in affected_ids
                    .into_iter()
                    .zip(&capsule.derived_kpi_observation_mutations)
                    .enumerate()
                {
                    let observation = observations
                        .get_mut(&observation_id)
                        .ok_or(PortfolioRehydrationError::MissingParent)?;
                    let expected_version = observation
                        .version
                        .next()
                        .ok_or(PortfolioRehydrationError::VersionLineageMismatch)?;
                    let expected_classification =
                        v.classification.combine(observation.classification);
                    let derived_audit_id = capsule
                        .audit_event_ids
                        .get(index + 1)
                        .ok_or(PortfolioRehydrationError::AuditMismatch)?;
                    let derived_audit = audit_map
                        .get(derived_audit_id)
                        .ok_or(PortfolioRehydrationError::AuditMismatch)?;
                    if mutation.observation_id != observation_id
                        || mutation.previous_version != observation.version
                        || mutation.resulting_version != expected_version
                        || mutation.previous_classification != observation.classification
                        || mutation.resulting_classification != expected_classification
                        || mutation.previous_updated_at != observation.updated_at
                        || mutation.resulting_updated_at
                            != std::cmp::max(
                                outcome.audit_event.occurred_at(),
                                observation.updated_at,
                            )
                        || mutation.audit_event_id != *derived_audit_id
                        || derived_audit.occurred_at() != outcome.audit_event.occurred_at()
                        || derived_audit.actor() != AuditActor::HeadOfProducts
                        || derived_audit.module() != AuditModule::Portfolio
                        || derived_audit.code().as_str()
                            != "kpi.observation.classification.inherited"
                        || derived_audit.target()
                            != &AuditTarget::KpiObservation(observation_id.clone())
                        || derived_audit.correlation_id() != &capsule.correlation_id
                        || derived_audit.policy_outcome() != AuditPolicyOutcome::NotRequired
                        || derived_audit.approval_outcome() != AuditApprovalOutcome::NotRequired
                        || derived_audit.execution_outcome() != AuditExecutionOutcome::Succeeded
                        || derived_audit.effect_scope() != AuditEffectScope::Complete
                        || derived_audit.actual_effects().len() != 1
                        || derived_audit.actual_effects()[0].as_str() != "portfolio-record-mutated"
                    {
                        return Err(PortfolioRehydrationError::AuditMismatch);
                    }
                    observation.version = expected_version;
                    observation.classification = expected_classification;
                    observation.updated_at = mutation.resulting_updated_at;
                }
                kpis.insert(id.clone(), v.clone());
                (
                    &outcome.audit_event,
                    AuditTarget::Kpi(id.clone()),
                    "kpi.definition.updated",
                )
            }
            (
                CommandIdentity::CreateObservation {
                    id,
                    kpi_id,
                    value,
                    observed_at,
                    source,
                    classification: class,
                    provenance,
                },
                PortfolioPersistenceResult::KpiObservation { outcome },
            ) => {
                let parent = kpis
                    .get(kpi_id)
                    .ok_or(PortfolioRehydrationError::MissingParent)?;
                let v = &outcome.record;
                let expected_class =
                    class.map_or(parent.classification, |c| c.combine(parent.classification));
                if &v.id != id
                    || &v.kpi_id != kpi_id
                    || &v.value != value
                    || &v.observed_at != observed_at
                    || &v.source != source
                    || v.classification != expected_class
                    || &v.provenance != provenance
                    || v.version != AggregateVersion::initial()
                    || v.created_at != v.updated_at
                    || v.created_at != outcome.audit_event.occurred_at()
                    || observations.insert(v.id.clone(), v.clone()).is_some()
                {
                    return Err(PortfolioRehydrationError::ResultMismatch);
                }
                (
                    &outcome.audit_event,
                    AuditTarget::KpiObservation(id.clone()),
                    "kpi.observation.created",
                )
            }
            (
                CommandIdentity::UpdateObservation {
                    id,
                    expected_version: expected,
                    value,
                    observed_at,
                    source,
                    classification: class,
                },
                PortfolioPersistenceResult::KpiObservation { outcome },
            ) => {
                let previous = observations
                    .get(id)
                    .ok_or(PortfolioRehydrationError::VersionLineageMismatch)?;
                let parent = kpis
                    .get(&previous.kpi_id)
                    .ok_or(PortfolioRehydrationError::MissingParent)?;
                let v = &outcome.record;
                let requested = class.map_or(previous.classification, |c| c);
                if previous.version != *expected
                    || expected.next() != Some(v.version)
                    || &v.id != id
                    || v.kpi_id != previous.kpi_id
                    || &v.value != value
                    || &v.observed_at != observed_at
                    || &v.source != source
                    || v.provenance != previous.provenance
                    || v.created_at != previous.created_at
                    || v.updated_at
                        != std::cmp::max(outcome.audit_event.occurred_at(), previous.updated_at)
                    || previous.classification.combine(requested) != requested
                    || v.classification != requested.combine(parent.classification)
                {
                    return Err(PortfolioRehydrationError::ClassificationMismatch);
                }
                observations.insert(id.clone(), v.clone());
                (
                    &outcome.audit_event,
                    AuditTarget::KpiObservation(id.clone()),
                    "kpi.observation.updated",
                )
            }
            _ => return Err(PortfolioRehydrationError::ResultMismatch),
        };
        if outcome_audit != *persisted_audit
            || outcome_audit.id() != audit_id
            || outcome_audit.occurred_at() != persisted_audit.occurred_at()
            || outcome_audit.actor() != AuditActor::HeadOfProducts
            || outcome_audit.module() != AuditModule::Portfolio
            || outcome_audit.code().as_str() != code
            || outcome_audit.target() != &target
            || outcome_audit.correlation_id() != &capsule.correlation_id
            || outcome_audit.policy_outcome() != AuditPolicyOutcome::NotRequired
            || outcome_audit.approval_outcome() != AuditApprovalOutcome::NotRequired
            || outcome_audit.execution_outcome() != AuditExecutionOutcome::Succeeded
            || outcome_audit.effect_scope() != AuditEffectScope::Complete
            || outcome_audit.actual_effects().len() != 1
            || outcome_audit.actual_effects()[0].as_str() != "portfolio-record-mutated"
        {
            return Err(PortfolioRehydrationError::AuditMismatch);
        }
    }
    if claimed.len() != audits.len() {
        return Err(PortfolioRehydrationError::OrphanAuditEvent);
    }
    if !maps_equal(&portfolios, expected_portfolios)
        || !maps_equal(&products, expected_products)
        || !maps_equal(&roadmaps, expected_roadmaps)
        || !maps_equal(&kpis, expected_kpis)
        || !maps_equal(&observations, expected_observations)
    {
        return Err(PortfolioRehydrationError::VersionLineageMismatch);
    }
    Ok(())
}

fn validate_simple_update<I: Eq, R>(
    previous: &R,
    value: &R,
    expected: AggregateVersion,
    name: &ShortText,
    details: &LongText,
    class: Option<DataClassification>,
) -> Result<(), PortfolioRehydrationError>
where
    R: SimplePersistenceRecord<Id = I>,
{
    if previous.version() != expected
        || expected.next() != Some(value.version())
        || value.id() != previous.id()
        || value.name() != name
        || value.details() != details
        || value.provenance() != previous.provenance()
        || previous.classification().combine(value.classification()) != value.classification()
        || !class.map_or(value.classification() == previous.classification(), |c| {
            c == value.classification()
        })
    {
        return Err(PortfolioRehydrationError::ResultMismatch);
    }
    Ok(())
}

trait SimplePersistenceRecord {
    type Id: Eq;
    fn id(&self) -> &Self::Id;
    fn name(&self) -> &ShortText;
    fn details(&self) -> &LongText;
    fn classification(&self) -> DataClassification;
    fn provenance(&self) -> &Provenance;
    fn version(&self) -> AggregateVersion;
}
macro_rules! simple_persistence_record {
    ($t:ty, $id:ty) => {
        impl SimplePersistenceRecord for $t {
            type Id = $id;
            fn id(&self) -> &$id {
                &self.id
            }
            fn name(&self) -> &ShortText {
                &self.name
            }
            fn details(&self) -> &LongText {
                &self.details
            }
            fn classification(&self) -> DataClassification {
                self.classification
            }
            fn provenance(&self) -> &Provenance {
                &self.provenance
            }
            fn version(&self) -> AggregateVersion {
                self.version
            }
        }
    };
}
simple_persistence_record!(PortfolioRecord, PortfolioId);
simple_persistence_record!(ProductRecord, ProductId);
simple_persistence_record!(RoadmapRecord, RoadmapId);

fn maps_equal<K: Eq + std::hash::Hash, V: Eq>(
    actual: &HashMap<K, V>,
    expected: &HashMap<&K, &V>,
) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .all(|(id, value)| expected.get(id).is_some_and(|v| *v == value))
}

#[derive(Clone)]
pub struct InMemoryPortfolioService<C, A> {
    portfolios: HashMap<PortfolioId, PortfolioRecord>,
    products: HashMap<ProductId, ProductRecord>,
    roadmaps: HashMap<RoadmapId, RoadmapRecord>,
    kpis: HashMap<KpiId, KpiDefinitionRecord>,
    observations: HashMap<KpiObservationId, KpiObservationRecord>,
    outcomes: HashMap<IdempotencyId, StoredOutcome>,
    audit_events: Vec<AuditEvent>,
    next_operation_ordinal: u64,
    fail_next_commit: bool,
    clock: C,
    audit_ids: A,
    // Prepared-but-not-yet-approved Lower Data Classification
    // intents, plus a small idempotency index over them keyed by the
    // Prepare call's own `IdempotencyId`. Deliberately NOT part of
    // `PortfolioPersistenceSnapshot` / `rehydrate` (see the doc comment
    // above `PrepareLowerPortfolioClassification`) -- an unapproved H2a
    // preview is documented (DG0 6.7) to not silently survive a restart, so
    // losing this scratch state on rehydrate is the intended behavior, not
    // a gap. Once approved, the resulting classification change itself is
    // durable via the ordinary `outcomes` path below.
    prepared_lowerings: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    prepared_lowering_requests: HashMap<IdempotencyId, (CommandIdentity, PreparedIntentId)>,
}

impl<C: Clock, A: AuditEventIdSource> InMemoryPortfolioService<C, A> {
    pub fn new(clock: C, audit_ids: A) -> Self {
        Self {
            portfolios: HashMap::new(),
            products: HashMap::new(),
            roadmaps: HashMap::new(),
            kpis: HashMap::new(),
            observations: HashMap::new(),
            outcomes: HashMap::new(),
            audit_events: Vec::new(),
            next_operation_ordinal: 0,
            fail_next_commit: false,
            clock,
            audit_ids,
            prepared_lowerings: HashMap::new(),
            prepared_lowering_requests: HashMap::new(),
        }
    }

    #[doc(hidden)]
    pub fn persistence_snapshot(&self) -> PortfolioPersistenceSnapshot {
        let mut replay: Vec<_> = self
            .outcomes
            .iter()
            .map(|(id, v)| PortfolioReplayCapsule {
                idempotency_id: id.clone(),
                command: v.command.clone(),
                result: v.result.clone().into(),
                correlation_id: v.correlation_id.clone(),
                audit_event_ids: v.audit_event_ids.clone(),
                operation_ordinal: v.operation_ordinal,
                derived_kpi_observation_mutations: v.derived_kpi_observation_mutations.clone(),
            })
            .collect();
        replay.sort_by_key(|value| value.operation_ordinal);
        PortfolioPersistenceSnapshot {
            portfolios: self.portfolios(),
            products: self.products(),
            roadmaps: self.roadmaps(),
            kpi_definitions: self.kpi_definitions(),
            kpi_observations: self.kpi_observations(),
            replay,
            audits: self.audit_events.clone(),
        }
    }

    #[doc(hidden)]
    pub fn rehydrate(clock: C, audit_ids: A, snapshot: PortfolioPersistenceSnapshot) -> Self {
        let next_operation_ordinal = snapshot
            .replay
            .iter()
            .map(|v| v.operation_ordinal)
            .max()
            .map_or(0, |v| v.saturating_add(1));
        Self {
            portfolios: snapshot
                .portfolios
                .into_iter()
                .map(|v| (v.id.clone(), v))
                .collect(),
            products: snapshot
                .products
                .into_iter()
                .map(|v| (v.id.clone(), v))
                .collect(),
            roadmaps: snapshot
                .roadmaps
                .into_iter()
                .map(|v| (v.id.clone(), v))
                .collect(),
            kpis: snapshot
                .kpi_definitions
                .into_iter()
                .map(|v| (v.id.clone(), v))
                .collect(),
            observations: snapshot
                .kpi_observations
                .into_iter()
                .map(|v| (v.id.clone(), v))
                .collect(),
            outcomes: snapshot
                .replay
                .into_iter()
                .map(|v| {
                    (
                        v.idempotency_id,
                        StoredOutcome {
                            command: v.command,
                            result: v.result.into(),
                            correlation_id: v.correlation_id,
                            audit_event_ids: v.audit_event_ids,
                            operation_ordinal: v.operation_ordinal,
                            derived_kpi_observation_mutations: v.derived_kpi_observation_mutations,
                        },
                    )
                })
                .collect(),
            audit_events: snapshot.audits,
            next_operation_ordinal,
            fail_next_commit: false,
            clock,
            audit_ids,
            prepared_lowerings: HashMap::new(),
            prepared_lowering_requests: HashMap::new(),
        }
    }

    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }

    #[must_use]
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.audit_events
    }

    #[must_use]
    pub fn inspect_portfolio(&self, id: &PortfolioId) -> Option<PortfolioRecord> {
        self.portfolios.get(id).cloned()
    }

    #[must_use]
    pub fn inspect_product(&self, id: &ProductId) -> Option<ProductRecord> {
        self.products.get(id).cloned()
    }

    #[must_use]
    pub fn inspect_roadmap(&self, id: &RoadmapId) -> Option<RoadmapRecord> {
        self.roadmaps.get(id).cloned()
    }

    #[must_use]
    pub fn inspect_kpi_definition(&self, id: &KpiId) -> Option<KpiDefinitionRecord> {
        self.kpis.get(id).cloned()
    }

    #[must_use]
    pub fn inspect_kpi_observation(&self, id: &KpiObservationId) -> Option<KpiObservationRecord> {
        self.observations.get(id).cloned()
    }

    /// Return all authoritative records in deterministic identifier order.
    ///
    /// These are cloned value snapshots; callers cannot mutate service state.
    #[must_use]
    pub fn portfolios(&self) -> Vec<PortfolioRecord> {
        let mut values: Vec<_> = self.portfolios.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use]
    pub fn products(&self) -> Vec<ProductRecord> {
        let mut values: Vec<_> = self.products.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use]
    pub fn roadmaps(&self) -> Vec<RoadmapRecord> {
        let mut values: Vec<_> = self.roadmaps.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use]
    pub fn kpi_definitions(&self) -> Vec<KpiDefinitionRecord> {
        let mut values: Vec<_> = self.kpis.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use]
    pub fn kpi_observations(&self) -> Vec<KpiObservationRecord> {
        let mut values: Vec<_> = self.observations.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    /// Return current relationship resolver snapshots for this service's
    /// endpoint families. KPI observations are records, not relationship
    /// endpoints, and therefore are intentionally omitted.
    #[must_use]
    pub fn endpoint_snapshots(&self) -> Vec<EndpointSnapshot> {
        let mut values = Vec::with_capacity(
            self.portfolios.len() + self.products.len() + self.roadmaps.len() + self.kpis.len(),
        );
        values.extend(self.portfolios().into_iter().map(|v| {
            EndpointSnapshot::Portfolio(PortfolioSnapshot::new(v.id, v.version, v.classification))
        }));
        values.extend(self.products().into_iter().map(|v| {
            EndpointSnapshot::Product(ProductSnapshot::new(v.id, v.version, v.classification))
        }));
        values.extend(self.roadmaps().into_iter().map(|v| {
            EndpointSnapshot::Roadmap(RoadmapSnapshot::new(v.id, v.version, v.classification))
        }));
        values.extend(
            self.kpi_definitions().into_iter().map(|v| {
                EndpointSnapshot::Kpi(KpiSnapshot::new(v.id, v.version, v.classification))
            }),
        );
        values.sort_by_key(|a| a.removal_digest_fields());
        values
    }

    pub fn create_portfolio(
        &mut self,
        intent: CreatePortfolio,
    ) -> Result<MutationOutcome<PortfolioRecord>, DomainError> {
        let command = CommandIdentity::CreatePortfolio {
            id: intent.id.clone(),
            name: intent.name.clone(),
            details: intent.details.clone(),
            classification: intent.classification,
            provenance: intent.provenance.clone(),
        };
        if let Some(outcome) = self.replay_portfolio(&command, &intent.context)? {
            return Ok(outcome);
        }
        ensure_absent(
            self.portfolios.contains_key(&intent.id),
            "portfolio.already_exists",
            &intent.context,
        )?;
        let record = PortfolioRecord {
            id: intent.id.clone(),
            name: intent.name,
            details: intent.details,
            classification: intent.classification.unwrap_or_default(),
            provenance: intent.provenance,
            version: AggregateVersion::initial(),
        };
        self.commit_portfolio(record, intent.context, command, "portfolio.created")
    }

    pub fn update_portfolio_details(
        &mut self,
        intent: UpdatePortfolioDetails,
    ) -> Result<MutationOutcome<PortfolioRecord>, DomainError> {
        let command = CommandIdentity::UpdatePortfolio {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            name: intent.name.clone(),
            details: intent.details.clone(),
            classification: intent.classification,
        };
        if let Some(outcome) = self.replay_portfolio(&command, &intent.context)? {
            return Ok(outcome);
        }
        let mut record = self.portfolios.get(&intent.id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "portfolio.not_found",
                &intent.context,
            )
        })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        record.version = next_version(record.version, &intent.context)?;
        record.name = intent.name;
        record.details = intent.details;
        if let Some(classification) = intent.classification {
            ensure_classification_not_lowered(
                record.classification,
                classification,
                &intent.context,
            )?;
            record.classification = classification;
        }
        self.commit_portfolio(record, intent.context, command, "portfolio.updated")
    }

    pub fn create_product(
        &mut self,
        intent: CreateProduct,
    ) -> Result<MutationOutcome<ProductRecord>, DomainError> {
        let command = CommandIdentity::CreateProduct {
            id: intent.id.clone(),
            name: intent.name.clone(),
            details: intent.details.clone(),
            classification: intent.classification,
            provenance: intent.provenance.clone(),
        };
        if let Some(outcome) = self.replay_product(&command, &intent.context)? {
            return Ok(outcome);
        }
        ensure_absent(
            self.products.contains_key(&intent.id),
            "product.already_exists",
            &intent.context,
        )?;
        let record = ProductRecord {
            id: intent.id.clone(),
            name: intent.name,
            details: intent.details,
            classification: intent.classification.unwrap_or_default(),
            provenance: intent.provenance,
            version: AggregateVersion::initial(),
        };
        self.commit_product(record, intent.context, command, "product.created")
    }

    pub fn update_product_details(
        &mut self,
        intent: UpdateProductDetails,
    ) -> Result<MutationOutcome<ProductRecord>, DomainError> {
        let command = CommandIdentity::UpdateProduct {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            name: intent.name.clone(),
            details: intent.details.clone(),
            classification: intent.classification,
        };
        if let Some(outcome) = self.replay_product(&command, &intent.context)? {
            return Ok(outcome);
        }
        let mut record = self.products.get(&intent.id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "product.not_found",
                &intent.context,
            )
        })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        record.version = next_version(record.version, &intent.context)?;
        record.name = intent.name;
        record.details = intent.details;
        if let Some(value) = intent.classification {
            ensure_classification_not_lowered(record.classification, value, &intent.context)?;
            record.classification = value;
        }
        self.commit_product(record, intent.context, command, "product.updated")
    }

    pub fn create_roadmap(
        &mut self,
        intent: CreateRoadmap,
    ) -> Result<MutationOutcome<RoadmapRecord>, DomainError> {
        let command = CommandIdentity::CreateRoadmap {
            id: intent.id.clone(),
            name: intent.name.clone(),
            details: intent.details.clone(),
            classification: intent.classification,
            provenance: intent.provenance.clone(),
        };
        if let Some(outcome) = self.replay_roadmap(&command, &intent.context)? {
            return Ok(outcome);
        }
        ensure_absent(
            self.roadmaps.contains_key(&intent.id),
            "roadmap.already_exists",
            &intent.context,
        )?;
        let record = RoadmapRecord {
            id: intent.id.clone(),
            name: intent.name,
            details: intent.details,
            classification: intent.classification.unwrap_or_default(),
            provenance: intent.provenance,
            version: AggregateVersion::initial(),
        };
        self.commit_roadmap(record, intent.context, command, "roadmap.created")
    }

    pub fn update_roadmap_details(
        &mut self,
        intent: UpdateRoadmapDetails,
    ) -> Result<MutationOutcome<RoadmapRecord>, DomainError> {
        let command = CommandIdentity::UpdateRoadmap {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            name: intent.name.clone(),
            details: intent.details.clone(),
            classification: intent.classification,
        };
        if let Some(outcome) = self.replay_roadmap(&command, &intent.context)? {
            return Ok(outcome);
        }
        let mut record = self.roadmaps.get(&intent.id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "roadmap.not_found",
                &intent.context,
            )
        })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        record.version = next_version(record.version, &intent.context)?;
        record.name = intent.name;
        record.details = intent.details;
        if let Some(value) = intent.classification {
            ensure_classification_not_lowered(record.classification, value, &intent.context)?;
            record.classification = value;
        }
        self.commit_roadmap(record, intent.context, command, "roadmap.updated")
    }

    pub fn create_kpi_definition(
        &mut self,
        intent: CreateKpiDefinition,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, DomainError> {
        let command = CommandIdentity::CreateKpi {
            id: intent.id.clone(),
            name: intent.name.clone(),
            definition: intent.definition.clone(),
            owner: intent.owner.clone(),
            target: intent.target.clone(),
            cadence: intent.cadence.clone(),
            source: intent.source.clone(),
            classification: intent.classification,
            provenance: intent.provenance.clone(),
        };
        if let Some(outcome) = self.replay_kpi(&command, &intent.context)? {
            return Ok(outcome);
        }
        ensure_absent(
            self.kpis.contains_key(&intent.id),
            "kpi.already_exists",
            &intent.context,
        )?;
        let record = KpiDefinitionRecord {
            id: intent.id.clone(),
            name: intent.name,
            definition: intent.definition,
            owner: intent.owner,
            target: intent.target,
            cadence: intent.cadence,
            source: intent.source,
            classification: intent.classification.unwrap_or_default(),
            provenance: intent.provenance,
            version: AggregateVersion::initial(),
        };
        self.commit_kpi(record, intent.context, command, "kpi.definition.created")
    }

    pub fn update_kpi_definition_details(
        &mut self,
        intent: UpdateKpiDefinitionDetails,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, DomainError> {
        let command = CommandIdentity::UpdateKpi {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            name: intent.name.clone(),
            definition: intent.definition.clone(),
            owner: intent.owner.clone(),
            target: intent.target.clone(),
            cadence: intent.cadence.clone(),
            source: intent.source.clone(),
            classification: intent.classification,
        };
        if let Some(outcome) = self.replay_kpi(&command, &intent.context)? {
            return Ok(outcome);
        }
        let mut record = self.kpis.get(&intent.id).cloned().ok_or_else(|| {
            operation_error(ErrorCode::DomainNotFound, "kpi.not_found", &intent.context)
        })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        record.version = next_version(record.version, &intent.context)?;
        record.name = intent.name;
        record.definition = intent.definition;
        record.owner = intent.owner;
        record.target = intent.target;
        record.cadence = intent.cadence;
        record.source = intent.source;
        if let Some(value) = intent.classification {
            ensure_classification_not_lowered(record.classification, value, &intent.context)?;
            record.classification = value;
        }
        self.commit_kpi(record, intent.context, command, "kpi.definition.updated")
    }

    pub fn create_kpi_observation(
        &mut self,
        intent: CreateKpiObservation,
    ) -> Result<MutationOutcome<KpiObservationRecord>, DomainError> {
        let command = CommandIdentity::CreateObservation {
            id: intent.id.clone(),
            kpi_id: intent.kpi_id.clone(),
            value: intent.value.clone(),
            observed_at: intent.observed_at,
            source: intent.source.clone(),
            classification: intent.classification,
            provenance: intent.provenance.clone(),
        };
        if let Some(outcome) = self.replay_observation(&command, &intent.context)? {
            return Ok(outcome);
        }
        ensure_absent(
            self.observations.contains_key(&intent.id),
            "kpi.observation.already_exists",
            &intent.context,
        )?;
        let definition_classification = self
            .kpis
            .get(&intent.kpi_id)
            .map(|definition| definition.classification)
            .ok_or_else(|| {
                operation_error(ErrorCode::DomainNotFound, "kpi.not_found", &intent.context)
            })?;
        let classification = intent
            .classification
            .map_or(definition_classification, |value| {
                value.combine(definition_classification)
            });
        let now = self.clock.now();
        let record = KpiObservationRecord {
            id: intent.id.clone(),
            kpi_id: intent.kpi_id,
            value: intent.value,
            observed_at: intent.observed_at,
            source: intent.source,
            classification,
            provenance: intent.provenance,
            version: AggregateVersion::initial(),
            created_at: now,
            updated_at: now,
        };
        self.commit_observation(
            record,
            intent.context,
            command,
            "kpi.observation.created",
            now,
        )
    }

    pub fn update_kpi_observation_details(
        &mut self,
        intent: UpdateKpiObservationDetails,
    ) -> Result<MutationOutcome<KpiObservationRecord>, DomainError> {
        let command = CommandIdentity::UpdateObservation {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            value: intent.value.clone(),
            observed_at: intent.observed_at,
            source: intent.source.clone(),
            classification: intent.classification,
        };
        if let Some(outcome) = self.replay_observation(&command, &intent.context)? {
            return Ok(outcome);
        }
        let mut record = self.observations.get(&intent.id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "kpi.observation.not_found",
                &intent.context,
            )
        })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        record.version = next_version(record.version, &intent.context)?;
        record.value = intent.value;
        record.observed_at = intent.observed_at;
        record.source = intent.source;
        let operation_occurred_at = self.clock.now();
        record.updated_at = std::cmp::max(operation_occurred_at, record.updated_at);
        if let Some(value) = intent.classification {
            ensure_classification_not_lowered(record.classification, value, &intent.context)?;
            record.classification = value;
        }
        let definition_classification = self
            .kpis
            .get(&record.kpi_id)
            .map(|definition| definition.classification)
            .ok_or_else(|| {
                operation_error(ErrorCode::DomainNotFound, "kpi.not_found", &intent.context)
            })?;
        record.classification = record.classification.combine(definition_classification);
        self.commit_observation(
            record,
            intent.context,
            command,
            "kpi.observation.updated",
            operation_occurred_at,
        )
    }
}

macro_rules! replay {
    ($name:ident, $variant:ident, $record:ty) => {
        fn $name(
            &self,
            command: &CommandIdentity,
            context: &OperationContext,
        ) -> Result<Option<MutationOutcome<$record>>, DomainError> {
            match self.outcomes.get(&context.idempotency_id) {
                None => Ok(None),
                Some(stored) if &stored.command != command => Err(idempotency_error(context)),
                Some(StoredOutcome {
                    result: StoredResult::$variant(outcome),
                    ..
                }) => Ok(Some(outcome.clone())),
                Some(_) => Err(idempotency_error(context)),
            }
        }
    };
}

impl<C: Clock, A: AuditEventIdSource> InMemoryPortfolioService<C, A> {
    replay!(replay_portfolio, Portfolio, PortfolioRecord);
    replay!(replay_product, Product, ProductRecord);
    replay!(replay_roadmap, Roadmap, RoadmapRecord);
    replay!(replay_kpi, Kpi, KpiDefinitionRecord);
    replay!(replay_observation, Observation, KpiObservationRecord);

    fn prepare_audit(
        &mut self,
        context: &OperationContext,
        target: AuditTarget,
        code: &str,
        occurred_at: UtcTimestamp,
    ) -> Result<AuditEvent, DomainError> {
        let event_id = self.audit_ids.next_audit_event_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "portfolio.audit_id_unavailable",
                context,
            )
        })?;
        let action = AuditAction::new(AuditModule::Portfolio, parse_audit_code(code), target);
        let disposition = successful_disposition();
        Ok(AuditEvent::new(
            event_id,
            occurred_at,
            AuditActor::HeadOfProducts,
            action,
            context.correlation_id.clone(),
            disposition,
        ))
    }

    fn commit_portfolio(
        &mut self,
        record: PortfolioRecord,
        context: OperationContext,
        command: CommandIdentity,
        code: &str,
    ) -> Result<MutationOutcome<PortfolioRecord>, DomainError> {
        let now = self.clock.now();
        let audit = self.prepare_audit(
            &context,
            AuditTarget::Portfolio(record.id.clone()),
            code,
            now,
        )?;
        self.check_commit(&context)?;
        let operation_ordinal = self.take_operation_ordinal(&context)?;
        let outcome = MutationOutcome {
            record: record.clone(),
            audit_event: audit.clone(),
        };
        self.portfolios.insert(record.id.clone(), record);
        self.audit_events.push(audit.clone());
        self.outcomes.insert(
            context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Portfolio(outcome.clone()),
                correlation_id: context.correlation_id,
                audit_event_ids: vec![audit.id().clone()],
                operation_ordinal,
                derived_kpi_observation_mutations: vec![],
            },
        );
        Ok(outcome)
    }

    fn commit_product(
        &mut self,
        record: ProductRecord,
        context: OperationContext,
        command: CommandIdentity,
        code: &str,
    ) -> Result<MutationOutcome<ProductRecord>, DomainError> {
        let now = self.clock.now();
        let audit =
            self.prepare_audit(&context, AuditTarget::Product(record.id.clone()), code, now)?;
        self.check_commit(&context)?;
        let operation_ordinal = self.take_operation_ordinal(&context)?;
        let outcome = MutationOutcome {
            record: record.clone(),
            audit_event: audit.clone(),
        };
        self.products.insert(record.id.clone(), record);
        self.audit_events.push(audit.clone());
        self.outcomes.insert(
            context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Product(outcome.clone()),
                correlation_id: context.correlation_id,
                audit_event_ids: vec![audit.id().clone()],
                operation_ordinal,
                derived_kpi_observation_mutations: vec![],
            },
        );
        Ok(outcome)
    }

    fn commit_roadmap(
        &mut self,
        record: RoadmapRecord,
        context: OperationContext,
        command: CommandIdentity,
        code: &str,
    ) -> Result<MutationOutcome<RoadmapRecord>, DomainError> {
        let now = self.clock.now();
        let audit =
            self.prepare_audit(&context, AuditTarget::Roadmap(record.id.clone()), code, now)?;
        self.check_commit(&context)?;
        let operation_ordinal = self.take_operation_ordinal(&context)?;
        let outcome = MutationOutcome {
            record: record.clone(),
            audit_event: audit.clone(),
        };
        self.roadmaps.insert(record.id.clone(), record);
        self.audit_events.push(audit.clone());
        self.outcomes.insert(
            context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Roadmap(outcome.clone()),
                correlation_id: context.correlation_id,
                audit_event_ids: vec![audit.id().clone()],
                operation_ordinal,
                derived_kpi_observation_mutations: vec![],
            },
        );
        Ok(outcome)
    }

    fn commit_kpi(
        &mut self,
        record: KpiDefinitionRecord,
        context: OperationContext,
        command: CommandIdentity,
        code: &str,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, DomainError> {
        let now = self.clock.now();
        let audit = self.prepare_audit(&context, AuditTarget::Kpi(record.id.clone()), code, now)?;
        let mut next_observations = self.observations.clone();
        let mut affected_ids: Vec<_> = next_observations
            .values()
            .filter(|observation| {
                observation.kpi_id == record.id
                    && record.classification.combine(observation.classification)
                        != observation.classification
            })
            .map(|observation| observation.id.clone())
            .collect();
        affected_ids.sort();
        let mut derived_audits = Vec::with_capacity(affected_ids.len());
        let mut derived_mutations = Vec::with_capacity(affected_ids.len());
        for observation_id in affected_ids {
            let observation = next_observations.get_mut(&observation_id).ok_or_else(|| {
                operation_error(
                    ErrorCode::PlatformInternal,
                    "portfolio.fan_out_state_invalid",
                    &context,
                )
            })?;
            let previous_version = observation.version;
            let previous_classification = observation.classification;
            let previous_updated_at = observation.updated_at;
            let resulting_version = next_version(observation.version, &context)?;
            let resulting_classification =
                record.classification.combine(observation.classification);
            let derived_audit = self.prepare_audit(
                &context,
                AuditTarget::KpiObservation(observation_id.clone()),
                "kpi.observation.classification.inherited",
                now,
            )?;
            observation.version = resulting_version;
            observation.classification = resulting_classification;
            let resulting_updated_at = std::cmp::max(now, observation.updated_at);
            observation.updated_at = resulting_updated_at;
            derived_mutations.push(DerivedKpiObservationMutation::new(
                observation_id,
                previous_version,
                resulting_version,
                previous_classification,
                resulting_classification,
                previous_updated_at,
                resulting_updated_at,
                derived_audit.id().clone(),
            ));
            derived_audits.push(derived_audit);
        }
        self.check_commit(&context)?;
        let operation_ordinal = self.take_operation_ordinal(&context)?;
        let outcome = MutationOutcome {
            record: record.clone(),
            audit_event: audit.clone(),
        };
        self.kpis.insert(record.id.clone(), record);
        self.audit_events.push(audit.clone());
        let mut audit_event_ids = vec![audit.id().clone()];
        for derived_audit in derived_audits {
            audit_event_ids.push(derived_audit.id().clone());
            self.audit_events.push(derived_audit);
        }
        self.observations = next_observations;
        self.outcomes.insert(
            context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Kpi(outcome.clone()),
                correlation_id: context.correlation_id,
                audit_event_ids,
                operation_ordinal,
                derived_kpi_observation_mutations: derived_mutations,
            },
        );
        Ok(outcome)
    }

    fn commit_observation(
        &mut self,
        record: KpiObservationRecord,
        context: OperationContext,
        command: CommandIdentity,
        code: &str,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiObservationRecord>, DomainError> {
        let audit = self.prepare_audit(
            &context,
            AuditTarget::KpiObservation(record.id.clone()),
            code,
            occurred_at,
        )?;
        self.check_commit(&context)?;
        let operation_ordinal = self.take_operation_ordinal(&context)?;
        let outcome = MutationOutcome {
            record: record.clone(),
            audit_event: audit.clone(),
        };
        self.observations.insert(record.id.clone(), record);
        self.audit_events.push(audit.clone());
        self.outcomes.insert(
            context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Observation(outcome.clone()),
                correlation_id: context.correlation_id,
                audit_event_ids: vec![audit.id().clone()],
                operation_ordinal,
                derived_kpi_observation_mutations: vec![],
            },
        );
        Ok(outcome)
    }

    /// H2a step 1: preview a Portfolio classification lowering. Produces a
    /// `WorkManagementPreparedIntent` the caller shows the Head of Products
    /// for explicit approval; nothing is mutated yet. Idempotent on
    /// `context.idempotency_id`, matching every other command in this file.
    pub fn prepare_lower_portfolio_classification<I: PortfolioClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerPortfolioClassification,
        ids: &mut I,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerPortfolioClassification {
            portfolio_id: intent.portfolio_id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_error(&intent.context));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_error(&intent.context));
        }
        let record = self
            .portfolios
            .get(&intent.portfolio_id)
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainNotFound,
                    "portfolio.not_found",
                    &intent.context,
                )
            })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "portfolio.classification_lowering_not_a_lowering",
                &intent.context,
            ));
        }
        self.check_commit(&intent.context)?;
        let op = WorkManagementOperation::LowerPortfolioClassification {
            portfolio_id: record.id.clone(),
            portfolio_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids.next_prepared_intent_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "portfolio.prepared_intent_id_unavailable",
                &intent.context,
            )
        })?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "portfolio.classification_lowering_invalid",
                &intent.context,
            )
        })?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2: approve and atomically apply a previously prepared
    /// Portfolio classification lowering. Reuses `commit_portfolio` for the
    /// actual mutation/audit/idempotency bookkeeping, so a successful
    /// lowering is indistinguishable in `outcomes`/replay from an ordinary
    /// `update_portfolio_details` outcome -- both are a `StoredResult::Portfolio`.
    pub fn approve_and_execute_lower_portfolio_classification<
        I: PortfolioClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerPortfolioClassification,
        ids: &mut I,
        authorization: &Z,
    ) -> Result<MutationOutcome<PortfolioRecord>, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "portfolio.classification_lowering_approval_mismatch",
                &intent.context,
            ));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerPortfolioClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(outcome) = self.replay_portfolio(&command, &intent.context)? {
            return Ok(outcome);
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainConflict,
                    "portfolio.classification_lowering_preview_changed",
                    &intent.context,
                )
            })?;
        let (portfolio_id, portfolio_version, proposed_classification, rationale) =
            match prepared.operation() {
                WorkManagementOperation::LowerPortfolioClassification {
                    portfolio_id,
                    portfolio_version,
                    proposed_classification,
                    rationale,
                    ..
                } => (
                    portfolio_id.clone(),
                    *portfolio_version,
                    *proposed_classification,
                    rationale.clone(),
                ),
                _ => {
                    return Err(operation_error(
                        ErrorCode::DomainConflict,
                        "portfolio.classification_lowering_preview_changed",
                        &intent.context,
                    ));
                }
            };
        let record = self.portfolios.get(&portfolio_id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "portfolio.not_found",
                &intent.context,
            )
        })?;
        if record.version != portfolio_version {
            return Err(operation_error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "portfolio.classification_lowering_preview_changed",
                &intent.context,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(record.version)));
        }
        let current_op = WorkManagementOperation::LowerPortfolioClassification {
            portfolio_id: portfolio_id.clone(),
            portfolio_version: record.version,
            current_classification: record.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "portfolio.classification_lowering_preview_changed",
                &intent.context,
            )
        })?;
        let current_snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids.next_approval_receipt_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "portfolio.approval_receipt_id_unavailable",
                &intent.context,
            )
        })?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &current_snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|error| match error {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                operation_error(
                    ErrorCode::SecurityPolicyDenied,
                    "portfolio.classification_lowering_approval_denied",
                    &intent.context,
                )
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch
            | crate::work_management::WorkManagementApprovalValidationError::Expired
            | crate::work_management::WorkManagementApprovalValidationError::PreviewChanged
            | crate::work_management::WorkManagementApprovalValidationError::PreparedIntentMismatch => {
                operation_error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "portfolio.classification_lowering_preview_changed",
                    &intent.context,
                )
            }
        })?;
        let mut mutated = record;
        mutated.version = next_version(mutated.version, &intent.context)?;
        mutated.classification = proposed_classification;
        self.commit_portfolio(
            mutated,
            intent.context,
            command,
            "portfolio.classification_lowered",
        )
    }

    /// H2a step 1: preview a Product classification lowering. See
    /// `prepare_lower_portfolio_classification` -- identical shape, applied
    /// to `ProductRecord`.
    pub fn prepare_lower_product_classification<I: PortfolioClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerProductClassification,
        ids: &mut I,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerProductClassification {
            product_id: intent.product_id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_error(&intent.context));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_error(&intent.context));
        }
        let record = self
            .products
            .get(&intent.product_id)
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainNotFound,
                    "product.not_found",
                    &intent.context,
                )
            })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "product.classification_lowering_not_a_lowering",
                &intent.context,
            ));
        }
        self.check_commit(&intent.context)?;
        let op = WorkManagementOperation::LowerProductClassification {
            product_id: record.id.clone(),
            product_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids.next_prepared_intent_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "product.prepared_intent_id_unavailable",
                &intent.context,
            )
        })?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "product.classification_lowering_invalid",
                &intent.context,
            )
        })?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for Product. See `approve_and_execute_lower_portfolio_classification`.
    pub fn approve_and_execute_lower_product_classification<
        I: PortfolioClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerProductClassification,
        ids: &mut I,
        authorization: &Z,
    ) -> Result<MutationOutcome<ProductRecord>, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "product.classification_lowering_approval_mismatch",
                &intent.context,
            ));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerProductClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(outcome) = self.replay_product(&command, &intent.context)? {
            return Ok(outcome);
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainConflict,
                    "product.classification_lowering_preview_changed",
                    &intent.context,
                )
            })?;
        let (product_id, product_version, proposed_classification, rationale) =
            match prepared.operation() {
                WorkManagementOperation::LowerProductClassification {
                    product_id,
                    product_version,
                    proposed_classification,
                    rationale,
                    ..
                } => (
                    product_id.clone(),
                    *product_version,
                    *proposed_classification,
                    rationale.clone(),
                ),
                _ => {
                    return Err(operation_error(
                        ErrorCode::DomainConflict,
                        "product.classification_lowering_preview_changed",
                        &intent.context,
                    ));
                }
            };
        let record = self.products.get(&product_id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "product.not_found",
                &intent.context,
            )
        })?;
        if record.version != product_version {
            return Err(operation_error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "product.classification_lowering_preview_changed",
                &intent.context,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(record.version)));
        }
        let current_op = WorkManagementOperation::LowerProductClassification {
            product_id: product_id.clone(),
            product_version: record.version,
            current_classification: record.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "product.classification_lowering_preview_changed",
                &intent.context,
            )
        })?;
        let current_snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids.next_approval_receipt_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "product.approval_receipt_id_unavailable",
                &intent.context,
            )
        })?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &current_snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|error| match error {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                operation_error(
                    ErrorCode::SecurityPolicyDenied,
                    "product.classification_lowering_approval_denied",
                    &intent.context,
                )
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch
            | crate::work_management::WorkManagementApprovalValidationError::Expired
            | crate::work_management::WorkManagementApprovalValidationError::PreviewChanged
            | crate::work_management::WorkManagementApprovalValidationError::PreparedIntentMismatch => {
                operation_error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "product.classification_lowering_preview_changed",
                    &intent.context,
                )
            }
        })?;
        let mut mutated = record;
        mutated.version = next_version(mutated.version, &intent.context)?;
        mutated.classification = proposed_classification;
        self.commit_product(
            mutated,
            intent.context,
            command,
            "product.classification_lowered",
        )
    }

    /// H2a step 1: preview a Roadmap classification lowering. See
    /// `prepare_lower_portfolio_classification` -- identical shape, applied
    /// to `RoadmapRecord`.
    pub fn prepare_lower_roadmap_classification<I: PortfolioClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerRoadmapClassification,
        ids: &mut I,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerRoadmapClassification {
            roadmap_id: intent.roadmap_id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_error(&intent.context));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_error(&intent.context));
        }
        let record = self
            .roadmaps
            .get(&intent.roadmap_id)
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainNotFound,
                    "roadmap.not_found",
                    &intent.context,
                )
            })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "roadmap.classification_lowering_not_a_lowering",
                &intent.context,
            ));
        }
        self.check_commit(&intent.context)?;
        let op = WorkManagementOperation::LowerRoadmapClassification {
            roadmap_id: record.id.clone(),
            roadmap_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids.next_prepared_intent_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "roadmap.prepared_intent_id_unavailable",
                &intent.context,
            )
        })?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "roadmap.classification_lowering_invalid",
                &intent.context,
            )
        })?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for Roadmap. See `approve_and_execute_lower_portfolio_classification`.
    pub fn approve_and_execute_lower_roadmap_classification<
        I: PortfolioClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerRoadmapClassification,
        ids: &mut I,
        authorization: &Z,
    ) -> Result<MutationOutcome<RoadmapRecord>, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "roadmap.classification_lowering_approval_mismatch",
                &intent.context,
            ));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerRoadmapClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(outcome) = self.replay_roadmap(&command, &intent.context)? {
            return Ok(outcome);
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainConflict,
                    "roadmap.classification_lowering_preview_changed",
                    &intent.context,
                )
            })?;
        let (roadmap_id, roadmap_version, proposed_classification, rationale) =
            match prepared.operation() {
                WorkManagementOperation::LowerRoadmapClassification {
                    roadmap_id,
                    roadmap_version,
                    proposed_classification,
                    rationale,
                    ..
                } => (
                    roadmap_id.clone(),
                    *roadmap_version,
                    *proposed_classification,
                    rationale.clone(),
                ),
                _ => {
                    return Err(operation_error(
                        ErrorCode::DomainConflict,
                        "roadmap.classification_lowering_preview_changed",
                        &intent.context,
                    ));
                }
            };
        let record = self.roadmaps.get(&roadmap_id).cloned().ok_or_else(|| {
            operation_error(
                ErrorCode::DomainNotFound,
                "roadmap.not_found",
                &intent.context,
            )
        })?;
        if record.version != roadmap_version {
            return Err(operation_error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "roadmap.classification_lowering_preview_changed",
                &intent.context,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(record.version)));
        }
        let current_op = WorkManagementOperation::LowerRoadmapClassification {
            roadmap_id: roadmap_id.clone(),
            roadmap_version: record.version,
            current_classification: record.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "roadmap.classification_lowering_preview_changed",
                &intent.context,
            )
        })?;
        let current_snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids.next_approval_receipt_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "roadmap.approval_receipt_id_unavailable",
                &intent.context,
            )
        })?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &current_snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|error| match error {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                operation_error(
                    ErrorCode::SecurityPolicyDenied,
                    "roadmap.classification_lowering_approval_denied",
                    &intent.context,
                )
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch
            | crate::work_management::WorkManagementApprovalValidationError::Expired
            | crate::work_management::WorkManagementApprovalValidationError::PreviewChanged
            | crate::work_management::WorkManagementApprovalValidationError::PreparedIntentMismatch => {
                operation_error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "roadmap.classification_lowering_preview_changed",
                    &intent.context,
                )
            }
        })?;
        let mut mutated = record;
        mutated.version = next_version(mutated.version, &intent.context)?;
        mutated.classification = proposed_classification;
        self.commit_roadmap(
            mutated,
            intent.context,
            command,
            "roadmap.classification_lowered",
        )
    }

    /// H2a step 1: preview a Kpi classification lowering. See
    /// `prepare_lower_portfolio_classification` -- identical shape, applied
    /// to `KpiDefinitionRecord`.
    pub fn prepare_lower_kpi_classification<I: PortfolioClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerKpiClassification,
        ids: &mut I,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerKpiClassification {
            kpi_id: intent.kpi_id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_error(&intent.context));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_error(&intent.context));
        }
        let record = self.kpis.get(&intent.kpi_id).cloned().ok_or_else(|| {
            operation_error(ErrorCode::DomainNotFound, "kpi.not_found", &intent.context)
        })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "kpi.classification_lowering_not_a_lowering",
                &intent.context,
            ));
        }
        self.check_commit(&intent.context)?;
        let op = WorkManagementOperation::LowerKpiClassification {
            kpi_id: record.id.clone(),
            kpi_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids.next_prepared_intent_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "kpi.prepared_intent_id_unavailable",
                &intent.context,
            )
        })?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "kpi.classification_lowering_invalid",
                &intent.context,
            )
        })?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for Kpi. See `approve_and_execute_lower_portfolio_classification`.
    /// Reuses `commit_kpi`, so the existing downstream KpiObservation
    /// classification fan-out (a stricter `combine`d value only, never a
    /// lowering) applies exactly as it does for an ordinary
    /// `update_kpi_definition_details` call.
    pub fn approve_and_execute_lower_kpi_classification<
        I: PortfolioClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerKpiClassification,
        ids: &mut I,
        authorization: &Z,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "kpi.classification_lowering_approval_mismatch",
                &intent.context,
            ));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerKpiClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(outcome) = self.replay_kpi(&command, &intent.context)? {
            return Ok(outcome);
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainConflict,
                    "kpi.classification_lowering_preview_changed",
                    &intent.context,
                )
            })?;
        let (kpi_id, kpi_version, proposed_classification, rationale) = match prepared.operation() {
            WorkManagementOperation::LowerKpiClassification {
                kpi_id,
                kpi_version,
                proposed_classification,
                rationale,
                ..
            } => (
                kpi_id.clone(),
                *kpi_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => {
                return Err(operation_error(
                    ErrorCode::DomainConflict,
                    "kpi.classification_lowering_preview_changed",
                    &intent.context,
                ));
            }
        };
        let record = self.kpis.get(&kpi_id).cloned().ok_or_else(|| {
            operation_error(ErrorCode::DomainNotFound, "kpi.not_found", &intent.context)
        })?;
        if record.version != kpi_version {
            return Err(operation_error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "kpi.classification_lowering_preview_changed",
                &intent.context,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(record.version)));
        }
        let current_op = WorkManagementOperation::LowerKpiClassification {
            kpi_id: kpi_id.clone(),
            kpi_version: record.version,
            current_classification: record.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "kpi.classification_lowering_preview_changed",
                &intent.context,
            )
        })?;
        let current_snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids.next_approval_receipt_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "kpi.approval_receipt_id_unavailable",
                &intent.context,
            )
        })?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &current_snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|error| match error {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                operation_error(
                    ErrorCode::SecurityPolicyDenied,
                    "kpi.classification_lowering_approval_denied",
                    &intent.context,
                )
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch
            | crate::work_management::WorkManagementApprovalValidationError::Expired
            | crate::work_management::WorkManagementApprovalValidationError::PreviewChanged
            | crate::work_management::WorkManagementApprovalValidationError::PreparedIntentMismatch => {
                operation_error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "kpi.classification_lowering_preview_changed",
                    &intent.context,
                )
            }
        })?;
        let mut mutated = record;
        mutated.version = next_version(mutated.version, &intent.context)?;
        mutated.classification = proposed_classification;
        self.commit_kpi(
            mutated,
            intent.context,
            command,
            "kpi.classification_lowered",
        )
    }

    /// H2a step 1: preview a KpiObservation classification lowering. See
    /// `prepare_lower_portfolio_classification` -- identical shape, applied
    /// to `KpiObservationRecord`.
    pub fn prepare_lower_kpi_observation_classification<
        I: PortfolioClassificationLoweringIdSource,
    >(
        &mut self,
        intent: PrepareLowerKpiObservationClassification,
        ids: &mut I,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerKpiObservationClassification {
            observation_id: intent.observation_id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_error(&intent.context));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_error(&intent.context));
        }
        let record = self
            .observations
            .get(&intent.observation_id)
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainNotFound,
                    "kpi.observation.not_found",
                    &intent.context,
                )
            })?;
        ensure_version(record.version, intent.expected_version, &intent.context)?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "kpi.observation.classification_lowering_not_a_lowering",
                &intent.context,
            ));
        }
        self.check_commit(&intent.context)?;
        let op = WorkManagementOperation::LowerKpiObservationClassification {
            observation_id: record.id.clone(),
            observation_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids.next_prepared_intent_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "kpi.observation.prepared_intent_id_unavailable",
                &intent.context,
            )
        })?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "kpi.observation.classification_lowering_invalid",
                &intent.context,
            )
        })?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for KpiObservation. See `approve_and_execute_lower_portfolio_classification`.
    pub fn approve_and_execute_lower_kpi_observation_classification<
        I: PortfolioClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerKpiObservationClassification,
        ids: &mut I,
        authorization: &Z,
    ) -> Result<MutationOutcome<KpiObservationRecord>, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(operation_error(
                ErrorCode::DomainConflict,
                "kpi.observation.classification_lowering_approval_mismatch",
                &intent.context,
            ));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerKpiObservationClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(outcome) = self.replay_observation(&command, &intent.context)? {
            return Ok(outcome);
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainConflict,
                    "kpi.observation.classification_lowering_preview_changed",
                    &intent.context,
                )
            })?;
        let (observation_id, observation_version, proposed_classification, rationale) =
            match prepared.operation() {
                WorkManagementOperation::LowerKpiObservationClassification {
                    observation_id,
                    observation_version,
                    proposed_classification,
                    rationale,
                    ..
                } => (
                    observation_id.clone(),
                    *observation_version,
                    *proposed_classification,
                    rationale.clone(),
                ),
                _ => {
                    return Err(operation_error(
                        ErrorCode::DomainConflict,
                        "kpi.observation.classification_lowering_preview_changed",
                        &intent.context,
                    ));
                }
            };
        let record = self
            .observations
            .get(&observation_id)
            .cloned()
            .ok_or_else(|| {
                operation_error(
                    ErrorCode::DomainNotFound,
                    "kpi.observation.not_found",
                    &intent.context,
                )
            })?;
        if record.version != observation_version {
            return Err(operation_error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "kpi.observation.classification_lowering_preview_changed",
                &intent.context,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(record.version)));
        }
        let current_op = WorkManagementOperation::LowerKpiObservationClassification {
            observation_id: observation_id.clone(),
            observation_version: record.version,
            current_classification: record.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            operation_error(
                ErrorCode::DomainConflict,
                "kpi.observation.classification_lowering_preview_changed",
                &intent.context,
            )
        })?;
        let current_snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids.next_approval_receipt_id().map_err(|_| {
            operation_error(
                ErrorCode::PlatformInternal,
                "kpi.observation.approval_receipt_id_unavailable",
                &intent.context,
            )
        })?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &current_snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|error| match error {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => {
                operation_error(
                    ErrorCode::SecurityPolicyDenied,
                    "kpi.observation.classification_lowering_approval_denied",
                    &intent.context,
                )
            }
            crate::work_management::WorkManagementApprovalValidationError::DigestMismatch
            | crate::work_management::WorkManagementApprovalValidationError::Expired
            | crate::work_management::WorkManagementApprovalValidationError::PreviewChanged
            | crate::work_management::WorkManagementApprovalValidationError::PreparedIntentMismatch => {
                operation_error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "kpi.observation.classification_lowering_preview_changed",
                    &intent.context,
                )
            }
        })?;
        let mut mutated = record;
        mutated.version = next_version(mutated.version, &intent.context)?;
        mutated.classification = proposed_classification;
        let occurred_at = self.clock.now();
        mutated.updated_at = std::cmp::max(occurred_at, mutated.updated_at);
        self.commit_observation(
            mutated,
            intent.context,
            command,
            "kpi.observation.classification_lowered",
            occurred_at,
        )
    }

    fn check_commit(&mut self, context: &OperationContext) -> Result<(), DomainError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(operation_error(
                ErrorCode::PlatformInternal,
                "portfolio.repository_unavailable",
                context,
            ));
        }
        Ok(())
    }

    fn take_operation_ordinal(&mut self, context: &OperationContext) -> Result<u64, DomainError> {
        let current = self.next_operation_ordinal;
        self.next_operation_ordinal = current.checked_add(1).ok_or_else(|| {
            operation_error(
                ErrorCode::PlatformInternal,
                "portfolio.operation_ordinal_exhausted",
                context,
            )
        })?;
        Ok(current)
    }
}

fn ensure_version(
    actual: AggregateVersion,
    expected: AggregateVersion,
    context: &OperationContext,
) -> Result<(), DomainError> {
    if actual != expected {
        return Err(operation_error(
            ErrorCode::DomainConflict,
            "portfolio.stale_version",
            context,
        )
        .with_extension(SafeErrorExtension::CurrentVersion(actual)));
    }
    Ok(())
}

fn ensure_absent(
    already_exists: bool,
    key: &str,
    context: &OperationContext,
) -> Result<(), DomainError> {
    if already_exists {
        return Err(operation_error(ErrorCode::DomainConflict, key, context));
    }
    Ok(())
}

/// True only for a genuine lowering (`proposed` strictly less restrictive
/// than `current`) -- the mirror image of `ensure_classification_not_lowered`'s
/// own check. Used to fail closed against a `PrepareLowerPortfolioClassification`
/// that does not actually lower anything (an unchanged or a raised value),
/// since the governed H2a path is reserved for real lowering.
fn is_genuine_lowering(current: DataClassification, proposed: DataClassification) -> bool {
    proposed.combine(current) != proposed
}

fn ensure_classification_not_lowered(
    current: DataClassification,
    requested: DataClassification,
    context: &OperationContext,
) -> Result<(), DomainError> {
    if requested.combine(current) != requested {
        return Err(operation_error(
            ErrorCode::SecurityPolicyDenied,
            "classification.lowering_requires_governed_intent",
            context,
        ));
    }
    Ok(())
}

fn next_version(
    version: AggregateVersion,
    context: &OperationContext,
) -> Result<AggregateVersion, DomainError> {
    version.next().ok_or_else(|| {
        operation_error(
            ErrorCode::DomainConflict,
            "portfolio.version_exhausted",
            context,
        )
    })
}

fn operation_error(code: ErrorCode, key: &str, context: &OperationContext) -> DomainError {
    DomainError::new(
        code,
        parse_message_key(key),
        context.correlation_id.clone(),
        matches!(code, ErrorCode::PlatformInternal),
    )
}

fn idempotency_error(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        parse_message_key("portfolio.idempotency_conflict"),
        context.correlation_id.clone(),
        false,
    )
}

fn successful_disposition() -> AuditDisposition {
    match AuditDisposition::new(
        AuditPolicyOutcome::NotRequired,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![parse_effect_code("portfolio-record-mutated")],
    ) {
        Ok(value) => value,
        Err(_) => unreachable!("constant successful audit disposition is valid"),
    }
}

fn parse_message_key(value: &str) -> MessageKey {
    match MessageKey::parse(value) {
        Ok(v) => v,
        Err(_) => unreachable!("constant message key is valid"),
    }
}
fn parse_audit_code(value: &str) -> AuditEventCode {
    match AuditEventCode::parse(value) {
        Ok(v) => v,
        Err(_) => unreachable!("constant audit code is valid"),
    }
}
fn parse_effect_code(value: &str) -> AuditEffectCode {
    match AuditEffectCode::parse(value) {
        Ok(v) => v,
        Err(_) => unreachable!("constant effect code is valid"),
    }
}
