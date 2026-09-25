//! Record entry for People (slice 6D; DG3 record-entry amendment §2–§4):
//! what S09 submits — a Stakeholder, its edit, and a Stakeholder's
//! relationship to a subject (responsible for it, or depending on it).
//! Same shape as `portfolio_entry`: `enter_*` reserves then creates,
//! `revise_*` updates at the version the sheet read, `attach_*` links two
//! records the sheet read at the versions it read. Provenance is
//! `UserEntered`, set by the Ledger's reservation-checked writers.

#![allow(clippy::result_large_err)]

use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{AggregateVersion, RelationshipId, StakeholderId};
use pmc_domain::relationships::{
    MutationOutcome, OperationContext, RelationshipRecord, StakeholderKind, StakeholderName,
    StakeholderRecord, StakeholderRelationshipPurpose, StakeholderSubject,
    UpdateStakeholderDetails,
};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{SqliteProductLedger, CREATE_STAKEHOLDER, LINK_STAKEHOLDER_SUBJECT};

use crate::desktop_runtime::OpaqueIdSource;
use crate::record_entry::{reserve_record_id, RecordEntryError};

/// What a Stakeholder sheet sends. The kind is chosen on a create and is
/// not part of an edit: the domain's update carries name and classification
/// only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeholderEntry {
    pub name: StakeholderName,
    pub classification: Option<DataClassification>,
}

fn audit_id(
    ids: &mut OpaqueIdSource,
) -> Result<pmc_domain::identity::AuditEventId, RecordEntryError> {
    AuditEventIdSource::next_audit_event_id(ids).map_err(RecordEntryError::Id)
}

pub fn enter_stakeholder(
    ledger: &mut SqliteProductLedger,
    entry: StakeholderEntry,
    kind: StakeholderKind,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<StakeholderRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<StakeholderId>(
        ledger,
        &context.idempotency_id,
        CREATE_STAKEHOLDER,
        now,
        || ids.next_stakeholder_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_stakeholder_from_reservation(
        &reserved,
        entry.name,
        kind,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

/// Raising a Stakeholder's classification raises every relationship that
/// inherits it, each with its own audit; the Ledger asks for as many audit
/// ids as it needs.
pub fn revise_stakeholder(
    ledger: &mut SqliteProductLedger,
    id: StakeholderId,
    expected_version: AggregateVersion,
    entry: StakeholderEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<StakeholderRecord>, RecordEntryError> {
    // As in `revise_kpi_definition`: the first audit id is minted here so a
    // source that cannot mint surfaces as the host bug it would be.
    let first = audit_id(ids)?;
    let mut pending = Some(first);
    Ok(ledger.update_stakeholder(
        UpdateStakeholderDetails {
            id,
            expected_version,
            name: entry.name,
            classification: entry.classification,
            context,
        },
        || {
            pending.take().unwrap_or_else(|| {
                AuditEventIdSource::next_audit_event_id(&mut *ids)
                    .unwrap_or_else(|_| unreachable!("opaque audit ids are always valid"))
            })
        },
        now,
    )?)
}

/// A Stakeholder's relationship to a subject it is responsible for or
/// depends on, at the versions the sheet read of both.
pub fn attach_stakeholder_subject(
    ledger: &mut SqliteProductLedger,
    stakeholder: (StakeholderId, AggregateVersion),
    subject: (StakeholderSubject, AggregateVersion),
    purpose: StakeholderRelationshipPurpose,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<RelationshipRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RelationshipId>(
        ledger,
        &context.idempotency_id,
        LINK_STAKEHOLDER_SUBJECT,
        now,
        || ids.next_relationship_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.link_stakeholder_subject_from_reservation(
        &reserved,
        stakeholder.0,
        stakeholder.1,
        subject.0,
        subject.1,
        purpose,
        context,
        audit,
        now,
    )?)
}
