//! Record entry for the Portfolio family (slice 6B; DG3 record-entry
//! amendment §2–§4): the reservation-checked creates and links the desktop
//! reaches, and the edit-detail reads a sheet opens with.
//!
//! Every create here takes a [`ReservedId`] the Ledger itself issued for the
//! sheet's `clientRequestId`, checks that the reservation row still names
//! exactly that id, kind and operation, and only then runs the ordinary
//! writer with that id — so a retry always names the same record, and a
//! record cannot be created from an id the Ledger did not reserve.
//! Reservation rows are immutable and never removed (v47 triggers), so a
//! check just before the writer's own transaction holds for as long as the
//! row exists, which is forever; nothing between the check and the commit
//! can change what was checked.
//!
//! Provenance is `UserEntered` here, always: these are the records a person
//! types in. The seed tool and the tests keep the plain writers, which take
//! any provenance and any id.
//!
//! The reads return every editable field with the record's version, so a
//! sheet edits what the Ledger holds and sends the version it read. They
//! read the authoritative rows, not a projection.

use pmc_domain::actions::{
    ActionDetails, ActionMutationOutcome, ActionOperationContext, ActionRequestRecord, ActionTitle,
    CreateActionRequestDraft,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::decisions::{
    CreateDecisionRequestDraft, DecisionMutationOutcome, DecisionOperationContext,
    DecisionRequestRecord, DecisionSubject, DecisionText,
};
use pmc_domain::delivery::{
    CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome, Initiative, Milestone,
    OperationContext as DeliveryContext, Project, RecordName, VerificationCriteria,
};
use pmc_domain::error::{DomainError, ErrorCode, MessageKey};
use pmc_domain::identity::{
    ActionRequestId, AggregateVersion, AuditEventId, DecisionRequestId, InitiativeId, IssueId,
    KpiId, KpiObservationId, MilestoneId, PortfolioId, ProductId, ProjectId, RelationshipId,
    RoadmapId, StakeholderId,
};
use pmc_domain::issues::{
    CreateIssue, IssueDetails, IssueMutationOutcome, IssueOperationContext, IssueTitle,
};
use pmc_domain::portfolio::{
    CreateKpiDefinition, CreateKpiObservation, CreatePortfolio, CreateProduct, CreateRoadmap,
    KpiDefinitionRecord, KpiObservationRecord, LongText, MutationOutcome, OperationContext,
    PortfolioRecord, ProductRecord, RoadmapRecord, ShortText,
};
use pmc_domain::provenance::Provenance;
use pmc_domain::relationships::{
    CreateStakeholder, LinkInitiativeProject, LinkPortfolioProduct, LinkProductKpi,
    LinkProductRoadmap, LinkProjectProduct, LinkStakeholderRelationship,
    MutationOutcome as LinkOutcome, OperationContext as LinkContext, RelationshipRecord,
    StakeholderKind, StakeholderName, StakeholderRecord, StakeholderRelationshipPurpose,
    StakeholderSubject,
};
use pmc_domain::time::UtcTimestamp;
use rusqlite::OptionalExtension;

use super::delivery_repository::DeliveryMutationOutcome;
use super::reservation_repository::{
    reservation_matches, ReservableId, ReservationRequest, ReservedEntityKind, ReservedId,
};
use super::{LedgerOpenError, LedgerTransactionError, SqliteProductLedger};

/// The reservation every Delivery-family create and link names (slice 6C).
pub const CREATE_INITIATIVE: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Initiative,
    operation: "create_initiative",
};
pub const CREATE_PROJECT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Project,
    operation: "create_project",
};
pub const CREATE_MILESTONE: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Milestone,
    operation: "create_milestone",
};
pub const LINK_INITIATIVE_PROJECT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Relationship,
    operation: "link_initiative_project",
};
pub const LINK_PROJECT_PRODUCT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Relationship,
    operation: "link_project_product",
};

/// An Initiative as a sheet edits it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitiativeEntryRecord {
    pub id: String,
    pub name: String,
    pub defined_outcome: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEntryRecord {
    pub id: String,
    pub name: String,
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneEntryRecord {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub verification_criteria: String,
    pub due_at: UtcTimestamp,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// The Delivery family (slice 6C): Initiative, Project and Milestone
/// creates through the Ledger's reservation, the InitiativeProject and
/// ProjectProduct links, and the entry reads a sheet opens with. Same
/// contract as the Portfolio family above: the reservation must name this
/// request, kind, operation and id; provenance is `UserEntered`.
impl SqliteProductLedger {
    #[allow(clippy::too_many_arguments)]
    pub fn create_initiative_from_reservation(
        &mut self,
        reserved: &ReservedId<InitiativeId>,
        name: RecordName,
        defined_outcome: DefinedOutcome,
        classification: Option<DataClassification>,
        context: DeliveryContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Initiative>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_INITIATIVE,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_initiative(
            CreateInitiative {
                context,
                id: reserved.id().clone(),
                name,
                defined_outcome,
                classification,
                provenance: Provenance::UserEntered,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_project_from_reservation(
        &mut self,
        reserved: &ReservedId<ProjectId>,
        name: RecordName,
        start_at: UtcTimestamp,
        end_at: UtcTimestamp,
        classification: Option<DataClassification>,
        context: DeliveryContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Project>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_PROJECT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_project(
            CreateProject {
                context,
                id: reserved.id().clone(),
                name,
                start_at,
                end_at,
                classification,
                provenance: Provenance::UserEntered,
            },
            audit_event_id,
            occurred_at,
        )
    }

    /// A Milestone belongs to the Project the sheet was opened from; the
    /// Ledger's `milestones.project_id` holds that, no link is needed. The
    /// Project must still be at the version the sheet read.
    #[allow(clippy::too_many_arguments)]
    pub fn create_milestone_from_reservation(
        &mut self,
        reserved: &ReservedId<MilestoneId>,
        project_id: ProjectId,
        expected_project_version: AggregateVersion,
        name: RecordName,
        verification_criteria: VerificationCriteria,
        due_at: UtcTimestamp,
        classification: Option<DataClassification>,
        context: DeliveryContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Milestone>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_MILESTONE,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.require_current_version(
            "project",
            project_id.as_str(),
            expected_project_version,
            "delivery.stale_version",
            "delivery.not_found",
            &context.correlation_id,
        )?;
        self.create_milestone(
            CreateMilestone {
                context,
                id: reserved.id().clone(),
                project_id,
                name,
                verification_criteria,
                due_at,
                classification,
                provenance: Provenance::UserEntered,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link_initiative_project_from_reservation(
        &mut self,
        reserved: &ReservedId<RelationshipId>,
        initiative_id: InitiativeId,
        expected_initiative_version: AggregateVersion,
        project_id: ProjectId,
        expected_project_version: AggregateVersion,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            LINK_INITIATIVE_PROJECT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.link_initiative_project(
            LinkInitiativeProject {
                id: reserved.id().clone(),
                initiative_id,
                project_id,
                expected_initiative_version,
                expected_project_version,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link_project_product_from_reservation(
        &mut self,
        reserved: &ReservedId<RelationshipId>,
        project_id: ProjectId,
        expected_project_version: AggregateVersion,
        product_id: ProductId,
        expected_product_version: AggregateVersion,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            LINK_PROJECT_PRODUCT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.link_project_product(
            LinkProjectProduct {
                id: reserved.id().clone(),
                project_id,
                product_id,
                expected_project_version,
                expected_product_version,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    /// The provenance a Delivery record already has. An edit keeps it: the
    /// domain's update commands carry a provenance, but a person editing a
    /// seeded record does not change where that record came from, and the
    /// row itself is never rewritten by an update.
    pub fn delivery_provenance(
        &self,
        table: &str,
        id: &str,
        context: &DeliveryContext,
    ) -> Result<Provenance, LedgerTransactionError<DomainError>> {
        let link_context = LinkContext {
            idempotency_id: context.idempotency_id.clone(),
            correlation_id: context.correlation_id.clone(),
        };
        let row: Option<(String, Option<String>)> = self
            .connection
            .query_row(
                &format!("SELECT provenance_kind,provenance_reference FROM {table} WHERE id=?1"),
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| {
                LedgerTransactionError::Operation(storage_unavailable(&context.correlation_id))
            })?;
        match row {
            None => Err(LedgerTransactionError::Operation(DomainError::new(
                ErrorCode::DomainNotFound,
                MessageKey::parse("delivery.not_found").unwrap_or_else(|_| unreachable!()),
                context.correlation_id.clone(),
                false,
            ))),
            Some((kind, reference)) => {
                super::relationship_repository::decode_provenance(&kind, reference, &link_context)
                    .map_err(LedgerTransactionError::Operation)
            }
        }
    }

    // ---- Delivery entry reads -------------------------------------------

    /// Every Initiative, in name order: the candidates a Project links.
    pub fn list_initiative_entries(&self) -> Result<Vec<InitiativeEntryRecord>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare("SELECT record.id,record.name,record.defined_outcome,registry.classification,registry.version FROM initiatives record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='initiative' ORDER BY record.name,record.id")
            .map_err(read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(read_error)?;
        rows.into_iter().map(initiative_entry).collect()
    }

    /// Every Project, in name order: the candidates a Product links.
    pub fn list_project_entries(&self) -> Result<Vec<ProjectEntryRecord>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare("SELECT record.id,record.name,record.start_at,record.end_at,registry.classification,registry.version FROM projects record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='project' ORDER BY record.name,record.id")
            .map_err(read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(read_error)?;
        rows.into_iter().map(project_entry).collect()
    }

    pub fn read_initiative_entry(
        &self,
        id: &InitiativeId,
    ) -> Result<Option<InitiativeEntryRecord>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT record.id,record.name,record.defined_outcome,registry.classification,registry.version FROM initiatives record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='initiative' WHERE record.id=?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?
            .map(initiative_entry)
            .transpose()
    }

    pub fn read_project_entry(
        &self,
        id: &ProjectId,
    ) -> Result<Option<ProjectEntryRecord>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT record.id,record.name,record.start_at,record.end_at,registry.classification,registry.version FROM projects record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='project' WHERE record.id=?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?
            .map(project_entry)
            .transpose()
    }

    pub fn read_milestone_entry(
        &self,
        id: &MilestoneId,
    ) -> Result<Option<MilestoneEntryRecord>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT record.id,record.project_id,record.name,record.verification_criteria,record.due_at,registry.classification,registry.version FROM milestones record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='milestone' WHERE record.id=?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?
            .map(|row| {
                Ok(MilestoneEntryRecord {
                    id: row.0,
                    project_id: row.1,
                    name: row.2,
                    verification_criteria: row.3,
                    due_at: UtcTimestamp::from_unix_millis(row.4),
                    classification: DataClassification::from_persisted(&row.5)
                        .map_err(|_| LedgerOpenError::InvalidMetadata)?,
                    version: version_of(row.6)?,
                })
            })
            .transpose()
    }
}

fn initiative_entry(
    row: (String, String, String, String, i64),
) -> Result<InitiativeEntryRecord, LedgerOpenError> {
    Ok(InitiativeEntryRecord {
        id: row.0,
        name: row.1,
        defined_outcome: row.2,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| LedgerOpenError::InvalidMetadata)?,
        version: version_of(row.4)?,
    })
}

fn project_entry(
    row: (String, String, i64, i64, String, i64),
) -> Result<ProjectEntryRecord, LedgerOpenError> {
    Ok(ProjectEntryRecord {
        id: row.0,
        name: row.1,
        start_at: UtcTimestamp::from_unix_millis(row.2),
        end_at: UtcTimestamp::from_unix_millis(row.3),
        classification: DataClassification::from_persisted(&row.4)
            .map_err(|_| LedgerOpenError::InvalidMetadata)?,
        version: version_of(row.5)?,
    })
}

/// The reservation every Portfolio-family create and link names.
pub const CREATE_PORTFOLIO: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Portfolio,
    operation: "create_portfolio",
};
pub const CREATE_PRODUCT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Product,
    operation: "create_product",
};
pub const CREATE_ROADMAP: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Roadmap,
    operation: "create_roadmap",
};
pub const CREATE_KPI_DEFINITION: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::KpiDefinition,
    operation: "create_kpi_definition",
};
pub const CREATE_KPI_OBSERVATION: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::KpiObservation,
    operation: "create_kpi_observation",
};
pub const LINK_PORTFOLIO_PRODUCT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Relationship,
    operation: "link_portfolio_product",
};
pub const LINK_PRODUCT_ROADMAP: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Relationship,
    operation: "link_product_roadmap",
};
pub const LINK_PRODUCT_KPI: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Relationship,
    operation: "link_product_kpi",
};

/// A Portfolio, Product or Roadmap as a sheet edits it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimpleEntryRecord {
    pub id: String,
    pub name: String,
    pub details: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiDefinitionEntryRecord {
    pub id: String,
    pub name: String,
    pub definition: String,
    pub owner: String,
    pub target: String,
    pub cadence: String,
    pub source: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiObservationEntryRecord {
    pub id: String,
    pub kpi_id: String,
    pub value: String,
    pub observed_at: UtcTimestamp,
    pub source: String,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

fn idempotency_conflict(correlation: &pmc_domain::identity::CorrelationId) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("ledger.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        correlation.clone(),
        false,
    )
}

fn storage_unavailable(correlation: &pmc_domain::identity::CorrelationId) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("ledger.persistence_failed").unwrap_or_else(|_| unreachable!()),
        correlation.clone(),
        true,
    )
}

pub(super) fn read_error(error: rusqlite::Error) -> LedgerOpenError {
    match error {
        rusqlite::Error::SqliteFailure(ref details, _)
            if details.code == rusqlite::ErrorCode::DatabaseBusy =>
        {
            LedgerOpenError::Busy
        }
        _ => LedgerOpenError::StorageUnavailable,
    }
}

impl SqliteProductLedger {
    /// The reservation this create or link rests on exists in this Ledger and
    /// names exactly `request`, this idempotency id and this id.
    fn require_reservation<T: ReservableId>(
        &self,
        reserved: &ReservedId<T>,
        request: ReservationRequest,
        idempotency_id: &pmc_domain::identity::IdempotencyId,
        correlation: &pmc_domain::identity::CorrelationId,
    ) -> Result<(), LedgerTransactionError<DomainError>> {
        if reserved.idempotency_id() != idempotency_id || reserved.request() != request {
            return Err(LedgerTransactionError::Operation(idempotency_conflict(
                correlation,
            )));
        }
        match reservation_matches(&self.connection, reserved) {
            Ok(true) => Ok(()),
            Ok(false) => Err(LedgerTransactionError::Operation(idempotency_conflict(
                correlation,
            ))),
            Err(_) => Err(LedgerTransactionError::Operation(storage_unavailable(
                correlation,
            ))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_portfolio_from_reservation(
        &mut self,
        reserved: &ReservedId<PortfolioId>,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
        context: OperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<PortfolioRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_PORTFOLIO,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_portfolio(
            CreatePortfolio {
                id: reserved.id().clone(),
                name,
                details,
                classification,
                provenance: Provenance::UserEntered,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_product_from_reservation(
        &mut self,
        reserved: &ReservedId<ProductId>,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
        context: OperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<ProductRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_PRODUCT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_product(
            CreateProduct {
                id: reserved.id().clone(),
                name,
                details,
                classification,
                provenance: Provenance::UserEntered,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_roadmap_from_reservation(
        &mut self,
        reserved: &ReservedId<RoadmapId>,
        name: ShortText,
        details: LongText,
        classification: Option<DataClassification>,
        context: OperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RoadmapRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_ROADMAP,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_roadmap(
            CreateRoadmap {
                id: reserved.id().clone(),
                name,
                details,
                classification,
                provenance: Provenance::UserEntered,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    /// The fields of a KPI definition a sheet enters, in the domain's order.
    #[allow(clippy::too_many_arguments)]
    pub fn create_kpi_definition_from_reservation(
        &mut self,
        reserved: &ReservedId<KpiId>,
        name: ShortText,
        definition: LongText,
        owner: ShortText,
        target: ShortText,
        cadence: ShortText,
        source: LongText,
        classification: Option<DataClassification>,
        context: OperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_KPI_DEFINITION,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_kpi_definition(
            CreateKpiDefinition {
                id: reserved.id().clone(),
                name,
                definition,
                owner,
                target,
                cadence,
                source,
                classification,
                provenance: Provenance::UserEntered,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    /// The record a sheet read is still at the version it read. The domain's
    /// create commands for a child record (an observation under a KPI, a
    /// Milestone under a Project) bind no parent version, so the boundary
    /// rule's "the version the sheet read" (§4) is checked here, under the
    /// host's one write lock, before the create runs.
    fn require_current_version(
        &self,
        aggregate_type: &str,
        id: &str,
        expected: AggregateVersion,
        stale_key: &str,
        not_found_key: &str,
        correlation: &pmc_domain::identity::CorrelationId,
    ) -> Result<(), LedgerTransactionError<DomainError>> {
        let current: Option<i64> = self
            .connection
            .query_row(
                "SELECT version FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                [id, aggregate_type],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| LedgerTransactionError::Operation(storage_unavailable(correlation)))?;
        let key = |key: &str| MessageKey::parse(key).unwrap_or_else(|_| unreachable!());
        match current {
            None => Err(LedgerTransactionError::Operation(DomainError::new(
                ErrorCode::DomainNotFound,
                key(not_found_key),
                correlation.clone(),
                false,
            ))),
            Some(current) if current == i64::try_from(expected.get()).unwrap_or(-1) => Ok(()),
            Some(current) => {
                let mut error = DomainError::new(
                    ErrorCode::DomainConflict,
                    key(stale_key),
                    correlation.clone(),
                    false,
                );
                if let Some(version) = u64::try_from(current)
                    .ok()
                    .and_then(|raw| AggregateVersion::new(raw).ok())
                {
                    error = error.with_extension(
                        pmc_domain::error::SafeErrorExtension::CurrentVersion(version),
                    );
                }
                Err(LedgerTransactionError::Operation(error))
            }
        }
    }

    /// An observation is entered under the KPI the sheet read, at the version
    /// it read: a definition changed since is a conflict, not a silent
    /// attachment to something the person did not see.
    #[allow(clippy::too_many_arguments)]
    pub fn create_kpi_observation_from_reservation(
        &mut self,
        reserved: &ReservedId<KpiObservationId>,
        kpi_id: KpiId,
        expected_kpi_version: AggregateVersion,
        value: ShortText,
        observed_at: UtcTimestamp,
        source: LongText,
        classification: Option<DataClassification>,
        context: OperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiObservationRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_KPI_OBSERVATION,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.require_current_version(
            "kpi_definition",
            kpi_id.as_str(),
            expected_kpi_version,
            "kpi.stale_version",
            "kpi.not_found",
            &context.correlation_id,
        )?;
        self.create_kpi_observation(
            CreateKpiObservation {
                id: reserved.id().clone(),
                kpi_id,
                value,
                observed_at,
                source,
                classification,
                provenance: Provenance::UserEntered,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link_portfolio_product_from_reservation(
        &mut self,
        reserved: &ReservedId<RelationshipId>,
        portfolio_id: PortfolioId,
        expected_portfolio_version: AggregateVersion,
        product_id: ProductId,
        expected_product_version: AggregateVersion,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            LINK_PORTFOLIO_PRODUCT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.link_portfolio_product(
            LinkPortfolioProduct {
                id: reserved.id().clone(),
                portfolio_id,
                product_id,
                expected_portfolio_version,
                expected_product_version,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link_product_roadmap_from_reservation(
        &mut self,
        reserved: &ReservedId<RelationshipId>,
        product_id: ProductId,
        expected_product_version: AggregateVersion,
        roadmap_id: RoadmapId,
        expected_roadmap_version: AggregateVersion,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            LINK_PRODUCT_ROADMAP,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.link_product_roadmap(
            LinkProductRoadmap {
                id: reserved.id().clone(),
                product_id,
                roadmap_id,
                expected_product_version,
                expected_roadmap_version,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn link_product_kpi_from_reservation(
        &mut self,
        reserved: &ReservedId<RelationshipId>,
        product_id: ProductId,
        expected_product_version: AggregateVersion,
        kpi_id: KpiId,
        expected_kpi_version: AggregateVersion,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            LINK_PRODUCT_KPI,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.link_product_kpi(
            LinkProductKpi {
                id: reserved.id().clone(),
                product_id,
                kpi_id,
                expected_product_version,
                expected_kpi_version,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    // ---- Edit-detail reads --------------------------------------------

    fn read_simple_entry(
        &self,
        table: &str,
        aggregate_type: &str,
        id: &str,
    ) -> Result<Option<SimpleEntryRecord>, LedgerOpenError> {
        let row = self
            .connection
            .query_row(
                &format!("SELECT record.id,record.name,record.details,registry.classification,registry.version FROM {table} record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='{aggregate_type}' WHERE record.id=?1"),
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?;
        row.map(|(id, name, details, classification, version)| {
            Ok(SimpleEntryRecord {
                id,
                name,
                details,
                classification: DataClassification::from_persisted(&classification)
                    .map_err(|_| LedgerOpenError::InvalidMetadata)?,
                version: version_of(version)?,
            })
        })
        .transpose()
    }

    /// Every Portfolio, in name order, as a sheet lists and edits them.
    pub fn list_portfolio_entries(&self) -> Result<Vec<SimpleEntryRecord>, LedgerOpenError> {
        self.list_simple_entries("portfolios", "portfolio")
    }

    /// Every Product, in name order: the candidates a Portfolio links.
    pub fn list_product_entries(&self) -> Result<Vec<SimpleEntryRecord>, LedgerOpenError> {
        self.list_simple_entries("products", "product")
    }

    /// Every Roadmap, in name order: the candidates a Product links.
    pub fn list_roadmap_entries(&self) -> Result<Vec<SimpleEntryRecord>, LedgerOpenError> {
        self.list_simple_entries("roadmaps", "roadmap")
    }

    /// Every KPI definition, in name order: the candidates a Product links.
    pub fn list_kpi_definition_entries(
        &self,
    ) -> Result<Vec<KpiDefinitionEntryRecord>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare("SELECT record.id,record.name,record.definition,record.owner,record.target,record.cadence,record.source,registry.classification,registry.version FROM kpi_definitions record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='kpi_definition' ORDER BY record.name,record.id")
            .map_err(read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .map_err(read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(read_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(KpiDefinitionEntryRecord {
                    id: row.0,
                    name: row.1,
                    definition: row.2,
                    owner: row.3,
                    target: row.4,
                    cadence: row.5,
                    source: row.6,
                    classification: DataClassification::from_persisted(&row.7)
                        .map_err(|_| LedgerOpenError::InvalidMetadata)?,
                    version: version_of(row.8)?,
                })
            })
            .collect()
    }

    fn list_simple_entries(
        &self,
        table: &str,
        aggregate_type: &str,
    ) -> Result<Vec<SimpleEntryRecord>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT record.id,record.name,record.details,registry.classification,registry.version FROM {table} record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='{aggregate_type}' ORDER BY record.name,record.id"))
            .map_err(read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(read_error)?;
        rows.into_iter()
            .map(|(id, name, details, classification, version)| {
                Ok(SimpleEntryRecord {
                    id,
                    name,
                    details,
                    classification: DataClassification::from_persisted(&classification)
                        .map_err(|_| LedgerOpenError::InvalidMetadata)?,
                    version: version_of(version)?,
                })
            })
            .collect()
    }

    pub fn read_portfolio_entry(
        &self,
        id: &PortfolioId,
    ) -> Result<Option<SimpleEntryRecord>, LedgerOpenError> {
        self.read_simple_entry("portfolios", "portfolio", id.as_str())
    }

    pub fn read_product_entry(
        &self,
        id: &ProductId,
    ) -> Result<Option<SimpleEntryRecord>, LedgerOpenError> {
        self.read_simple_entry("products", "product", id.as_str())
    }

    pub fn read_roadmap_entry(
        &self,
        id: &RoadmapId,
    ) -> Result<Option<SimpleEntryRecord>, LedgerOpenError> {
        self.read_simple_entry("roadmaps", "roadmap", id.as_str())
    }

    pub fn read_kpi_definition_entry(
        &self,
        id: &KpiId,
    ) -> Result<Option<KpiDefinitionEntryRecord>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT record.id,record.name,record.definition,record.owner,record.target,record.cadence,record.source,registry.classification,registry.version FROM kpi_definitions record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='kpi_definition' WHERE record.id=?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?
            .map(|row| {
                Ok(KpiDefinitionEntryRecord {
                    id: row.0,
                    name: row.1,
                    definition: row.2,
                    owner: row.3,
                    target: row.4,
                    cadence: row.5,
                    source: row.6,
                    classification: DataClassification::from_persisted(&row.7)
                        .map_err(|_| LedgerOpenError::InvalidMetadata)?,
                    version: version_of(row.8)?,
                })
            })
            .transpose()
    }

    pub fn read_kpi_observation_entry(
        &self,
        id: &KpiObservationId,
    ) -> Result<Option<KpiObservationEntryRecord>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT record.id,record.kpi_id,record.value,record.observed_at,record.source,registry.classification,registry.version FROM kpi_observations record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='kpi_observation' WHERE record.id=?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?
            .map(|row| {
                Ok(KpiObservationEntryRecord {
                    id: row.0,
                    kpi_id: row.1,
                    value: row.2,
                    observed_at: UtcTimestamp::from_unix_millis(row.3),
                    source: row.4,
                    classification: DataClassification::from_persisted(&row.5)
                        .map_err(|_| LedgerOpenError::InvalidMetadata)?,
                    version: version_of(row.6)?,
                })
            })
            .transpose()
    }
}

fn version_of(value: i64) -> Result<AggregateVersion, LedgerOpenError> {
    u64::try_from(value)
        .ok()
        .and_then(|value| AggregateVersion::new(value).ok())
        .ok_or(LedgerOpenError::InvalidMetadata)
}

/// The reservation every People create and link names (slice 6D).
pub const CREATE_STAKEHOLDER: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Stakeholder,
    operation: "create_stakeholder",
};
pub const LINK_STAKEHOLDER_SUBJECT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Relationship,
    operation: "link_stakeholder_relationship",
};

/// A Stakeholder as a sheet edits it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderEntryRecord {
    pub id: String,
    pub name: String,
    pub kind: StakeholderKind,
    pub classification: DataClassification,
    pub version: AggregateVersion,
}

/// People (slice 6D): a Stakeholder create through the Ledger's
/// reservation, the StakeholderSubject link, and the entry reads. Same
/// contract as the families above.
impl SqliteProductLedger {
    #[allow(clippy::too_many_arguments)]
    pub fn create_stakeholder_from_reservation(
        &mut self,
        reserved: &ReservedId<StakeholderId>,
        name: StakeholderName,
        kind: StakeholderKind,
        classification: Option<DataClassification>,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<StakeholderRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_STAKEHOLDER,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_stakeholder(
            CreateStakeholder {
                id: reserved.id().clone(),
                name,
                kind,
                classification,
                provenance: Provenance::UserEntered,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    /// A Stakeholder's relationship to a subject: the domain binds both
    /// versions and folds the classification of both ends.
    #[allow(clippy::too_many_arguments)]
    pub fn link_stakeholder_subject_from_reservation(
        &mut self,
        reserved: &ReservedId<RelationshipId>,
        stakeholder_id: StakeholderId,
        expected_stakeholder_version: AggregateVersion,
        subject: StakeholderSubject,
        expected_subject_version: AggregateVersion,
        purpose: StakeholderRelationshipPurpose,
        context: LinkContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<LinkOutcome<RelationshipRecord>, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            LINK_STAKEHOLDER_SUBJECT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.link_stakeholder_relationship(
            LinkStakeholderRelationship {
                id: reserved.id().clone(),
                stakeholder_id,
                subject,
                purpose,
                expected_stakeholder_version,
                expected_subject_version,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    /// Every Stakeholder, in name order.
    pub fn list_stakeholder_entries(&self) -> Result<Vec<StakeholderEntryRecord>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare("SELECT record.id,record.name,record.kind,registry.classification,registry.version FROM stakeholders record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='stakeholder' ORDER BY record.name,record.id")
            .map_err(read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(read_error)?;
        rows.into_iter().map(stakeholder_entry).collect()
    }

    pub fn read_stakeholder_entry(
        &self,
        id: &StakeholderId,
    ) -> Result<Option<StakeholderEntryRecord>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT record.id,record.name,record.kind,registry.classification,registry.version FROM stakeholders record JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='stakeholder' WHERE record.id=?1",
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?
            .map(stakeholder_entry)
            .transpose()
    }
}

fn stakeholder_entry(
    row: (String, String, String, String, i64),
) -> Result<StakeholderEntryRecord, LedgerOpenError> {
    Ok(StakeholderEntryRecord {
        id: row.0,
        name: row.1,
        kind: StakeholderKind::from_persisted(&row.2)
            .map_err(|_| LedgerOpenError::InvalidMetadata)?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| LedgerOpenError::InvalidMetadata)?,
        version: version_of(row.4)?,
    })
}

/// The reservation every work create names (slice 6E).
pub const CREATE_ACTION_REQUEST_DRAFT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::ActionRequest,
    operation: "create_action_request_draft",
};
pub const CREATE_DECISION_REQUEST_DRAFT: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::DecisionRequest,
    operation: "create_decision_request_draft",
};
pub const CREATE_ISSUE: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::Issue,
    operation: "create_issue",
};

/// The fields of an Action Request draft a sheet enters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequestDraftFields {
    pub title: ActionTitle,
    pub details: ActionDetails,
    pub intended_owner: Option<StakeholderId>,
    pub response_due_at: Option<UtcTimestamp>,
    pub intended_action_due_at: Option<UtcTimestamp>,
    pub classification: DataClassification,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRequestDraftFields {
    pub subject: DecisionSubject,
    pub details: DecisionText,
    pub intended_owner: Option<StakeholderId>,
    pub classification: DataClassification,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueFields {
    pub title: IssueTitle,
    pub details: IssueDetails,
    pub classification: DataClassification,
    pub recurrence_of: Option<IssueId>,
}

/// Work records (slice 6E): an Action Request draft, a Decision Request
/// draft and an Issue through the Ledger's reservation. Same contract as
/// the families above. These records carry no provenance field.
impl SqliteProductLedger {
    pub fn create_action_request_draft_from_reservation(
        &mut self,
        reserved: &ReservedId<ActionRequestId>,
        fields: ActionRequestDraftFields,
        context: ActionOperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        self.require_reservation(
            reserved,
            CREATE_ACTION_REQUEST_DRAFT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_action_request_draft(
            CreateActionRequestDraft {
                id: reserved.id().clone(),
                title: fields.title,
                details: fields.details,
                intended_owner: fields.intended_owner,
                response_due_at: fields.response_due_at,
                intended_action_due_at: fields.intended_action_due_at,
                classification: fields.classification,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    pub fn create_decision_request_draft_from_reservation(
        &mut self,
        reserved: &ReservedId<DecisionRequestId>,
        fields: DecisionRequestDraftFields,
        context: DecisionOperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        self.require_reservation(
            reserved,
            CREATE_DECISION_REQUEST_DRAFT,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_decision_request_draft(
            CreateDecisionRequestDraft {
                id: reserved.id().clone(),
                subject: fields.subject,
                details: fields.details,
                intended_owner: fields.intended_owner,
                classification: fields.classification,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }

    pub fn create_issue_from_reservation(
        &mut self,
        reserved: &ReservedId<IssueId>,
        fields: IssueFields,
        context: IssueOperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>> {
        self.require_reservation(
            reserved,
            CREATE_ISSUE,
            &context.idempotency_id,
            &context.correlation_id,
        )?;
        self.create_issue(
            CreateIssue {
                id: reserved.id().clone(),
                title: fields.title,
                details: fields.details,
                classification: fields.classification,
                recurrence_of: fields.recurrence_of,
                context,
            },
            audit_event_id,
            occurred_at,
        )
    }
}
