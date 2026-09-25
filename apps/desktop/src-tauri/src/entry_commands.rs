//! Record entry for the Portfolio family (slice 6B; DG3 record-entry
//! amendment §2–§4): the H0 reads a sheet opens with and the H1 commands it
//! submits, for Portfolio, Product, Roadmap, KPI definition and KPI
//! observation, and the three links between them.
//!
//! The boundary rule (§4): the webview supplies the fields the person
//! entered, the id and version of a record it read, and one
//! `clientRequestId` per opened sheet. The host mints every new id (through
//! the Ledger's reservation, so a retry names the same record), the
//! correlation, the audit ids and the instant; provenance is `UserEntered`
//! wherever the record carries one. The work aggregates (slice 6E: Action
//! Request, Decision Request, Risk, Issue) carry no provenance field in the
//! domain or the schema: for them `UserEntered` is an invariant of this one
//! trusted create path and its audit history, not a stored value (product
//! owner, 2026-09-22). No path crosses here in either direction.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use pmc_application::delivery_entry::{
    attach_initiative_project, attach_project_product, enter_initiative, enter_milestone,
    enter_project, revise_initiative, revise_milestone, revise_project, InitiativeEntry,
    MilestoneEntry, ProjectEntry,
};
use pmc_application::people_entry::{
    attach_stakeholder_subject, enter_stakeholder, revise_stakeholder, StakeholderEntry,
};
use pmc_application::portfolio_entry::{
    attach_portfolio_product, attach_product_kpi, attach_product_roadmap, enter_kpi_definition,
    enter_kpi_observation, enter_portfolio, enter_product, enter_roadmap, revise_kpi_definition,
    revise_kpi_observation, revise_portfolio, revise_product, revise_roadmap, KpiDefinitionEntry,
    KpiObservationEntry, SimpleEntry,
};
use pmc_application::record_entry::{enter_risk, RecordEntryError, RiskEntry};
use pmc_application::risk_lifecycle::{update_risk_response, RiskFlowError};
use pmc_application::work_entry::{
    enter_action_request_draft, enter_decision_request_draft, enter_issue, submit_action_request,
    submit_decision_request,
};
use pmc_domain::actions::{ActionDetails, ActionOperationContext, ActionTitle};
use pmc_domain::classification::DataClassification;
use pmc_domain::decisions::{DecisionOperationContext, DecisionSubject, DecisionText};
use pmc_domain::delivery::{
    DefinedOutcome, OperationContext as DeliveryContext, RecordName, VerificationCriteria,
};
use pmc_domain::identity::{
    ActionRequestId, AggregateVersion, CorrelationId, DecisionRequestId, IdempotencyId,
    InitiativeId, IssueId, KpiId, KpiObservationId, MilestoneId, PortfolioId, ProductId, ProjectId,
    RiskId, RoadmapId, StakeholderId,
};
use pmc_domain::issues::{IssueDetails, IssueOperationContext, IssueTitle};
use pmc_domain::portfolio::{LongText, OperationContext, ShortText};
use pmc_domain::relationships::{
    OperationContext as LinkContext, StakeholderKind, StakeholderName,
    StakeholderRelationshipPurpose, StakeholderSubject,
};
use pmc_domain::risks::{
    ResidualExposure, RiskDetails, RiskOperationContext, RiskRationale, RiskTitle,
    UpdateRiskResponse,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::RiskResponseType;
use pmc_ledger::sqlite::{
    ActionRequestDraftFields, DecisionRequestDraftFields, InitiativeEntryRecord, IssueFields,
    KpiDefinitionEntryRecord, KpiObservationEntryRecord, LedgerOpenError, MilestoneEntryRecord,
    ProjectEntryRecord, SimpleEntryRecord, StakeholderEntryRecord,
};
use serde::Serialize;
use tauri::State;

use crate::backup_gate::BackupGate;
use crate::ledger_state::LedgerState;
use crate::runtime::{host_correlation, HostRuntime};
use crate::safe_error::SafeErrorDto;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// A Portfolio, Product or Roadmap as a sheet edits it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleEntryDto {
    pub id: String,
    pub name: String,
    pub details: String,
    pub classification: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KpiDefinitionEntryDto {
    pub id: String,
    pub name: String,
    pub definition: String,
    pub owner: String,
    pub target: String,
    pub cadence: String,
    pub source: String,
    pub classification: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KpiObservationEntryDto {
    pub id: String,
    pub kpi_id: String,
    pub value: String,
    pub observed_at_millis: i64,
    pub source: String,
    pub classification: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitiativeEntryDto {
    pub id: String,
    pub name: String,
    pub defined_outcome: String,
    pub classification: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntryDto {
    pub id: String,
    pub name: String,
    pub start_at_millis: i64,
    pub end_at_millis: i64,
    pub classification: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneEntryDto {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub verification_criteria: String,
    pub due_at_millis: i64,
    pub classification: &'static str,
    pub version: u64,
}

/// One record's editable fields, tagged by kind.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryRecordDto {
    Portfolio(SimpleEntryDto),
    Product(SimpleEntryDto),
    Roadmap(SimpleEntryDto),
    KpiDefinition(KpiDefinitionEntryDto),
    KpiObservation(KpiObservationEntryDto),
    Initiative(InitiativeEntryDto),
    Project(ProjectEntryDto),
    Milestone(MilestoneEntryDto),
    Stakeholder(StakeholderEntryDto),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryListDto {
    pub ledger_revision: u64,
    pub records: Vec<EntryRecordDto>,
}

/// What every create, edit and link answers with: the record as the Ledger
/// now holds it, and the correlation of this command.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryOutcomeDto {
    pub kind: &'static str,
    pub id: String,
    pub classification: &'static str,
    pub version: u64,
    pub correlation_id: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn simple_dto(record: SimpleEntryRecord) -> SimpleEntryDto {
    SimpleEntryDto {
        id: record.id,
        name: record.name,
        details: record.details,
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn kpi_definition_dto(record: KpiDefinitionEntryRecord) -> KpiDefinitionEntryDto {
    KpiDefinitionEntryDto {
        id: record.id,
        name: record.name,
        definition: record.definition,
        owner: record.owner,
        target: record.target,
        cadence: record.cadence,
        source: record.source,
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn kpi_observation_dto(record: KpiObservationEntryRecord) -> KpiObservationEntryDto {
    KpiObservationEntryDto {
        id: record.id,
        kpi_id: record.kpi_id,
        value: record.value,
        observed_at_millis: record.observed_at.unix_millis(),
        source: record.source,
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn initiative_dto(record: InitiativeEntryRecord) -> InitiativeEntryDto {
    InitiativeEntryDto {
        id: record.id,
        name: record.name,
        defined_outcome: record.defined_outcome,
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn project_dto(record: ProjectEntryRecord) -> ProjectEntryDto {
    ProjectEntryDto {
        id: record.id,
        name: record.name,
        start_at_millis: record.start_at.unix_millis(),
        end_at_millis: record.end_at.unix_millis(),
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn milestone_dto(record: MilestoneEntryRecord) -> MilestoneEntryDto {
    MilestoneEntryDto {
        id: record.id,
        project_id: record.project_id,
        name: record.name,
        verification_criteria: record.verification_criteria,
        due_at_millis: record.due_at.unix_millis(),
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn invalid(correlation: &CorrelationId, key: &'static str) -> SafeErrorDto {
    SafeErrorDto::host("VALIDATION_INVALID_FIELD", key, correlation, false)
}

fn argument<T, E>(parsed: Result<T, E>, correlation: &CorrelationId) -> Result<T, SafeErrorDto> {
    parsed.map_err(|_| invalid(correlation, "desktop.invalid_argument"))
}

/// A text field as the domain bounds it (UTF-8 bytes, §3.4).
fn short(value: String, correlation: &CorrelationId) -> Result<ShortText, SafeErrorDto> {
    ShortText::parse(value).map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

fn long(value: String, correlation: &CorrelationId) -> Result<LongText, SafeErrorDto> {
    LongText::parse(value).map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

/// The Delivery family's texts (200 and 4000 bytes) parse against the
/// correlation, and refuse for the same reasons as every other text.
fn record_name(value: String, correlation: &CorrelationId) -> Result<RecordName, SafeErrorDto> {
    RecordName::parse(value, correlation)
        .map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

fn defined_outcome(
    value: String,
    correlation: &CorrelationId,
) -> Result<DefinedOutcome, SafeErrorDto> {
    DefinedOutcome::parse(value, correlation)
        .map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

fn verification_criteria(
    value: String,
    correlation: &CorrelationId,
) -> Result<VerificationCriteria, SafeErrorDto> {
    VerificationCriteria::parse(value, correlation)
        .map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

/// An instant the person entered, as the webview converted it; never
/// before the epoch, which the domain refuses too.
fn instant(millis: i64, correlation: &CorrelationId) -> Result<UtcTimestamp, SafeErrorDto> {
    if millis < 0 {
        return Err(invalid(correlation, "desktop.invalid_argument"));
    }
    Ok(UtcTimestamp::from_unix_millis(millis))
}

fn delivery_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<DeliveryContext, SafeErrorDto> {
    Ok(DeliveryContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

/// The classification the person chose; a create must choose one (§3.3),
/// which the sheet enforces and the host repeats here.
fn classification(
    value: Option<String>,
    required: bool,
    correlation: &CorrelationId,
) -> Result<Option<DataClassification>, SafeErrorDto> {
    match value {
        Some(value) => DataClassification::from_persisted(&value)
            .map(Some)
            .map_err(|_| invalid(correlation, "desktop.invalid_argument")),
        None if required => Err(invalid(
            correlation,
            "desktop.entry_classification_required",
        )),
        None => Ok(None),
    }
}

fn version(value: u64, correlation: &CorrelationId) -> Result<AggregateVersion, SafeErrorDto> {
    argument(AggregateVersion::new(value), correlation)
}

fn context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<OperationContext, SafeErrorDto> {
    Ok(OperationContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn link_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<LinkContext, SafeErrorDto> {
    Ok(LinkContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

pub(crate) fn entry_error(error: RecordEntryError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        RecordEntryError::RequestReused => SafeErrorDto::host(
            "DOMAIN_IDEMPOTENCY_CONFLICT",
            "ledger.idempotency_conflict",
            correlation,
            false,
        ),
        RecordEntryError::ReservationCorrupt => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.entry_reservation_corrupt",
            correlation,
            false,
        ),
        RecordEntryError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_id_source_failed",
            correlation,
            false,
        ),
        RecordEntryError::Domain(domain) => SafeErrorDto::from_domain(&domain),
        RecordEntryError::Ledger(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        RecordEntryError::IncompatibleLedger => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "ledger.transaction.incompatible_ledger",
            correlation,
            false,
        ),
        RecordEntryError::Storage => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "ledger.persistence_failed",
            correlation,
            true,
        ),
    }
}

fn read_error(error: LedgerOpenError, correlation: &CorrelationId) -> SafeErrorDto {
    SafeErrorDto::from_open(error, correlation)
}

fn not_found(correlation: &CorrelationId) -> SafeErrorDto {
    SafeErrorDto::host(
        "DOMAIN_NOT_FOUND",
        "desktop.entry_record_not_found",
        correlation,
        false,
    )
}

fn outcome(
    kind: &'static str,
    id: &str,
    classification: DataClassification,
    version: AggregateVersion,
    correlation: &CorrelationId,
) -> EntryOutcomeDto {
    EntryOutcomeDto {
        kind,
        id: id.to_owned(),
        classification: classification.as_persisted(),
        version: version.get(),
        correlation_id: correlation.as_str().to_owned(),
    }
}

// ---------------------------------------------------------------------------
// H0 reads
// ---------------------------------------------------------------------------

/// H0: every record of one kind with its editable fields and version: the
/// Portfolios S02 lists, and the candidates a link sheet offers.
#[tauri::command]
pub fn list_entry_records(
    state: State<'_, LedgerState>,
    kind: String,
) -> Result<EntryListDto, SafeErrorDto> {
    let correlation = host_correlation();
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&correlation))?;
    let ledger_revision = ledger
        .revision()
        .map_err(|error| read_error(error, &correlation))?;
    let read = |error| read_error(error, &correlation);
    let records = match kind.as_str() {
        "portfolio" => ledger
            .list_portfolio_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::Portfolio(simple_dto(record)))
            .collect(),
        "product" => ledger
            .list_product_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::Product(simple_dto(record)))
            .collect(),
        "roadmap" => ledger
            .list_roadmap_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::Roadmap(simple_dto(record)))
            .collect(),
        "kpi_definition" => ledger
            .list_kpi_definition_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::KpiDefinition(kpi_definition_dto(record)))
            .collect(),
        "initiative" => ledger
            .list_initiative_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::Initiative(initiative_dto(record)))
            .collect(),
        "project" => ledger
            .list_project_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::Project(project_dto(record)))
            .collect(),
        "stakeholder" => ledger
            .list_stakeholder_entries()
            .map_err(read)?
            .into_iter()
            .map(|record| EntryRecordDto::Stakeholder(stakeholder_dto(record)))
            .collect(),
        _ => return Err(invalid(&correlation, "desktop.invalid_argument")),
    };
    Ok(EntryListDto {
        ledger_revision,
        records,
    })
}

/// H0: one record's editable fields at its current version, for an edit
/// sheet (§3.8: an edit sends the version it read).
#[tauri::command]
pub fn get_entry_record(
    state: State<'_, LedgerState>,
    kind: String,
    id: String,
) -> Result<EntryRecordDto, SafeErrorDto> {
    let correlation = host_correlation();
    let ledger = state
        .read()
        .map_err(|closed| closed.to_safe_error(&correlation))?;
    let read = |error| read_error(error, &correlation);
    let record = match kind.as_str() {
        "portfolio" => ledger
            .read_portfolio_entry(&argument(PortfolioId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Portfolio(simple_dto(record))),
        "product" => ledger
            .read_product_entry(&argument(ProductId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Product(simple_dto(record))),
        "roadmap" => ledger
            .read_roadmap_entry(&argument(RoadmapId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Roadmap(simple_dto(record))),
        "kpi_definition" => ledger
            .read_kpi_definition_entry(&argument(KpiId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::KpiDefinition(kpi_definition_dto(record))),
        "kpi_observation" => ledger
            .read_kpi_observation_entry(&argument(KpiObservationId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::KpiObservation(kpi_observation_dto(record))),
        "initiative" => ledger
            .read_initiative_entry(&argument(InitiativeId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Initiative(initiative_dto(record))),
        "project" => ledger
            .read_project_entry(&argument(ProjectId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Project(project_dto(record))),
        "milestone" => ledger
            .read_milestone_entry(&argument(MilestoneId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Milestone(milestone_dto(record))),
        "stakeholder" => ledger
            .read_stakeholder_entry(&argument(StakeholderId::parse(id), &correlation)?)
            .map_err(read)?
            .map(|record| EntryRecordDto::Stakeholder(stakeholder_dto(record))),
        _ => return Err(invalid(&correlation, "desktop.invalid_argument")),
    };
    record.ok_or_else(|| not_found(&correlation))
}

// ---------------------------------------------------------------------------
// H1 creates and edits
// ---------------------------------------------------------------------------

fn simple_entry(
    name: String,
    details: String,
    classification_value: Option<String>,
    required: bool,
    correlation: &CorrelationId,
) -> Result<SimpleEntry, SafeErrorDto> {
    Ok(SimpleEntry {
        name: short(name, correlation)?,
        details: long(details, correlation)?,
        classification: classification(classification_value, required, correlation)?,
    })
}

#[tauri::command]
pub fn create_portfolio_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = simple_entry(name, details, classification, true, &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_portfolio(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "portfolio",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_portfolio_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(PortfolioId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = simple_entry(name, details, classification, false, &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_portfolio(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "portfolio",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
pub fn create_product_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = simple_entry(name, details, classification, true, &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_product(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "product",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_product_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(ProductId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = simple_entry(name, details, classification, false, &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_product(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "product",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
pub fn create_roadmap_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = simple_entry(name, details, classification, true, &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_roadmap(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "roadmap",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_roadmap_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(RoadmapId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = simple_entry(name, details, classification, false, &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_roadmap(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "roadmap",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[allow(clippy::too_many_arguments)]
fn kpi_definition_entry(
    name: String,
    definition: String,
    owner: String,
    target: String,
    cadence: String,
    source: String,
    classification_value: Option<String>,
    required: bool,
    correlation: &CorrelationId,
) -> Result<KpiDefinitionEntry, SafeErrorDto> {
    Ok(KpiDefinitionEntry {
        name: short(name, correlation)?,
        definition: long(definition, correlation)?,
        owner: short(owner, correlation)?,
        target: short(target, correlation)?,
        cadence: short(cadence, correlation)?,
        source: long(source, correlation)?,
        classification: classification(classification_value, required, correlation)?,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_kpi_definition_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    definition: String,
    owner: String,
    target: String,
    cadence: String,
    source: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = kpi_definition_entry(
        name,
        definition,
        owner,
        target,
        cadence,
        source,
        classification,
        true,
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_kpi_definition(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "kpi_definition",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_kpi_definition_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    definition: String,
    owner: String,
    target: String,
    cadence: String,
    source: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(KpiId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = kpi_definition_entry(
        name,
        definition,
        owner,
        target,
        cadence,
        source,
        classification,
        false,
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_kpi_definition(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "kpi_definition",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

fn kpi_observation_entry(
    value: String,
    observed_at_millis: i64,
    source: String,
    classification_value: Option<String>,
    correlation: &CorrelationId,
) -> Result<KpiObservationEntry, SafeErrorDto> {
    if observed_at_millis < 0 {
        return Err(invalid(correlation, "desktop.invalid_argument"));
    }
    Ok(KpiObservationEntry {
        value: short(value, correlation)?,
        observed_at: UtcTimestamp::from_unix_millis(observed_at_millis),
        source: long(source, correlation)?,
        // An observation may inherit its definition's classification: no
        // choice is a valid choice here (the domain combines them).
        classification: classification(classification_value, false, correlation)?,
    })
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_kpi_observation_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    kpi_id: String,
    kpi_version: u64,
    value: String,
    observed_at_millis: i64,
    source: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let kpi = (
        argument(KpiId::parse(kpi_id), &correlation)?,
        version(kpi_version, &correlation)?,
    );
    let entry = kpi_observation_entry(
        value,
        observed_at_millis,
        source,
        classification,
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_kpi_observation(&mut ledger, kpi, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "kpi_observation",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_kpi_observation_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    value: String,
    observed_at_millis: i64,
    source: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(KpiObservationId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = kpi_observation_entry(
        value,
        observed_at_millis,
        source,
        classification,
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_kpi_observation(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "kpi_observation",
        record.id.as_str(),
        record.classification,
        record.version,
        &correlation,
    ))
}

// ---------------------------------------------------------------------------
// H1 links
// ---------------------------------------------------------------------------

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_portfolio_product_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    portfolio_id: String,
    portfolio_version: u64,
    product_id: String,
    product_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let portfolio = (
        argument(PortfolioId::parse(portfolio_id), &correlation)?,
        version(portfolio_version, &correlation)?,
    );
    let product = (
        argument(ProductId::parse(product_id), &correlation)?,
        version(product_version, &correlation)?,
    );
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let linked = attach_portfolio_product(&mut ledger, portfolio, product, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = linked.into_value();
    Ok(outcome(
        "relationship",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_product_roadmap_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    product_id: String,
    product_version: u64,
    roadmap_id: String,
    roadmap_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let product = (
        argument(ProductId::parse(product_id), &correlation)?,
        version(product_version, &correlation)?,
    );
    let roadmap = (
        argument(RoadmapId::parse(roadmap_id), &correlation)?,
        version(roadmap_version, &correlation)?,
    );
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let linked = attach_product_roadmap(&mut ledger, product, roadmap, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = linked.into_value();
    Ok(outcome(
        "relationship",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_product_kpi_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    product_id: String,
    product_version: u64,
    kpi_id: String,
    kpi_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let product = (
        argument(ProductId::parse(product_id), &correlation)?,
        version(product_version, &correlation)?,
    );
    let kpi = (
        argument(KpiId::parse(kpi_id), &correlation)?,
        version(kpi_version, &correlation)?,
    );
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let linked = attach_product_kpi(&mut ledger, product, kpi, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = linked.into_value();
    Ok(outcome(
        "relationship",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

// ---------------------------------------------------------------------------
// H1: the Delivery family (slice 6C)
// ---------------------------------------------------------------------------

fn initiative_entry(
    name: String,
    defined_outcome_text: String,
    classification_value: Option<String>,
    required: bool,
    correlation: &CorrelationId,
) -> Result<InitiativeEntry, SafeErrorDto> {
    Ok(InitiativeEntry {
        name: record_name(name, correlation)?,
        defined_outcome: defined_outcome(defined_outcome_text, correlation)?,
        classification: classification(classification_value, required, correlation)?,
    })
}

fn project_entry(
    name: String,
    start_at_millis: i64,
    end_at_millis: i64,
    classification_value: Option<String>,
    required: bool,
    correlation: &CorrelationId,
) -> Result<ProjectEntry, SafeErrorDto> {
    Ok(ProjectEntry {
        name: record_name(name, correlation)?,
        start_at: instant(start_at_millis, correlation)?,
        end_at: instant(end_at_millis, correlation)?,
        classification: classification(classification_value, required, correlation)?,
    })
}

fn milestone_entry(
    name: String,
    criteria: String,
    due_at_millis: i64,
    classification_value: Option<String>,
    required: bool,
    correlation: &CorrelationId,
) -> Result<MilestoneEntry, SafeErrorDto> {
    Ok(MilestoneEntry {
        name: record_name(name, correlation)?,
        verification_criteria: verification_criteria(criteria, correlation)?,
        due_at: instant(due_at_millis, correlation)?,
        classification: classification(classification_value, required, correlation)?,
    })
}

#[tauri::command]
pub fn create_initiative_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    defined_outcome: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = initiative_entry(name, defined_outcome, classification, true, &correlation)?;
    let context = delivery_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_initiative(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "initiative",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_initiative_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    defined_outcome: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(InitiativeId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = initiative_entry(name, defined_outcome, classification, false, &correlation)?;
    let context = delivery_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_initiative(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "initiative",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_project_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    start_at_millis: i64,
    end_at_millis: i64,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = project_entry(
        name,
        start_at_millis,
        end_at_millis,
        classification,
        true,
        &correlation,
    )?;
    let context = delivery_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_project(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "project",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_project_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    start_at_millis: i64,
    end_at_millis: i64,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(ProjectId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = project_entry(
        name,
        start_at_millis,
        end_at_millis,
        classification,
        false,
        &correlation,
    )?;
    let context = delivery_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_project(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "project",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_milestone_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    project_id: String,
    project_version: u64,
    name: String,
    verification_criteria: String,
    due_at_millis: i64,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let project = (
        argument(ProjectId::parse(project_id), &correlation)?,
        version(project_version, &correlation)?,
    );
    let entry = milestone_entry(
        name,
        verification_criteria,
        due_at_millis,
        classification,
        true,
        &correlation,
    )?;
    let context = delivery_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_milestone(&mut ledger, project, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "milestone",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_milestone_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    verification_criteria: String,
    due_at_millis: i64,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(MilestoneId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = milestone_entry(
        name,
        verification_criteria,
        due_at_millis,
        classification,
        false,
        &correlation,
    )?;
    let context = delivery_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_milestone(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "milestone",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_initiative_project_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    initiative_id: String,
    initiative_version: u64,
    project_id: String,
    project_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let initiative = (
        argument(InitiativeId::parse(initiative_id), &correlation)?,
        version(initiative_version, &correlation)?,
    );
    let project = (
        argument(ProjectId::parse(project_id), &correlation)?,
        version(project_version, &correlation)?,
    );
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let linked =
        attach_initiative_project(&mut ledger, initiative, project, context, &mut ids, now)
            .map_err(|error| entry_error(error, &correlation))?;
    let record = linked.into_value();
    Ok(outcome(
        "relationship",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_project_product_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    project_id: String,
    project_version: u64,
    product_id: String,
    product_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let project = (
        argument(ProjectId::parse(project_id), &correlation)?,
        version(project_version, &correlation)?,
    );
    let product = (
        argument(ProductId::parse(product_id), &correlation)?,
        version(product_version, &correlation)?,
    );
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let linked = attach_project_product(&mut ledger, project, product, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = linked.into_value();
    Ok(outcome(
        "relationship",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StakeholderEntryDto {
    pub id: String,
    pub name: String,
    /// `person` or `organization`. Not `kind`: that is the DTO's tag.
    pub stakeholder_kind: &'static str,
    pub classification: &'static str,
    pub version: u64,
}

fn stakeholder_dto(record: StakeholderEntryRecord) -> StakeholderEntryDto {
    StakeholderEntryDto {
        id: record.id,
        name: record.name,
        stakeholder_kind: record.kind.as_persisted(),
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn stakeholder_name(
    value: String,
    correlation: &CorrelationId,
) -> Result<StakeholderName, SafeErrorDto> {
    StakeholderName::parse(value).map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

/// The subject of a Stakeholder relationship, from the kind and id the sheet
/// read. Milestone is a domain subject but no sheet offers it yet.
fn stakeholder_subject(
    kind: &str,
    id: String,
    correlation: &CorrelationId,
) -> Result<StakeholderSubject, SafeErrorDto> {
    Ok(match kind {
        "portfolio" => {
            StakeholderSubject::Portfolio(argument(PortfolioId::parse(id), correlation)?)
        }
        "product" => StakeholderSubject::Product(argument(ProductId::parse(id), correlation)?),
        "initiative" => {
            StakeholderSubject::Initiative(argument(InitiativeId::parse(id), correlation)?)
        }
        "project" => StakeholderSubject::Project(argument(ProjectId::parse(id), correlation)?),
        "roadmap" => StakeholderSubject::Roadmap(argument(RoadmapId::parse(id), correlation)?),
        // Milestone is a domain subject too, but no sheet offers it and the
        // relationship writer needs its parent Project, so it is refused
        // here rather than reserved and then refused.
        "kpi_definition" => StakeholderSubject::Kpi(argument(KpiId::parse(id), correlation)?),
        _ => return Err(invalid(correlation, "desktop.invalid_argument")),
    })
}

#[tauri::command]
pub fn create_stakeholder_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    name: String,
    kind: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = StakeholderEntry {
        name: stakeholder_name(name, &correlation)?,
        classification: self::classification(classification, true, &correlation)?,
    };
    let kind = argument(StakeholderKind::from_persisted(&kind), &correlation)?;
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_stakeholder(&mut ledger, entry, kind, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.into_value();
    Ok(outcome(
        "stakeholder",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_stakeholder_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    name: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(StakeholderId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let entry = StakeholderEntry {
        name: stakeholder_name(name, &correlation)?,
        classification: self::classification(classification, false, &correlation)?,
    };
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = revise_stakeholder(&mut ledger, id, expected, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = updated.into_value();
    Ok(outcome(
        "stakeholder",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_stakeholder_subject_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    stakeholder_id: String,
    stakeholder_version: u64,
    subject_kind: String,
    subject_id: String,
    subject_version: u64,
    purpose: String,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let stakeholder = (
        argument(StakeholderId::parse(stakeholder_id), &correlation)?,
        version(stakeholder_version, &correlation)?,
    );
    let subject = (
        stakeholder_subject(&subject_kind, subject_id, &correlation)?,
        version(subject_version, &correlation)?,
    );
    let purpose = argument(
        StakeholderRelationshipPurpose::from_persisted(&purpose),
        &correlation,
    )?;
    let context = link_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let linked = attach_stakeholder_subject(
        &mut ledger,
        stakeholder,
        subject,
        purpose,
        context,
        &mut ids,
        now,
    )
    .map_err(|error| entry_error(error, &correlation))?;
    let record = linked.into_value();
    Ok(outcome(
        "relationship",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

// ---------------------------------------------------------------------------
// H1: work records (slice 6E)
// ---------------------------------------------------------------------------

/// Every work text parses like the others: blank, over the byte limit or a
/// control character is `desktop.entry_text_invalid`.
fn work_text<T>(
    parsed: Result<T, pmc_domain::DomainValueError>,
    correlation: &CorrelationId,
) -> Result<T, SafeErrorDto> {
    parsed.map_err(|_| invalid(correlation, "desktop.entry_text_invalid"))
}

/// A text the person may leave empty; `None` then, never a blank string
/// the domain would refuse.
fn optional_work_text<T>(
    value: Option<String>,
    parse: impl FnOnce(String) -> Result<T, pmc_domain::DomainValueError>,
    correlation: &CorrelationId,
) -> Result<Option<T>, SafeErrorDto> {
    match value {
        Some(text) if !text.trim().is_empty() => work_text(parse(text), correlation).map(Some),
        _ => Ok(None),
    }
}

fn optional_instant(
    millis: Option<i64>,
    correlation: &CorrelationId,
) -> Result<Option<UtcTimestamp>, SafeErrorDto> {
    millis.map(|value| instant(value, correlation)).transpose()
}

fn optional_stakeholder(
    id: Option<String>,
    correlation: &CorrelationId,
) -> Result<Option<StakeholderId>, SafeErrorDto> {
    id.filter(|value| !value.is_empty())
        .map(|value| argument(StakeholderId::parse(value), correlation))
        .transpose()
}

/// A create must choose a classification (§3.3); the domain decides which
/// it accepts for this kind of record.
fn chosen_classification(
    value: Option<String>,
    correlation: &CorrelationId,
) -> Result<DataClassification, SafeErrorDto> {
    classification(value, true, correlation)?
        .ok_or_else(|| invalid(correlation, "desktop.entry_classification_required"))
}

fn action_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<ActionOperationContext, SafeErrorDto> {
    Ok(ActionOperationContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn decision_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<DecisionOperationContext, SafeErrorDto> {
    Ok(DecisionOperationContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn issue_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<IssueOperationContext, SafeErrorDto> {
    Ok(IssueOperationContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn risk_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<RiskOperationContext, SafeErrorDto> {
    Ok(RiskOperationContext {
        idempotency_id: argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn risk_error(error: RiskFlowError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        RiskFlowError::Domain(domain) => SafeErrorDto::from_domain(&domain),
        RiskFlowError::Ledger(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        RiskFlowError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_id_source_failed",
            correlation,
            false,
        ),
        _ => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "risk.infrastructure",
            correlation,
            false,
        ),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_action_request_draft_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    title: String,
    details: String,
    intended_owner_id: Option<String>,
    response_due_at_millis: Option<i64>,
    intended_action_due_at_millis: Option<i64>,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let fields = ActionRequestDraftFields {
        title: work_text(ActionTitle::parse(title), &correlation)?,
        details: work_text(ActionDetails::parse(details), &correlation)?,
        intended_owner: optional_stakeholder(intended_owner_id, &correlation)?,
        response_due_at: optional_instant(response_due_at_millis, &correlation)?,
        intended_action_due_at: optional_instant(intended_action_due_at_millis, &correlation)?,
        classification: chosen_classification(classification, &correlation)?,
    };
    let context = action_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_action_request_draft(&mut ledger, fields, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "action_request",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
pub fn submit_action_request_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(ActionRequestId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let context = action_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let submitted = submit_action_request(&mut ledger, id, expected, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = submitted.record;
    Ok(outcome(
        "action_request",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_decision_request_draft_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    subject: String,
    details: String,
    intended_owner_id: Option<String>,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let fields = DecisionRequestDraftFields {
        subject: work_text(DecisionSubject::parse(subject), &correlation)?,
        details: work_text(DecisionText::parse(details), &correlation)?,
        intended_owner: optional_stakeholder(intended_owner_id, &correlation)?,
        classification: chosen_classification(classification, &correlation)?,
    };
    let context = decision_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_decision_request_draft(&mut ledger, fields, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "decision_request",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
pub fn submit_decision_request_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let id = argument(DecisionRequestId::parse(id), &correlation)?;
    let expected = version(expected_version, &correlation)?;
    let context = decision_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let submitted = submit_decision_request(&mut ledger, id, expected, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = submitted.record;
    Ok(outcome(
        "decision_request",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_issue_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    title: String,
    details: String,
    classification: Option<String>,
    recurrence_of_id: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let fields = IssueFields {
        title: work_text(IssueTitle::parse(title), &correlation)?,
        details: work_text(IssueDetails::parse(details), &correlation)?,
        classification: chosen_classification(classification, &correlation)?,
        recurrence_of: recurrence_of_id
            .filter(|value| !value.is_empty())
            .map(|value| argument(IssueId::parse(value), &correlation))
            .transpose()?,
    };
    let context = issue_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_issue(&mut ledger, fields, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "issue",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

#[tauri::command]
pub fn create_risk_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    title: String,
    details: String,
    classification: Option<String>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let entry = RiskEntry {
        title: work_text(RiskTitle::parse(title), &correlation)?,
        details: work_text(RiskDetails::parse(details), &correlation)?,
        classification: chosen_classification(classification, &correlation)?,
    };
    let context = risk_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = enter_risk(&mut ledger, entry, context, &mut ids, now)
        .map_err(|error| entry_error(error, &correlation))?;
    let record = created.record;
    Ok(outcome(
        "risk",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}

/// H1: a Risk's response (mitigate, accept, transfer, avoid) with the
/// owner, rationale, residual exposure and next review the person entered;
/// accept and transfer need all four, which the domain enforces.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn update_risk_response_record(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    id: String,
    expected_version: u64,
    response: String,
    owner_id: Option<String>,
    rationale: Option<String>,
    residual_exposure: Option<String>,
    next_review_at_millis: Option<i64>,
    client_request_id: String,
) -> Result<EntryOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = UpdateRiskResponse {
        risk_id: argument(RiskId::parse(id), &correlation)?,
        expected_version: version(expected_version, &correlation)?,
        response: argument(RiskResponseType::from_persisted(&response), &correlation)?,
        owner: optional_stakeholder(owner_id, &correlation)?,
        rationale: optional_work_text(rationale, RiskRationale::parse, &correlation)?,
        residual_exposure: optional_work_text(
            residual_exposure,
            ResidualExposure::parse,
            &correlation,
        )?,
        next_review_at: optional_instant(next_review_at_millis, &correlation)?,
        context: risk_context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let updated = update_risk_response(&mut ledger, command, &mut ids, now)
        .map_err(|error| risk_error(error, &correlation))?;
    let record = updated.record;
    Ok(outcome(
        "risk",
        record.id().as_str(),
        record.classification(),
        record.version(),
        &correlation,
    ))
}
