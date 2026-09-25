//! Record entry for the Delivery family (slice 6C; DG3 record-entry
//! amendment §2–§4): what the O01 Structure tab submits for Initiative,
//! Project and Milestone, and the InitiativeProject / ProjectProduct links.
//! Same shape as `portfolio_entry`: `enter_*` reserves then creates,
//! `revise_*` updates at the version the sheet read, `attach_*` links two
//! records the sheet read at the versions it read. Provenance is
//! `UserEntered`, set by the Ledger's reservation-checked writers.

#![allow(clippy::result_large_err)]

use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::delivery::{
    DefinedOutcome, Initiative, Milestone, OperationContext, Project, RecordName, UpdateInitiative,
    UpdateMilestone, UpdateProject, VerificationCriteria,
};
use pmc_domain::error::{DomainError, ErrorCode, MessageKey};
use pmc_domain::identity::{
    AggregateVersion, InitiativeId, MilestoneId, ProductId, ProjectId, RelationshipId,
};
use pmc_domain::relationships::{
    MutationOutcome as LinkOutcome, OperationContext as LinkContext, RelationshipRecord,
};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    DeliveryMutationOutcome, SqliteProductLedger, CREATE_INITIATIVE, CREATE_MILESTONE,
    CREATE_PROJECT, LINK_INITIATIVE_PROJECT, LINK_PROJECT_PRODUCT,
};

use crate::desktop_runtime::OpaqueIdSource;
use crate::record_entry::{reserve_record_id, RecordEntryError};

/// What an Initiative sheet sends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitiativeEntry {
    pub name: RecordName,
    pub defined_outcome: DefinedOutcome,
    /// Chosen by the person on a create (§3.3); on an edit, the current one
    /// or a higher one.
    pub classification: Option<DataClassification>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectEntry {
    pub name: RecordName,
    /// Instants the person entered, already in UTC.
    pub start_at: UtcTimestamp,
    pub end_at: UtcTimestamp,
    pub classification: Option<DataClassification>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MilestoneEntry {
    pub name: RecordName,
    pub verification_criteria: VerificationCriteria,
    pub due_at: UtcTimestamp,
    pub classification: Option<DataClassification>,
}

fn audit_id(
    ids: &mut OpaqueIdSource,
) -> Result<pmc_domain::identity::AuditEventId, RecordEntryError> {
    AuditEventIdSource::next_audit_event_id(ids).map_err(RecordEntryError::Id)
}

pub fn enter_initiative(
    ledger: &mut SqliteProductLedger,
    entry: InitiativeEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DeliveryMutationOutcome<Initiative>, RecordEntryError> {
    let reserved = reserve_record_id::<InitiativeId>(
        ledger,
        &context.idempotency_id,
        CREATE_INITIATIVE,
        now,
        || ids.next_initiative_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_initiative_from_reservation(
        &reserved,
        entry.name,
        entry.defined_outcome,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

pub fn revise_initiative(
    ledger: &mut SqliteProductLedger,
    id: InitiativeId,
    expected_version: AggregateVersion,
    entry: InitiativeEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DeliveryMutationOutcome<Initiative>, RecordEntryError> {
    let provenance = ledger.delivery_provenance("initiatives", id.as_str(), &context)?;
    let audit = audit_id(ids)?;
    Ok(ledger.update_initiative(
        UpdateInitiative {
            context,
            id,
            expected_version,
            name: entry.name,
            defined_outcome: entry.defined_outcome,
            classification: entry.classification,
            provenance,
        },
        audit,
        now,
    )?)
}

pub fn enter_project(
    ledger: &mut SqliteProductLedger,
    entry: ProjectEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DeliveryMutationOutcome<Project>, RecordEntryError> {
    let reserved = reserve_record_id::<ProjectId>(
        ledger,
        &context.idempotency_id,
        CREATE_PROJECT,
        now,
        || ids.next_project_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_project_from_reservation(
        &reserved,
        entry.name,
        entry.start_at,
        entry.end_at,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

/// Raising a Project's classification raises every Milestone under it that
/// does not already dominate it, each with its own audit; the Ledger asks
/// for as many audit ids as it needs.
pub fn revise_project(
    ledger: &mut SqliteProductLedger,
    id: ProjectId,
    expected_version: AggregateVersion,
    entry: ProjectEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DeliveryMutationOutcome<Project>, RecordEntryError> {
    let provenance = ledger.delivery_provenance("projects", id.as_str(), &context)?;
    let audit = audit_id(ids)?;
    let correlation = context.correlation_id.clone();
    Ok(ledger.update_project(
        UpdateProject {
            context,
            id,
            expected_version,
            name: entry.name,
            start_at: entry.start_at,
            end_at: entry.end_at,
            classification: entry.classification,
            provenance,
        },
        audit,
        || {
            AuditEventIdSource::next_audit_event_id(&mut *ids).map_err(|_| {
                DomainError::new(
                    ErrorCode::PlatformInternal,
                    MessageKey::parse("desktop.host_id_source_failed")
                        .unwrap_or_else(|_| unreachable!("a literal key")),
                    correlation.clone(),
                    false,
                )
            })
        },
        now,
    )?)
}

/// A Milestone under the Project the sheet read, at the version it read.
pub fn enter_milestone(
    ledger: &mut SqliteProductLedger,
    project: (ProjectId, AggregateVersion),
    entry: MilestoneEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DeliveryMutationOutcome<Milestone>, RecordEntryError> {
    let reserved = reserve_record_id::<MilestoneId>(
        ledger,
        &context.idempotency_id,
        CREATE_MILESTONE,
        now,
        || ids.next_milestone_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_milestone_from_reservation(
        &reserved,
        project.0,
        project.1,
        entry.name,
        entry.verification_criteria,
        entry.due_at,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

pub fn revise_milestone(
    ledger: &mut SqliteProductLedger,
    id: MilestoneId,
    expected_version: AggregateVersion,
    entry: MilestoneEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DeliveryMutationOutcome<Milestone>, RecordEntryError> {
    let provenance = ledger.delivery_provenance("milestones", id.as_str(), &context)?;
    let audit = audit_id(ids)?;
    Ok(ledger.update_milestone(
        UpdateMilestone {
            context,
            id,
            expected_version,
            name: entry.name,
            verification_criteria: entry.verification_criteria,
            due_at: entry.due_at,
            classification: entry.classification,
            provenance,
        },
        audit,
        now,
    )?)
}

pub fn attach_initiative_project(
    ledger: &mut SqliteProductLedger,
    initiative: (InitiativeId, AggregateVersion),
    project: (ProjectId, AggregateVersion),
    context: LinkContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<LinkOutcome<RelationshipRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RelationshipId>(
        ledger,
        &context.idempotency_id,
        LINK_INITIATIVE_PROJECT,
        now,
        || ids.next_relationship_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.link_initiative_project_from_reservation(
        &reserved,
        initiative.0,
        initiative.1,
        project.0,
        project.1,
        context,
        audit,
        now,
    )?)
}

pub fn attach_project_product(
    ledger: &mut SqliteProductLedger,
    project: (ProjectId, AggregateVersion),
    product: (ProductId, AggregateVersion),
    context: LinkContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<LinkOutcome<RelationshipRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RelationshipId>(
        ledger,
        &context.idempotency_id,
        LINK_PROJECT_PRODUCT,
        now,
        || ids.next_relationship_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.link_project_product_from_reservation(
        &reserved, project.0, project.1, product.0, product.1, context, audit, now,
    )?)
}
