//! Typed SQLite persistence seam for the Delivery family:
//! Initiative, Project, and Milestone. Every H1 command is ordinary
//! create/update: no Prepared Intent, no approval receipt. The persistence
//! contract mirrors the domain layer's own `InMemoryDeliveryService` checks
//! (existence, expected-version CAS, classification-not-lowered, Milestone's
//! classification-inherits-from-parent-Project rule) directly against
//! durable state, matching `portfolio_repository.rs`'s established
//! "no service reconstruction for a single-record H1 write" convention.
//!
//! Unlike Portfolio (five independent record types, five dedicated command
//! tables), Delivery's V1 schema already models all six H1 commands with one
//! shared, closed-CHECK-constrained `delivery_command_results` table --
//! this file's write methods honor that existing shape rather than adding a
//! parallel per-type table.
//!
//! It also carries H2a "Lower Data Classification" for all three record
//! types: a prepare step that persists the prepared intent and an
//! approve-and-execute step that consumes it.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditDisposition, AuditEffectCode, AuditEffectScope, AuditEvent,
        AuditEventCode, AuditEventIdSource, AuditExecutionOutcome, AuditModule, AuditPolicyOutcome,
        AuditTarget,
    },
    classification::DataClassification,
    delivery::{
        ApproveAndExecuteLowerInitiativeClassification,
        ApproveAndExecuteLowerMilestoneClassification, ApproveAndExecuteLowerProjectClassification,
        CommandIdentity, CreateInitiative, CreateMilestone, CreateProject, DefinedOutcome,
        DeliveryClassificationLoweringIdSource, DeliveryPersistenceResult,
        DeliveryPersistenceSnapshot, DeliveryRehydrationError, DeliveryReplayCapsule,
        DerivedMilestoneMutation, InMemoryDeliveryService, Initiative, InitiativePersistenceRecord,
        Milestone, MilestonePersistenceRecord, OperationContext,
        PrepareLowerInitiativeClassification, PrepareLowerMilestoneClassification,
        PrepareLowerProjectClassification, Project, ProjectPersistenceRecord, RecordName,
        UpdateInitiative, UpdateMilestone, UpdateProject, VerificationCriteria,
    },
    error::{DomainError, ErrorCode, MessageKey},
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId,
        InitiativeId, MilestoneId, PreparedIntentId, ProjectId,
    },
    provenance::{Provenance, ProvenanceReference},
    time::{Clock, UtcTimestamp},
    work_management::{
        ApprovalAuthorizationPort, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPayloadDigest, WorkManagementPreparedIntent, WorkManagementRationale,
    },
    DomainValueError,
};
use rusqlite::{OptionalExtension, Row, Transaction};

use super::{LedgerTransactionError, SqliteProductLedger};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryPersistenceLoadError {
    StorageUnavailable,
    InvalidDeliverySnapshot,
}

impl From<DeliveryRehydrationError> for DeliveryPersistenceLoadError {
    fn from(_: DeliveryRehydrationError) -> Self {
        Self::InvalidDeliverySnapshot
    }
}

/// The single constant effect code every Delivery mutation uses -- matches
/// `validate_delivery_audit`'s own hardcoded expectation in
/// `pmc-domain/src/delivery.rs`.
const DELIVERY_EFFECT_CODE: &str = "delivery.authoritative-record-changed";

/// One committed Delivery H1 mutation: the resulting record, the audit
/// event(s) it produced (always exactly one except `update_project`, which
/// additionally emits one `milestone.classification.inherited` audit per
/// cascaded Milestone), and any Milestones whose classification/version the
/// same commit cascaded.
#[derive(Debug)]
pub struct DeliveryMutationOutcome<T> {
    pub record: T,
    pub audit_events: Vec<AuditEvent>,
    pub cascaded_milestones: Vec<Milestone>,
}

/// A clock pinned to the exact stored prepare-time timestamp until
/// `enter_execute_phase` is called, after which it returns the real
/// execute-time timestamp. See `portfolio_repository.rs::
/// PortfolioReplayThenNowClock` for the full rationale -- same technique,
/// duplicated per module.
#[derive(Clone)]
struct DeliveryReplayThenNowClock {
    prepare_at: UtcTimestamp,
    execute_at: UtcTimestamp,
    executing: std::rc::Rc<std::cell::Cell<bool>>,
}

impl DeliveryReplayThenNowClock {
    fn new(prepare_at: UtcTimestamp, execute_at: UtcTimestamp) -> Self {
        Self {
            prepare_at,
            execute_at,
            executing: std::rc::Rc::new(std::cell::Cell::new(false)),
        }
    }

    fn enter_execute_phase(&self) {
        self.executing.set(true);
    }
}

impl Clock for DeliveryReplayThenNowClock {
    fn now(&self) -> UtcTimestamp {
        if self.executing.get() {
            self.execute_at
        } else {
            self.prepare_at
        }
    }
}

/// Supplies the exact stored prepared-intent id to the deterministic
/// re-prepare call, then the caller-supplied approval receipt id to the
/// execute call that immediately follows on the same service instance.
struct DeliveryReplayIds {
    prepared_intent_id: Option<PreparedIntentId>,
    approval_receipt_id: Option<ApprovalReceiptId>,
}

impl DeliveryReplayIds {
    fn new(prepared_intent_id: PreparedIntentId, approval_receipt_id: ApprovalReceiptId) -> Self {
        Self {
            prepared_intent_id: Some(prepared_intent_id),
            approval_receipt_id: Some(approval_receipt_id),
        }
    }
}

impl DeliveryClassificationLoweringIdSource for DeliveryReplayIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.prepared_intent_id
            .take()
            .ok_or_else(missing_delivery_h2a_id)
    }

    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.approval_receipt_id
            .take()
            .ok_or_else(missing_delivery_h2a_id)
    }
}

fn missing_delivery_h2a_id() -> DomainValueError {
    match InitiativeId::parse("") {
        Err(error) => error,
        Ok(_) => unreachable!("empty opaque identifiers must be rejected"),
    }
}

/// Hands out a pre-built queue of audit ids in order: one synthetic id per
/// seeding call in `seed_*_service_for_lowering`, then finally the one real,
/// caller-supplied audit id for the actual execute commit.
struct QueuedDeliveryAuditIds(std::collections::VecDeque<AuditEventId>);

impl AuditEventIdSource for QueuedDeliveryAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0.pop_front().ok_or_else(missing_delivery_h2a_id)
    }
}

/// The only actor ever allowed to approve a persisted Delivery H2a operation.
#[derive(Clone, Copy)]
struct AllowPersistedDeliveryApproval;

impl ApprovalAuthorizationPort for AllowPersistedDeliveryApproval {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

impl SqliteProductLedger {
    pub fn create_initiative(
        &mut self,
        command: CreateInitiative,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Initiative>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_name,command_defined_outcome,command_classification,command_provenance_kind,command_provenance_reference FROM delivery_command_results WHERE namespace='delivery' AND operation='create_initiative' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == command.name.as_str()
                    && existing.2 == command.defined_outcome.as_str()
                    && existing.3.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.4 == command.provenance.kind_persisted()
                    && existing.5.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_delivery_initiative_outcome(tx, "create_initiative", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let exists: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
                    [command.id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if exists != 0 {
                return Err(domain_conflict(&context, "delivery.already_exists"));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'initiative',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO initiatives(id,name,defined_outcome,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.defined_outcome.as_str(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_delivery_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Initiative(command.id.clone()),
                "initiative.created",
                &context,
            )?;
            persist_delivery_audit(tx, &audit, "initiative", command.id.as_str(), &context)?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            persist_delivery_idempotency_claim(tx, "create_initiative", &context, ordinal, occurred_millis)?;
            tx.execute(
                "INSERT INTO delivery_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_defined_outcome,command_project_id,command_verification_criteria,command_start_at,command_end_at,command_due_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_project_id,result_name,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('delivery','create_initiative',?1,'create_initiative',?2,NULL,?3,?4,NULL,NULL,NULL,NULL,NULL,?5,?6,?7,'initiative',?2,NULL,?3,?4,NULL,NULL,NULL,NULL,?8,?6,?7,1,?9,?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.defined_outcome.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_delivery_idempotency_outcome_audit(tx, "create_initiative", &context, 0, audit.id().as_str())?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(DeliveryMutationOutcome {
                record: Initiative::rehydrate(pmc_domain::delivery::InitiativePersistenceRecord {
                    id: command.id,
                    name: command.name,
                    defined_outcome: command.defined_outcome,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                    created_at: occurred_at,
                    updated_at: occurred_at,
                })
                .map_err(|_| storage_error(&context))?,
                audit_events: vec![audit],
                cascaded_milestones: Vec::new(),
            })
        })
    }

    pub fn update_initiative(
        &mut self,
        command: UpdateInitiative,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Initiative>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_defined_outcome,command_classification FROM delivery_command_results WHERE namespace='delivery' AND operation='update_initiative' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.name.as_str()
                    && existing.3 == command.defined_outcome.as_str()
                    && existing.4.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_delivery_initiative_outcome(tx, "update_initiative", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM initiatives JOIN aggregate_registry registry ON registry.id=initiatives.id AND registry.aggregate_type='initiative' WHERE initiatives.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "delivery.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = monotonic_classification(current_classification, command.classification, &context)?;
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "delivery.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE initiatives SET name=?1,defined_outcome=?2 WHERE id=?3",
                rusqlite::params![command.name.as_str(), command.defined_outcome.as_str(), command.id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='initiative'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_delivery_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Initiative(command.id.clone()),
                "initiative.updated",
                &context,
            )?;
            persist_delivery_audit(tx, &audit, "initiative", command.id.as_str(), &context)?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            persist_delivery_idempotency_claim(tx, "update_initiative", &context, ordinal, occurred_millis)?;
            tx.execute(
                "INSERT INTO delivery_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_defined_outcome,command_project_id,command_verification_criteria,command_start_at,command_end_at,command_due_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_project_id,result_name,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('delivery','update_initiative',?1,'update_initiative',?2,?3,?4,?5,NULL,NULL,NULL,NULL,NULL,?6,?7,?8,'initiative',?2,NULL,?4,?5,NULL,NULL,NULL,NULL,?9,?7,?8,?10,?11,?12)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.defined_outcome.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    tx.query_row("SELECT created_at FROM aggregate_registry WHERE id=?1", [command.id.as_str()], |r| r.get::<_, i64>(0)).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_delivery_idempotency_outcome_audit(tx, "update_initiative", &context, 0, audit.id().as_str())?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            decode_delivery_initiative_outcome(tx, "update_initiative", &context)
        })
    }

    pub fn create_project(
        &mut self,
        command: CreateProject,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Project>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_name,command_start_at,command_end_at,command_classification,command_provenance_kind,command_provenance_reference FROM delivery_command_results WHERE namespace='delivery' AND operation='create_project' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == command.name.as_str()
                    && existing.2 == command.start_at.unix_millis()
                    && existing.3 == command.end_at.unix_millis()
                    && existing.4.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.5 == command.provenance.kind_persisted()
                    && existing.6.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_delivery_project_outcome(tx, "create_project", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            if command.end_at.unix_millis() < command.start_at.unix_millis() {
                return Err(domain_conflict(&context, "delivery.invalid_period"));
            }
            let exists: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
                    [command.id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if exists != 0 {
                return Err(domain_conflict(&context, "delivery.already_exists"));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'project',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO projects(id,name,start_at,end_at,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.start_at.unix_millis(),
                    command.end_at.unix_millis(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_delivery_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Project(command.id.clone()),
                "project.created",
                &context,
            )?;
            persist_delivery_audit(tx, &audit, "project", command.id.as_str(), &context)?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            persist_delivery_idempotency_claim(tx, "create_project", &context, ordinal, occurred_millis)?;
            tx.execute(
                "INSERT INTO delivery_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_defined_outcome,command_project_id,command_verification_criteria,command_start_at,command_end_at,command_due_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_project_id,result_name,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('delivery','create_project',?1,'create_project',?2,NULL,?3,NULL,NULL,NULL,?4,?5,NULL,?6,?7,?8,'project',?2,NULL,?3,NULL,NULL,?4,?5,NULL,?9,?7,?8,1,?10,?10)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.start_at.unix_millis(),
                    command.end_at.unix_millis(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_delivery_idempotency_outcome_audit(tx, "create_project", &context, 0, audit.id().as_str())?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(DeliveryMutationOutcome {
                record: Project::rehydrate(pmc_domain::delivery::ProjectPersistenceRecord {
                    id: command.id,
                    name: command.name,
                    start_at: command.start_at,
                    end_at: command.end_at,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                    created_at: occurred_at,
                    updated_at: occurred_at,
                })
                .map_err(|_| storage_error(&context))?,
                audit_events: vec![audit],
                cascaded_milestones: Vec::new(),
            })
        })
    }

    /// Unlike every other Delivery H1 write, an update to a Project's
    /// classification can cascade: every child Milestone whose current
    /// classification does not already dominate the Project's new
    /// classification gets raised (never lowered) to
    /// `project.classification.combine(milestone.classification)`, each
    /// producing its own `milestone.classification.inherited` audit and
    /// `delivery_derived_milestone_mutations` row -- mirrors
    /// `update_project`'s exact cascade logic in `pmc-domain/src/delivery.rs`.
    pub fn update_project(
        &mut self,
        command: UpdateProject,
        audit_event_id: AuditEventId,
        cascade_audit_event_ids: impl FnMut() -> Result<AuditEventId, DomainError>,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Project>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        let mut cascade_audit_event_ids = cascade_audit_event_ids;
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_start_at,command_end_at,command_classification FROM delivery_command_results WHERE namespace='delivery' AND operation='update_project' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.name.as_str()
                    && existing.3 == command.start_at.unix_millis()
                    && existing.4 == command.end_at.unix_millis()
                    && existing.5.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_delivery_project_outcome(tx, "update_project", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            if command.end_at.unix_millis() < command.start_at.unix_millis() {
                return Err(domain_conflict(&context, "delivery.invalid_period"));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM projects JOIN aggregate_registry registry ON registry.id=projects.id AND registry.aggregate_type='project' WHERE projects.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "delivery.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = monotonic_classification(current_classification, command.classification, &context)?;
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "delivery.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE projects SET name=?1,start_at=?2,end_at=?3 WHERE id=?4",
                rusqlite::params![
                    command.name.as_str(),
                    command.start_at.unix_millis(),
                    command.end_at.unix_millis(),
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='project'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_delivery_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Project(command.id.clone()),
                "project.updated",
                &context,
            )?;
            persist_delivery_audit(tx, &audit, "project", command.id.as_str(), &context)?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            persist_delivery_idempotency_claim(tx, "update_project", &context, ordinal, occurred_millis)?;
            persist_delivery_idempotency_outcome_audit(tx, "update_project", &context, 0, audit.id().as_str())?;
            let mut affected: Vec<(MilestoneId, DataClassification, AggregateVersion, i64)> = tx
                .prepare("SELECT milestones.id,registry.classification,registry.version,registry.updated_at FROM milestones JOIN aggregate_registry registry ON registry.id=milestones.id AND registry.aggregate_type='milestone' WHERE milestones.project_id=?1 ORDER BY milestones.id")
                .map_err(|_| storage_error(&context))?
                .query_map([command.id.as_str()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|_| storage_error(&context))?
                .map(|row| {
                    let row = row.map_err(|_| storage_error(&context))?;
                    Ok((
                        MilestoneId::parse(row.0).map_err(|_| storage_error(&context))?,
                        DataClassification::from_persisted(&row.1).map_err(|_| storage_error(&context))?,
                        AggregateVersion::new(u64::try_from(row.2).map_err(|_| storage_error(&context))?)
                            .map_err(|_| storage_error(&context))?,
                        row.3,
                    ))
                })
                .collect::<Result<Vec<_>, DomainError>>()?;
            affected.retain(|(_, milestone_classification, _, _)| {
                classification.combine(*milestone_classification) != *milestone_classification
            });
            let mut cascaded_milestones = Vec::new();
            for (index, (milestone_id, previous_classification, previous_version, previous_updated_at)) in affected.iter().enumerate() {
                let resulting_classification = classification.combine(*previous_classification);
                let resulting_version = previous_version
                    .next()
                    .ok_or_else(|| domain_conflict(&context, "delivery.version_exhausted"))?;
                tx.execute(
                    "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='milestone'",
                    rusqlite::params![
                        i64::try_from(resulting_version.get()).map_err(|_| storage_error(&context))?,
                        resulting_classification.as_persisted(),
                        occurred_millis,
                        milestone_id.as_str(),
                    ],
                )
                .map_err(|_| storage_error(&context))?;
                let cascade_audit_event_id = cascade_audit_event_ids().map_err(|_| storage_error(&context))?;
                let cascade_audit = build_delivery_audit(
                    cascade_audit_event_id,
                    occurred_at,
                    AuditTarget::Milestone(milestone_id.clone()),
                    "milestone.classification.inherited",
                    &context,
                )?;
                persist_delivery_audit(tx, &cascade_audit, "milestone", milestone_id.as_str(), &context)?;
                persist_delivery_idempotency_outcome_audit(
                    tx,
                    "update_project",
                    &context,
                    i64::try_from(index + 1).map_err(|_| storage_error(&context))?,
                    cascade_audit.id().as_str(),
                )?;
                tx.execute(
                    "INSERT INTO delivery_derived_milestone_mutations VALUES('delivery','update_project',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    rusqlite::params![
                        context.idempotency_id.as_str(),
                        i64::try_from(index).map_err(|_| storage_error(&context))?,
                        milestone_id.as_str(),
                        i64::try_from(previous_version.get()).map_err(|_| storage_error(&context))?,
                        i64::try_from(resulting_version.get()).map_err(|_| storage_error(&context))?,
                        previous_classification.as_persisted(),
                        resulting_classification.as_persisted(),
                        previous_updated_at,
                        occurred_millis,
                        cascade_audit.id().as_str(),
                    ],
                )
                .map_err(|_| storage_error(&context))?;
                let row = tx
                    .query_row(
                        "SELECT project_id,name,verification_criteria,due_at,provenance_kind,provenance_reference,created_at FROM milestones JOIN aggregate_registry registry ON registry.id=milestones.id AND registry.aggregate_type='milestone' WHERE milestones.id=?1",
                        [milestone_id.as_str()],
                        |r| Ok((
                            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
                            r.get::<_, i64>(3)?, r.get::<_, String>(4)?, r.get::<_, Option<String>>(5)?,
                            r.get::<_, i64>(6)?,
                        )),
                    )
                    .map_err(|_| storage_error(&context))?;
                cascaded_milestones.push(
                    Milestone::rehydrate(pmc_domain::delivery::MilestonePersistenceRecord {
                        id: milestone_id.clone(),
                        project_id: ProjectId::parse(row.0).map_err(|_| storage_error(&context))?,
                        name: RecordName::parse(row.1, &context.correlation_id).map_err(|_| storage_error(&context))?,
                        verification_criteria: VerificationCriteria::parse(row.2, &context.correlation_id).map_err(|_| storage_error(&context))?,
                        due_at: UtcTimestamp::from_unix_millis(row.3),
                        classification: resulting_classification,
                        provenance: decode_delivery_provenance(&row.4, row.5, &context)?,
                        version: resulting_version,
                        created_at: UtcTimestamp::from_unix_millis(row.6),
                        updated_at: occurred_at,
                    })
                    .map_err(|_| storage_error(&context))?,
                );
            }
            tx.execute(
                "INSERT INTO delivery_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_defined_outcome,command_project_id,command_verification_criteria,command_start_at,command_end_at,command_due_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_project_id,result_name,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('delivery','update_project',?1,'update_project',?2,?3,?4,NULL,NULL,NULL,?5,?6,NULL,?7,?8,?9,'project',?2,NULL,?4,NULL,NULL,?5,?6,NULL,?10,?8,?9,?11,?12,?13)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.start_at.unix_millis(),
                    command.end_at.unix_millis(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    tx.query_row("SELECT created_at FROM aggregate_registry WHERE id=?1", [command.id.as_str()], |r| r.get::<_, i64>(0)).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let mut outcome = decode_delivery_project_outcome(tx, "update_project", &context)?;
            outcome.cascaded_milestones = cascaded_milestones;
            Ok(outcome)
        })
    }

    pub fn create_milestone(
        &mut self,
        command: CreateMilestone,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Milestone>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_project_id,command_name,command_verification_criteria,command_due_at,command_classification,command_provenance_kind,command_provenance_reference FROM delivery_command_results WHERE namespace='delivery' AND operation='create_milestone' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, Option<String>>(7)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1.as_deref() == Some(command.project_id.as_str())
                    && existing.2 == command.name.as_str()
                    && existing.3 == command.verification_criteria.as_str()
                    && existing.4 == command.due_at.unix_millis()
                    && existing.5.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.6 == command.provenance.kind_persisted()
                    && existing.7.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_delivery_milestone_outcome(tx, "create_milestone", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let exists: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
                    [command.id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if exists != 0 {
                return Err(domain_conflict(&context, "delivery.already_exists"));
            }
            let parent_classification: String = tx
                .query_row(
                    "SELECT registry.classification FROM projects JOIN aggregate_registry registry ON registry.id=projects.id AND registry.aggregate_type='project' WHERE projects.id=?1",
                    [command.project_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            let parent_classification = DataClassification::from_persisted(&parent_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = command
                .classification
                .map_or(parent_classification, |value| parent_classification.combine(value));
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'milestone',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO milestones(id,project_id,name,verification_criteria,due_at,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    command.id.as_str(),
                    command.project_id.as_str(),
                    command.name.as_str(),
                    command.verification_criteria.as_str(),
                    command.due_at.unix_millis(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_delivery_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Milestone(command.id.clone()),
                "milestone.created",
                &context,
            )?;
            persist_delivery_audit(tx, &audit, "milestone", command.id.as_str(), &context)?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            persist_delivery_idempotency_claim(tx, "create_milestone", &context, ordinal, occurred_millis)?;
            tx.execute(
                "INSERT INTO delivery_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_defined_outcome,command_project_id,command_verification_criteria,command_start_at,command_end_at,command_due_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_project_id,result_name,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('delivery','create_milestone',?1,'create_milestone',?2,NULL,?3,NULL,?4,?5,NULL,NULL,?6,?7,?8,?9,'milestone',?2,?4,?3,NULL,?5,NULL,NULL,?6,?10,?8,?9,1,?11,?11)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.project_id.as_str(),
                    command.verification_criteria.as_str(),
                    command.due_at.unix_millis(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_delivery_idempotency_outcome_audit(tx, "create_milestone", &context, 0, audit.id().as_str())?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(DeliveryMutationOutcome {
                record: Milestone::rehydrate(pmc_domain::delivery::MilestonePersistenceRecord {
                    id: command.id,
                    project_id: command.project_id,
                    name: command.name,
                    verification_criteria: command.verification_criteria,
                    due_at: command.due_at,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                    created_at: occurred_at,
                    updated_at: occurred_at,
                })
                .map_err(|_| storage_error(&context))?,
                audit_events: vec![audit],
                cascaded_milestones: Vec::new(),
            })
        })
    }

    pub fn update_milestone(
        &mut self,
        command: UpdateMilestone,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Milestone>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_verification_criteria,command_due_at,command_classification FROM delivery_command_results WHERE namespace='delivery' AND operation='update_milestone' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.name.as_str()
                    && existing.3 == command.verification_criteria.as_str()
                    && existing.4 == command.due_at.unix_millis()
                    && existing.5.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_delivery_milestone_outcome(tx, "update_milestone", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version, project_id): (String, i64, String) = tx
                .query_row(
                    "SELECT registry.classification,registry.version,milestones.project_id FROM milestones JOIN aggregate_registry registry ON registry.id=milestones.id AND registry.aggregate_type='milestone' WHERE milestones.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "delivery.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let requested_classification = monotonic_classification(current_classification, command.classification, &context)?;
            let parent_classification: String = tx
                .query_row(
                    "SELECT registry.classification FROM projects JOIN aggregate_registry registry ON registry.id=projects.id AND registry.aggregate_type='project' WHERE projects.id=?1",
                    [project_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            let parent_classification = DataClassification::from_persisted(&parent_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = parent_classification.combine(requested_classification);
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "delivery.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE milestones SET name=?1,verification_criteria=?2,due_at=?3 WHERE id=?4",
                rusqlite::params![
                    command.name.as_str(),
                    command.verification_criteria.as_str(),
                    command.due_at.unix_millis(),
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='milestone'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_delivery_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Milestone(command.id.clone()),
                "milestone.updated",
                &context,
            )?;
            persist_delivery_audit(tx, &audit, "milestone", command.id.as_str(), &context)?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            persist_delivery_idempotency_claim(tx, "update_milestone", &context, ordinal, occurred_millis)?;
            tx.execute(
                "INSERT INTO delivery_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_defined_outcome,command_project_id,command_verification_criteria,command_start_at,command_end_at,command_due_at,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_project_id,result_name,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('delivery','update_milestone',?1,'update_milestone',?2,?3,?4,NULL,NULL,?5,NULL,NULL,?6,?7,?8,?9,'milestone',?2,?10,?4,NULL,?5,NULL,NULL,?6,?11,?8,?9,?12,?13,?14)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.verification_criteria.as_str(),
                    command.due_at.unix_millis(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    project_id.as_str(),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    tx.query_row("SELECT created_at FROM aggregate_registry WHERE id=?1", [command.id.as_str()], |r| r.get::<_, i64>(0)).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_delivery_idempotency_outcome_audit(tx, "update_milestone", &context, 0, audit.id().as_str())?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            decode_delivery_milestone_outcome(tx, "update_milestone", &context)
        })
    }

    /// H2a step 1: persist one already-canonical Initiative
    /// classification-lowering preview. See `portfolio_repository.rs::
    /// prepare_lower_portfolio_classification` for the shared rationale --
    /// this adapter never reconstructs a domain service for prepare; it
    /// re-checks the caller-supplied `WorkManagementPreparedIntent` against
    /// durable state directly, matching Delivery's own H1 convention.
    pub fn prepare_lower_initiative_classification(
        &mut self,
        command: PrepareLowerInitiativeClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if delivery_idempotency_claimed_by_other_operation(
                tx,
                &context,
                "prepare_lower_classification",
            )? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT target_id,expected_version,proposed_classification,rationale,result_reference FROM delivery_h2a_prepare_replay_operations replay JOIN delivery_h2a_command_prepares command USING(idempotency_id) WHERE replay.idempotency_id=?1 AND command.target_kind='initiative'",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && delivery_prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM initiatives JOIN aggregate_registry registry ON registry.id=initiatives.id AND registry.aggregate_type='initiative' WHERE initiatives.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "delivery.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command.proposed_classification.combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "delivery.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerInitiativeClassification {
                initiative_id,
                initiative_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(&context, "delivery.classification_lowering_invalid"));
            };
            if initiative_id != &command.id
                || initiative_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(&context, "delivery.classification_lowering_invalid"));
            }
            let ordinal = next_delivery_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_delivery_prepared_intent(
                tx,
                &prepared,
                "lower_initiative_classification",
                &context,
            )?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'initiative',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO delivery_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO delivery_h2a_command_prepares (idempotency_id,target_kind,target_id,expected_version,proposed_classification,rationale) VALUES (?1,'initiative',?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// H2a step 2 for Initiative. See `portfolio_repository.rs
    /// ::approve_and_execute_lower_portfolio_classification` for why this
    /// rehydrates a throwaway `InMemoryDeliveryService` and re-runs prepare
    /// once (deterministically, from stored fields) before calling execute
    /// on that same instance.
    pub fn approve_and_execute_lower_initiative_classification(
        &mut self,
        command: ApproveAndExecuteLowerInitiativeClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Initiative>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if delivery_idempotency_claimed_by_other_operation(
                tx,
                &context,
                "execute_lower_classification",
            )? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM delivery_h2a_command_executes WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 != command.approval.prepared_id().as_str()
                    || existing.1 != command.approval.actor().as_persisted()
                    || existing.2 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                let (result, audit_event) = replay_lowered_delivery_outcome(tx, &context)?;
                return match result {
                    DeliveryPersistenceResult::Initiative(record) => Ok(DeliveryMutationOutcome {
                        record,
                        audit_events: vec![audit_event],
                        cascaded_milestones: Vec::new(),
                    }),
                    _ => Err(storage_error(&context)),
                };
            }
            let (
                stored_target_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.target_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM delivery_h2a_prepare_replay_operations replay JOIN delivery_h2a_command_prepares command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1 AND command.target_kind='initiative'",
                    [command.approval.prepared_id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| {
                    domain_conflict(&context, "delivery.classification_lowering_preview_changed")
                })?;
            let initiative_id =
                InitiativeId::parse(stored_target_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = WorkManagementRationale::parse(stored_rationale)
                .map_err(|_| storage_error(&context))?;
            let clock =
                DeliveryReplayThenNowClock::new(UtcTimestamp::from_unix_millis(stored_created_at), occurred_at);
            let mut ids =
                DeliveryReplayIds::new(command.approval.prepared_id().clone(), approval_receipt_id.clone());
            let mut service = seed_initiative_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &initiative_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_initiative_classification(
                    PrepareLowerInitiativeClassification {
                        id: initiative_id.clone(),
                        expected_version,
                        proposed_classification,
                        rationale,
                        context: OperationContext {
                            idempotency_id: replay_idempotency_id,
                            correlation_id: context.correlation_id.clone(),
                        },
                    },
                    &mut ids,
                )
                .map_err(|_| {
                    domain_conflict(&context, "delivery.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let record = service
                .approve_and_execute_lower_initiative_classification(
                    ApproveAndExecuteLowerInitiativeClassification {
                        approval: command.approval.clone(),
                        context: context.clone(),
                    },
                    &mut ids,
                    &AllowPersistedDeliveryApproval,
                )
                ?;
            let audit_event = service
                .audit_events()
                .iter()
                .find(|event| event.id() == &audit_event_id)
                .cloned()
                .ok_or_else(|| storage_error(&context))?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            let result = DeliveryPersistenceResult::Initiative(record.clone());
            persist_lowered_delivery_bundle(
                tx,
                &result,
                &audit_event,
                &command.approval,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(DeliveryMutationOutcome {
                record,
                audit_events: vec![audit_event],
                cascaded_milestones: Vec::new(),
            })
        })
    }

    /// H2a step 1: persist one already-canonical Project
    /// classification-lowering preview. See
    /// `prepare_lower_initiative_classification` -- identical shape.
    pub fn prepare_lower_project_classification(
        &mut self,
        command: PrepareLowerProjectClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if delivery_idempotency_claimed_by_other_operation(
                tx,
                &context,
                "prepare_lower_classification",
            )? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT target_id,expected_version,proposed_classification,rationale,result_reference FROM delivery_h2a_prepare_replay_operations replay JOIN delivery_h2a_command_prepares command USING(idempotency_id) WHERE replay.idempotency_id=?1 AND command.target_kind='project'",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && delivery_prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM projects JOIN aggregate_registry registry ON registry.id=projects.id AND registry.aggregate_type='project' WHERE projects.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "delivery.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command.proposed_classification.combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "delivery.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerProjectClassification {
                project_id,
                project_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(&context, "delivery.classification_lowering_invalid"));
            };
            if project_id != &command.id
                || project_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(&context, "delivery.classification_lowering_invalid"));
            }
            let ordinal = next_delivery_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_delivery_prepared_intent(tx, &prepared, "lower_project_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'project',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO delivery_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO delivery_h2a_command_prepares (idempotency_id,target_kind,target_id,expected_version,proposed_classification,rationale) VALUES (?1,'project',?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// H2a step 2 for Project. See
    /// `approve_and_execute_lower_initiative_classification` -- identical
    /// shape.
    pub fn approve_and_execute_lower_project_classification(
        &mut self,
        command: ApproveAndExecuteLowerProjectClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Project>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if delivery_idempotency_claimed_by_other_operation(
                tx,
                &context,
                "execute_lower_classification",
            )? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM delivery_h2a_command_executes WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 != command.approval.prepared_id().as_str()
                    || existing.1 != command.approval.actor().as_persisted()
                    || existing.2 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                let (result, audit_event) = replay_lowered_delivery_outcome(tx, &context)?;
                return match result {
                    DeliveryPersistenceResult::Project(record) => Ok(DeliveryMutationOutcome {
                        record,
                        audit_events: vec![audit_event],
                        cascaded_milestones: Vec::new(),
                    }),
                    _ => Err(storage_error(&context)),
                };
            }
            let (
                stored_target_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.target_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM delivery_h2a_prepare_replay_operations replay JOIN delivery_h2a_command_prepares command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1 AND command.target_kind='project'",
                    [command.approval.prepared_id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| {
                    domain_conflict(&context, "delivery.classification_lowering_preview_changed")
                })?;
            let project_id = ProjectId::parse(stored_target_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = WorkManagementRationale::parse(stored_rationale)
                .map_err(|_| storage_error(&context))?;
            let clock =
                DeliveryReplayThenNowClock::new(UtcTimestamp::from_unix_millis(stored_created_at), occurred_at);
            let mut ids =
                DeliveryReplayIds::new(command.approval.prepared_id().clone(), approval_receipt_id.clone());
            let mut service = seed_project_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &project_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_project_classification(
                    PrepareLowerProjectClassification {
                        id: project_id.clone(),
                        expected_version,
                        proposed_classification,
                        rationale,
                        context: OperationContext {
                            idempotency_id: replay_idempotency_id,
                            correlation_id: context.correlation_id.clone(),
                        },
                    },
                    &mut ids,
                )
                .map_err(|_| {
                    domain_conflict(&context, "delivery.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let record = service
                .approve_and_execute_lower_project_classification(
                    ApproveAndExecuteLowerProjectClassification {
                        approval: command.approval.clone(),
                        context: context.clone(),
                    },
                    &mut ids,
                    &AllowPersistedDeliveryApproval,
                )
                ?;
            let audit_event = service
                .audit_events()
                .iter()
                .find(|event| event.id() == &audit_event_id)
                .cloned()
                .ok_or_else(|| storage_error(&context))?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            let result = DeliveryPersistenceResult::Project(record.clone());
            persist_lowered_delivery_bundle(
                tx,
                &result,
                &audit_event,
                &command.approval,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(DeliveryMutationOutcome {
                record,
                audit_events: vec![audit_event],
                cascaded_milestones: Vec::new(),
            })
        })
    }

    /// H2a step 1: persist one already-canonical Milestone
    /// classification-lowering preview. See
    /// `prepare_lower_initiative_classification` -- identical shape.
    pub fn prepare_lower_milestone_classification(
        &mut self,
        command: PrepareLowerMilestoneClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if delivery_idempotency_claimed_by_other_operation(
                tx,
                &context,
                "prepare_lower_classification",
            )? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT target_id,expected_version,proposed_classification,rationale,result_reference FROM delivery_h2a_prepare_replay_operations replay JOIN delivery_h2a_command_prepares command USING(idempotency_id) WHERE replay.idempotency_id=?1 AND command.target_kind='milestone'",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && delivery_prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM milestones JOIN aggregate_registry registry ON registry.id=milestones.id AND registry.aggregate_type='milestone' WHERE milestones.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "delivery.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "delivery.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command.proposed_classification.combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "delivery.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerMilestoneClassification {
                milestone_id,
                milestone_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(&context, "delivery.classification_lowering_invalid"));
            };
            if milestone_id != &command.id
                || milestone_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(&context, "delivery.classification_lowering_invalid"));
            }
            let ordinal = next_delivery_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_delivery_prepared_intent(tx, &prepared, "lower_milestone_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'milestone',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO delivery_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO delivery_h2a_command_prepares (idempotency_id,target_kind,target_id,expected_version,proposed_classification,rationale) VALUES (?1,'milestone',?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// H2a step 2 for Milestone. See
    /// `approve_and_execute_lower_initiative_classification` -- identical
    /// shape, except its seed function additionally reconstructs a
    /// synthetic parent Project (see `seed_milestone_service_for_lowering`).
    pub fn approve_and_execute_lower_milestone_classification(
        &mut self,
        command: ApproveAndExecuteLowerMilestoneClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<DeliveryMutationOutcome<Milestone>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if delivery_idempotency_claimed_by_other_operation(
                tx,
                &context,
                "execute_lower_classification",
            )? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM delivery_h2a_command_executes WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 != command.approval.prepared_id().as_str()
                    || existing.1 != command.approval.actor().as_persisted()
                    || existing.2 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                let (result, audit_event) = replay_lowered_delivery_outcome(tx, &context)?;
                return match result {
                    DeliveryPersistenceResult::Milestone(record) => Ok(DeliveryMutationOutcome {
                        record,
                        audit_events: vec![audit_event],
                        cascaded_milestones: Vec::new(),
                    }),
                    _ => Err(storage_error(&context)),
                };
            }
            let (
                stored_target_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.target_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM delivery_h2a_prepare_replay_operations replay JOIN delivery_h2a_command_prepares command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1 AND command.target_kind='milestone'",
                    [command.approval.prepared_id().as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| {
                    domain_conflict(&context, "delivery.classification_lowering_preview_changed")
                })?;
            let milestone_id =
                MilestoneId::parse(stored_target_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = WorkManagementRationale::parse(stored_rationale)
                .map_err(|_| storage_error(&context))?;
            let clock =
                DeliveryReplayThenNowClock::new(UtcTimestamp::from_unix_millis(stored_created_at), occurred_at);
            let mut ids =
                DeliveryReplayIds::new(command.approval.prepared_id().clone(), approval_receipt_id.clone());
            let mut service = seed_milestone_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &milestone_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_milestone_classification(
                    PrepareLowerMilestoneClassification {
                        id: milestone_id.clone(),
                        expected_version,
                        proposed_classification,
                        rationale,
                        context: OperationContext {
                            idempotency_id: replay_idempotency_id,
                            correlation_id: context.correlation_id.clone(),
                        },
                    },
                    &mut ids,
                )
                .map_err(|_| {
                    domain_conflict(&context, "delivery.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let record = service
                .approve_and_execute_lower_milestone_classification(
                    ApproveAndExecuteLowerMilestoneClassification {
                        approval: command.approval.clone(),
                        context: context.clone(),
                    },
                    &mut ids,
                    &AllowPersistedDeliveryApproval,
                )
                ?;
            let audit_event = service
                .audit_events()
                .iter()
                .find(|event| event.id() == &audit_event_id)
                .cloned()
                .ok_or_else(|| storage_error(&context))?;
            let ordinal = next_delivery_operation_ordinal(tx, &context)?;
            let result = DeliveryPersistenceResult::Milestone(record.clone());
            persist_lowered_delivery_bundle(
                tx,
                &result,
                &audit_event,
                &command.approval,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(DeliveryMutationOutcome {
                record,
                audit_events: vec![audit_event],
                cascaded_milestones: Vec::new(),
            })
        })
    }

    /// Load the complete durable Delivery namespace through the typed,
    /// validated domain seam -- reconstructs every Initiative/Project/
    /// Milestone plus the full closed-set replay history (H1 create/update
    /// and H2a lowering execute; H2a lowering *prepare* is deliberately
    /// excluded per DG0 6.7 -- an unapproved preview must not survive a
    /// restart) and hands it to `DeliveryPersistenceSnapshot::validate`,
    /// which itself requires the reconstructed `replay` to be
    /// gapless-ordinal from zero -- see
    /// `pmc-domain/src/delivery.rs::validate_operation_timeline`.
    pub fn load_delivery_persistence_snapshot(
        &self,
    ) -> Result<DeliveryPersistenceSnapshot, DeliveryPersistenceLoadError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| DeliveryPersistenceLoadError::StorageUnavailable)?;
        let snapshot = decode_delivery_namespace(&transaction)?;
        transaction
            .commit()
            .map_err(|_| DeliveryPersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }
}

fn storage_error(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("ledger.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}

fn idempotency_conflict(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("delivery.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn domain_conflict(context: &OperationContext, key: &str) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn domain_not_found(context: &OperationContext, key: &str) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn stale_version(context: &OperationContext, key: &str, current_version: i64) -> DomainError {
    let mut error = DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    );
    if let Ok(raw) = u64::try_from(current_version) {
        if let Ok(version) = AggregateVersion::new(raw) {
            error = error.with_extension(pmc_domain::error::SafeErrorExtension::CurrentVersion(
                version,
            ));
        }
    }
    error
}

fn policy_denied(context: &OperationContext, key: &str) -> DomainError {
    DomainError::new(
        ErrorCode::SecurityPolicyDenied,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

/// Mirrors `pmc-domain/src/delivery.rs`'s own private `monotonic_classification`:
/// an ordinary H1 update may only raise (or keep) a record's classification,
/// never silently lower it -- lowering requires the governed H2a "Lower
/// Data Classification" intent.
fn monotonic_classification(
    current: DataClassification,
    requested: Option<DataClassification>,
    context: &OperationContext,
) -> Result<DataClassification, DomainError> {
    match requested {
        None => Ok(current),
        Some(requested) => {
            if requested.combine(current) != requested {
                return Err(policy_denied(
                    context,
                    "classification.lowering_requires_governed_intent",
                ));
            }
            Ok(requested)
        }
    }
}

fn decode_delivery_provenance(
    kind: &str,
    reference: Option<String>,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    match kind {
        "user_entered" => Ok(Provenance::UserEntered),
        "authoritative_transition" => Ok(Provenance::AuthoritativeTransition(
            ProvenanceReference::parse(reference.ok_or_else(|| storage_error(context))?)
                .map_err(|_| storage_error(context))?,
        )),
        "synthetic_fixture" => Ok(Provenance::SyntheticFixture(
            ProvenanceReference::parse(reference.ok_or_else(|| storage_error(context))?)
                .map_err(|_| storage_error(context))?,
        )),
        _ => Err(storage_error(context)),
    }
}

fn build_delivery_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    target: AuditTarget,
    code: &str,
    context: &OperationContext,
) -> Result<AuditEvent, DomainError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::Portfolio,
            AuditEventCode::parse(code).map_err(|_| storage_error(context))?,
            target,
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::NotRequired,
            pmc_domain::audit::AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse(DELIVERY_EFFECT_CODE).map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn persist_delivery_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
    target_type: &str,
    target_id: &str,
    context: &OperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','portfolio',?3,?4,?5,?6,'not_required','not_required','succeeded','complete')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_type,
            target_id,
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete',?3,?4)",
        rusqlite::params![audit.id().as_str(), DELIVERY_EFFECT_CODE, target_type, target_id],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn decode_delivery_audit(
    tx: &Transaction<'_>,
    audit_event_id: &str,
    context: &OperationContext,
) -> Result<AuditEvent, DomainError> {
    let row = tx
        .query_row(
            "SELECT occurred_at,event_code,target_type,target_id,correlation_id FROM audit_events WHERE id=?1",
            [audit_event_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let target = match row.2.as_str() {
        "initiative" => {
            AuditTarget::Initiative(InitiativeId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        "project" => {
            AuditTarget::Project(ProjectId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        "milestone" => {
            AuditTarget::Milestone(MilestoneId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        _ => return Err(storage_error(context)),
    };
    Ok(AuditEvent::new(
        AuditEventId::parse(audit_event_id).map_err(|_| storage_error(context))?,
        UtcTimestamp::from_unix_millis(row.0),
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::Portfolio,
            AuditEventCode::parse(&row.1).map_err(|_| storage_error(context))?,
            target,
        ),
        pmc_domain::identity::CorrelationId::parse(row.4).map_err(|_| storage_error(context))?,
        AuditDisposition::new(
            AuditPolicyOutcome::NotRequired,
            pmc_domain::audit::AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse(DELIVERY_EFFECT_CODE).map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

/// Delivery's replay-capsule model requires one *global* gapless-from-zero
/// `operation_ordinal` spanning every H1 write and H2a lowering execute
/// together -- confirmed by reading `pmc-domain/src/delivery.rs::
/// validate_operation_timeline` directly, a stricter requirement than
/// Portfolio's own independent per-table ordinal sequences (H2a lowering
/// *prepare* is the one exception: it never produces a `DeliveryReplayCapsule`
/// per DG0 6.7, so it keeps its own separate sequence in
/// `next_delivery_h2a_prepare_operation_ordinal`). Widened here (mirrors
/// `action_repository.rs`'s equivalent widening) to union
/// `delivery_h2a_execute_replay_operations`, which a lowering execute writes
/// to instead of `delivery_idempotency_outcomes`.
fn next_delivery_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM delivery_idempotency_outcomes WHERE namespace='delivery' UNION ALL SELECT operation_ordinal FROM delivery_h2a_execute_replay_operations)",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_delivery_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM delivery_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn delivery_idempotency_claimed_by_other_operation(
    tx: &Transaction<'_>,
    context: &OperationContext,
    operation: &str,
) -> Result<bool, DomainError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1 AND operation!=?2)",
        rusqlite::params![context.idempotency_id.as_str(), operation],
        |row| row.get::<_, i64>(0),
    )
    .map(|found| found != 0)
    .map_err(|_| storage_error(context))
}

/// See `portfolio_repository.rs::prepared_intent_digest_matches`: a prepare
/// replay must bind to the exact previously persisted preview's
/// cryptographic digest, not merely to matching command scalars.
fn delivery_prepared_intent_digest_matches(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    let stored_digest: Option<String> = tx
        .query_row(
            "SELECT payload_digest FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| storage_error(context))?;
    Ok(stored_digest.as_deref() == Some(prepared.payload_digest().as_str()))
}

/// Persists the generic `prepared_intents` row shared by every H2a preview
/// family. Callers still separately insert the target and typed payload
/// rows this row's foreign keys require.
fn persist_delivery_prepared_intent(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    intent_kind: &str,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let expires_at = prepared.preview().expires_at().unix_millis();
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES (?1,?2,?3,?4,?5,'allowed','not_cancellable_after_submit','head_of_products',?6,?7)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(prepared.preview().contract_version()),
            intent_kind,
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            expires_at,
            expires_at - pmc_domain::work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS,
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Re-reads a previously committed classification lowering by the exact
/// execute-time result snapshot stored in `delivery_h2a_execute_replay_
/// operations` -- NOT the target's current durable row, which may since
/// have been mutated again by a later H1 update. Mirrors
/// `portfolio_repository.rs::replay_lowered_outcome`'s intent (re-read a
/// previously committed lowering), but Portfolio's own model can safely
/// read current state because its `validate` never replays a strict global
/// ordinal sequence; Delivery's `validate_operation_timeline` does, so every
/// historical capsule must reflect its state *at that point*, not now --
/// see the V35 migration's schema doc comment for the bug this snapshot
/// column set fixes.
fn replay_lowered_delivery_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<(DeliveryPersistenceResult, AuditEvent), DomainError> {
    let row = tx
        .query_row(
            "SELECT target_kind,target_id,result_name,result_project_id,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at FROM delivery_h2a_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            delivery_h2a_execute_snapshot_row,
        )
        .map_err(|_| storage_error(context))?;
    let result = decode_delivery_lowered_snapshot(row, context)?;
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM delivery_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    Ok((result, decode_delivery_audit(tx, &audit_event_id, context)?))
}

/// Raw columns of one stored H2a execute result snapshot -- shared by the
/// live idempotent-replay path (`replay_lowered_delivery_outcome`) and
/// restart decode (`decode_all_delivery_h2a_execute_capsules`).
struct DeliveryH2aExecuteSnapshotRow {
    target_kind: String,
    target_id: String,
    name: String,
    project_id: Option<String>,
    defined_outcome: Option<String>,
    verification_criteria: Option<String>,
    start_at: Option<i64>,
    end_at: Option<i64>,
    due_at: Option<i64>,
    classification: String,
    provenance_kind: String,
    provenance_reference: Option<String>,
    version: i64,
    created_at: i64,
    updated_at: i64,
}

fn delivery_h2a_execute_snapshot_row(
    row: &Row<'_>,
) -> rusqlite::Result<DeliveryH2aExecuteSnapshotRow> {
    Ok(DeliveryH2aExecuteSnapshotRow {
        target_kind: row.get(0)?,
        target_id: row.get(1)?,
        name: row.get(2)?,
        project_id: row.get(3)?,
        defined_outcome: row.get(4)?,
        verification_criteria: row.get(5)?,
        start_at: row.get(6)?,
        end_at: row.get(7)?,
        due_at: row.get(8)?,
        classification: row.get(9)?,
        provenance_kind: row.get(10)?,
        provenance_reference: row.get(11)?,
        version: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn decode_delivery_lowered_snapshot(
    row: DeliveryH2aExecuteSnapshotRow,
    context: &OperationContext,
) -> Result<DeliveryPersistenceResult, DomainError> {
    let classification = DataClassification::from_persisted(&row.classification)
        .map_err(|_| storage_error(context))?;
    let provenance =
        decode_delivery_provenance(&row.provenance_kind, row.provenance_reference, context)?;
    let version =
        AggregateVersion::new(u64::try_from(row.version).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?;
    let created_at = UtcTimestamp::from_unix_millis(row.created_at);
    let updated_at = UtcTimestamp::from_unix_millis(row.updated_at);
    let name =
        RecordName::parse(row.name, &context.correlation_id).map_err(|_| storage_error(context))?;
    match row.target_kind.as_str() {
        "initiative" => Ok(DeliveryPersistenceResult::Initiative(
            Initiative::rehydrate(InitiativePersistenceRecord {
                id: InitiativeId::parse(row.target_id).map_err(|_| storage_error(context))?,
                name,
                defined_outcome: DefinedOutcome::parse(
                    row.defined_outcome.ok_or_else(|| storage_error(context))?,
                    &context.correlation_id,
                )
                .map_err(|_| storage_error(context))?,
                classification,
                provenance,
                version,
                created_at,
                updated_at,
            })
            .map_err(|_| storage_error(context))?,
        )),
        "project" => Ok(DeliveryPersistenceResult::Project(
            Project::rehydrate(ProjectPersistenceRecord {
                id: ProjectId::parse(row.target_id).map_err(|_| storage_error(context))?,
                name,
                start_at: UtcTimestamp::from_unix_millis(
                    row.start_at.ok_or_else(|| storage_error(context))?,
                ),
                end_at: UtcTimestamp::from_unix_millis(
                    row.end_at.ok_or_else(|| storage_error(context))?,
                ),
                classification,
                provenance,
                version,
                created_at,
                updated_at,
            })
            .map_err(|_| storage_error(context))?,
        )),
        "milestone" => Ok(DeliveryPersistenceResult::Milestone(
            Milestone::rehydrate(MilestonePersistenceRecord {
                id: MilestoneId::parse(row.target_id).map_err(|_| storage_error(context))?,
                project_id: ProjectId::parse(row.project_id.ok_or_else(|| storage_error(context))?)
                    .map_err(|_| storage_error(context))?,
                name,
                verification_criteria: VerificationCriteria::parse(
                    row.verification_criteria
                        .ok_or_else(|| storage_error(context))?,
                    &context.correlation_id,
                )
                .map_err(|_| storage_error(context))?,
                due_at: UtcTimestamp::from_unix_millis(
                    row.due_at.ok_or_else(|| storage_error(context))?,
                ),
                classification,
                provenance,
                version,
                created_at,
                updated_at,
            })
            .map_err(|_| storage_error(context))?,
        )),
        _ => Err(storage_error(context)),
    }
}

/// Persists one successful Delivery classification-lowering execution:
/// the mutation (classification + version bump on the existing
/// `aggregate_registry` row -- the typed `initiatives`/`projects`/
/// `milestones` tables carry no classification column, so nothing there
/// changes), its audit event, the consumed prepared intent, the minted
/// approval receipt, and this adapter's own typed H2a execute replay
/// bookkeeping. One shared function across all three record types (unlike
/// `portfolio_repository.rs::persist_lowered_bundle`'s per-type copies)
/// since Delivery's H2a execute uses one shared table set with a
/// `target_kind` discriminator.
#[allow(clippy::too_many_arguments)]
/// The fields of a lowering's resulting record needed to persist an exact,
/// immutable snapshot alongside `delivery_h2a_execute_replay_operations` --
/// see that constant's schema doc comment (V35) for why a snapshot is
/// required rather than re-querying the target's current durable row.
struct DeliveryLoweredSnapshot<'a> {
    target_kind: &'static str,
    target_id: &'a str,
    name: &'a str,
    project_id: Option<&'a str>,
    defined_outcome: Option<&'a str>,
    verification_criteria: Option<&'a str>,
    start_at: Option<i64>,
    end_at: Option<i64>,
    due_at: Option<i64>,
    classification: DataClassification,
    provenance_kind: &'static str,
    provenance_reference: Option<&'a str>,
    version: AggregateVersion,
    created_at: i64,
    updated_at: i64,
}

fn delivery_lowered_snapshot(result: &DeliveryPersistenceResult) -> DeliveryLoweredSnapshot<'_> {
    match result {
        DeliveryPersistenceResult::Initiative(v) => DeliveryLoweredSnapshot {
            target_kind: "initiative",
            target_id: v.id().as_str(),
            name: v.name(),
            project_id: None,
            defined_outcome: Some(v.defined_outcome()),
            verification_criteria: None,
            start_at: None,
            end_at: None,
            due_at: None,
            classification: v.classification(),
            provenance_kind: v.provenance().kind_persisted(),
            provenance_reference: v.provenance().reference().map(ProvenanceReference::as_str),
            version: v.version(),
            created_at: v.created_at().unix_millis(),
            updated_at: v.updated_at().unix_millis(),
        },
        DeliveryPersistenceResult::Project(v) => DeliveryLoweredSnapshot {
            target_kind: "project",
            target_id: v.id().as_str(),
            name: v.name(),
            project_id: None,
            defined_outcome: None,
            verification_criteria: None,
            start_at: Some(v.start_at().unix_millis()),
            end_at: Some(v.end_at().unix_millis()),
            due_at: None,
            classification: v.classification(),
            provenance_kind: v.provenance().kind_persisted(),
            provenance_reference: v.provenance().reference().map(ProvenanceReference::as_str),
            version: v.version(),
            created_at: v.created_at().unix_millis(),
            updated_at: v.updated_at().unix_millis(),
        },
        DeliveryPersistenceResult::Milestone(v) => DeliveryLoweredSnapshot {
            target_kind: "milestone",
            target_id: v.id().as_str(),
            name: v.name(),
            project_id: Some(v.project_id().as_str()),
            defined_outcome: None,
            verification_criteria: Some(v.verification_criteria()),
            start_at: None,
            end_at: None,
            due_at: Some(v.due_at().unix_millis()),
            classification: v.classification(),
            provenance_kind: v.provenance().kind_persisted(),
            provenance_reference: v.provenance().reference().map(ProvenanceReference::as_str),
            version: v.version(),
            created_at: v.created_at().unix_millis(),
            updated_at: v.updated_at().unix_millis(),
        },
    }
}

fn persist_lowered_delivery_bundle(
    tx: &Transaction<'_>,
    result: &DeliveryPersistenceResult,
    audit: &AuditEvent,
    approval: &WorkManagementApproval,
    approval_receipt_id: &ApprovalReceiptId,
    ordinal: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let snapshot = delivery_lowered_snapshot(result);
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type=?5",
        rusqlite::params![
            i64::try_from(snapshot.version.get()).map_err(|_| storage_error(context))?,
            snapshot.classification.as_persisted(),
            occurred_millis,
            snapshot.target_id,
            snapshot.target_kind,
        ],
    )
    .map_err(|_| storage_error(context))?;
    persist_delivery_audit(tx, audit, snapshot.target_kind, snapshot.target_id, context)?;
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![
            approval_receipt_id.as_str(),
            context.idempotency_id.as_str(),
            occurred_millis,
            approval.prepared_id().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO delivery_h2a_execute_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,target_kind,target_id,prepared_intent_id,approval_receipt_id,result_name,result_project_id,result_defined_outcome,result_verification_criteria,result_start_at,result_end_at,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES (?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            snapshot.target_kind,
            snapshot.target_id,
            approval.prepared_id().as_str(),
            approval_receipt_id.as_str(),
            snapshot.name,
            snapshot.project_id,
            snapshot.defined_outcome,
            snapshot.verification_criteria,
            snapshot.start_at,
            snapshot.end_at,
            snapshot.due_at,
            snapshot.classification.as_persisted(),
            snapshot.provenance_kind,
            snapshot.provenance_reference,
            i64::try_from(snapshot.version.get()).map_err(|_| storage_error(context))?,
            snapshot.created_at,
            snapshot.updated_at,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO delivery_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO delivery_h2a_command_executes(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Seeds a throwaway `InMemoryDeliveryService` with exactly the one
/// Initiative an H2a lowering touches, reconstructed to its exact current
/// (name, defined_outcome, classification, provenance, version) via the
/// service's own public H1 API. See `portfolio_repository.rs::
/// seed_portfolio_service_for_lowering` for the full rationale -- same
/// technique, duplicated per module and per Delivery record type.
fn seed_initiative_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    initiative_id: &InitiativeId,
    context: &OperationContext,
) -> Result<InMemoryDeliveryService<C, QueuedDeliveryAuditIds>, DomainError> {
    let (name, defined_outcome, classification, provenance_kind, provenance_reference, version): (
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT i.name,i.defined_outcome,r.classification,i.provenance_kind,i.provenance_reference,r.version FROM initiatives i JOIN aggregate_registry r ON r.id=i.id AND r.aggregate_type='initiative' WHERE i.id=?1",
            [initiative_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| domain_not_found(context, "delivery.not_found"))?;
    let name =
        RecordName::parse(name, &context.correlation_id).map_err(|_| storage_error(context))?;
    let defined_outcome = DefinedOutcome::parse(defined_outcome, &context.correlation_id)
        .map_err(|_| storage_error(context))?;
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_delivery_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!(
                "replay-seed-audit-{}-{step}",
                initiative_id.as_str()
            ))
            .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryDeliveryService::new(clock, QueuedDeliveryAuditIds(synthetic_ids));
    service
        .create_initiative(CreateInitiative {
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    initiative_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
            id: initiative_id.clone(),
            name: name.clone(),
            defined_outcome: defined_outcome.clone(),
            classification: Some(classification),
            provenance: provenance.clone(),
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_initiative(UpdateInitiative {
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        initiative_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
                id: initiative_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                defined_outcome: defined_outcome.clone(),
                classification: None,
                provenance: provenance.clone(),
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

/// See `seed_initiative_service_for_lowering` -- identical shape for Project.
fn seed_project_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    project_id: &ProjectId,
    context: &OperationContext,
) -> Result<InMemoryDeliveryService<C, QueuedDeliveryAuditIds>, DomainError> {
    let (name, start_at, end_at, classification, provenance_kind, provenance_reference, version): (
        String,
        i64,
        i64,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT p.name,p.start_at,p.end_at,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM projects p JOIN aggregate_registry r ON r.id=p.id AND r.aggregate_type='project' WHERE p.id=?1",
            [project_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| domain_not_found(context, "delivery.not_found"))?;
    let name =
        RecordName::parse(name, &context.correlation_id).map_err(|_| storage_error(context))?;
    let start_at = UtcTimestamp::from_unix_millis(start_at);
    let end_at = UtcTimestamp::from_unix_millis(end_at);
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_delivery_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!("replay-seed-audit-{}-{step}", project_id.as_str()))
                .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryDeliveryService::new(clock, QueuedDeliveryAuditIds(synthetic_ids));
    service
        .create_project(CreateProject {
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    project_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
            id: project_id.clone(),
            name: name.clone(),
            start_at,
            end_at,
            classification: Some(classification),
            provenance: provenance.clone(),
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_project(UpdateProject {
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        project_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
                id: project_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                start_at,
                end_at,
                classification: None,
                provenance: provenance.clone(),
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

/// See `seed_initiative_service_for_lowering`. Milestone additionally needs
/// a synthetic parent Project seeded first (`create_milestone` requires the
/// parent to exist in the throwaway service's own state) -- built with the
/// real parent's *current* classification, which by the standing
/// parent-dominates-child invariant is guaranteed to be dominated by the
/// target Milestone's own current classification, so
/// `CreateMilestone{classification: Some(target)}`'s
/// `parent.combine(target)` computation reproduces `target` exactly. The
/// synthetic parent's other fields are throwaway placeholders: Milestone
/// lowering never reads anything about its parent besides classification.
fn seed_milestone_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    milestone_id: &MilestoneId,
    context: &OperationContext,
) -> Result<InMemoryDeliveryService<C, QueuedDeliveryAuditIds>, DomainError> {
    let (
        project_id,
        name,
        verification_criteria,
        due_at,
        classification,
        provenance_kind,
        provenance_reference,
        version,
    ): (String, String, String, i64, String, String, Option<String>, i64) = tx
        .query_row(
            "SELECT m.project_id,m.name,m.verification_criteria,m.due_at,r.classification,m.provenance_kind,m.provenance_reference,r.version FROM milestones m JOIN aggregate_registry r ON r.id=m.id AND r.aggregate_type='milestone' WHERE m.id=?1",
            [milestone_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| domain_not_found(context, "delivery.not_found"))?;
    let project_id = ProjectId::parse(project_id).map_err(|_| storage_error(context))?;
    let name =
        RecordName::parse(name, &context.correlation_id).map_err(|_| storage_error(context))?;
    let verification_criteria =
        VerificationCriteria::parse(verification_criteria, &context.correlation_id)
            .map_err(|_| storage_error(context))?;
    let due_at = UtcTimestamp::from_unix_millis(due_at);
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_delivery_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;
    let parent_classification: String = tx
        .query_row(
            "SELECT classification FROM aggregate_registry WHERE id=?1 AND aggregate_type='project'",
            [project_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let parent_classification = DataClassification::from_persisted(&parent_classification)
        .map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 2,
    );
    // One extra synthetic id for the synthetic parent Project's own create.
    synthetic_ids.push_back(
        AuditEventId::parse(format!(
            "replay-seed-audit-parent-{}",
            milestone_id.as_str()
        ))
        .map_err(|_| storage_error(context))?,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!(
                "replay-seed-audit-{}-{step}",
                milestone_id.as_str()
            ))
            .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryDeliveryService::new(clock, QueuedDeliveryAuditIds(synthetic_ids));
    service
        .create_project(CreateProject {
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-parent-{}",
                    milestone_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
            id: project_id.clone(),
            name: RecordName::parse("seed", &context.correlation_id)
                .map_err(|_| storage_error(context))?,
            start_at: UtcTimestamp::from_unix_millis(0),
            end_at: UtcTimestamp::from_unix_millis(0),
            classification: Some(parent_classification),
            provenance: provenance.clone(),
        })
        .map_err(|_| storage_error(context))?;
    service
        .create_milestone(CreateMilestone {
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    milestone_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
            id: milestone_id.clone(),
            project_id: project_id.clone(),
            name: name.clone(),
            verification_criteria: verification_criteria.clone(),
            due_at,
            classification: Some(classification),
            provenance: provenance.clone(),
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_milestone(UpdateMilestone {
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        milestone_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
                id: milestone_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                verification_criteria: verification_criteria.clone(),
                due_at,
                classification: None,
                provenance: provenance.clone(),
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

fn persist_delivery_idempotency_claim(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
    ordinal: i64,
    occurred_millis: i64,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO idempotency_outcomes VALUES('delivery',?1,?2,?2,'succeeded',?3,?4)",
        rusqlite::params![
            operation,
            context.idempotency_id.as_str(),
            context.idempotency_id.as_str(),
            occurred_millis,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO delivery_idempotency_outcomes VALUES('delivery',?1,?2,?1,?3,?4)",
        rusqlite::params![
            operation,
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn persist_delivery_idempotency_outcome_audit(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
    ordinal: i64,
    audit_event_id: &str,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO delivery_idempotency_outcome_audits VALUES('delivery',?1,?2,?3,?4)",
        rusqlite::params![
            operation,
            context.idempotency_id.as_str(),
            ordinal,
            audit_event_id
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn decode_delivery_initiative_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<DeliveryMutationOutcome<Initiative>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_defined_outcome,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at FROM delivery_command_results WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = Initiative::rehydrate(pmc_domain::delivery::InitiativePersistenceRecord {
        id: InitiativeId::parse(row.0).map_err(|_| storage_error(context))?,
        name: RecordName::parse(row.1, &context.correlation_id)
            .map_err(|_| storage_error(context))?,
        defined_outcome: DefinedOutcome::parse(
            row.2.ok_or_else(|| storage_error(context))?,
            &context.correlation_id,
        )
        .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_delivery_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.7),
        updated_at: UtcTimestamp::from_unix_millis(row.8),
    })
    .map_err(|_| storage_error(context))?;
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM delivery_idempotency_outcome_audits WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    Ok(DeliveryMutationOutcome {
        record,
        audit_events: vec![decode_delivery_audit(tx, &audit_event_id, context)?],
        cascaded_milestones: Vec::new(),
    })
}

fn decode_delivery_project_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<DeliveryMutationOutcome<Project>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_start_at,result_end_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at FROM delivery_command_results WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = Project::rehydrate(pmc_domain::delivery::ProjectPersistenceRecord {
        id: ProjectId::parse(row.0).map_err(|_| storage_error(context))?,
        name: RecordName::parse(row.1, &context.correlation_id)
            .map_err(|_| storage_error(context))?,
        start_at: UtcTimestamp::from_unix_millis(row.2.ok_or_else(|| storage_error(context))?),
        end_at: UtcTimestamp::from_unix_millis(row.3.ok_or_else(|| storage_error(context))?),
        classification: DataClassification::from_persisted(&row.4)
            .map_err(|_| storage_error(context))?,
        provenance: decode_delivery_provenance(&row.5, row.6, context)?,
        version: AggregateVersion::new(u64::try_from(row.7).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.8),
        updated_at: UtcTimestamp::from_unix_millis(row.9),
    })
    .map_err(|_| storage_error(context))?;
    let audit_event_ids: Vec<String> = tx
        .prepare("SELECT audit_event_id FROM delivery_idempotency_outcome_audits WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal")
        .map_err(|_| storage_error(context))?
        .query_map(rusqlite::params![operation, context.idempotency_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    let mut audit_events = Vec::with_capacity(audit_event_ids.len());
    for audit_event_id in &audit_event_ids {
        audit_events.push(decode_delivery_audit(tx, audit_event_id, context)?);
    }
    Ok(DeliveryMutationOutcome {
        record,
        audit_events,
        cascaded_milestones: Vec::new(),
    })
}

fn decode_delivery_milestone_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<DeliveryMutationOutcome<Milestone>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_project_id,result_name,result_verification_criteria,result_due_at,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at FROM delivery_command_results WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = Milestone::rehydrate(pmc_domain::delivery::MilestonePersistenceRecord {
        id: MilestoneId::parse(row.0).map_err(|_| storage_error(context))?,
        project_id: ProjectId::parse(row.1.ok_or_else(|| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        name: RecordName::parse(row.2, &context.correlation_id)
            .map_err(|_| storage_error(context))?,
        verification_criteria: VerificationCriteria::parse(
            row.3.ok_or_else(|| storage_error(context))?,
            &context.correlation_id,
        )
        .map_err(|_| storage_error(context))?,
        due_at: UtcTimestamp::from_unix_millis(row.4.ok_or_else(|| storage_error(context))?),
        classification: DataClassification::from_persisted(&row.5)
            .map_err(|_| storage_error(context))?,
        provenance: decode_delivery_provenance(&row.6, row.7, context)?,
        version: AggregateVersion::new(u64::try_from(row.8).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.9),
        updated_at: UtcTimestamp::from_unix_millis(row.10),
    })
    .map_err(|_| storage_error(context))?;
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM delivery_idempotency_outcome_audits WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    Ok(DeliveryMutationOutcome {
        record,
        audit_events: vec![decode_delivery_audit(tx, &audit_event_id, context)?],
        cascaded_milestones: Vec::new(),
    })
}

fn decode_delivery_namespace(
    tx: &Transaction<'_>,
) -> Result<DeliveryPersistenceSnapshot, DeliveryPersistenceLoadError> {
    let initiatives = decode_all_initiatives(tx)?;
    let projects = decode_all_projects(tx)?;
    let milestones = decode_all_milestones(tx)?;
    let raw_rows = tx
        .prepare(
            "SELECT r.operation,r.idempotency_id,o.correlation_id,o.operation_ordinal,r.command_kind,r.command_target_id,r.command_expected_version,r.command_name,r.command_defined_outcome,r.command_project_id,r.command_verification_criteria,r.command_start_at,r.command_end_at,r.command_due_at,r.command_classification,r.command_provenance_kind,r.command_provenance_reference,r.result_kind,r.result_id,r.result_project_id,r.result_name,r.result_defined_outcome,r.result_verification_criteria,r.result_start_at,r.result_end_at,r.result_due_at,r.result_classification,r.result_provenance_kind,r.result_provenance_reference,r.result_version,r.result_created_at,r.result_updated_at FROM delivery_command_results r JOIN delivery_idempotency_outcomes o USING(namespace,operation,idempotency_id) WHERE r.namespace='delivery' ORDER BY o.operation_ordinal",
        )
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .query_map([], delivery_raw_row)
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
    let mut replay = Vec::with_capacity(raw_rows.len());
    for row in raw_rows {
        replay.push(decode_delivery_capsule(row, tx)?);
    }
    replay.extend(decode_all_delivery_h2a_execute_capsules(tx)?);
    let audits = decode_all_delivery_audits(tx)?;
    DeliveryPersistenceSnapshot::validate(initiatives, projects, milestones, replay, audits)
        .map_err(DeliveryPersistenceLoadError::from)
}

/// Reconstructs the H2a lowering-execute half of the replay history --
/// deliberately excludes any outstanding (unapproved) lowering *preview*,
/// per DG0 6.7: an unapproved H2a preview must not silently survive a
/// restart, so `delivery_h2a_prepare_replay_operations` is never read here.
fn decode_all_delivery_h2a_execute_capsules(
    tx: &Transaction<'_>,
) -> Result<Vec<DeliveryReplayCapsule>, DeliveryPersistenceLoadError> {
    let err = || DeliveryPersistenceLoadError::InvalidDeliverySnapshot;
    struct ExecuteRow {
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        prepared_id: String,
        actor: String,
        acknowledged_digest: String,
        snapshot: DeliveryH2aExecuteSnapshotRow,
    }
    let rows: Vec<ExecuteRow> = tx
        .prepare("SELECT r.idempotency_id,r.correlation_id,r.operation_ordinal,c.prepared_id,c.actor,c.acknowledged_digest,r.target_kind,r.target_id,r.result_name,r.result_project_id,r.result_defined_outcome,r.result_verification_criteria,r.result_start_at,r.result_end_at,r.result_due_at,r.result_classification,r.result_provenance_kind,r.result_provenance_reference,r.result_version,r.result_created_at,r.result_updated_at FROM delivery_h2a_execute_replay_operations r JOIN delivery_h2a_command_executes c USING(idempotency_id)")
        .map_err(|_| err())?
        .query_map([], |row| {
            Ok(ExecuteRow {
                idempotency_id: row.get(0)?,
                correlation_id: row.get(1)?,
                ordinal: row.get(2)?,
                prepared_id: row.get(3)?,
                actor: row.get(4)?,
                acknowledged_digest: row.get(5)?,
                snapshot: DeliveryH2aExecuteSnapshotRow {
                    target_kind: row.get(6)?,
                    target_id: row.get(7)?,
                    name: row.get(8)?,
                    project_id: row.get(9)?,
                    defined_outcome: row.get(10)?,
                    verification_criteria: row.get(11)?,
                    start_at: row.get(12)?,
                    end_at: row.get(13)?,
                    due_at: row.get(14)?,
                    classification: row.get(15)?,
                    provenance_kind: row.get(16)?,
                    provenance_reference: row.get(17)?,
                    version: row.get(18)?,
                    created_at: row.get(19)?,
                    updated_at: row.get(20)?,
                },
            })
        })
        .map_err(|_| err())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| err())?;
    let mut capsules = Vec::with_capacity(rows.len());
    for row in rows {
        let correlation = CorrelationId::parse(row.correlation_id).map_err(|_| err())?;
        let prepared_id = PreparedIntentId::parse(row.prepared_id).map_err(|_| err())?;
        let actor = AuditActor::from_persisted(&row.actor).map_err(|_| err())?;
        let acknowledged_payload_digest =
            WorkManagementPayloadDigest::from_persisted(row.acknowledged_digest)
                .map_err(|_| err())?;
        let decode_context = dummy_delivery_decode_context(&correlation)?;
        let result =
            decode_delivery_lowered_snapshot(row.snapshot, &decode_context).map_err(|_| err())?;
        let command = match &result {
            DeliveryPersistenceResult::Initiative(_) => {
                CommandIdentity::ApproveAndExecuteLowerInitiativeClassification {
                    prepared_id,
                    actor,
                    acknowledged_payload_digest,
                }
            }
            DeliveryPersistenceResult::Project(_) => {
                CommandIdentity::ApproveAndExecuteLowerProjectClassification {
                    prepared_id,
                    actor,
                    acknowledged_payload_digest,
                }
            }
            DeliveryPersistenceResult::Milestone(_) => {
                CommandIdentity::ApproveAndExecuteLowerMilestoneClassification {
                    prepared_id,
                    actor,
                    acknowledged_payload_digest,
                }
            }
        };
        let audit_event_id: String = tx
            .query_row(
                "SELECT audit_event_id FROM delivery_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
                [row.idempotency_id.as_str()],
                |r| r.get(0),
            )
            .map_err(|_| err())?;
        let audit_event_id =
            pmc_domain::identity::AuditEventId::parse(audit_event_id).map_err(|_| err())?;
        capsules.push(DeliveryReplayCapsule::new(
            IdempotencyId::parse(row.idempotency_id).map_err(|_| err())?,
            command,
            result,
            correlation,
            vec![audit_event_id],
            u64::try_from(row.ordinal).map_err(|_| err())?,
            Vec::new(),
        ));
    }
    Ok(capsules)
}

/// A throwaway `OperationContext` for decode-path helpers that only need it
/// to satisfy `storage_error`'s signature -- never observed by a caller.
fn dummy_delivery_decode_context(
    correlation_id: &CorrelationId,
) -> Result<OperationContext, DeliveryPersistenceLoadError> {
    Ok(OperationContext {
        correlation_id: correlation_id.clone(),
        idempotency_id: IdempotencyId::parse("delivery-decode")
            .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
    })
}

fn decode_all_initiatives(
    tx: &Transaction<'_>,
) -> Result<Vec<Initiative>, DeliveryPersistenceLoadError> {
    tx.prepare("SELECT initiatives.id,initiatives.name,initiatives.defined_outcome,registry.classification,initiatives.provenance_kind,initiatives.provenance_reference,registry.version,registry.created_at,registry.updated_at FROM initiatives JOIN aggregate_registry registry ON registry.id=initiatives.id AND registry.aggregate_type='initiative' ORDER BY initiatives.id")
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)?, row.get::<_, i64>(7)?, row.get::<_, i64>(8)?,
            ))
        })
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .map(|row| {
            let row = row.map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
            let correlation = CorrelationId::parse("delivery-decode").map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
            Initiative::rehydrate(InitiativePersistenceRecord {
                id: InitiativeId::parse(row.0).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                name: RecordName::parse(row.1, &correlation).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                defined_outcome: DefinedOutcome::parse(row.2, &correlation).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                classification: DataClassification::from_persisted(&row.3).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                provenance: decode_persisted_delivery_provenance(&row.4, row.5)?,
                version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                created_at: UtcTimestamp::from_unix_millis(row.7),
                updated_at: UtcTimestamp::from_unix_millis(row.8),
            })
            .map_err(DeliveryPersistenceLoadError::from)
        })
        .collect()
}

fn decode_all_projects(tx: &Transaction<'_>) -> Result<Vec<Project>, DeliveryPersistenceLoadError> {
    tx.prepare("SELECT projects.id,projects.name,projects.start_at,projects.end_at,registry.classification,projects.provenance_kind,projects.provenance_reference,registry.version,registry.created_at,registry.updated_at FROM projects JOIN aggregate_registry registry ON registry.id=projects.id AND registry.aggregate_type='project' ORDER BY projects.id")
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?, row.get::<_, i64>(7)?, row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .map(|row| {
            let row = row.map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
            let correlation = CorrelationId::parse("delivery-decode").map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
            Project::rehydrate(ProjectPersistenceRecord {
                id: ProjectId::parse(row.0).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                name: RecordName::parse(row.1, &correlation).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                start_at: UtcTimestamp::from_unix_millis(row.2),
                end_at: UtcTimestamp::from_unix_millis(row.3),
                classification: DataClassification::from_persisted(&row.4).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                provenance: decode_persisted_delivery_provenance(&row.5, row.6)?,
                version: AggregateVersion::new(u64::try_from(row.7).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                created_at: UtcTimestamp::from_unix_millis(row.8),
                updated_at: UtcTimestamp::from_unix_millis(row.9),
            })
            .map_err(DeliveryPersistenceLoadError::from)
        })
        .collect()
}

fn decode_all_milestones(
    tx: &Transaction<'_>,
) -> Result<Vec<Milestone>, DeliveryPersistenceLoadError> {
    tx.prepare("SELECT milestones.id,milestones.project_id,milestones.name,milestones.verification_criteria,milestones.due_at,registry.classification,milestones.provenance_kind,milestones.provenance_reference,registry.version,registry.created_at,registry.updated_at FROM milestones JOIN aggregate_registry registry ON registry.id=milestones.id AND registry.aggregate_type='milestone' ORDER BY milestones.id")
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, String>(5)?,
                row.get::<_, String>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?, row.get::<_, i64>(10)?,
            ))
        })
        .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?
        .map(|row| {
            let row = row.map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
            let correlation = CorrelationId::parse("delivery-decode").map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?;
            Milestone::rehydrate(MilestonePersistenceRecord {
                id: MilestoneId::parse(row.0).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                project_id: ProjectId::parse(row.1).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                name: RecordName::parse(row.2, &correlation).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                verification_criteria: VerificationCriteria::parse(row.3, &correlation).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                due_at: UtcTimestamp::from_unix_millis(row.4),
                classification: DataClassification::from_persisted(&row.5).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                provenance: decode_persisted_delivery_provenance(&row.6, row.7)?,
                version: AggregateVersion::new(u64::try_from(row.8).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?).map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
                created_at: UtcTimestamp::from_unix_millis(row.9),
                updated_at: UtcTimestamp::from_unix_millis(row.10),
            })
            .map_err(DeliveryPersistenceLoadError::from)
        })
        .collect()
}

fn decode_persisted_delivery_provenance(
    kind: &str,
    reference: Option<String>,
) -> Result<Provenance, DeliveryPersistenceLoadError> {
    match kind {
        "user_entered" => Ok(Provenance::UserEntered),
        "authoritative_transition" => Ok(Provenance::AuthoritativeTransition(
            ProvenanceReference::parse(
                reference.ok_or(DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
            )
            .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
        )),
        "synthetic_fixture" => Ok(Provenance::SyntheticFixture(
            ProvenanceReference::parse(
                reference.ok_or(DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
            )
            .map_err(|_| DeliveryPersistenceLoadError::InvalidDeliverySnapshot)?,
        )),
        _ => Err(DeliveryPersistenceLoadError::InvalidDeliverySnapshot),
    }
}

#[allow(clippy::type_complexity)]
struct DeliveryRawRow {
    operation: String,
    idempotency: String,
    correlation: String,
    ordinal: i64,
    kind: String,
    target: String,
    expected: Option<i64>,
    name: String,
    defined: Option<String>,
    project: Option<String>,
    criteria: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    due: Option<i64>,
    class: Option<String>,
    pkind: String,
    pref: Option<String>,
    rkind: String,
    rid: String,
    rproject: Option<String>,
    rname: String,
    rdefined: Option<String>,
    rcriteria: Option<String>,
    rstart: Option<i64>,
    rend: Option<i64>,
    rdue: Option<i64>,
    rclass: String,
    rpkind: String,
    rpref: Option<String>,
    version: i64,
    created: i64,
    updated: i64,
}

fn delivery_raw_row(row: &Row<'_>) -> rusqlite::Result<DeliveryRawRow> {
    Ok(DeliveryRawRow {
        operation: row.get(0)?,
        idempotency: row.get(1)?,
        correlation: row.get(2)?,
        ordinal: row.get(3)?,
        kind: row.get(4)?,
        target: row.get(5)?,
        expected: row.get(6)?,
        name: row.get(7)?,
        defined: row.get(8)?,
        project: row.get(9)?,
        criteria: row.get(10)?,
        start: row.get(11)?,
        end: row.get(12)?,
        due: row.get(13)?,
        class: row.get(14)?,
        pkind: row.get(15)?,
        pref: row.get(16)?,
        rkind: row.get(17)?,
        rid: row.get(18)?,
        rproject: row.get(19)?,
        rname: row.get(20)?,
        rdefined: row.get(21)?,
        rcriteria: row.get(22)?,
        rstart: row.get(23)?,
        rend: row.get(24)?,
        rdue: row.get(25)?,
        rclass: row.get(26)?,
        rpkind: row.get(27)?,
        rpref: row.get(28)?,
        version: row.get(29)?,
        created: row.get(30)?,
        updated: row.get(31)?,
    })
}

fn decode_delivery_capsule(
    row: DeliveryRawRow,
    tx: &Transaction<'_>,
) -> Result<DeliveryReplayCapsule, DeliveryPersistenceLoadError> {
    let err = || DeliveryPersistenceLoadError::InvalidDeliverySnapshot;
    let correlation = CorrelationId::parse(row.correlation).map_err(|_| err())?;
    let p = decode_persisted_delivery_provenance(&row.pkind, row.pref)?;
    let class = row
        .class
        .as_deref()
        .map(DataClassification::from_persisted)
        .transpose()
        .map_err(|_| err())?;
    let expected =
        || AggregateVersion::new(row.expected.ok_or_else(err)? as u64).map_err(|_| err());
    let command = match row.kind.as_str() {
        "create_initiative" => CommandIdentity::CreateInitiative {
            id: InitiativeId::parse(&row.target).map_err(|_| err())?,
            name: RecordName::parse(&row.name, &correlation).map_err(|_| err())?,
            defined_outcome: DefinedOutcome::parse(row.defined.ok_or_else(err)?, &correlation)
                .map_err(|_| err())?,
            classification: class,
            provenance: p,
        },
        "update_initiative" => CommandIdentity::UpdateInitiative {
            id: InitiativeId::parse(&row.target).map_err(|_| err())?,
            expected_version: expected()?,
            name: RecordName::parse(&row.name, &correlation).map_err(|_| err())?,
            defined_outcome: DefinedOutcome::parse(row.defined.ok_or_else(err)?, &correlation)
                .map_err(|_| err())?,
            classification: class,
            provenance: p,
        },
        "create_project" => CommandIdentity::CreateProject {
            id: ProjectId::parse(&row.target).map_err(|_| err())?,
            name: RecordName::parse(&row.name, &correlation).map_err(|_| err())?,
            start_at: UtcTimestamp::from_unix_millis(row.start.ok_or_else(err)?),
            end_at: UtcTimestamp::from_unix_millis(row.end.ok_or_else(err)?),
            classification: class,
            provenance: p,
        },
        "update_project" => CommandIdentity::UpdateProject {
            id: ProjectId::parse(&row.target).map_err(|_| err())?,
            expected_version: expected()?,
            name: RecordName::parse(&row.name, &correlation).map_err(|_| err())?,
            start_at: UtcTimestamp::from_unix_millis(row.start.ok_or_else(err)?),
            end_at: UtcTimestamp::from_unix_millis(row.end.ok_or_else(err)?),
            classification: class,
            provenance: p,
        },
        "create_milestone" => CommandIdentity::CreateMilestone {
            id: MilestoneId::parse(&row.target).map_err(|_| err())?,
            project_id: ProjectId::parse(row.project.ok_or_else(err)?).map_err(|_| err())?,
            name: RecordName::parse(&row.name, &correlation).map_err(|_| err())?,
            verification_criteria: VerificationCriteria::parse(
                row.criteria.ok_or_else(err)?,
                &correlation,
            )
            .map_err(|_| err())?,
            due_at: UtcTimestamp::from_unix_millis(row.due.ok_or_else(err)?),
            classification: class,
            provenance: p,
        },
        "update_milestone" => CommandIdentity::UpdateMilestone {
            id: MilestoneId::parse(&row.target).map_err(|_| err())?,
            expected_version: expected()?,
            name: RecordName::parse(&row.name, &correlation).map_err(|_| err())?,
            verification_criteria: VerificationCriteria::parse(
                row.criteria.ok_or_else(err)?,
                &correlation,
            )
            .map_err(|_| err())?,
            due_at: UtcTimestamp::from_unix_millis(row.due.ok_or_else(err)?),
            classification: class,
            provenance: p,
        },
        _ => return Err(err()),
    };
    let rp = decode_persisted_delivery_provenance(&row.rpkind, row.rpref)?;
    let version =
        AggregateVersion::new(u64::try_from(row.version).map_err(|_| err())?).map_err(|_| err())?;
    let created = UtcTimestamp::from_unix_millis(row.created);
    let updated = UtcTimestamp::from_unix_millis(row.updated);
    let correlation_for_result = correlation.clone();
    let result = match row.rkind.as_str() {
        "initiative" => DeliveryPersistenceResult::Initiative(
            Initiative::rehydrate(InitiativePersistenceRecord {
                id: InitiativeId::parse(row.rid).map_err(|_| err())?,
                name: RecordName::parse(row.rname, &correlation_for_result).map_err(|_| err())?,
                defined_outcome: DefinedOutcome::parse(
                    row.rdefined.ok_or_else(err)?,
                    &correlation_for_result,
                )
                .map_err(|_| err())?,
                classification: DataClassification::from_persisted(&row.rclass)
                    .map_err(|_| err())?,
                provenance: rp,
                version,
                created_at: created,
                updated_at: updated,
            })
            .map_err(|_| err())?,
        ),
        "project" => DeliveryPersistenceResult::Project(
            Project::rehydrate(ProjectPersistenceRecord {
                id: ProjectId::parse(row.rid).map_err(|_| err())?,
                name: RecordName::parse(row.rname, &correlation_for_result).map_err(|_| err())?,
                start_at: UtcTimestamp::from_unix_millis(row.rstart.ok_or_else(err)?),
                end_at: UtcTimestamp::from_unix_millis(row.rend.ok_or_else(err)?),
                classification: DataClassification::from_persisted(&row.rclass)
                    .map_err(|_| err())?,
                provenance: rp,
                version,
                created_at: created,
                updated_at: updated,
            })
            .map_err(|_| err())?,
        ),
        "milestone" => DeliveryPersistenceResult::Milestone(
            Milestone::rehydrate(MilestonePersistenceRecord {
                id: MilestoneId::parse(row.rid).map_err(|_| err())?,
                project_id: ProjectId::parse(row.rproject.ok_or_else(err)?).map_err(|_| err())?,
                name: RecordName::parse(row.rname, &correlation_for_result).map_err(|_| err())?,
                verification_criteria: VerificationCriteria::parse(
                    row.rcriteria.ok_or_else(err)?,
                    &correlation_for_result,
                )
                .map_err(|_| err())?,
                due_at: UtcTimestamp::from_unix_millis(row.rdue.ok_or_else(err)?),
                classification: DataClassification::from_persisted(&row.rclass)
                    .map_err(|_| err())?,
                provenance: rp,
                version,
                created_at: created,
                updated_at: updated,
            })
            .map_err(|_| err())?,
        ),
        _ => return Err(err()),
    };
    let audit_event_ids = tx
        .prepare("SELECT audit_event_id FROM delivery_idempotency_outcome_audits WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal")
        .map_err(|_| err())?
        .query_map(rusqlite::params![&row.operation, &row.idempotency], |r| r.get::<_, String>(0))
        .map_err(|_| err())?
        .map(|v| AuditEventId::parse(v.map_err(|_| err())?).map_err(|_| err()))
        .collect::<Result<Vec<_>, _>>()?;
    let derived_milestone_mutations = tx
        .prepare("SELECT milestone_id,previous_version,resulting_version,previous_classification,resulting_classification,previous_updated_at,resulting_updated_at,audit_event_id FROM delivery_derived_milestone_mutations WHERE namespace='delivery' AND operation=?1 AND idempotency_id=?2 ORDER BY ordinal")
        .map_err(|_| err())?
        .query_map(rusqlite::params![&row.operation, &row.idempotency], |r| {
            Ok((
                r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?, r.get::<_, String>(7)?,
            ))
        })
        .map_err(|_| err())?
        .map(|r| {
            let r = r.map_err(|_| err())?;
            Ok(DerivedMilestoneMutation::new(
                MilestoneId::parse(r.0).map_err(|_| err())?,
                AggregateVersion::new(u64::try_from(r.1).map_err(|_| err())?).map_err(|_| err())?,
                AggregateVersion::new(u64::try_from(r.2).map_err(|_| err())?).map_err(|_| err())?,
                DataClassification::from_persisted(&r.3).map_err(|_| err())?,
                DataClassification::from_persisted(&r.4).map_err(|_| err())?,
                UtcTimestamp::from_unix_millis(r.5),
                UtcTimestamp::from_unix_millis(r.6),
                AuditEventId::parse(r.7).map_err(|_| err())?,
            ))
        })
        .collect::<Result<Vec<_>, DeliveryPersistenceLoadError>>()?;
    Ok(DeliveryReplayCapsule::new(
        IdempotencyId::parse(row.idempotency).map_err(|_| err())?,
        command,
        result,
        correlation,
        audit_event_ids,
        u64::try_from(row.ordinal).map_err(|_| err())?,
        derived_milestone_mutations,
    ))
}

fn decode_all_delivery_audits(
    tx: &Transaction<'_>,
) -> Result<Vec<AuditEvent>, DeliveryPersistenceLoadError> {
    let err = || DeliveryPersistenceLoadError::InvalidDeliverySnapshot;
    let ids: Vec<String> = tx
        .prepare("SELECT e.id FROM (SELECT a.audit_event_id AS audit_event_id,o.operation_ordinal AS ordinal,a.ordinal AS sub_ordinal FROM delivery_idempotency_outcomes o JOIN delivery_idempotency_outcome_audits a USING(namespace,operation,idempotency_id) WHERE o.namespace='delivery' UNION ALL SELECT a.audit_event_id AS audit_event_id,r.operation_ordinal AS ordinal,a.ordinal AS sub_ordinal FROM delivery_h2a_execute_replay_operations r JOIN delivery_h2a_execute_replay_audits a USING(idempotency_id)) x JOIN audit_events e ON e.id=x.audit_event_id ORDER BY x.ordinal,x.sub_ordinal")
        .map_err(|_| err())?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| err())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| err())?;
    let mut audits = Vec::with_capacity(ids.len());
    for id in ids {
        let dummy_context = OperationContext {
            correlation_id: CorrelationId::parse("delivery-decode").map_err(|_| err())?,
            idempotency_id: IdempotencyId::parse("delivery-decode").map_err(|_| err())?,
        };
        audits.push(decode_delivery_audit(tx, &id, &dummy_context).map_err(|_| err())?);
    }
    Ok(audits)
}
