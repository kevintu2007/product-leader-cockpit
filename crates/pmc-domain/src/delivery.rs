#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use crate::audit::{
    AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
    AuditEffectScope, AuditEvent, AuditEventCode, AuditEventIdSource, AuditExecutionOutcome,
    AuditModule, AuditPolicyOutcome, AuditTarget,
};
use crate::classification::DataClassification;
use crate::error::{DomainError, ErrorCode, FieldError, MessageKey, SafeErrorExtension};
use crate::identity::{
    AggregateVersion, ApprovalReceiptId, CorrelationId, IdempotencyId, InitiativeId, MilestoneId,
    PreparedIntentId, ProjectId,
};
use crate::provenance::Provenance;
use crate::relationships::{
    EndpointSnapshot, InitiativeSnapshot, MilestoneSnapshot, ProjectSnapshot,
};
use crate::time::{Clock, UtcTimestamp};
use crate::value::BoundedText;
use crate::work_management::{
    validate_and_mint_work_management_h2a_receipt, ApprovalAuthorizationPort,
    WorkManagementApproval, WorkManagementAuthoritativeSnapshot, WorkManagementCurrentPolicy,
    WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
};

const MAX_NAME_LENGTH: usize = 200;
const MAX_DETAIL_LENGTH: usize = 4000;

macro_rules! delivery_text {
    ($name:ident, $maximum:expr, $field:literal) => {
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(BoundedText<$maximum>);

        impl $name {
            pub fn parse(
                value: impl Into<String>,
                correlation_id: &CorrelationId,
            ) -> Result<Self, DomainError> {
                BoundedText::parse(value).map(Self).map_err(|_| {
                    validation_error($field, "delivery.validation.invalid_text", correlation_id)
                })
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }
    };
}

delivery_text!(RecordName, MAX_NAME_LENGTH, "delivery.name");
delivery_text!(
    DefinedOutcome,
    MAX_DETAIL_LENGTH,
    "initiative.defined_outcome"
);
delivery_text!(
    VerificationCriteria,
    MAX_DETAIL_LENGTH,
    "milestone.verification_criteria"
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Initiative {
    id: InitiativeId,
    name: RecordName,
    defined_outcome: DefinedOutcome,
    classification: DataClassification,
    provenance: Provenance,
    version: AggregateVersion,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}

impl Initiative {
    #[doc(hidden)]
    pub fn rehydrate(v: InitiativePersistenceRecord) -> Result<Self, DeliveryRehydrationError> {
        if v.created_at > v.updated_at {
            return Err(DeliveryRehydrationError::InvalidTimestampOrder);
        }
        Ok(Self {
            id: v.id,
            name: v.name,
            defined_outcome: v.defined_outcome,
            classification: v.classification,
            provenance: v.provenance,
            version: v.version,
            created_at: v.created_at,
            updated_at: v.updated_at,
        })
    }
    pub fn id(&self) -> &InitiativeId {
        &self.id
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn defined_outcome(&self) -> &str {
        self.defined_outcome.as_str()
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
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    id: ProjectId,
    name: RecordName,
    start_at: UtcTimestamp,
    end_at: UtcTimestamp,
    classification: DataClassification,
    provenance: Provenance,
    version: AggregateVersion,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}

impl Project {
    #[doc(hidden)]
    pub fn rehydrate(v: ProjectPersistenceRecord) -> Result<Self, DeliveryRehydrationError> {
        if v.created_at > v.updated_at || v.start_at > v.end_at {
            return Err(DeliveryRehydrationError::InvalidTimestampOrder);
        }
        Ok(Self {
            id: v.id,
            name: v.name,
            start_at: v.start_at,
            end_at: v.end_at,
            classification: v.classification,
            provenance: v.provenance,
            version: v.version,
            created_at: v.created_at,
            updated_at: v.updated_at,
        })
    }
    pub fn id(&self) -> &ProjectId {
        &self.id
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub const fn start_at(&self) -> UtcTimestamp {
        self.start_at
    }
    pub const fn end_at(&self) -> UtcTimestamp {
        self.end_at
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
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Milestone {
    id: MilestoneId,
    project_id: ProjectId,
    name: RecordName,
    verification_criteria: VerificationCriteria,
    due_at: UtcTimestamp,
    classification: DataClassification,
    provenance: Provenance,
    version: AggregateVersion,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}

impl Milestone {
    #[doc(hidden)]
    pub fn rehydrate(v: MilestonePersistenceRecord) -> Result<Self, DeliveryRehydrationError> {
        if v.created_at > v.updated_at {
            return Err(DeliveryRehydrationError::InvalidTimestampOrder);
        }
        Ok(Self {
            id: v.id,
            project_id: v.project_id,
            name: v.name,
            verification_criteria: v.verification_criteria,
            due_at: v.due_at,
            classification: v.classification,
            provenance: v.provenance,
            version: v.version,
            created_at: v.created_at,
            updated_at: v.updated_at,
        })
    }
    pub fn id(&self) -> &MilestoneId {
        &self.id
    }
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn verification_criteria(&self) -> &str {
        self.verification_criteria.as_str()
    }
    pub const fn due_at(&self) -> UtcTimestamp {
        self.due_at
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
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

#[derive(Clone, Debug)]
pub struct OperationContext {
    pub correlation_id: CorrelationId,
    pub idempotency_id: IdempotencyId,
}

#[derive(Clone, Debug)]
pub struct CreateInitiative {
    pub context: OperationContext,
    pub id: InitiativeId,
    pub name: RecordName,
    pub defined_outcome: DefinedOutcome,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug)]
pub struct UpdateInitiative {
    pub context: OperationContext,
    pub id: InitiativeId,
    pub expected_version: AggregateVersion,
    pub name: RecordName,
    pub defined_outcome: DefinedOutcome,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug)]
pub struct CreateProject {
    pub context: OperationContext,
    pub id: ProjectId,
    pub name: RecordName,
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug)]
pub struct UpdateProject {
    pub context: OperationContext,
    pub id: ProjectId,
    pub expected_version: AggregateVersion,
    pub name: RecordName,
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug)]
pub struct CreateMilestone {
    pub context: OperationContext,
    pub id: MilestoneId,
    pub project_id: ProjectId,
    pub name: RecordName,
    pub verification_criteria: VerificationCriteria,
    pub due_at: UtcTimestamp,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug)]
pub struct UpdateMilestone {
    pub context: OperationContext,
    pub id: MilestoneId,
    pub expected_version: AggregateVersion,
    pub name: RecordName,
    pub verification_criteria: VerificationCriteria,
    pub due_at: UtcTimestamp,
    pub classification: Option<DataClassification>,
    pub provenance: Provenance,
}

/// H2a "Lower Data Classification" for the Delivery family
/// (Initiative, Project, Milestone) -- mirrors Portfolio's identical
/// pattern exactly, including the same method-level-generic ID-source
/// workaround (see `DeliveryClassificationLoweringIdSource`'s doc comment).
#[derive(Clone, Debug)]
pub struct PrepareLowerInitiativeClassification {
    pub id: InitiativeId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}
#[derive(Clone, Debug)]
pub struct ApproveAndExecuteLowerInitiativeClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}
#[derive(Clone, Debug)]
pub struct PrepareLowerProjectClassification {
    pub id: ProjectId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}
#[derive(Clone, Debug)]
pub struct ApproveAndExecuteLowerProjectClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}
#[derive(Clone, Debug)]
pub struct PrepareLowerMilestoneClassification {
    pub id: MilestoneId,
    pub expected_version: AggregateVersion,
    pub proposed_classification: DataClassification,
    pub rationale: WorkManagementRationale,
    pub context: OperationContext,
}
#[derive(Clone, Debug)]
pub struct ApproveAndExecuteLowerMilestoneClassification {
    pub approval: WorkManagementApproval,
    pub context: OperationContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StoredResult {
    Initiative(Initiative),
    Project(Project),
    Milestone(Milestone),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredOutcome {
    command: CommandIdentity,
    result: StoredResult,
    correlation_id: CorrelationId,
    audit_event_ids: Vec<crate::identity::AuditEventId>,
    operation_ordinal: u64,
    derived_milestone_mutations: Vec<DerivedMilestoneMutation>,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandIdentity {
    CreateInitiative {
        id: InitiativeId,
        name: RecordName,
        defined_outcome: DefinedOutcome,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateInitiative {
        id: InitiativeId,
        expected_version: AggregateVersion,
        name: RecordName,
        defined_outcome: DefinedOutcome,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    CreateProject {
        id: ProjectId,
        name: RecordName,
        start_at: UtcTimestamp,
        end_at: UtcTimestamp,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateProject {
        id: ProjectId,
        expected_version: AggregateVersion,
        name: RecordName,
        start_at: UtcTimestamp,
        end_at: UtcTimestamp,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    CreateMilestone {
        id: MilestoneId,
        project_id: ProjectId,
        name: RecordName,
        verification_criteria: VerificationCriteria,
        due_at: UtcTimestamp,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    UpdateMilestone {
        id: MilestoneId,
        expected_version: AggregateVersion,
        name: RecordName,
        verification_criteria: VerificationCriteria,
        due_at: UtcTimestamp,
        classification: Option<DataClassification>,
        provenance: Provenance,
    },
    PrepareLowerInitiativeClassification {
        id: InitiativeId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerInitiativeClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
    PrepareLowerProjectClassification {
        id: ProjectId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerProjectClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
    PrepareLowerMilestoneClassification {
        id: MilestoneId,
        expected_version: AggregateVersion,
        proposed_classification: DataClassification,
        rationale: WorkManagementRationale,
    },
    ApproveAndExecuteLowerMilestoneClassification {
        prepared_id: PreparedIntentId,
        actor: AuditActor,
        acknowledged_payload_digest: crate::work_management::WorkManagementPayloadDigest,
    },
}

#[derive(Clone, Default)]
struct DeliveryState {
    initiatives: HashMap<InitiativeId, Initiative>,
    projects: HashMap<ProjectId, Project>,
    milestones: HashMap<MilestoneId, Milestone>,
    idempotency: HashMap<IdempotencyId, StoredOutcome>,
    audit_events: Vec<AuditEvent>,
    next_operation_ordinal: u64,
}

#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedMilestoneMutation {
    milestone_id: MilestoneId,
    previous_version: AggregateVersion,
    resulting_version: AggregateVersion,
    previous_classification: DataClassification,
    resulting_classification: DataClassification,
    previous_updated_at: UtcTimestamp,
    resulting_updated_at: UtcTimestamp,
    audit_event_id: crate::identity::AuditEventId,
}

impl DerivedMilestoneMutation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        milestone_id: MilestoneId,
        previous_version: AggregateVersion,
        resulting_version: AggregateVersion,
        previous_classification: DataClassification,
        resulting_classification: DataClassification,
        previous_updated_at: UtcTimestamp,
        resulting_updated_at: UtcTimestamp,
        audit_event_id: crate::identity::AuditEventId,
    ) -> Self {
        Self {
            milestone_id,
            previous_version,
            resulting_version,
            previous_classification,
            resulting_classification,
            previous_updated_at,
            resulting_updated_at,
            audit_event_id,
        }
    }
    pub fn milestone_id(&self) -> &MilestoneId {
        &self.milestone_id
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
    pub fn audit_event_id(&self) -> &crate::identity::AuditEventId {
        &self.audit_event_id
    }
}

#[doc(hidden)]
pub struct InitiativePersistenceRecord {
    pub id: InitiativeId,
    pub name: RecordName,
    pub defined_outcome: DefinedOutcome,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

#[doc(hidden)]
pub struct ProjectPersistenceRecord {
    pub id: ProjectId,
    pub name: RecordName,
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

#[doc(hidden)]
pub struct MilestonePersistenceRecord {
    pub id: MilestoneId,
    pub project_id: ProjectId,
    pub name: RecordName,
    pub verification_criteria: VerificationCriteria,
    pub due_at: UtcTimestamp,
    pub classification: DataClassification,
    pub provenance: Provenance,
    pub version: AggregateVersion,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
}

/// Adapter-only, domain-typed persistence capsule. This API is public because
/// the SQLite adapter is a sibling crate; callers cannot supply raw rows.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryPersistenceResult {
    Initiative(Initiative),
    Project(Project),
    Milestone(Milestone),
}

impl From<StoredResult> for DeliveryPersistenceResult {
    fn from(value: StoredResult) -> Self {
        match value {
            StoredResult::Initiative(v) => Self::Initiative(v),
            StoredResult::Project(v) => Self::Project(v),
            StoredResult::Milestone(v) => Self::Milestone(v),
        }
    }
}

impl From<DeliveryPersistenceResult> for StoredResult {
    fn from(value: DeliveryPersistenceResult) -> Self {
        match value {
            DeliveryPersistenceResult::Initiative(v) => Self::Initiative(v),
            DeliveryPersistenceResult::Project(v) => Self::Project(v),
            DeliveryPersistenceResult::Milestone(v) => Self::Milestone(v),
        }
    }
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct DeliveryReplayCapsule {
    idempotency_id: IdempotencyId,
    command: CommandIdentity,
    result: DeliveryPersistenceResult,
    correlation_id: CorrelationId,
    audit_event_ids: Vec<crate::identity::AuditEventId>,
    operation_ordinal: u64,
    derived_milestone_mutations: Vec<DerivedMilestoneMutation>,
}

impl DeliveryReplayCapsule {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        idempotency_id: IdempotencyId,
        command: CommandIdentity,
        result: DeliveryPersistenceResult,
        correlation_id: CorrelationId,
        audit_event_ids: Vec<crate::identity::AuditEventId>,
        operation_ordinal: u64,
        derived_milestone_mutations: Vec<DerivedMilestoneMutation>,
    ) -> Self {
        Self {
            idempotency_id,
            command,
            result,
            correlation_id,
            audit_event_ids,
            operation_ordinal,
            derived_milestone_mutations,
        }
    }
    pub fn idempotency_id(&self) -> &IdempotencyId {
        &self.idempotency_id
    }
    pub fn command(&self) -> &CommandIdentity {
        &self.command
    }
    pub fn result(&self) -> &DeliveryPersistenceResult {
        &self.result
    }
    pub fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }
    pub fn audit_event_ids(&self) -> &[crate::identity::AuditEventId] {
        &self.audit_event_ids
    }
    pub const fn operation_ordinal(&self) -> u64 {
        self.operation_ordinal
    }
    pub fn derived_milestone_mutations(&self) -> &[DerivedMilestoneMutation] {
        &self.derived_milestone_mutations
    }
}

#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct DeliveryPersistenceSnapshot {
    initiatives: Vec<Initiative>,
    projects: Vec<Project>,
    milestones: Vec<Milestone>,
    replay: Vec<DeliveryReplayCapsule>,
    audits: Vec<AuditEvent>,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryRehydrationError {
    DuplicateRecord,
    MissingParent,
    InvalidTimestampOrder,
    DuplicateAuditEvent,
    DuplicateIdempotency,
    IdempotencyMismatch,
    ResultMismatch,
    VersionLineageMismatch,
    ClassificationMismatch,
    AuditMismatch,
    OrphanAuditEvent,
}

impl DeliveryPersistenceSnapshot {
    pub fn initiatives(&self) -> &[Initiative] {
        &self.initiatives
    }
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }
    pub fn milestones(&self) -> &[Milestone] {
        &self.milestones
    }
    pub fn replay(&self) -> &[DeliveryReplayCapsule] {
        &self.replay
    }
    pub fn audits(&self) -> &[AuditEvent] {
        &self.audits
    }

    pub fn validate(
        initiatives: Vec<Initiative>,
        projects: Vec<Project>,
        milestones: Vec<Milestone>,
        replay: Vec<DeliveryReplayCapsule>,
        audits: Vec<AuditEvent>,
    ) -> Result<Self, DeliveryRehydrationError> {
        let initiative_map = unique_map(initiatives.iter().map(|v| (v.id.clone(), v)))?;
        let project_map = unique_map(projects.iter().map(|v| (v.id.clone(), v)))?;
        let milestone_map = unique_map(milestones.iter().map(|v| (v.id.clone(), v)))?;
        if initiatives.iter().any(|v| v.created_at > v.updated_at)
            || projects
                .iter()
                .any(|v| v.created_at > v.updated_at || v.start_at > v.end_at)
            || milestones.iter().any(|v| v.created_at > v.updated_at)
        {
            return Err(DeliveryRehydrationError::InvalidTimestampOrder);
        }
        if milestones
            .iter()
            .any(|v| !project_map.contains_key(&v.project_id))
        {
            return Err(DeliveryRehydrationError::MissingParent);
        }
        if milestones.iter().any(|v| {
            let parent = project_map[&v.project_id];
            parent.classification.combine(v.classification) != v.classification
        }) {
            return Err(DeliveryRehydrationError::ClassificationMismatch);
        }
        let mut audit_ids = std::collections::HashSet::new();
        if audits.iter().any(|v| !audit_ids.insert(v.id().clone())) {
            return Err(DeliveryRehydrationError::DuplicateAuditEvent);
        }
        let mut replay_ids = std::collections::HashSet::new();
        for capsule in &replay {
            if !replay_ids.insert(capsule.idempotency_id.clone()) {
                return Err(DeliveryRehydrationError::DuplicateIdempotency);
            }
            validate_replay_capsule(capsule)?;
            let target_exists = match &capsule.result {
                DeliveryPersistenceResult::Initiative(v) => initiative_map.contains_key(&v.id),
                DeliveryPersistenceResult::Project(v) => project_map.contains_key(&v.id),
                DeliveryPersistenceResult::Milestone(v) => milestone_map.contains_key(&v.id),
            };
            if !target_exists {
                return Err(DeliveryRehydrationError::ResultMismatch);
            }
        }
        validate_lineage(
            &replay,
            &audits,
            &initiative_map,
            &project_map,
            &milestone_map,
        )?;
        validate_milestone_semantics(&replay, &audits)?;
        validate_audit_obligations(&replay, &audits, &milestone_map)?;
        validate_operation_timeline(&replay, &initiative_map, &project_map, &milestone_map)?;
        Ok(Self {
            initiatives,
            projects,
            milestones,
            replay,
            audits,
        })
    }
}

fn unique_map<K: Eq + std::hash::Hash, V>(
    values: impl IntoIterator<Item = (K, V)>,
) -> Result<HashMap<K, V>, DeliveryRehydrationError> {
    let mut map = HashMap::new();
    for (key, value) in values {
        if map.insert(key, value).is_some() {
            return Err(DeliveryRehydrationError::DuplicateRecord);
        }
    }
    Ok(map)
}

fn validate_replay_capsule(
    capsule: &DeliveryReplayCapsule,
) -> Result<(), DeliveryRehydrationError> {
    let expected = match (&capsule.command, &capsule.result) {
        (
            CommandIdentity::CreateInitiative {
                id,
                name,
                defined_outcome,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Initiative(v),
        ) => {
            id == &v.id
                && name == &v.name
                && defined_outcome == &v.defined_outcome
                && classification.unwrap_or_default() == v.classification
                && provenance == &v.provenance
                && v.version == AggregateVersion::initial()
                && v.created_at == v.updated_at
        }
        (
            CommandIdentity::UpdateInitiative {
                id,
                expected_version,
                name,
                defined_outcome,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Initiative(v),
        ) => {
            id == &v.id
                && name == &v.name
                && defined_outcome == &v.defined_outcome
                && classification.is_none_or(|c| c == v.classification)
                && provenance == &v.provenance
                && expected_version.next() == Some(v.version)
        }
        (
            CommandIdentity::CreateProject {
                id,
                name,
                start_at,
                end_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Project(v),
        ) => {
            id == &v.id
                && name == &v.name
                && start_at == &v.start_at
                && end_at == &v.end_at
                && classification.unwrap_or_default() == v.classification
                && provenance == &v.provenance
                && v.version == AggregateVersion::initial()
                && v.created_at == v.updated_at
        }
        (
            CommandIdentity::UpdateProject {
                id,
                expected_version,
                name,
                start_at,
                end_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Project(v),
        ) => {
            id == &v.id
                && name == &v.name
                && start_at == &v.start_at
                && end_at == &v.end_at
                && classification.is_none_or(|c| c == v.classification)
                && provenance == &v.provenance
                && expected_version.next() == Some(v.version)
        }
        (
            CommandIdentity::CreateMilestone {
                id,
                project_id,
                name,
                verification_criteria,
                due_at,
                classification: _,
                provenance,
            },
            DeliveryPersistenceResult::Milestone(v),
        ) => {
            id == &v.id
                && project_id == &v.project_id
                && name == &v.name
                && verification_criteria == &v.verification_criteria
                && due_at == &v.due_at
                && provenance == &v.provenance
                && v.version == AggregateVersion::initial()
                && v.created_at == v.updated_at
        }
        (
            CommandIdentity::UpdateMilestone {
                id,
                expected_version,
                name,
                verification_criteria,
                due_at,
                classification,
                provenance,
            },
            DeliveryPersistenceResult::Milestone(v),
        ) => {
            id == &v.id
                && name == &v.name
                && verification_criteria == &v.verification_criteria
                && due_at == &v.due_at
                && classification.is_none_or(|c| v.classification.combine(c) == v.classification)
                && provenance == &v.provenance
                && expected_version.next() == Some(v.version)
        }
        // H2a "Lower Data Classification" execute for the
        // Delivery family. The command carries only `prepared_id`/`actor`/
        // `acknowledged_payload_digest` -- no target id, version, or
        // classification -- so there is nothing further this isolated,
        // per-capsule check can cross-verify. The state-sequenced checks
        // (genuine lowering, version succession, field preservation) run in
        // `validate_lineage` and `validate_operation_timeline`, which have
        // access to the record's prior state.
        (
            CommandIdentity::ApproveAndExecuteLowerInitiativeClassification { .. },
            DeliveryPersistenceResult::Initiative(_),
        )
        | (
            CommandIdentity::ApproveAndExecuteLowerProjectClassification { .. },
            DeliveryPersistenceResult::Project(_),
        )
        | (
            CommandIdentity::ApproveAndExecuteLowerMilestoneClassification { .. },
            DeliveryPersistenceResult::Milestone(_),
        ) => true,
        _ => return Err(DeliveryRehydrationError::ResultMismatch),
    };
    if !expected {
        return Err(DeliveryRehydrationError::ResultMismatch);
    }
    Ok(())
}

fn validate_lineage(
    replay: &[DeliveryReplayCapsule],
    audits: &[AuditEvent],
    initiatives: &HashMap<InitiativeId, &Initiative>,
    projects: &HashMap<ProjectId, &Project>,
    milestones: &HashMap<MilestoneId, &Milestone>,
) -> Result<(), DeliveryRehydrationError> {
    for current in initiatives.values() {
        let mut history: Vec<_> = replay
            .iter()
            .filter_map(|c| match &c.result {
                DeliveryPersistenceResult::Initiative(v) if v.id == current.id => Some((c, v)),
                _ => None,
            })
            .collect();
        history.sort_by_key(|(_, v)| v.version);
        validate_simple_versions(&history.iter().map(|(_, v)| v.version).collect::<Vec<_>>())?;
        if history.last().map(|(_, v)| *v) != Some(*current) {
            return Err(DeliveryRehydrationError::VersionLineageMismatch);
        }
        for pair in history.windows(2) {
            let (previous, next) = (pair[0].1, pair[1].1);
            if next.created_at != previous.created_at || next.updated_at < previous.updated_at {
                return Err(DeliveryRehydrationError::VersionLineageMismatch);
            }
            // H2a: a lowering execute is the one legitimate
            // exception to "classification only ever rises" -- it must be a
            // *genuine* lowering instead.
            if matches!(
                &pair[1].0.command,
                CommandIdentity::ApproveAndExecuteLowerInitiativeClassification { .. }
            ) {
                if !is_genuine_lowering(previous.classification, next.classification) {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
            } else if previous.classification.combine(next.classification) != next.classification {
                return Err(DeliveryRehydrationError::ClassificationMismatch);
            }
            if let CommandIdentity::UpdateInitiative {
                classification: None,
                ..
            } = &pair[1].0.command
            {
                if next.classification != previous.classification {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
            }
        }
    }
    for current in projects.values() {
        let mut history: Vec<_> = replay
            .iter()
            .filter_map(|c| match &c.result {
                DeliveryPersistenceResult::Project(v) if v.id == current.id => Some((c, v)),
                _ => None,
            })
            .collect();
        history.sort_by_key(|(_, v)| v.version);
        validate_simple_versions(&history.iter().map(|(_, v)| v.version).collect::<Vec<_>>())?;
        if history.last().map(|(_, v)| *v) != Some(*current) {
            return Err(DeliveryRehydrationError::VersionLineageMismatch);
        }
        for pair in history.windows(2) {
            let (previous, next) = (pair[0].1, pair[1].1);
            if next.created_at != previous.created_at || next.updated_at < previous.updated_at {
                return Err(DeliveryRehydrationError::VersionLineageMismatch);
            }
            // H2a: same lowering-execute exception as Initiative.
            if matches!(
                &pair[1].0.command,
                CommandIdentity::ApproveAndExecuteLowerProjectClassification { .. }
            ) {
                if !is_genuine_lowering(previous.classification, next.classification) {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
            } else if previous.classification.combine(next.classification) != next.classification {
                return Err(DeliveryRehydrationError::ClassificationMismatch);
            }
            if let CommandIdentity::UpdateProject {
                classification: None,
                ..
            } = &pair[1].0.command
            {
                if next.classification != previous.classification {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
            }
        }
    }
    for current in milestones.values() {
        let mut history: Vec<_> = replay
            .iter()
            .filter_map(|c| match &c.result {
                DeliveryPersistenceResult::Milestone(v) if v.id == current.id => Some((c, v)),
                _ => None,
            })
            .collect();
        history.sort_by_key(|(_, v)| v.version);
        if history
            .first()
            .is_none_or(|(_, v)| v.version != AggregateVersion::initial())
            || history
                .last()
                .is_none_or(|(_, v)| v.version > current.version)
        {
            return Err(DeliveryRehydrationError::VersionLineageMismatch);
        }
        for pair in history.windows(2) {
            let (previous, next) = (pair[0].1, pair[1].1);
            if next.created_at != previous.created_at
                || next.updated_at < previous.updated_at
                || next.version.get() <= previous.version.get()
            {
                return Err(DeliveryRehydrationError::VersionLineageMismatch);
            }
            if let CommandIdentity::UpdateMilestone {
                expected_version,
                classification: None,
                ..
            } = &pair[1].0.command
            {
                if *expected_version < previous.version
                    || next.classification.combine(previous.classification) != next.classification
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
            }
        }
        let inherited_count = audits
            .iter()
            .filter(|audit| {
                audit.code().as_str() == "milestone.classification.inherited"
                    && audit.target() == &AuditTarget::Milestone(current.id.clone())
            })
            .count() as u64;
        if current.version.get() != history.len() as u64 + inherited_count {
            return Err(DeliveryRehydrationError::VersionLineageMismatch);
        }
        let last = history
            .last()
            .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?
            .1;
        if current.id != last.id
            || current.project_id != last.project_id
            || current.name != last.name
            || current.verification_criteria != last.verification_criteria
            || current.due_at != last.due_at
            || current.provenance != last.provenance
            || current.created_at != last.created_at
            || current.updated_at < last.updated_at
            || current.classification.combine(last.classification) != current.classification
        {
            return Err(DeliveryRehydrationError::VersionLineageMismatch);
        }
    }
    Ok(())
}

fn validate_simple_versions(versions: &[AggregateVersion]) -> Result<(), DeliveryRehydrationError> {
    if versions.is_empty()
        || versions[0] != AggregateVersion::initial()
        || versions.windows(2).any(|v| v[0].next() != Some(v[1]))
    {
        return Err(DeliveryRehydrationError::VersionLineageMismatch);
    }
    Ok(())
}

fn validate_audit_obligations(
    replay: &[DeliveryReplayCapsule],
    audits: &[AuditEvent],
    milestones: &HashMap<MilestoneId, &Milestone>,
) -> Result<(), DeliveryRehydrationError> {
    let mut ordered_outcomes: Vec<_> = replay.iter().collect();
    ordered_outcomes.sort_by_key(|value| value.operation_ordinal);
    let declared_order: Vec<_> = ordered_outcomes
        .into_iter()
        .flat_map(|value| value.audit_event_ids.iter().cloned())
        .collect();
    let authoritative_order: Vec<_> = audits.iter().map(|value| value.id().clone()).collect();
    if declared_order != authoritative_order {
        return Err(DeliveryRehydrationError::AuditMismatch);
    }
    let audit_map: HashMap<_, _> = audits.iter().map(|v| (v.id().clone(), v)).collect();
    let mut claimed = std::collections::HashSet::new();
    for capsule in replay {
        if capsule.audit_event_ids.is_empty() {
            return Err(DeliveryRehydrationError::AuditMismatch);
        }
        for id in &capsule.audit_event_ids {
            if !claimed.insert(id.clone()) {
                return Err(DeliveryRehydrationError::AuditMismatch);
            }
        }
        let primary = audit_map
            .get(&capsule.audit_event_ids[0])
            .ok_or(DeliveryRehydrationError::AuditMismatch)?;
        let (code, target, occurred_at) = match (&capsule.command, &capsule.result) {
            (
                CommandIdentity::CreateInitiative { id, .. },
                DeliveryPersistenceResult::Initiative(v),
            ) => (
                "initiative.created",
                AuditTarget::Initiative(id.clone()),
                v.updated_at,
            ),
            (
                CommandIdentity::UpdateInitiative { id, .. },
                DeliveryPersistenceResult::Initiative(v),
            ) => (
                "initiative.updated",
                AuditTarget::Initiative(id.clone()),
                v.updated_at,
            ),
            (CommandIdentity::CreateProject { id, .. }, DeliveryPersistenceResult::Project(v)) => (
                "project.created",
                AuditTarget::Project(id.clone()),
                v.updated_at,
            ),
            (CommandIdentity::UpdateProject { id, .. }, DeliveryPersistenceResult::Project(v)) => (
                "project.updated",
                AuditTarget::Project(id.clone()),
                v.updated_at,
            ),
            (
                CommandIdentity::CreateMilestone { id, .. },
                DeliveryPersistenceResult::Milestone(v),
            ) => (
                "milestone.created",
                AuditTarget::Milestone(id.clone()),
                v.updated_at,
            ),
            (
                CommandIdentity::UpdateMilestone { id, .. },
                DeliveryPersistenceResult::Milestone(v),
            ) => (
                "milestone.updated",
                AuditTarget::Milestone(id.clone()),
                v.updated_at,
            ),
            // H2a "Lower Data Classification" execute for the
            // Delivery family -- audit codes mirror the exact strings minted
            // by `InMemoryDeliveryService::approve_and_execute_lower_*_classification`.
            (
                CommandIdentity::ApproveAndExecuteLowerInitiativeClassification { .. },
                DeliveryPersistenceResult::Initiative(v),
            ) => (
                "initiative.classification_lowered",
                AuditTarget::Initiative(v.id.clone()),
                v.updated_at,
            ),
            (
                CommandIdentity::ApproveAndExecuteLowerProjectClassification { .. },
                DeliveryPersistenceResult::Project(v),
            ) => (
                "project.classification_lowered",
                AuditTarget::Project(v.id.clone()),
                v.updated_at,
            ),
            (
                CommandIdentity::ApproveAndExecuteLowerMilestoneClassification { .. },
                DeliveryPersistenceResult::Milestone(v),
            ) => (
                "milestone.classification_lowered",
                AuditTarget::Milestone(v.id.clone()),
                v.updated_at,
            ),
            _ => return Err(DeliveryRehydrationError::AuditMismatch),
        };
        validate_delivery_audit(primary, &capsule.correlation_id, &target, code, occurred_at)?;
        if !matches!(capsule.command, CommandIdentity::UpdateProject { .. })
            && capsule.audit_event_ids.len() != 1
        {
            return Err(DeliveryRehydrationError::AuditMismatch);
        }
        for id in capsule.audit_event_ids.iter().skip(1) {
            let audit = audit_map
                .get(id)
                .ok_or(DeliveryRehydrationError::AuditMismatch)?;
            let AuditTarget::Milestone(id) = audit.target() else {
                return Err(DeliveryRehydrationError::AuditMismatch);
            };
            if !milestones.contains_key(id) {
                return Err(DeliveryRehydrationError::AuditMismatch);
            }
            validate_delivery_audit(
                audit,
                &capsule.correlation_id,
                audit.target(),
                "milestone.classification.inherited",
                occurred_at,
            )?;
        }
    }
    if claimed.len() != audits.len() {
        return Err(DeliveryRehydrationError::OrphanAuditEvent);
    }
    Ok(())
}

fn validate_milestone_semantics(
    replay: &[DeliveryReplayCapsule],
    audits: &[AuditEvent],
) -> Result<(), DeliveryRehydrationError> {
    let positions: HashMap<_, _> = audits
        .iter()
        .enumerate()
        .map(|(index, audit)| (audit.id().clone(), index))
        .collect();
    let primary_position = |capsule: &DeliveryReplayCapsule| {
        capsule
            .audit_event_ids
            .first()
            .and_then(|id| positions.get(id))
            .copied()
    };
    for capsule in replay {
        let position = primary_position(capsule).ok_or(DeliveryRehydrationError::AuditMismatch)?;
        let (result, requested, expected_version) = match (&capsule.command, &capsule.result) {
            (
                CommandIdentity::CreateMilestone { classification, .. },
                DeliveryPersistenceResult::Milestone(value),
            ) => (value, *classification, None),
            (
                CommandIdentity::UpdateMilestone {
                    classification,
                    expected_version,
                    ..
                },
                DeliveryPersistenceResult::Milestone(value),
            ) => (value, *classification, Some(*expected_version)),
            _ => continue,
        };
        let parent_classification = replay
            .iter()
            .filter_map(|candidate| {
                let candidate_position = primary_position(candidate)?;
                match &candidate.result {
                    DeliveryPersistenceResult::Project(value)
                        if value.id == result.project_id && candidate_position < position =>
                    {
                        Some((candidate_position, value.classification))
                    }
                    _ => None,
                }
            })
            .max_by_key(|(candidate_position, _)| *candidate_position)
            .map(|(_, classification)| classification)
            .ok_or(DeliveryRehydrationError::MissingParent)?;
        let base = if let Some(expected_version) = expected_version {
            replay
                .iter()
                .filter_map(|candidate| match &candidate.result {
                    DeliveryPersistenceResult::Milestone(value)
                        if value.id == result.id && value.version <= expected_version =>
                    {
                        Some(value)
                    }
                    _ => None,
                })
                .max_by_key(|value| value.version)
                .map(|value| value.classification)
                .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?
        } else {
            parent_classification
        };
        let requested = requested.map_or(base, |value| base.combine(value));
        if result.classification != parent_classification.combine(requested) {
            return Err(DeliveryRehydrationError::ClassificationMismatch);
        }
    }
    Ok(())
}

fn validate_operation_timeline(
    replay: &[DeliveryReplayCapsule],
    expected_initiatives: &HashMap<InitiativeId, &Initiative>,
    expected_projects: &HashMap<ProjectId, &Project>,
    expected_milestones: &HashMap<MilestoneId, &Milestone>,
) -> Result<(), DeliveryRehydrationError> {
    let mut ordered: Vec<_> = replay.iter().collect();
    ordered.sort_by_key(|value| value.operation_ordinal);
    if ordered
        .iter()
        .enumerate()
        .any(|(index, value)| value.operation_ordinal != index as u64)
    {
        return Err(DeliveryRehydrationError::VersionLineageMismatch);
    }
    let mut initiatives = HashMap::new();
    let mut projects = HashMap::new();
    let mut milestones: HashMap<MilestoneId, Milestone> = HashMap::new();
    for capsule in ordered {
        match (&capsule.command, &capsule.result) {
            (
                CommandIdentity::CreateInitiative { .. },
                DeliveryPersistenceResult::Initiative(value),
            ) => {
                if initiatives
                    .insert(value.id.clone(), value.clone())
                    .is_some()
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::VersionLineageMismatch);
                }
            }
            (
                CommandIdentity::UpdateInitiative {
                    expected_version,
                    classification,
                    ..
                },
                DeliveryPersistenceResult::Initiative(value),
            ) => {
                let previous = initiatives
                    .get(&value.id)
                    .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?;
                if previous.version != *expected_version
                    || previous.version.next() != Some(value.version)
                    || previous.classification.combine(value.classification) != value.classification
                    || !classification.map_or(
                        previous.classification == value.classification,
                        |requested| requested == value.classification,
                    )
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                initiatives.insert(value.id.clone(), value.clone());
            }
            (CommandIdentity::CreateProject { .. }, DeliveryPersistenceResult::Project(value)) => {
                if projects.insert(value.id.clone(), value.clone()).is_some()
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::VersionLineageMismatch);
                }
            }
            (
                CommandIdentity::UpdateProject {
                    expected_version,
                    classification,
                    ..
                },
                DeliveryPersistenceResult::Project(value),
            ) => {
                let previous = projects
                    .get(&value.id)
                    .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?;
                if previous.version != *expected_version
                    || previous.version.next() != Some(value.version)
                    || previous.classification.combine(value.classification) != value.classification
                    || !classification.map_or(
                        previous.classification == value.classification,
                        |requested| requested == value.classification,
                    )
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                let mut expected = Vec::new();
                let mut affected: Vec<_> = milestones
                    .values_mut()
                    .filter(|milestone| {
                        milestone.project_id == value.id
                            && value.classification.combine(milestone.classification)
                                != milestone.classification
                    })
                    .collect();
                affected.sort_by(|left, right| left.id.cmp(&right.id));
                for (index, milestone) in affected.into_iter().enumerate() {
                    let audit_event_id = capsule
                        .audit_event_ids
                        .get(index + 1)
                        .ok_or(DeliveryRehydrationError::AuditMismatch)?
                        .clone();
                    let mutation = DerivedMilestoneMutation {
                        milestone_id: milestone.id.clone(),
                        previous_version: milestone.version,
                        resulting_version: milestone
                            .version
                            .next()
                            .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?,
                        previous_classification: milestone.classification,
                        resulting_classification: value
                            .classification
                            .combine(milestone.classification),
                        previous_updated_at: milestone.updated_at,
                        resulting_updated_at: value.updated_at,
                        audit_event_id,
                    };
                    milestone.version = mutation.resulting_version;
                    milestone.classification = mutation.resulting_classification;
                    milestone.updated_at = mutation.resulting_updated_at;
                    expected.push(mutation);
                }
                if expected != capsule.derived_milestone_mutations
                    || capsule.audit_event_ids.len() != expected.len() + 1
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                projects.insert(value.id.clone(), value.clone());
            }
            (
                CommandIdentity::CreateMilestone { classification, .. },
                DeliveryPersistenceResult::Milestone(value),
            ) => {
                let parent = projects
                    .get(&value.project_id)
                    .ok_or(DeliveryRehydrationError::MissingParent)?;
                let expected_classification = classification
                    .map_or(parent.classification, |requested| {
                        parent.classification.combine(requested)
                    });
                if value.classification != expected_classification
                    || milestones.insert(value.id.clone(), value.clone()).is_some()
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
            }
            (
                CommandIdentity::UpdateMilestone {
                    expected_version,
                    classification,
                    ..
                },
                DeliveryPersistenceResult::Milestone(value),
            ) => {
                let previous = milestones
                    .get(&value.id)
                    .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?;
                let parent = projects
                    .get(&previous.project_id)
                    .ok_or(DeliveryRehydrationError::MissingParent)?;
                let requested = classification.map_or(previous.classification, |requested| {
                    previous.classification.combine(requested)
                });
                if classification.is_some_and(|requested| {
                    previous.classification.combine(requested) != requested
                }) {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                if previous.version != *expected_version
                    || previous.version.next() != Some(value.version)
                    || value.classification != parent.classification.combine(requested)
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                milestones.insert(value.id.clone(), value.clone());
            }
            // H2a "Lower Data Classification" execute for the
            // Delivery family. Mirrors `InMemoryDeliveryService::approve_and_
            // execute_lower_*_classification` exactly: version increments by
            // one, classification must be a genuine lowering relative to the
            // record's prior state, every other field is preserved verbatim,
            // and (per the doc comment on the Milestone variant) lowering
            // never cascades -- `derived_milestone_mutations` is always
            // empty for all three record types.
            (
                CommandIdentity::ApproveAndExecuteLowerInitiativeClassification { .. },
                DeliveryPersistenceResult::Initiative(value),
            ) => {
                let previous = initiatives
                    .get(&value.id)
                    .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?;
                if previous.version.next() != Some(value.version)
                    || !is_genuine_lowering(previous.classification, value.classification)
                    || previous.name != value.name
                    || previous.defined_outcome != value.defined_outcome
                    || previous.provenance != value.provenance
                    || previous.created_at != value.created_at
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                initiatives.insert(value.id.clone(), value.clone());
            }
            (
                CommandIdentity::ApproveAndExecuteLowerProjectClassification { .. },
                DeliveryPersistenceResult::Project(value),
            ) => {
                let previous = projects
                    .get(&value.id)
                    .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?;
                if previous.version.next() != Some(value.version)
                    || !is_genuine_lowering(previous.classification, value.classification)
                    || previous.name != value.name
                    || previous.start_at != value.start_at
                    || previous.end_at != value.end_at
                    || previous.provenance != value.provenance
                    || previous.created_at != value.created_at
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                projects.insert(value.id.clone(), value.clone());
            }
            (
                CommandIdentity::ApproveAndExecuteLowerMilestoneClassification { .. },
                DeliveryPersistenceResult::Milestone(value),
            ) => {
                let previous = milestones
                    .get(&value.id)
                    .ok_or(DeliveryRehydrationError::VersionLineageMismatch)?;
                if previous.version.next() != Some(value.version)
                    || !is_genuine_lowering(previous.classification, value.classification)
                    || previous.project_id != value.project_id
                    || previous.name != value.name
                    || previous.verification_criteria != value.verification_criteria
                    || previous.due_at != value.due_at
                    || previous.provenance != value.provenance
                    || previous.created_at != value.created_at
                    || !capsule.derived_milestone_mutations.is_empty()
                {
                    return Err(DeliveryRehydrationError::ClassificationMismatch);
                }
                milestones.insert(value.id.clone(), value.clone());
            }
            _ => return Err(DeliveryRehydrationError::ResultMismatch),
        }
    }
    let exact_initiatives = initiatives
        .iter()
        .all(|(id, value)| expected_initiatives.get(id).copied() == Some(value))
        && initiatives.len() == expected_initiatives.len();
    let exact_projects = projects
        .iter()
        .all(|(id, value)| expected_projects.get(id).copied() == Some(value))
        && projects.len() == expected_projects.len();
    let exact_milestones = milestones
        .iter()
        .all(|(id, value)| expected_milestones.get(id).copied() == Some(value))
        && milestones.len() == expected_milestones.len();
    if exact_initiatives && exact_projects && exact_milestones {
        Ok(())
    } else {
        Err(DeliveryRehydrationError::VersionLineageMismatch)
    }
}

fn validate_delivery_audit(
    audit: &AuditEvent,
    correlation_id: &CorrelationId,
    target: &AuditTarget,
    code: &str,
    occurred_at: UtcTimestamp,
) -> Result<(), DeliveryRehydrationError> {
    let exact = audit.occurred_at() == occurred_at
        && audit.actor() == AuditActor::HeadOfProducts
        && audit.module() == AuditModule::Portfolio
        && audit.code().as_str() == code
        && audit.correlation_id() == correlation_id
        && audit.target() == target
        && audit.policy_outcome() == AuditPolicyOutcome::NotRequired
        && audit.approval_outcome() == AuditApprovalOutcome::NotRequired
        && audit.execution_outcome() == AuditExecutionOutcome::Succeeded
        && audit.effect_scope() == AuditEffectScope::Complete
        && audit.actual_effects().len() == 1
        && audit.actual_effects()[0].as_str() == "delivery.authoritative-record-changed";
    if exact {
        Ok(())
    } else {
        Err(DeliveryRehydrationError::AuditMismatch)
    }
}

#[derive(Clone)]
pub struct InMemoryDeliveryService<C, I> {
    clock: C,
    audit_event_ids: I,
    state: DeliveryState,
    fail_next_commit: bool,
    // Prepared-but-not-yet-approved Lower Data Classification
    // intents, plus a small idempotency index over them keyed by the
    // Prepare call's own `IdempotencyId`. Deliberately NOT part of
    // `DeliveryPersistenceSnapshot`/`rehydrate` -- see
    // `portfolio::InMemoryPortfolioService`'s identical fields for why
    // (DG0 6.7: an unapproved H2a preview must not silently survive a
    // restart). Shared by all three Delivery record types (Initiative,
    // Project, Milestone), disambiguated by the `WorkManagementOperation`
    // variant each prepared intent wraps.
    prepared_lowerings: HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    prepared_lowering_requests: HashMap<IdempotencyId, (CommandIdentity, PreparedIntentId)>,
}

/// Mints the two identifiers a Delivery classification-lowering H2a round
/// trip needs. Threaded in as method-level generics on the six new
/// prepare/execute methods rather than baked into the service's own
/// constructor -- `InMemoryDeliveryService::new(clock, audit_event_ids)`
/// stays untouched, matching Portfolio's identical precedent and for the
/// identical reason: avoiding a breaking constructor change on a service
/// `pmc-ledger` already constructs.
pub trait DeliveryClassificationLoweringIdSource {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, crate::DomainValueError>;
    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, crate::DomainValueError>;
}

impl<C: Clock, I: AuditEventIdSource> InMemoryDeliveryService<C, I> {
    pub fn new(clock: C, audit_event_ids: I) -> Self {
        Self {
            clock,
            audit_event_ids,
            state: DeliveryState::default(),
            fail_next_commit: false,
            prepared_lowerings: HashMap::new(),
            prepared_lowering_requests: HashMap::new(),
        }
    }

    #[doc(hidden)]
    pub fn persistence_snapshot(&self) -> DeliveryPersistenceSnapshot {
        let replay = self
            .state
            .idempotency
            .iter()
            .map(|(id, outcome)| DeliveryReplayCapsule {
                idempotency_id: id.clone(),
                command: outcome.command.clone(),
                result: outcome.result.clone().into(),
                correlation_id: outcome.correlation_id.clone(),
                audit_event_ids: outcome.audit_event_ids.clone(),
                operation_ordinal: outcome.operation_ordinal,
                derived_milestone_mutations: outcome.derived_milestone_mutations.clone(),
            })
            .collect();
        DeliveryPersistenceSnapshot {
            initiatives: self.initiatives(),
            projects: self.projects(),
            milestones: self.milestones(),
            replay,
            audits: self.state.audit_events.clone(),
        }
    }

    #[doc(hidden)]
    pub fn rehydrate(clock: C, audit_event_ids: I, snapshot: DeliveryPersistenceSnapshot) -> Self {
        let next_operation_ordinal = snapshot
            .replay
            .iter()
            .map(|value| value.operation_ordinal)
            .max()
            .map_or(0, |value| value.saturating_add(1));
        let initiatives = snapshot
            .initiatives
            .into_iter()
            .map(|v| (v.id.clone(), v))
            .collect();
        let projects = snapshot
            .projects
            .into_iter()
            .map(|v| (v.id.clone(), v))
            .collect();
        let milestones = snapshot
            .milestones
            .into_iter()
            .map(|v| (v.id.clone(), v))
            .collect();
        let idempotency = snapshot
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
                        derived_milestone_mutations: v.derived_milestone_mutations,
                    },
                )
            })
            .collect();
        Self {
            clock,
            audit_event_ids,
            state: DeliveryState {
                initiatives,
                projects,
                milestones,
                idempotency,
                audit_events: snapshot.audits,
                next_operation_ordinal,
            },
            fail_next_commit: false,
            prepared_lowerings: HashMap::new(),
            prepared_lowering_requests: HashMap::new(),
        }
    }

    pub fn inject_next_commit_failure(&mut self) {
        self.fail_next_commit = true;
    }
    pub fn audit_events(&self) -> &[AuditEvent] {
        &self.state.audit_events
    }
    pub fn inspect_initiative(&self, id: &InitiativeId) -> Option<&Initiative> {
        self.state.initiatives.get(id)
    }
    pub fn inspect_project(&self, id: &ProjectId) -> Option<&Project> {
        self.state.projects.get(id)
    }
    pub fn inspect_milestone(&self, id: &MilestoneId) -> Option<&Milestone> {
        self.state.milestones.get(id)
    }

    /// Return cloned authoritative records in deterministic identifier order.
    #[must_use]
    pub fn initiatives(&self) -> Vec<Initiative> {
        let mut values: Vec<_> = self.state.initiatives.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use]
    pub fn projects(&self) -> Vec<Project> {
        let mut values: Vec<_> = self.state.projects.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    #[must_use]
    pub fn milestones(&self) -> Vec<Milestone> {
        let mut values: Vec<_> = self.state.milestones.values().cloned().collect();
        values.sort_by(|a, b| a.id.cmp(&b.id));
        values
    }

    /// Return current relationship resolver snapshots, retaining each
    /// milestone's authoritative project parent.
    #[must_use]
    pub fn endpoint_snapshots(&self) -> Vec<EndpointSnapshot> {
        let mut values = Vec::with_capacity(
            self.state.initiatives.len() + self.state.projects.len() + self.state.milestones.len(),
        );
        values.extend(self.initiatives().into_iter().map(|v| {
            EndpointSnapshot::Initiative(InitiativeSnapshot::new(v.id, v.version, v.classification))
        }));
        values.extend(self.projects().into_iter().map(|v| {
            EndpointSnapshot::Project(ProjectSnapshot::new(v.id, v.version, v.classification))
        }));
        values.extend(self.milestones().into_iter().map(|v| {
            EndpointSnapshot::Milestone(MilestoneSnapshot::new(
                v.id,
                v.project_id,
                v.version,
                v.classification,
            ))
        }));
        values.sort_by_key(|a| a.removal_digest_fields());
        values
    }

    pub fn create_initiative(
        &mut self,
        command: CreateInitiative,
    ) -> Result<Initiative, DomainError> {
        let fingerprint = CommandIdentity::CreateInitiative {
            id: command.id.clone(),
            name: command.name.clone(),
            defined_outcome: command.defined_outcome.clone(),
            classification: command.classification,
            provenance: command.provenance.clone(),
        };
        if let Some(result) = self.replay(&command.context, &fingerprint)? {
            return match result {
                StoredResult::Initiative(value) => Ok(value),
                _ => Err(conflict(&command.context.correlation_id, None)),
            };
        }
        if self.state.initiatives.contains_key(&command.id) {
            return Err(conflict(&command.context.correlation_id, None));
        }
        let now = self.clock.now();
        let value = Initiative {
            id: command.id.clone(),
            name: command.name,
            defined_outcome: command.defined_outcome,
            classification: command.classification.unwrap_or_default(),
            provenance: command.provenance,
            version: AggregateVersion::initial(),
            created_at: now,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.initiatives.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &command.context,
            AuditTarget::Initiative(value.id.clone()),
            "initiative.created",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &command.context.correlation_id)?;
        next.idempotency.insert(
            command.context.idempotency_id,
            StoredOutcome {
                command: fingerprint,
                result: StoredResult::Initiative(value.clone()),
                correlation_id: command.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.commit(next, &command.context.correlation_id)?;
        Ok(value)
    }

    pub fn update_initiative(
        &mut self,
        command: UpdateInitiative,
    ) -> Result<Initiative, DomainError> {
        let fingerprint = CommandIdentity::UpdateInitiative {
            id: command.id.clone(),
            expected_version: command.expected_version,
            name: command.name.clone(),
            defined_outcome: command.defined_outcome.clone(),
            classification: command.classification,
            provenance: command.provenance.clone(),
        };
        if let Some(result) = self.replay(&command.context, &fingerprint)? {
            return match result {
                StoredResult::Initiative(value) => Ok(value),
                _ => Err(conflict(&command.context.correlation_id, None)),
            };
        }
        let current = self
            .state
            .initiatives
            .get(&command.id)
            .ok_or_else(|| not_found(&command.context.correlation_id))?;
        ensure_version(
            current.version,
            command.expected_version,
            &command.context.correlation_id,
        )?;
        let classification = monotonic_classification(
            current.classification,
            command.classification,
            &command.context.correlation_id,
        )?;
        let version = next_version(current.version, &command.context.correlation_id)?;
        let now = self.clock.now();
        let value = Initiative {
            id: current.id.clone(),
            name: command.name,
            defined_outcome: command.defined_outcome,
            classification,
            provenance: command.provenance,
            version,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.initiatives.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &command.context,
            AuditTarget::Initiative(value.id.clone()),
            "initiative.updated",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &command.context.correlation_id)?;
        next.idempotency.insert(
            command.context.idempotency_id,
            StoredOutcome {
                command: fingerprint,
                result: StoredResult::Initiative(value.clone()),
                correlation_id: command.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.commit(next, &command.context.correlation_id)?;
        Ok(value)
    }

    pub fn create_project(&mut self, command: CreateProject) -> Result<Project, DomainError> {
        let fingerprint = CommandIdentity::CreateProject {
            id: command.id.clone(),
            name: command.name.clone(),
            start_at: command.start_at,
            end_at: command.end_at,
            classification: command.classification,
            provenance: command.provenance.clone(),
        };
        if let Some(result) = self.replay(&command.context, &fingerprint)? {
            return match result {
                StoredResult::Project(value) => Ok(value),
                _ => Err(conflict(&command.context.correlation_id, None)),
            };
        }
        validate_period(
            command.start_at,
            command.end_at,
            &command.context.correlation_id,
        )?;
        if self.state.projects.contains_key(&command.id) {
            return Err(conflict(&command.context.correlation_id, None));
        }
        let now = self.clock.now();
        let value = Project {
            id: command.id.clone(),
            name: command.name,
            start_at: command.start_at,
            end_at: command.end_at,
            classification: command.classification.unwrap_or_default(),
            provenance: command.provenance,
            version: AggregateVersion::initial(),
            created_at: now,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.projects.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &command.context,
            AuditTarget::Project(value.id.clone()),
            "project.created",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &command.context.correlation_id)?;
        next.idempotency.insert(
            command.context.idempotency_id,
            StoredOutcome {
                command: fingerprint,
                result: StoredResult::Project(value.clone()),
                correlation_id: command.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.commit(next, &command.context.correlation_id)?;
        Ok(value)
    }

    pub fn update_project(&mut self, command: UpdateProject) -> Result<Project, DomainError> {
        let fingerprint = CommandIdentity::UpdateProject {
            id: command.id.clone(),
            expected_version: command.expected_version,
            name: command.name.clone(),
            start_at: command.start_at,
            end_at: command.end_at,
            classification: command.classification,
            provenance: command.provenance.clone(),
        };
        if let Some(result) = self.replay(&command.context, &fingerprint)? {
            return match result {
                StoredResult::Project(value) => Ok(value),
                _ => Err(conflict(&command.context.correlation_id, None)),
            };
        }
        validate_period(
            command.start_at,
            command.end_at,
            &command.context.correlation_id,
        )?;
        let current = self
            .state
            .projects
            .get(&command.id)
            .ok_or_else(|| not_found(&command.context.correlation_id))?;
        ensure_version(
            current.version,
            command.expected_version,
            &command.context.correlation_id,
        )?;
        let classification = monotonic_classification(
            current.classification,
            command.classification,
            &command.context.correlation_id,
        )?;
        let version = next_version(current.version, &command.context.correlation_id)?;
        let now = self.clock.now();
        let value = Project {
            id: current.id.clone(),
            name: command.name,
            start_at: command.start_at,
            end_at: command.end_at,
            classification,
            provenance: command.provenance,
            version,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.projects.insert(value.id.clone(), value.clone());
        let mut raised_milestones = Vec::new();
        for milestone in next
            .milestones
            .values_mut()
            .filter(|milestone| milestone.project_id == value.id)
        {
            let inherited = value.classification.combine(milestone.classification);
            if inherited != milestone.classification {
                let previous_version = milestone.version;
                let previous_classification = milestone.classification;
                let previous_updated_at = milestone.updated_at;
                milestone.classification = inherited;
                milestone.version =
                    next_version(milestone.version, &command.context.correlation_id)?;
                milestone.updated_at = now;
                raised_milestones.push((
                    milestone.id.clone(),
                    previous_version,
                    milestone.version,
                    previous_classification,
                    milestone.classification,
                    previous_updated_at,
                    milestone.updated_at,
                ));
            }
        }
        raised_milestones.sort_by(|left, right| left.0.cmp(&right.0));
        let mut audit_event_ids = vec![append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &command.context,
            AuditTarget::Project(value.id.clone()),
            "project.updated",
            now,
        )?];
        let mut derived_milestone_mutations = Vec::new();
        for (
            milestone_id,
            previous_version,
            resulting_version,
            previous_classification,
            resulting_classification,
            previous_updated_at,
            resulting_updated_at,
        ) in raised_milestones
        {
            let audit_event_id = append_audit(
                &mut self.audit_event_ids,
                &mut next,
                &command.context,
                AuditTarget::Milestone(milestone_id.clone()),
                "milestone.classification.inherited",
                now,
            )?;
            audit_event_ids.push(audit_event_id.clone());
            derived_milestone_mutations.push(DerivedMilestoneMutation {
                milestone_id,
                previous_version,
                resulting_version,
                previous_classification,
                resulting_classification,
                previous_updated_at,
                resulting_updated_at,
                audit_event_id,
            });
        }
        let operation_ordinal = take_operation_ordinal(&mut next, &command.context.correlation_id)?;
        next.idempotency.insert(
            command.context.idempotency_id,
            StoredOutcome {
                command: fingerprint,
                result: StoredResult::Project(value.clone()),
                correlation_id: command.context.correlation_id.clone(),
                audit_event_ids,
                operation_ordinal,
                derived_milestone_mutations,
            },
        );
        self.commit(next, &command.context.correlation_id)?;
        Ok(value)
    }

    pub fn create_milestone(&mut self, command: CreateMilestone) -> Result<Milestone, DomainError> {
        let fingerprint = CommandIdentity::CreateMilestone {
            id: command.id.clone(),
            project_id: command.project_id.clone(),
            name: command.name.clone(),
            verification_criteria: command.verification_criteria.clone(),
            due_at: command.due_at,
            classification: command.classification,
            provenance: command.provenance.clone(),
        };
        if let Some(result) = self.replay(&command.context, &fingerprint)? {
            return match result {
                StoredResult::Milestone(value) => Ok(value),
                _ => Err(conflict(&command.context.correlation_id, None)),
            };
        }
        if self.state.milestones.contains_key(&command.id) {
            return Err(conflict(&command.context.correlation_id, None));
        }
        let project = self
            .state
            .projects
            .get(&command.project_id)
            .ok_or_else(|| not_found(&command.context.correlation_id))?;
        let classification = command
            .classification
            .map_or(project.classification, |value| {
                project.classification.combine(value)
            });
        let now = self.clock.now();
        let value = Milestone {
            id: command.id.clone(),
            project_id: command.project_id,
            name: command.name,
            verification_criteria: command.verification_criteria,
            due_at: command.due_at,
            classification,
            provenance: command.provenance,
            version: AggregateVersion::initial(),
            created_at: now,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.milestones.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &command.context,
            AuditTarget::Milestone(value.id.clone()),
            "milestone.created",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &command.context.correlation_id)?;
        next.idempotency.insert(
            command.context.idempotency_id,
            StoredOutcome {
                command: fingerprint,
                result: StoredResult::Milestone(value.clone()),
                correlation_id: command.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.commit(next, &command.context.correlation_id)?;
        Ok(value)
    }

    pub fn update_milestone(&mut self, command: UpdateMilestone) -> Result<Milestone, DomainError> {
        let fingerprint = CommandIdentity::UpdateMilestone {
            id: command.id.clone(),
            expected_version: command.expected_version,
            name: command.name.clone(),
            verification_criteria: command.verification_criteria.clone(),
            due_at: command.due_at,
            classification: command.classification,
            provenance: command.provenance.clone(),
        };
        if let Some(result) = self.replay(&command.context, &fingerprint)? {
            return match result {
                StoredResult::Milestone(value) => Ok(value),
                _ => Err(conflict(&command.context.correlation_id, None)),
            };
        }
        let current = self
            .state
            .milestones
            .get(&command.id)
            .ok_or_else(|| not_found(&command.context.correlation_id))?;
        ensure_version(
            current.version,
            command.expected_version,
            &command.context.correlation_id,
        )?;
        let requested_classification = monotonic_classification(
            current.classification,
            command.classification,
            &command.context.correlation_id,
        )?;
        let project_classification = self
            .state
            .projects
            .get(&current.project_id)
            .ok_or_else(|| not_found(&command.context.correlation_id))?
            .classification;
        let classification = project_classification.combine(requested_classification);
        let version = next_version(current.version, &command.context.correlation_id)?;
        let now = self.clock.now();
        let value = Milestone {
            id: current.id.clone(),
            project_id: current.project_id.clone(),
            name: command.name,
            verification_criteria: command.verification_criteria,
            due_at: command.due_at,
            classification,
            provenance: command.provenance,
            version,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.milestones.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &command.context,
            AuditTarget::Milestone(value.id.clone()),
            "milestone.updated",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &command.context.correlation_id)?;
        next.idempotency.insert(
            command.context.idempotency_id,
            StoredOutcome {
                command: fingerprint,
                result: StoredResult::Milestone(value.clone()),
                correlation_id: command.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.commit(next, &command.context.correlation_id)?;
        Ok(value)
    }

    fn replay(
        &self,
        context: &OperationContext,
        fingerprint: &CommandIdentity,
    ) -> Result<Option<StoredResult>, DomainError> {
        match self.state.idempotency.get(&context.idempotency_id) {
            None => Ok(None),
            Some(stored) if &stored.command == fingerprint => Ok(Some(stored.result.clone())),
            Some(_) => Err(idempotency_conflict(&context.correlation_id)),
        }
    }

    fn commit(
        &mut self,
        next: DeliveryState,
        correlation_id: &CorrelationId,
    ) -> Result<(), DomainError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(internal(correlation_id));
        }
        self.state = next;
        Ok(())
    }

    /// H2a step 1: preview an Initiative classification lowering. See
    /// `portfolio::InMemoryPortfolioService::prepare_lower_portfolio_classification`
    /// for the identical shape this mirrors.
    pub fn prepare_lower_initiative_classification<S: DeliveryClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerInitiativeClassification,
        ids: &mut S,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerInitiativeClassification {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_conflict(&intent.context.correlation_id));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_conflict(&intent.context.correlation_id));
        }
        let record = self
            .state
            .initiatives
            .get(&intent.id)
            .cloned()
            .ok_or_else(|| not_found(&intent.context.correlation_id))?;
        ensure_version(
            record.version,
            intent.expected_version,
            &intent.context.correlation_id,
        )?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(conflict(&intent.context.correlation_id, None));
        }
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(internal(&intent.context.correlation_id));
        }
        let op = WorkManagementOperation::LowerInitiativeClassification {
            initiative_id: record.id.clone(),
            initiative_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids
            .next_prepared_intent_id()
            .map_err(|_| internal(&intent.context.correlation_id))?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| conflict(&intent.context.correlation_id, None))?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for Initiative. See
    /// `portfolio::InMemoryPortfolioService::approve_and_execute_lower_portfolio_classification`.
    pub fn approve_and_execute_lower_initiative_classification<
        S: DeliveryClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerInitiativeClassification,
        ids: &mut S,
        authorization: &Z,
    ) -> Result<Initiative, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(conflict(&intent.context.correlation_id, None));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerInitiativeClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(result) = self.replay(&intent.context, &command)? {
            return match result {
                StoredResult::Initiative(value) => Ok(value),
                _ => Err(conflict(&intent.context.correlation_id, None)),
            };
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "delivery.preview_changed",
                    &intent.context.correlation_id,
                )
            })?;
        let (id, version, proposed_classification, rationale) = match prepared.operation() {
            WorkManagementOperation::LowerInitiativeClassification {
                initiative_id,
                initiative_version,
                proposed_classification,
                rationale,
                ..
            } => (
                initiative_id.clone(),
                *initiative_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => {
                return Err(error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "delivery.preview_changed",
                    &intent.context.correlation_id,
                ));
            }
        };
        let current = self
            .state
            .initiatives
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(&intent.context.correlation_id))?;
        if current.version != version {
            return Err(error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(current.version)));
        }
        let current_op = WorkManagementOperation::LowerInitiativeClassification {
            initiative_id: id.clone(),
            initiative_version: current.version,
            current_classification: current.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            current.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            )
        })?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids
            .next_approval_receipt_id()
            .map_err(|_| internal(&intent.context.correlation_id))?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|e| match e {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => error(
                ErrorCode::SecurityPolicyDenied,
                "delivery.classification.lowering_denied",
                &intent.context.correlation_id,
            ),
            _ => error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            ),
        })?;
        let now = self.clock.now();
        let value = Initiative {
            id: current.id.clone(),
            name: current.name,
            defined_outcome: current.defined_outcome,
            classification: proposed_classification,
            provenance: current.provenance,
            version: next_version(current.version, &intent.context.correlation_id)?,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.initiatives.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &intent.context,
            AuditTarget::Initiative(value.id.clone()),
            "initiative.classification_lowered",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &intent.context.correlation_id)?;
        next.idempotency.insert(
            intent.context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Initiative(value.clone()),
                correlation_id: intent.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.prepared_lowerings
            .remove(intent.approval.prepared_id());
        self.commit(next, &intent.context.correlation_id)?;
        Ok(value)
    }

    /// H2a step 1: preview a Project classification lowering. See
    /// `prepare_lower_initiative_classification` -- identical shape.
    pub fn prepare_lower_project_classification<S: DeliveryClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerProjectClassification,
        ids: &mut S,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerProjectClassification {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_conflict(&intent.context.correlation_id));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_conflict(&intent.context.correlation_id));
        }
        let record = self
            .state
            .projects
            .get(&intent.id)
            .cloned()
            .ok_or_else(|| not_found(&intent.context.correlation_id))?;
        ensure_version(
            record.version,
            intent.expected_version,
            &intent.context.correlation_id,
        )?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(conflict(&intent.context.correlation_id, None));
        }
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(internal(&intent.context.correlation_id));
        }
        let op = WorkManagementOperation::LowerProjectClassification {
            project_id: record.id.clone(),
            project_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids
            .next_prepared_intent_id()
            .map_err(|_| internal(&intent.context.correlation_id))?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| conflict(&intent.context.correlation_id, None))?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for Project. See `approve_and_execute_lower_initiative_classification`.
    pub fn approve_and_execute_lower_project_classification<
        S: DeliveryClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerProjectClassification,
        ids: &mut S,
        authorization: &Z,
    ) -> Result<Project, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(conflict(&intent.context.correlation_id, None));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerProjectClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(result) = self.replay(&intent.context, &command)? {
            return match result {
                StoredResult::Project(value) => Ok(value),
                _ => Err(conflict(&intent.context.correlation_id, None)),
            };
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "delivery.preview_changed",
                    &intent.context.correlation_id,
                )
            })?;
        let (id, version, proposed_classification, rationale) = match prepared.operation() {
            WorkManagementOperation::LowerProjectClassification {
                project_id,
                project_version,
                proposed_classification,
                rationale,
                ..
            } => (
                project_id.clone(),
                *project_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => {
                return Err(error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "delivery.preview_changed",
                    &intent.context.correlation_id,
                ));
            }
        };
        let current = self
            .state
            .projects
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(&intent.context.correlation_id))?;
        if current.version != version {
            return Err(error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(current.version)));
        }
        let current_op = WorkManagementOperation::LowerProjectClassification {
            project_id: id.clone(),
            project_version: current.version,
            current_classification: current.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            current.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            )
        })?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids
            .next_approval_receipt_id()
            .map_err(|_| internal(&intent.context.correlation_id))?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|e| match e {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => error(
                ErrorCode::SecurityPolicyDenied,
                "delivery.classification.lowering_denied",
                &intent.context.correlation_id,
            ),
            _ => error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            ),
        })?;
        let now = self.clock.now();
        let value = Project {
            id: current.id.clone(),
            name: current.name,
            start_at: current.start_at,
            end_at: current.end_at,
            classification: proposed_classification,
            provenance: current.provenance,
            version: next_version(current.version, &intent.context.correlation_id)?,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.projects.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &intent.context,
            AuditTarget::Project(value.id.clone()),
            "project.classification_lowered",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &intent.context.correlation_id)?;
        next.idempotency.insert(
            intent.context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Project(value.clone()),
                correlation_id: intent.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.prepared_lowerings
            .remove(intent.approval.prepared_id());
        self.commit(next, &intent.context.correlation_id)?;
        Ok(value)
    }

    /// H2a step 1: preview a Milestone classification lowering. See
    /// `prepare_lower_initiative_classification` -- identical shape.
    pub fn prepare_lower_milestone_classification<S: DeliveryClassificationLoweringIdSource>(
        &mut self,
        intent: PrepareLowerMilestoneClassification,
        ids: &mut S,
    ) -> Result<WorkManagementPreparedIntent, DomainError> {
        let command = CommandIdentity::PrepareLowerMilestoneClassification {
            id: intent.id.clone(),
            expected_version: intent.expected_version,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        if let Some((stored_command, prepared_id)) = self
            .prepared_lowering_requests
            .get(&intent.context.idempotency_id)
        {
            if *stored_command != command {
                return Err(idempotency_conflict(&intent.context.correlation_id));
            }
            return self
                .prepared_lowerings
                .get(prepared_id)
                .cloned()
                .ok_or_else(|| idempotency_conflict(&intent.context.correlation_id));
        }
        let record = self
            .state
            .milestones
            .get(&intent.id)
            .cloned()
            .ok_or_else(|| not_found(&intent.context.correlation_id))?;
        ensure_version(
            record.version,
            intent.expected_version,
            &intent.context.correlation_id,
        )?;
        if !is_genuine_lowering(record.classification, intent.proposed_classification) {
            return Err(conflict(&intent.context.correlation_id, None));
        }
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(internal(&intent.context.correlation_id));
        }
        let op = WorkManagementOperation::LowerMilestoneClassification {
            milestone_id: record.id.clone(),
            milestone_version: record.version,
            current_classification: record.classification,
            proposed_classification: intent.proposed_classification,
            rationale: intent.rationale.clone(),
        };
        let prepared_id = ids
            .next_prepared_intent_id()
            .map_err(|_| internal(&intent.context.correlation_id))?;
        let prepared = WorkManagementPreparedIntent::prepare(
            prepared_id,
            op,
            record.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| conflict(&intent.context.correlation_id, None))?;
        self.prepared_lowerings
            .insert(prepared.id().clone(), prepared.clone());
        self.prepared_lowering_requests.insert(
            intent.context.idempotency_id,
            (command, prepared.id().clone()),
        );
        Ok(prepared)
    }

    /// H2a step 2 for Milestone. See `approve_and_execute_lower_initiative_classification`.
    /// Deliberately does not touch `DerivedMilestoneMutation` fan-out --
    /// that mechanism exists for Project-level classification changes
    /// cascading down to their Milestones (see `update_project`), not the
    /// reverse; a Milestone's own classification lowering has no
    /// downstream of its own.
    pub fn approve_and_execute_lower_milestone_classification<
        S: DeliveryClassificationLoweringIdSource,
        Z: ApprovalAuthorizationPort,
    >(
        &mut self,
        intent: ApproveAndExecuteLowerMilestoneClassification,
        ids: &mut S,
        authorization: &Z,
    ) -> Result<Milestone, DomainError> {
        if intent.approval.idempotency_id() != &intent.context.idempotency_id {
            return Err(conflict(&intent.context.correlation_id, None));
        }
        let command = CommandIdentity::ApproveAndExecuteLowerMilestoneClassification {
            prepared_id: intent.approval.prepared_id().clone(),
            actor: intent.approval.actor(),
            acknowledged_payload_digest: intent.approval.acknowledged_payload_digest().clone(),
        };
        if let Some(result) = self.replay(&intent.context, &command)? {
            return match result {
                StoredResult::Milestone(value) => Ok(value),
                _ => Err(conflict(&intent.context.correlation_id, None)),
            };
        }
        let prepared = self
            .prepared_lowerings
            .get(intent.approval.prepared_id())
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "delivery.preview_changed",
                    &intent.context.correlation_id,
                )
            })?;
        let (id, version, proposed_classification, rationale) = match prepared.operation() {
            WorkManagementOperation::LowerMilestoneClassification {
                milestone_id,
                milestone_version,
                proposed_classification,
                rationale,
                ..
            } => (
                milestone_id.clone(),
                *milestone_version,
                *proposed_classification,
                rationale.clone(),
            ),
            _ => {
                return Err(error(
                    ErrorCode::SecurityPreviewExpiredOrChanged,
                    "delivery.preview_changed",
                    &intent.context.correlation_id,
                ));
            }
        };
        let current = self
            .state
            .milestones
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(&intent.context.correlation_id))?;
        if current.version != version {
            return Err(error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            )
            .with_extension(SafeErrorExtension::CurrentVersion(current.version)));
        }
        let current_op = WorkManagementOperation::LowerMilestoneClassification {
            milestone_id: id.clone(),
            milestone_version: current.version,
            current_classification: current.classification,
            proposed_classification,
            rationale,
        };
        let fresh = WorkManagementPreparedIntent::prepare(
            prepared.id().clone(),
            current_op.clone(),
            current.classification,
            None,
            self.clock.now(),
        )
        .map_err(|_| {
            error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            )
        })?;
        let snapshot = WorkManagementAuthoritativeSnapshot {
            operation: current_op,
            classification: fresh.classification(),
            classification_sources: fresh.preview().classification_sources().to_vec(),
            support: None,
            policy: WorkManagementCurrentPolicy::Allowed,
        };
        let receipt_id = ids
            .next_approval_receipt_id()
            .map_err(|_| internal(&intent.context.correlation_id))?;
        validate_and_mint_work_management_h2a_receipt(
            &prepared,
            &intent.approval,
            &snapshot,
            self.clock.now(),
            receipt_id,
            authorization,
        )
        .map_err(|e| match e {
            crate::work_management::WorkManagementApprovalValidationError::Unauthorized => error(
                ErrorCode::SecurityPolicyDenied,
                "delivery.classification.lowering_denied",
                &intent.context.correlation_id,
            ),
            _ => error(
                ErrorCode::SecurityPreviewExpiredOrChanged,
                "delivery.preview_changed",
                &intent.context.correlation_id,
            ),
        })?;
        let now = self.clock.now();
        let value = Milestone {
            id: current.id.clone(),
            project_id: current.project_id,
            name: current.name,
            verification_criteria: current.verification_criteria,
            due_at: current.due_at,
            classification: proposed_classification,
            provenance: current.provenance,
            version: next_version(current.version, &intent.context.correlation_id)?,
            created_at: current.created_at,
            updated_at: now,
        };
        let mut next = self.state.clone();
        next.milestones.insert(value.id.clone(), value.clone());
        let audit_event_id = append_audit(
            &mut self.audit_event_ids,
            &mut next,
            &intent.context,
            AuditTarget::Milestone(value.id.clone()),
            "milestone.classification_lowered",
            now,
        )?;
        let operation_ordinal = take_operation_ordinal(&mut next, &intent.context.correlation_id)?;
        next.idempotency.insert(
            intent.context.idempotency_id,
            StoredOutcome {
                command,
                result: StoredResult::Milestone(value.clone()),
                correlation_id: intent.context.correlation_id.clone(),
                audit_event_ids: vec![audit_event_id],
                operation_ordinal,
                derived_milestone_mutations: vec![],
            },
        );
        self.prepared_lowerings
            .remove(intent.approval.prepared_id());
        self.commit(next, &intent.context.correlation_id)?;
        Ok(value)
    }
}

/// True only for a genuine lowering (`proposed` strictly less restrictive
/// than `current`). See `portfolio::is_genuine_lowering` -- same check,
/// duplicated per module rather than shared.
fn is_genuine_lowering(current: DataClassification, proposed: DataClassification) -> bool {
    proposed.combine(current) != proposed
}

fn monotonic_classification(
    current: DataClassification,
    requested: Option<DataClassification>,
    correlation_id: &CorrelationId,
) -> Result<DataClassification, DomainError> {
    match requested {
        None => Ok(current),
        Some(value) if current.combine(value) == value => Ok(value),
        Some(_) => Err(error(
            ErrorCode::SecurityPolicyDenied,
            "delivery.classification.lowering_denied",
            correlation_id,
        )),
    }
}

fn take_operation_ordinal(
    state: &mut DeliveryState,
    correlation_id: &CorrelationId,
) -> Result<u64, DomainError> {
    let value = state.next_operation_ordinal;
    state.next_operation_ordinal = state
        .next_operation_ordinal
        .checked_add(1)
        .ok_or_else(|| internal(correlation_id))?;
    Ok(value)
}

fn validation_error(field: &str, reason: &str, correlation_id: &CorrelationId) -> DomainError {
    let field_error = match FieldError::new(field, reason) {
        Ok(value) => value,
        Err(_) => unreachable!("static delivery field error keys are valid"),
    };
    error(
        ErrorCode::ValidationInvalidField,
        "delivery.validation.invalid_field",
        correlation_id,
    )
    .with_extension(SafeErrorExtension::FieldErrors(vec![field_error]))
}

fn validate_period(
    start: UtcTimestamp,
    end: UtcTimestamp,
    correlation_id: &CorrelationId,
) -> Result<(), DomainError> {
    if start > end {
        Err(validation_error(
            "project.time_range",
            "delivery.validation.invalid_period",
            correlation_id,
        ))
    } else {
        Ok(())
    }
}

fn ensure_version(
    current: AggregateVersion,
    expected: AggregateVersion,
    correlation_id: &CorrelationId,
) -> Result<(), DomainError> {
    if current == expected {
        Ok(())
    } else {
        Err(conflict(correlation_id, Some(current)))
    }
}

fn next_version(
    current: AggregateVersion,
    correlation_id: &CorrelationId,
) -> Result<AggregateVersion, DomainError> {
    current.next().ok_or_else(|| internal(correlation_id))
}

fn append_audit<I: AuditEventIdSource>(
    audit_event_ids: &mut I,
    state: &mut DeliveryState,
    context: &OperationContext,
    target: AuditTarget,
    code: &str,
    now: UtcTimestamp,
) -> Result<crate::identity::AuditEventId, DomainError> {
    let audit_event_id = audit_event_ids
        .next_audit_event_id()
        .map_err(|_| internal(&context.correlation_id))?;
    if state
        .audit_events
        .iter()
        .any(|event| event.id() == &audit_event_id)
    {
        return Err(conflict(&context.correlation_id, None));
    }
    let action = AuditAction::new(
        AuditModule::Portfolio,
        parse_event_code(code, &context.correlation_id)?,
        target,
    );
    let effect = AuditEffectCode::parse("delivery.authoritative-record-changed")
        .map_err(|_| internal(&context.correlation_id))?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::NotRequired,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect],
    )
    .map_err(|_| internal(&context.correlation_id))?;
    state.audit_events.push(AuditEvent::new(
        audit_event_id.clone(),
        now,
        AuditActor::HeadOfProducts,
        action,
        context.correlation_id.clone(),
        disposition,
    ));
    Ok(audit_event_id)
}

fn parse_event_code(
    value: &str,
    correlation_id: &CorrelationId,
) -> Result<AuditEventCode, DomainError> {
    AuditEventCode::parse(value).map_err(|_| internal(correlation_id))
}

fn conflict(correlation_id: &CorrelationId, version: Option<AggregateVersion>) -> DomainError {
    let base = error(
        ErrorCode::DomainConflict,
        "delivery.conflict",
        correlation_id,
    );
    match version {
        Some(value) => base.with_extension(SafeErrorExtension::CurrentVersion(value)),
        None => base,
    }
}
fn not_found(correlation_id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::DomainNotFound,
        "delivery.not_found",
        correlation_id,
    )
}
fn idempotency_conflict(correlation_id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::DomainIdempotencyConflict,
        "delivery.idempotency_conflict",
        correlation_id,
    )
}
fn internal(correlation_id: &CorrelationId) -> DomainError {
    error(
        ErrorCode::PlatformInternal,
        "delivery.persistence_failed",
        correlation_id,
    )
}
fn error(code: ErrorCode, key: &str, correlation_id: &CorrelationId) -> DomainError {
    let message_key = match MessageKey::parse(key) {
        Ok(value) => value,
        Err(_) => unreachable!("static delivery message key is valid"),
    };
    DomainError::new(
        code,
        message_key,
        correlation_id.clone(),
        matches!(code, ErrorCode::PlatformInternal),
    )
}
