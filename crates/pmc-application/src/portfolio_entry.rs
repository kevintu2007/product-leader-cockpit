//! Record entry for the Portfolio family (slice 6B; DG3 record-entry
//! amendment §2–§4): what S02, the O01 header and the O01 Structure tab
//! submit. One function per operation, each one Ledger transaction (plus the
//! reservation's own, for a create or link):
//!
//! - `enter_*`: create, through the Ledger's reservation for the sheet's
//!   `clientRequestId`, so a retry names the same record;
//! - `revise_*`: update details at the version the sheet read; a classification
//!   may only be raised here (lowering is the H2a flow);
//! - `attach_*`: link two records the sheet read, at the versions it read.
//!
//! Provenance is `UserEntered`, set by the Ledger's reservation-checked
//! writers; the webview cannot choose it.

#![allow(clippy::result_large_err)]

use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    AggregateVersion, KpiId, KpiObservationId, PortfolioId, ProductId, RelationshipId, RoadmapId,
};
use pmc_domain::portfolio::{
    KpiDefinitionRecord, KpiObservationRecord, LongText, MutationOutcome, OperationContext,
    PortfolioRecord, ProductRecord, RoadmapRecord, ShortText, UpdateKpiDefinitionDetails,
    UpdateKpiObservationDetails, UpdatePortfolioDetails, UpdateProductDetails,
    UpdateRoadmapDetails,
};
use pmc_domain::relationships::{
    MutationOutcome as LinkOutcome, OperationContext as LinkContext, RelationshipRecord,
};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    SqliteProductLedger, CREATE_KPI_DEFINITION, CREATE_KPI_OBSERVATION, CREATE_PORTFOLIO,
    CREATE_PRODUCT, CREATE_ROADMAP, LINK_PORTFOLIO_PRODUCT, LINK_PRODUCT_KPI, LINK_PRODUCT_ROADMAP,
};

use crate::desktop_runtime::OpaqueIdSource;
use crate::record_entry::{reserve_record_id, RecordEntryError};

/// What a Portfolio, Product or Roadmap sheet sends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimpleEntry {
    pub name: ShortText,
    pub details: LongText,
    /// Chosen by the person on a create (§3.3); on an edit, the current one
    /// or a higher one.
    pub classification: Option<DataClassification>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiDefinitionEntry {
    pub name: ShortText,
    pub definition: LongText,
    pub owner: ShortText,
    pub target: ShortText,
    pub cadence: ShortText,
    pub source: LongText,
    pub classification: Option<DataClassification>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KpiObservationEntry {
    pub value: ShortText,
    /// The instant the person entered, already in UTC.
    pub observed_at: UtcTimestamp,
    pub source: LongText,
    pub classification: Option<DataClassification>,
}

fn audit_id(
    ids: &mut OpaqueIdSource,
) -> Result<pmc_domain::identity::AuditEventId, RecordEntryError> {
    AuditEventIdSource::next_audit_event_id(ids).map_err(RecordEntryError::Id)
}

pub fn enter_portfolio(
    ledger: &mut SqliteProductLedger,
    entry: SimpleEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<PortfolioRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<PortfolioId>(
        ledger,
        &context.idempotency_id,
        CREATE_PORTFOLIO,
        now,
        || ids.next_portfolio_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_portfolio_from_reservation(
        &reserved,
        entry.name,
        entry.details,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

pub fn revise_portfolio(
    ledger: &mut SqliteProductLedger,
    id: PortfolioId,
    expected_version: AggregateVersion,
    entry: SimpleEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<PortfolioRecord>, RecordEntryError> {
    let audit = audit_id(ids)?;
    Ok(ledger.update_portfolio_details(
        UpdatePortfolioDetails {
            id,
            expected_version,
            name: entry.name,
            details: entry.details,
            classification: entry.classification,
            context,
        },
        audit,
        now,
    )?)
}

pub fn enter_product(
    ledger: &mut SqliteProductLedger,
    entry: SimpleEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<ProductRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<ProductId>(
        ledger,
        &context.idempotency_id,
        CREATE_PRODUCT,
        now,
        || ids.next_product_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_product_from_reservation(
        &reserved,
        entry.name,
        entry.details,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

pub fn revise_product(
    ledger: &mut SqliteProductLedger,
    id: ProductId,
    expected_version: AggregateVersion,
    entry: SimpleEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<ProductRecord>, RecordEntryError> {
    let audit = audit_id(ids)?;
    Ok(ledger.update_product_details(
        UpdateProductDetails {
            id,
            expected_version,
            name: entry.name,
            details: entry.details,
            classification: entry.classification,
            context,
        },
        audit,
        now,
    )?)
}

pub fn enter_roadmap(
    ledger: &mut SqliteProductLedger,
    entry: SimpleEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<RoadmapRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RoadmapId>(
        ledger,
        &context.idempotency_id,
        CREATE_ROADMAP,
        now,
        || ids.next_roadmap_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_roadmap_from_reservation(
        &reserved,
        entry.name,
        entry.details,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

pub fn revise_roadmap(
    ledger: &mut SqliteProductLedger,
    id: RoadmapId,
    expected_version: AggregateVersion,
    entry: SimpleEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<RoadmapRecord>, RecordEntryError> {
    let audit = audit_id(ids)?;
    Ok(ledger.update_roadmap_details(
        UpdateRoadmapDetails {
            id,
            expected_version,
            name: entry.name,
            details: entry.details,
            classification: entry.classification,
            context,
        },
        audit,
        now,
    )?)
}

pub fn enter_kpi_definition(
    ledger: &mut SqliteProductLedger,
    entry: KpiDefinitionEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<KpiDefinitionRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<KpiId>(
        ledger,
        &context.idempotency_id,
        CREATE_KPI_DEFINITION,
        now,
        || ids.next_kpi_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_kpi_definition_from_reservation(
        &reserved,
        entry.name,
        entry.definition,
        entry.owner,
        entry.target,
        entry.cadence,
        entry.source,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

/// Raising a KPI definition's classification raises every observation that
/// inherits it, each with its own audit; the Ledger asks for as many audit
/// ids as it needs.
pub fn revise_kpi_definition(
    ledger: &mut SqliteProductLedger,
    id: KpiId,
    expected_version: AggregateVersion,
    entry: KpiDefinitionEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<KpiDefinitionRecord>, RecordEntryError> {
    // The Ledger asks for audit ids as it fans out, through an infallible
    // closure. An opaque id is hex, digits and hyphens well under the limit,
    // so minting one cannot fail; the first is minted here so a source that
    // somehow cannot mint surfaces as the host bug it would be.
    let first = audit_id(ids)?;
    let mut pending = Some(first);
    let outcome = ledger.update_kpi_definition_details(
        UpdateKpiDefinitionDetails {
            id,
            expected_version,
            name: entry.name,
            definition: entry.definition,
            owner: entry.owner,
            target: entry.target,
            cadence: entry.cadence,
            source: entry.source,
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
    )?;
    Ok(outcome)
}

/// An observation under the KPI the sheet read, at the version it read.
pub fn enter_kpi_observation(
    ledger: &mut SqliteProductLedger,
    kpi: (KpiId, AggregateVersion),
    entry: KpiObservationEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<KpiObservationRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<KpiObservationId>(
        ledger,
        &context.idempotency_id,
        CREATE_KPI_OBSERVATION,
        now,
        || ids.next_kpi_observation_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_kpi_observation_from_reservation(
        &reserved,
        kpi.0,
        kpi.1,
        entry.value,
        entry.observed_at,
        entry.source,
        entry.classification,
        context,
        audit,
        now,
    )?)
}

pub fn revise_kpi_observation(
    ledger: &mut SqliteProductLedger,
    id: KpiObservationId,
    expected_version: AggregateVersion,
    entry: KpiObservationEntry,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<MutationOutcome<KpiObservationRecord>, RecordEntryError> {
    let audit = audit_id(ids)?;
    Ok(ledger.update_kpi_observation_details(
        UpdateKpiObservationDetails {
            id,
            expected_version,
            value: entry.value,
            observed_at: entry.observed_at,
            source: entry.source,
            classification: entry.classification,
            context,
        },
        audit,
        now,
    )?)
}

pub fn attach_portfolio_product(
    ledger: &mut SqliteProductLedger,
    portfolio: (PortfolioId, AggregateVersion),
    product: (ProductId, AggregateVersion),
    context: LinkContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<LinkOutcome<RelationshipRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RelationshipId>(
        ledger,
        &context.idempotency_id,
        LINK_PORTFOLIO_PRODUCT,
        now,
        || ids.next_relationship_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.link_portfolio_product_from_reservation(
        &reserved,
        portfolio.0,
        portfolio.1,
        product.0,
        product.1,
        context,
        audit,
        now,
    )?)
}

pub fn attach_product_roadmap(
    ledger: &mut SqliteProductLedger,
    product: (ProductId, AggregateVersion),
    roadmap: (RoadmapId, AggregateVersion),
    context: LinkContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<LinkOutcome<RelationshipRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RelationshipId>(
        ledger,
        &context.idempotency_id,
        LINK_PRODUCT_ROADMAP,
        now,
        || ids.next_relationship_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.link_product_roadmap_from_reservation(
        &reserved, product.0, product.1, roadmap.0, roadmap.1, context, audit, now,
    )?)
}

pub fn attach_product_kpi(
    ledger: &mut SqliteProductLedger,
    product: (ProductId, AggregateVersion),
    kpi: (KpiId, AggregateVersion),
    context: LinkContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<LinkOutcome<RelationshipRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<RelationshipId>(
        ledger,
        &context.idempotency_id,
        LINK_PRODUCT_KPI,
        now,
        || ids.next_relationship_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger.link_product_kpi_from_reservation(
        &reserved, product.0, product.1, kpi.0, kpi.1, context, audit, now,
    )?)
}
