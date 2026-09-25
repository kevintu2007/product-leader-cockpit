//! Typed SQLite persistence seam for Portfolio, Product, Roadmap, and KPI
//! records. Every Portfolio H1 command is ordinary create/update:
//! no Prepared Intent, no approval receipt. The persistence contract mirrors
//! the domain layer's own `InMemoryPortfolioService` checks (existence,
//! expected-version CAS, classification-not-lowered) directly against
//! durable state, matching the established convention in
//! `issue_repository.rs::create_issue` rather than rehydrating a full
//! in-memory service for a single-record check.
//!
//! H2a Lower Data Classification adds the first H2a operation to this file --
//! `prepare_lower_portfolio_classification` /
//! `approve_and_execute_lower_portfolio_classification`. Prepare stays
//! hand-rolled (mirrors `risk_repository.rs`'s H2a prepare adapters: no
//! service reconstruction, just a direct re-check of the caller-supplied
//! `WorkManagementPreparedIntent` against durable state). Execute cannot:
//! the H2a approval-validation crypto (`validate_and_mint_work_management_...
//! _h2a_receipt`) is `pub(crate)` inside `pmc-domain`, reachable only through
//! `InMemoryPortfolioService::approve_and_execute_lower_portfolio_classification`,
//! which requires a live service instance holding the target record.
//!
//! `InMemoryPortfolioService::rehydrate` is deliberately *not* used to build
//! that instance: it requires a `PortfolioPersistenceSnapshot`, whose
//! `validate` replays and structurally verifies the complete durable H1
//! history of every Portfolio-family record across all five record types --
//! far more than this operation touches. Instead,
//! `seed_portfolio_service_for_lowering` builds a fresh, empty service and
//! drives it, through its own public H1 API, to reconstruct just the one
//! Portfolio record being lowered (one synthetic `create_portfolio` plus
//! `version - 1` synthetic `update_portfolio_details` calls, all using the
//! record's current field values). Also, `prepared_lowerings` is
//! deliberately excluded from `PortfolioPersistenceSnapshot` regardless (an
//! unapproved H2a preview must not survive a restart per DG0 6.7), so even a
//! full rehydrate would never carry the original preview in memory -- this
//! adapter re-runs `prepare_lower_portfolio_classification` once, on the
//! freshly seeded instance, with an id source and clock pinned to the exact
//! values the original prepare stored, to deterministically reproduce a
//! byte-identical `WorkManagementPreparedIntent` (same payload digest)
//! before immediately calling execute on that same service instance.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditDisposition, AuditEffectCode, AuditEffectScope, AuditEvent,
        AuditEventCode, AuditEventIdSource, AuditExecutionOutcome, AuditModule, AuditPolicyOutcome,
        AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, IdempotencyId, KpiId, KpiObservationId,
        PortfolioId, PreparedIntentId, ProductId, RoadmapId,
    },
    portfolio::{
        ApproveAndExecuteLowerKpiClassification,
        ApproveAndExecuteLowerKpiObservationClassification,
        ApproveAndExecuteLowerPortfolioClassification, ApproveAndExecuteLowerProductClassification,
        ApproveAndExecuteLowerRoadmapClassification, CreateKpiDefinition, CreateKpiObservation,
        CreatePortfolio, CreateProduct, CreateRoadmap, InMemoryPortfolioService,
        KpiDefinitionRecord, KpiObservationRecord, MutationOutcome, OperationContext,
        PortfolioClassificationLoweringIdSource, PortfolioRecord, PrepareLowerKpiClassification,
        PrepareLowerKpiObservationClassification, PrepareLowerPortfolioClassification,
        PrepareLowerProductClassification, PrepareLowerRoadmapClassification, ProductRecord,
        RoadmapRecord, UpdateKpiDefinitionDetails, UpdateKpiObservationDetails,
        UpdatePortfolioDetails, UpdateProductDetails, UpdateRoadmapDetails,
    },
    provenance::{Provenance, ProvenanceReference},
    time::{Clock, UtcTimestamp},
    work_management::{
        ApprovalAuthorizationPort, WorkManagementOperation, WorkManagementPreparedIntent,
    },
    DomainValueError,
};
use rusqlite::{OptionalExtension, Transaction};

use super::{LedgerTransactionError, SqliteProductLedger};

/// The single constant effect code every Portfolio-family mutation uses,
/// matching `portfolio.rs::successful_disposition`'s own constant -- unlike
/// Issue/Risk, this family does not vary its effect code per operation.
const PORTFOLIO_EFFECT_CODE: &str = "portfolio-record-mutated";

/// A clock pinned to the exact stored prepare-time timestamp until
/// `enter_execute_phase` is called, after which it returns the real
/// execute-time timestamp -- see the module doc comment on why the H2a
/// execute adapter needs to reproduce a byte-identical prepare before
/// calling execute on the same throwaway service instance. Cheap to clone
/// (an `Rc<Cell<_>>`); every clone shares the same phase flag, so flipping
/// it on the handle kept outside the service also flips it for the clone
/// moved into the service.
#[derive(Clone)]
struct PortfolioReplayThenNowClock {
    prepare_at: UtcTimestamp,
    execute_at: UtcTimestamp,
    executing: std::rc::Rc<std::cell::Cell<bool>>,
}

impl PortfolioReplayThenNowClock {
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

impl Clock for PortfolioReplayThenNowClock {
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
struct PortfolioReplayIds {
    prepared_intent_id: Option<PreparedIntentId>,
    approval_receipt_id: Option<ApprovalReceiptId>,
}

impl PortfolioReplayIds {
    fn new(prepared_intent_id: PreparedIntentId, approval_receipt_id: ApprovalReceiptId) -> Self {
        Self {
            prepared_intent_id: Some(prepared_intent_id),
            approval_receipt_id: Some(approval_receipt_id),
        }
    }
}

impl PortfolioClassificationLoweringIdSource for PortfolioReplayIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        self.prepared_intent_id
            .take()
            .ok_or_else(missing_portfolio_h2a_id)
    }

    fn next_approval_receipt_id(&mut self) -> Result<ApprovalReceiptId, DomainValueError> {
        self.approval_receipt_id
            .take()
            .ok_or_else(missing_portfolio_h2a_id)
    }
}

fn missing_portfolio_h2a_id() -> DomainValueError {
    match PortfolioId::parse("") {
        Err(error) => error,
        Ok(_) => unreachable!("empty opaque identifiers must be rejected"),
    }
}

/// Hands out a pre-built queue of audit ids in order: one synthetic id per
/// seeding call in `seed_portfolio_service_for_lowering` (see that
/// function's doc comment), then finally the one real, caller-supplied
/// audit id for the actual execute commit.
struct QueuedAuditIds(std::collections::VecDeque<AuditEventId>);

impl AuditEventIdSource for QueuedAuditIds {
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, DomainValueError> {
        self.0.pop_front().ok_or_else(missing_portfolio_h2a_id)
    }
}

/// The only actor ever allowed to approve a persisted Portfolio H2a
/// operation, matching `decision_repository.rs::AllowPersistedDecisionApproval`.
#[derive(Clone, Copy)]
struct AllowPersistedPortfolioApproval;

impl ApprovalAuthorizationPort for AllowPersistedPortfolioApproval {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

impl SqliteProductLedger {
    pub fn create_portfolio(
        &mut self,
        command: CreatePortfolio,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<PortfolioRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_name,command_details,command_classification,command_provenance_kind,command_provenance_reference FROM portfolio_command_results WHERE namespace='portfolio' AND operation='create_portfolio' AND idempotency_id=?1",
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
                    && existing.2 == command.details.as_str()
                    && existing.3.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.4 == command.provenance.kind_persisted()
                    && existing.5.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_portfolio_outcome(tx, "create_portfolio", &context);
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
                return Err(domain_conflict(&context, "portfolio.already_exists"));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'portfolio',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO portfolios(id,name,details,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.details.as_str(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Portfolio(command.id.clone()),
                "portfolio.created",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "portfolio", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "create_portfolio", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_name,command_details,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','create_portfolio',?1,'create_portfolio',?2,?3,?4,?5,?6,?7,'portfolio',?2,?3,?4,?8,?6,?7,1,?9,?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.details.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            Ok(MutationOutcome {
                record: PortfolioRecord {
                    id: command.id,
                    name: command.name,
                    details: command.details,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                },
                audit_event: audit,
            })
        })
    }

    pub fn update_portfolio_details(
        &mut self,
        command: UpdatePortfolioDetails,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<PortfolioRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_details,command_classification FROM portfolio_command_results WHERE namespace='portfolio' AND operation='update_portfolio' AND idempotency_id=?1",
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
                    && existing.3 == command.details.as_str()
                    && existing.4.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_portfolio_outcome(tx, "update_portfolio", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM portfolios JOIN aggregate_registry registry ON registry.id=portfolios.id AND registry.aggregate_type='portfolio' WHERE portfolios.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "portfolio.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "portfolio.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = match command.classification {
                Some(requested) => {
                    if requested.combine(current_classification) != requested {
                        return Err(policy_denied(&context, "classification.lowering_requires_governed_intent"));
                    }
                    requested
                }
                None => current_classification,
            };
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "portfolio.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE portfolios SET name=?1,details=?2 WHERE id=?3",
                rusqlite::params![command.name.as_str(), command.details.as_str(), command.id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='portfolio'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Portfolio(command.id.clone()),
                "portfolio.updated",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "portfolio", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "update_portfolio", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_details,command_classification,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','update_portfolio',?1,'update_portfolio',?2,?3,?4,?5,?6,'portfolio',?2,?4,?5,?7,(SELECT provenance_kind FROM portfolios WHERE id=?2),(SELECT provenance_reference FROM portfolios WHERE id=?2),?8,(SELECT created_at FROM aggregate_registry WHERE id=?2),?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.details.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            let provenance = decode_portfolio_provenance(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record: PortfolioRecord {
                    id: command.id,
                    name: command.name,
                    details: command.details,
                    classification,
                    provenance,
                    version: next_version,
                },
                audit_event: audit,
            })
        })
    }

    pub fn create_product(
        &mut self,
        command: CreateProduct,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<ProductRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_name,command_details,command_classification,command_provenance_kind,command_provenance_reference FROM portfolio_command_results WHERE namespace='portfolio' AND operation='create_product' AND idempotency_id=?1",
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
                    && existing.2 == command.details.as_str()
                    && existing.3.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.4 == command.provenance.kind_persisted()
                    && existing.5.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_product_outcome(tx, "create_product", &context);
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
                return Err(domain_conflict(&context, "product.already_exists"));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'product',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO products(id,name,details,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.details.as_str(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Product(command.id.clone()),
                "product.created",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "product", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "create_product", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_name,command_details,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','create_product',?1,'create_product',?2,?3,?4,?5,?6,?7,'product',?2,?3,?4,?8,?6,?7,1,?9,?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.details.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            Ok(MutationOutcome {
                record: ProductRecord {
                    id: command.id,
                    name: command.name,
                    details: command.details,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                },
                audit_event: audit,
            })
        })
    }

    pub fn update_product_details(
        &mut self,
        command: UpdateProductDetails,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<ProductRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_details,command_classification FROM portfolio_command_results WHERE namespace='portfolio' AND operation='update_product' AND idempotency_id=?1",
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
                    && existing.3 == command.details.as_str()
                    && existing.4.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_product_outcome(tx, "update_product", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM products JOIN aggregate_registry registry ON registry.id=products.id AND registry.aggregate_type='product' WHERE products.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "product.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "product.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = match command.classification {
                Some(requested) => {
                    if requested.combine(current_classification) != requested {
                        return Err(policy_denied(&context, "classification.lowering_requires_governed_intent"));
                    }
                    requested
                }
                None => current_classification,
            };
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "product.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE products SET name=?1,details=?2 WHERE id=?3",
                rusqlite::params![command.name.as_str(), command.details.as_str(), command.id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='product'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Product(command.id.clone()),
                "product.updated",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "product", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "update_product", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_details,command_classification,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','update_product',?1,'update_product',?2,?3,?4,?5,?6,'product',?2,?4,?5,?7,(SELECT provenance_kind FROM products WHERE id=?2),(SELECT provenance_reference FROM products WHERE id=?2),?8,(SELECT created_at FROM aggregate_registry WHERE id=?2),?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.details.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            let provenance = decode_product_provenance(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record: ProductRecord {
                    id: command.id,
                    name: command.name,
                    details: command.details,
                    classification,
                    provenance,
                    version: next_version,
                },
                audit_event: audit,
            })
        })
    }

    pub fn create_roadmap(
        &mut self,
        command: CreateRoadmap,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RoadmapRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_name,command_details,command_classification,command_provenance_kind,command_provenance_reference FROM portfolio_command_results WHERE namespace='portfolio' AND operation='create_roadmap' AND idempotency_id=?1",
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
                    && existing.2 == command.details.as_str()
                    && existing.3.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.4 == command.provenance.kind_persisted()
                    && existing.5.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_roadmap_outcome(tx, "create_roadmap", &context);
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
                return Err(domain_conflict(&context, "roadmap.already_exists"));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'roadmap',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO roadmaps(id,name,details,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.details.as_str(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Roadmap(command.id.clone()),
                "roadmap.created",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "roadmap", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "create_roadmap", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_name,command_details,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','create_roadmap',?1,'create_roadmap',?2,?3,?4,?5,?6,?7,'roadmap',?2,?3,?4,?8,?6,?7,1,?9,?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.details.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            Ok(MutationOutcome {
                record: RoadmapRecord {
                    id: command.id,
                    name: command.name,
                    details: command.details,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                },
                audit_event: audit,
            })
        })
    }

    pub fn update_roadmap_details(
        &mut self,
        command: UpdateRoadmapDetails,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RoadmapRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_details,command_classification FROM portfolio_command_results WHERE namespace='portfolio' AND operation='update_roadmap' AND idempotency_id=?1",
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
                    && existing.3 == command.details.as_str()
                    && existing.4.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_roadmap_outcome(tx, "update_roadmap", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM roadmaps JOIN aggregate_registry registry ON registry.id=roadmaps.id AND registry.aggregate_type='roadmap' WHERE roadmaps.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "roadmap.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "roadmap.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = match command.classification {
                Some(requested) => {
                    if requested.combine(current_classification) != requested {
                        return Err(policy_denied(&context, "classification.lowering_requires_governed_intent"));
                    }
                    requested
                }
                None => current_classification,
            };
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "roadmap.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE roadmaps SET name=?1,details=?2 WHERE id=?3",
                rusqlite::params![command.name.as_str(), command.details.as_str(), command.id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='roadmap'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Roadmap(command.id.clone()),
                "roadmap.updated",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "roadmap", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "update_roadmap", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_details,command_classification,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','update_roadmap',?1,'update_roadmap',?2,?3,?4,?5,?6,'roadmap',?2,?4,?5,?7,(SELECT provenance_kind FROM roadmaps WHERE id=?2),(SELECT provenance_reference FROM roadmaps WHERE id=?2),?8,(SELECT created_at FROM aggregate_registry WHERE id=?2),?9)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.details.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            let provenance = decode_roadmap_provenance(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record: RoadmapRecord {
                    id: command.id,
                    name: command.name,
                    details: command.details,
                    classification,
                    provenance,
                    version: next_version,
                },
                audit_event: audit,
            })
        })
    }

    pub fn create_kpi_definition(
        &mut self,
        command: CreateKpiDefinition,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_name,command_definition,command_owner,command_target,command_cadence,command_source,command_classification,command_provenance_kind,command_provenance_reference FROM portfolio_command_results WHERE namespace='portfolio' AND operation='create_kpi_definition' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, String>(8)?,
                            row.get::<_, Option<String>>(9)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == command.name.as_str()
                    && existing.2 == command.definition.as_str()
                    && existing.3 == command.owner.as_str()
                    && existing.4 == command.target.as_str()
                    && existing.5 == command.cadence.as_str()
                    && existing.6 == command.source.as_str()
                    && existing.7.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.8 == command.provenance.kind_persisted()
                    && existing.9.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_kpi_definition_outcome(tx, "create_kpi_definition", &context);
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
                return Err(domain_conflict(&context, "kpi.already_exists"));
            }
            let classification = command.classification.unwrap_or_default();
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'kpi_definition',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO kpi_definitions(id,name,definition,owner,target,cadence,source,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                rusqlite::params![
                    command.id.as_str(),
                    command.name.as_str(),
                    command.definition.as_str(),
                    command.owner.as_str(),
                    command.target.as_str(),
                    command.cadence.as_str(),
                    command.source.as_str(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Kpi(command.id.clone()),
                "kpi.definition.created",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "kpi_definition", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "create_kpi_definition", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_name,command_definition,command_owner,command_target,command_cadence,command_source,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_name,result_definition,result_owner,result_target,result_cadence,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','create_kpi_definition',?1,'create_kpi_definition',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'kpi_definition',?2,?3,?4,?5,?6,?7,?8,?12,?10,?11,1,?13,?13)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.name.as_str(),
                    command.definition.as_str(),
                    command.owner.as_str(),
                    command.target.as_str(),
                    command.cadence.as_str(),
                    command.source.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            Ok(MutationOutcome {
                record: KpiDefinitionRecord {
                    id: command.id,
                    name: command.name,
                    definition: command.definition,
                    owner: command.owner,
                    target: command.target,
                    cadence: command.cadence,
                    source: command.source,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                },
                audit_event: audit,
            })
        })
    }

    pub fn create_kpi_observation(
        &mut self,
        command: CreateKpiObservation,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiObservationRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_kpi_id,command_value,command_observed_at,command_source,command_classification,command_provenance_kind,command_provenance_reference FROM portfolio_command_results WHERE namespace='portfolio' AND operation='create_kpi_observation' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
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
                    && existing.1 == command.kpi_id.as_str()
                    && existing.2 == command.value.as_str()
                    && existing.3 == command.observed_at.unix_millis()
                    && existing.4 == command.source.as_str()
                    && existing.5.as_deref() == command.classification.map(DataClassification::as_persisted)
                    && existing.6 == command.provenance.kind_persisted()
                    && existing.7.as_deref() == command.provenance.reference().map(ProvenanceReference::as_str);
                if matches {
                    return decode_kpi_observation_outcome(tx, "create_kpi_observation", &context);
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
                return Err(domain_conflict(&context, "kpi.observation.already_exists"));
            }
            let definition_classification: String = tx
                .query_row(
                    "SELECT classification FROM aggregate_registry WHERE id=?1 AND aggregate_type='kpi_definition'",
                    [command.kpi_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "kpi.not_found"))?;
            let definition_classification = DataClassification::from_persisted(&definition_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = match command.classification {
                Some(requested) => requested.combine(definition_classification),
                None => definition_classification,
            };
            let occurred_millis = occurred_at.unix_millis();
            let observed_at_millis = command.observed_at.unix_millis();
            tx.execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'kpi_observation',1,?2,?3,?3)",
                rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO kpi_observations(id,kpi_id,value,observed_at,source,provenance_kind,provenance_reference) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    command.id.as_str(),
                    command.kpi_id.as_str(),
                    command.value.as_str(),
                    observed_at_millis,
                    command.source.as_str(),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::KpiObservation(command.id.clone()),
                "kpi.observation.created",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "kpi_observation", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "create_kpi_observation", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_kpi_id,command_value,command_observed_at,command_source,command_classification,command_provenance_kind,command_provenance_reference,result_kind,result_id,result_kpi_id,result_value,result_observed_at,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','create_kpi_observation',?1,'create_kpi_observation',?2,?3,?4,?5,?6,?7,?8,?9,'kpi_observation',?2,?3,?4,?5,?6,?10,?8,?9,1,?11,?11)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    command.kpi_id.as_str(),
                    command.value.as_str(),
                    observed_at_millis,
                    command.source.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    command.provenance.kind_persisted(),
                    command.provenance.reference().map(ProvenanceReference::as_str),
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            Ok(MutationOutcome {
                record: KpiObservationRecord {
                    id: command.id,
                    kpi_id: command.kpi_id,
                    value: command.value,
                    observed_at: command.observed_at,
                    source: command.source,
                    classification,
                    provenance: command.provenance,
                    version: AggregateVersion::initial(),
                    created_at: occurred_at,
                    updated_at: occurred_at,
                },
                audit_event: audit,
            })
        })
    }

    pub fn update_kpi_observation_details(
        &mut self,
        command: UpdateKpiObservationDetails,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiObservationRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_value,command_observed_at,command_source,command_classification FROM portfolio_command_results WHERE namespace='portfolio' AND operation='update_kpi_observation' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.value.as_str()
                    && existing.3 == command.observed_at.unix_millis()
                    && existing.4 == command.source.as_str()
                    && existing.5.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_kpi_observation_outcome(tx, "update_kpi_observation", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (kpi_id, current_classification, current_version, current_updated_at): (String, String, i64, i64) = tx
                .query_row(
                    "SELECT observations.kpi_id,registry.classification,registry.version,registry.updated_at FROM kpi_observations observations JOIN aggregate_registry registry ON registry.id=observations.id AND registry.aggregate_type='kpi_observation' WHERE observations.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "kpi.observation.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "kpi.observation.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let base_classification = match command.classification {
                Some(requested) => {
                    if requested.combine(current_classification) != requested {
                        return Err(policy_denied(&context, "classification.lowering_requires_governed_intent"));
                    }
                    requested
                }
                None => current_classification,
            };
            let definition_classification: String = tx
                .query_row(
                    "SELECT classification FROM aggregate_registry WHERE id=?1 AND aggregate_type='kpi_definition'",
                    [kpi_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "kpi.not_found"))?;
            let definition_classification = DataClassification::from_persisted(&definition_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = base_classification.combine(definition_classification);
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "kpi.observation.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            let observed_at_millis = command.observed_at.unix_millis();
            let updated_at_millis = std::cmp::max(occurred_millis, current_updated_at);
            tx.execute(
                "UPDATE kpi_observations SET value=?1,observed_at=?2,source=?3 WHERE id=?4",
                rusqlite::params![command.value.as_str(), observed_at_millis, command.source.as_str(), command.id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='kpi_observation'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    updated_at_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::KpiObservation(command.id.clone()),
                "kpi.observation.updated",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "kpi_observation", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "update_kpi_observation", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_value,command_observed_at,command_source,command_classification,result_kind,result_id,result_kpi_id,result_value,result_observed_at,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','update_kpi_observation',?1,'update_kpi_observation',?2,?3,?4,?5,?6,?7,'kpi_observation',?2,?8,?4,?5,?6,?9,(SELECT provenance_kind FROM kpi_observations WHERE id=?2),(SELECT provenance_reference FROM kpi_observations WHERE id=?2),?10,(SELECT created_at FROM aggregate_registry WHERE id=?2),?11)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.value.as_str(),
                    observed_at_millis,
                    command.source.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    kpi_id.as_str(),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    updated_at_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;
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
            let provenance = decode_kpi_observation_provenance(tx, &command.id, &context)?;
            let created_at: i64 = tx
                .query_row(
                    "SELECT created_at FROM aggregate_registry WHERE id=?1",
                    [command.id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            Ok(MutationOutcome {
                record: KpiObservationRecord {
                    id: command.id,
                    kpi_id: KpiId::parse(kpi_id).map_err(|_| storage_error(&context))?,
                    value: command.value,
                    observed_at: command.observed_at,
                    source: command.source,
                    classification,
                    provenance,
                    version: next_version,
                    created_at: UtcTimestamp::from_unix_millis(created_at),
                    updated_at: UtcTimestamp::from_unix_millis(updated_at_millis),
                },
                audit_event: audit,
            })
        })
    }

    /// `next_audit_event_id` is called once for the Definition's own audit
    /// and once more per Observation whose classification is raised by the
    /// cascade below -- the count is only known once the affected rows are
    /// found, so (unlike every other method in this file) a single external
    /// `AuditEventId` parameter cannot express it.
    pub fn update_kpi_definition_details(
        &mut self,
        command: UpdateKpiDefinitionDetails,
        mut next_audit_event_id: impl FnMut() -> AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_target_id,command_expected_version,command_name,command_definition,command_owner,command_target,command_cadence,command_source,command_classification FROM portfolio_command_results WHERE namespace='portfolio' AND operation='update_kpi_definition' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, Option<String>>(8)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.name.as_str()
                    && existing.3 == command.definition.as_str()
                    && existing.4 == command.owner.as_str()
                    && existing.5 == command.target.as_str()
                    && existing.6 == command.cadence.as_str()
                    && existing.7 == command.source.as_str()
                    && existing.8.as_deref() == command.classification.map(DataClassification::as_persisted);
                if matches {
                    return decode_kpi_definition_outcome(tx, "update_kpi_definition", &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM kpi_definitions definitions JOIN aggregate_registry registry ON registry.id=definitions.id AND registry.aggregate_type='kpi_definition' WHERE definitions.id=?1",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "kpi.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(&context, "kpi.stale_version", current_version));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            let classification = match command.classification {
                Some(requested) => {
                    if requested.combine(current_classification) != requested {
                        return Err(policy_denied(&context, "classification.lowering_requires_governed_intent"));
                    }
                    requested
                }
                None => current_classification,
            };
            let next_version = command
                .expected_version
                .next()
                .ok_or_else(|| domain_conflict(&context, "kpi.version_exhausted"))?;
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "UPDATE kpi_definitions SET name=?1,definition=?2,owner=?3,target=?4,cadence=?5,source=?6 WHERE id=?7",
                rusqlite::params![
                    command.name.as_str(),
                    command.definition.as_str(),
                    command.owner.as_str(),
                    command.target.as_str(),
                    command.cadence.as_str(),
                    command.source.as_str(),
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='kpi_definition'",
                rusqlite::params![
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    classification.as_persisted(),
                    occurred_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                next_audit_event_id(),
                occurred_at,
                AuditTarget::Kpi(command.id.clone()),
                "kpi.definition.updated",
                &context,
            )?;
            persist_portfolio_audit(tx, &audit, "kpi_definition", command.id.as_str(), &context)?;
            let ordinal = next_portfolio_operation_ordinal(tx, &context)?;
            persist_idempotency_claim(tx, "update_kpi_definition", &context, ordinal, command.id.as_str(), occurred_millis)?;
            tx.execute(
                "INSERT INTO portfolio_command_results (namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_definition,command_owner,command_target,command_cadence,command_source,command_classification,result_kind,result_id,result_name,result_definition,result_owner,result_target,result_cadence,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES ('portfolio','update_kpi_definition',?1,'update_kpi_definition',?2,?3,?4,?5,?6,?7,?8,?9,?10,'kpi_definition',?2,?4,?5,?6,?7,?8,?9,?11,(SELECT provenance_kind FROM kpi_definitions WHERE id=?2),(SELECT provenance_reference FROM kpi_definitions WHERE id=?2),?12,(SELECT created_at FROM aggregate_registry WHERE id=?2),?13)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.name.as_str(),
                    command.definition.as_str(),
                    command.owner.as_str(),
                    command.target.as_str(),
                    command.cadence.as_str(),
                    command.source.as_str(),
                    command.classification.map(DataClassification::as_persisted),
                    classification.as_persisted(),
                    i64::try_from(next_version.get()).map_err(|_| storage_error(&context))?,
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_idempotency_outcome_audit(tx, &context, 0, audit.id().as_str())?;

            let affected: Vec<(String, String, i64, i64)> = {
                let mut statement = tx
                    .prepare(
                        "SELECT observations.id,registry.classification,registry.version,registry.updated_at FROM kpi_observations observations JOIN aggregate_registry registry ON registry.id=observations.id AND registry.aggregate_type='kpi_observation' WHERE observations.kpi_id=?1 ORDER BY observations.id",
                    )
                    .map_err(|_| storage_error(&context))?;
                let rows = statement
                    .query_map([command.id.as_str()], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })
                    .map_err(|_| storage_error(&context))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| storage_error(&context))?;
                rows
            };
            let mut derived_ordinal: i64 = 0;
            for (observation_id, observation_classification, observation_version, observation_updated_at) in affected {
                let observation_classification = DataClassification::from_persisted(&observation_classification)
                    .map_err(|_| storage_error(&context))?;
                let resulting_classification = classification.combine(observation_classification);
                if resulting_classification == observation_classification {
                    continue;
                }
                let resulting_version = observation_version + 1;
                let resulting_updated_at = std::cmp::max(occurred_millis, observation_updated_at);
                let derived_audit = build_audit(
                    next_audit_event_id(),
                    occurred_at,
                    AuditTarget::KpiObservation(
                        KpiObservationId::parse(observation_id.as_str())
                            .map_err(|_| storage_error(&context))?,
                    ),
                    "kpi.observation.classification.inherited",
                    &context,
                )?;
                persist_portfolio_audit(tx, &derived_audit, "kpi_observation", observation_id.as_str(), &context)?;
                persist_idempotency_outcome_audit(tx, &context, derived_ordinal + 1, derived_audit.id().as_str())?;
                tx.execute(
                    "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='kpi_observation'",
                    rusqlite::params![
                        resulting_version,
                        resulting_classification.as_persisted(),
                        resulting_updated_at,
                        observation_id.as_str(),
                    ],
                )
                .map_err(|_| storage_error(&context))?;
                tx.execute(
                    "INSERT INTO portfolio_derived_kpi_observation_mutations(namespace,operation,idempotency_id,ordinal,observation_id,previous_version,resulting_version,previous_classification,resulting_classification,previous_updated_at,resulting_updated_at,audit_event_id) VALUES('portfolio','update_kpi_definition',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    rusqlite::params![
                        context.idempotency_id.as_str(),
                        derived_ordinal,
                        observation_id.as_str(),
                        observation_version,
                        resulting_version,
                        observation_classification.as_persisted(),
                        resulting_classification.as_persisted(),
                        observation_updated_at,
                        resulting_updated_at,
                        derived_audit.id().as_str(),
                    ],
                )
                .map_err(|_| storage_error(&context))?;
                derived_ordinal += 1;
            }

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
            let provenance = decode_kpi_definition_provenance(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record: KpiDefinitionRecord {
                    id: command.id,
                    name: command.name,
                    definition: command.definition,
                    owner: command.owner,
                    target: command.target,
                    cadence: command.cadence,
                    source: command.source,
                    classification,
                    provenance,
                    version: next_version,
                },
                audit_event: audit,
            })
        })
    }

    /// H2a step 1: persist one already-canonical Portfolio
    /// classification-lowering preview. The domain service is the only
    /// authority that may create `prepared`; this adapter re-verifies its
    /// exact topology against durable state before storing it -- mirrors
    /// `risk_repository.rs::prepare_close_risk`, without reconstructing
    /// `InMemoryPortfolioService` (see the module doc comment).
    pub fn prepare_lower_portfolio_classification(
        &mut self,
        command: PrepareLowerPortfolioClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT portfolio_id,expected_version,proposed_classification,rationale,result_reference FROM portfolio_h2a_prepare_replay_operations replay JOIN portfolio_h2a_command_prepare_lower_classifications command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.portfolio_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM portfolios JOIN aggregate_registry registry ON registry.id=portfolios.id AND registry.aggregate_type='portfolio' WHERE portfolios.id=?1",
                    [command.portfolio_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "portfolio.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(
                    &context,
                    "portfolio.stale_version",
                    current_version,
                ));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command
                .proposed_classification
                .combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "portfolio.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerPortfolioClassification {
                portfolio_id,
                portfolio_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(
                    &context,
                    "portfolio.classification_lowering_invalid",
                ));
            };
            if portfolio_id != &command.portfolio_id
                || portfolio_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(
                    &context,
                    "portfolio.classification_lowering_invalid",
                ));
            }
            let ordinal = next_portfolio_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "lower_portfolio_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'portfolio',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.portfolio_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO portfolio_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO portfolio_h2a_command_prepare_lower_classifications (idempotency_id,portfolio_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.portfolio_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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

    /// H2a step 2: approve and atomically apply a previously
    /// prepared Portfolio classification lowering. See the module doc
    /// comment for why this rehydrates a throwaway `InMemoryPortfolioService`
    /// and re-runs prepare once (deterministically, from stored fields)
    /// before calling execute -- unlike every other adapter in this file,
    /// which never touches the domain service.
    pub fn approve_and_execute_lower_portfolio_classification(
        &mut self,
        command: ApproveAndExecuteLowerPortfolioClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<PortfolioRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM portfolio_h2a_command_execute_lower_classifications WHERE idempotency_id=?1",
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
                return replay_lowered_outcome(tx, &context);
            }
            let (
                stored_portfolio_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.portfolio_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM portfolio_h2a_prepare_replay_operations replay JOIN portfolio_h2a_command_prepare_lower_classifications command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1",
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
                    domain_conflict(&context, "portfolio.classification_lowering_preview_changed")
                })?;
            let portfolio_id =
                PortfolioId::parse(stored_portfolio_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = pmc_domain::work_management::WorkManagementRationale::parse(
                stored_rationale,
            )
            .map_err(|_| storage_error(&context))?;

            let clock = PortfolioReplayThenNowClock::new(
                UtcTimestamp::from_unix_millis(stored_created_at),
                occurred_at,
            );
            let mut ids = PortfolioReplayIds::new(
                command.approval.prepared_id().clone(),
                approval_receipt_id.clone(),
            );
            let mut service = seed_portfolio_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &portfolio_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_portfolio_classification(
                    PrepareLowerPortfolioClassification {
                        portfolio_id: portfolio_id.clone(),
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
                    domain_conflict(&context, "portfolio.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let outcome = service.approve_and_execute_lower_portfolio_classification(
                ApproveAndExecuteLowerPortfolioClassification {
                    approval: command.approval.clone(),
                    context: context.clone(),
                },
                &mut ids,
                &AllowPersistedPortfolioApproval,
            )?;
            let ordinal = next_portfolio_h2a_execute_operation_ordinal(tx, &context)?;
            persist_lowered_bundle(
                tx,
                &command.approval,
                &outcome,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(outcome)
        })
    }

    /// H2a step 1: persist one already-canonical Product
    /// classification-lowering preview. See
    /// `prepare_lower_portfolio_classification` for the shared rationale.
    pub fn prepare_lower_product_classification(
        &mut self,
        command: PrepareLowerProductClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT product_id,expected_version,proposed_classification,rationale,result_reference FROM product_h2a_prepare_replay_operations replay JOIN product_h2a_command_prepare_lower_classifications command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.product_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM products JOIN aggregate_registry registry ON registry.id=products.id AND registry.aggregate_type='product' WHERE products.id=?1",
                    [command.product_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "product.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(
                    &context,
                    "product.stale_version",
                    current_version,
                ));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command
                .proposed_classification
                .combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "product.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerProductClassification {
                product_id,
                product_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(
                    &context,
                    "product.classification_lowering_invalid",
                ));
            };
            if product_id != &command.product_id
                || product_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(
                    &context,
                    "product.classification_lowering_invalid",
                ));
            }
            let ordinal = next_product_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "lower_product_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'product',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.product_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO product_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO product_h2a_command_prepare_lower_classifications (idempotency_id,product_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.product_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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

    /// H2a step 2 for Product. See
    /// `approve_and_execute_lower_portfolio_classification` for the shared
    /// rationale.
    pub fn approve_and_execute_lower_product_classification(
        &mut self,
        command: ApproveAndExecuteLowerProductClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<ProductRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM product_h2a_command_execute_lower_classifications WHERE idempotency_id=?1",
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
                return replay_product_lowered_outcome(tx, &context);
            }
            let (
                stored_product_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.product_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM product_h2a_prepare_replay_operations replay JOIN product_h2a_command_prepare_lower_classifications command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1",
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
                    domain_conflict(&context, "product.classification_lowering_preview_changed")
                })?;
            let product_id =
                ProductId::parse(stored_product_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = pmc_domain::work_management::WorkManagementRationale::parse(
                stored_rationale,
            )
            .map_err(|_| storage_error(&context))?;

            let clock = PortfolioReplayThenNowClock::new(
                UtcTimestamp::from_unix_millis(stored_created_at),
                occurred_at,
            );
            let mut ids = PortfolioReplayIds::new(
                command.approval.prepared_id().clone(),
                approval_receipt_id.clone(),
            );
            let mut service = seed_product_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &product_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_product_classification(
                    PrepareLowerProductClassification {
                        product_id: product_id.clone(),
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
                    domain_conflict(&context, "product.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let outcome = service.approve_and_execute_lower_product_classification(
                ApproveAndExecuteLowerProductClassification {
                    approval: command.approval.clone(),
                    context: context.clone(),
                },
                &mut ids,
                &AllowPersistedPortfolioApproval,
            )?;
            let ordinal = next_product_h2a_execute_operation_ordinal(tx, &context)?;
            persist_product_lowered_bundle(
                tx,
                &command.approval,
                &outcome,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(outcome)
        })
    }

    /// H2a step 1: persist one already-canonical Roadmap
    /// classification-lowering preview. See
    /// `prepare_lower_portfolio_classification` for the shared rationale.
    pub fn prepare_lower_roadmap_classification(
        &mut self,
        command: PrepareLowerRoadmapClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT roadmap_id,expected_version,proposed_classification,rationale,result_reference FROM roadmap_h2a_prepare_replay_operations replay JOIN roadmap_h2a_command_prepare_lower_classifications command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.roadmap_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM roadmaps JOIN aggregate_registry registry ON registry.id=roadmaps.id AND registry.aggregate_type='roadmap' WHERE roadmaps.id=?1",
                    [command.roadmap_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "roadmap.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(
                    &context,
                    "roadmap.stale_version",
                    current_version,
                ));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command
                .proposed_classification
                .combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "roadmap.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerRoadmapClassification {
                roadmap_id,
                roadmap_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(
                    &context,
                    "roadmap.classification_lowering_invalid",
                ));
            };
            if roadmap_id != &command.roadmap_id
                || roadmap_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(
                    &context,
                    "roadmap.classification_lowering_invalid",
                ));
            }
            let ordinal = next_roadmap_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "lower_roadmap_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'roadmap',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.roadmap_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO roadmap_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO roadmap_h2a_command_prepare_lower_classifications (idempotency_id,roadmap_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.roadmap_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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

    /// H2a step 2 for Roadmap. See
    /// `approve_and_execute_lower_portfolio_classification` for the shared
    /// rationale.
    pub fn approve_and_execute_lower_roadmap_classification(
        &mut self,
        command: ApproveAndExecuteLowerRoadmapClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<RoadmapRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM roadmap_h2a_command_execute_lower_classifications WHERE idempotency_id=?1",
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
                return replay_roadmap_lowered_outcome(tx, &context);
            }
            let (
                stored_roadmap_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.roadmap_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM roadmap_h2a_prepare_replay_operations replay JOIN roadmap_h2a_command_prepare_lower_classifications command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1",
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
                    domain_conflict(&context, "roadmap.classification_lowering_preview_changed")
                })?;
            let roadmap_id =
                RoadmapId::parse(stored_roadmap_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = pmc_domain::work_management::WorkManagementRationale::parse(
                stored_rationale,
            )
            .map_err(|_| storage_error(&context))?;

            let clock = PortfolioReplayThenNowClock::new(
                UtcTimestamp::from_unix_millis(stored_created_at),
                occurred_at,
            );
            let mut ids = PortfolioReplayIds::new(
                command.approval.prepared_id().clone(),
                approval_receipt_id.clone(),
            );
            let mut service = seed_roadmap_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &roadmap_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_roadmap_classification(
                    PrepareLowerRoadmapClassification {
                        roadmap_id: roadmap_id.clone(),
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
                    domain_conflict(&context, "roadmap.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let outcome = service.approve_and_execute_lower_roadmap_classification(
                ApproveAndExecuteLowerRoadmapClassification {
                    approval: command.approval.clone(),
                    context: context.clone(),
                },
                &mut ids,
                &AllowPersistedPortfolioApproval,
            )?;
            let ordinal = next_roadmap_h2a_execute_operation_ordinal(tx, &context)?;
            persist_roadmap_lowered_bundle(
                tx,
                &command.approval,
                &outcome,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(outcome)
        })
    }

    /// H2a step 1: persist one already-canonical Kpi
    /// classification-lowering preview. See
    /// `prepare_lower_portfolio_classification` for the shared rationale.
    pub fn prepare_lower_kpi_classification(
        &mut self,
        command: PrepareLowerKpiClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT kpi_id,expected_version,proposed_classification,rationale,result_reference FROM kpi_h2a_prepare_replay_operations replay JOIN kpi_h2a_command_prepare_lower_classifications command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.kpi_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM kpi_definitions JOIN aggregate_registry registry ON registry.id=kpi_definitions.id AND registry.aggregate_type='kpi_definition' WHERE kpi_definitions.id=?1",
                    [command.kpi_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "kpi.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(
                    &context,
                    "kpi.stale_version",
                    current_version,
                ));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command
                .proposed_classification
                .combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "kpi.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerKpiClassification {
                kpi_id,
                kpi_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(
                    &context,
                    "kpi.classification_lowering_invalid",
                ));
            };
            if kpi_id != &command.kpi_id
                || kpi_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(
                    &context,
                    "kpi.classification_lowering_invalid",
                ));
            }
            let ordinal = next_kpi_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "lower_kpi_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'kpi',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.kpi_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO kpi_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO kpi_h2a_command_prepare_lower_classifications (idempotency_id,kpi_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.kpi_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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

    /// H2a step 2 for Kpi. See
    /// `approve_and_execute_lower_portfolio_classification` for the shared
    /// rationale.
    pub fn approve_and_execute_lower_kpi_classification(
        &mut self,
        command: ApproveAndExecuteLowerKpiClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiDefinitionRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM kpi_h2a_command_execute_lower_classifications WHERE idempotency_id=?1",
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
                return replay_kpi_lowered_outcome(tx, &context);
            }
            let (
                stored_kpi_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.kpi_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM kpi_h2a_prepare_replay_operations replay JOIN kpi_h2a_command_prepare_lower_classifications command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1",
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
                    domain_conflict(&context, "kpi.classification_lowering_preview_changed")
                })?;
            let kpi_id = KpiId::parse(stored_kpi_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = pmc_domain::work_management::WorkManagementRationale::parse(
                stored_rationale,
            )
            .map_err(|_| storage_error(&context))?;

            let clock = PortfolioReplayThenNowClock::new(
                UtcTimestamp::from_unix_millis(stored_created_at),
                occurred_at,
            );
            let mut ids = PortfolioReplayIds::new(
                command.approval.prepared_id().clone(),
                approval_receipt_id.clone(),
            );
            let mut service = seed_kpi_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &kpi_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_kpi_classification(
                    PrepareLowerKpiClassification {
                        kpi_id: kpi_id.clone(),
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
                    domain_conflict(&context, "kpi.classification_lowering_preview_changed")
                })?;
            clock.enter_execute_phase();
            let outcome = service.approve_and_execute_lower_kpi_classification(
                ApproveAndExecuteLowerKpiClassification {
                    approval: command.approval.clone(),
                    context: context.clone(),
                },
                &mut ids,
                &AllowPersistedPortfolioApproval,
            )?;
            let ordinal = next_kpi_h2a_execute_operation_ordinal(tx, &context)?;
            persist_kpi_lowered_bundle(
                tx,
                &command.approval,
                &outcome,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(outcome)
        })
    }

    /// H2a step 1: persist one already-canonical KpiObservation
    /// classification-lowering preview. See
    /// `prepare_lower_portfolio_classification` for the shared rationale.
    pub fn prepare_lower_kpi_observation_classification(
        &mut self,
        command: PrepareLowerKpiObservationClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT observation_id,expected_version,proposed_classification,rationale,result_reference FROM kpi_observation_h2a_prepare_replay_operations replay JOIN kpi_observation_h2a_command_prepare_lower_classifications command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.observation_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (current_classification, current_version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM kpi_observations JOIN aggregate_registry registry ON registry.id=kpi_observations.id AND registry.aggregate_type='kpi_observation' WHERE kpi_observations.id=?1",
                    [command.observation_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context, "kpi.observation.not_found"))?;
            if current_version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(stale_version(
                    &context,
                    "kpi.observation.stale_version",
                    current_version,
                ));
            }
            let current_classification = DataClassification::from_persisted(&current_classification)
                .map_err(|_| storage_error(&context))?;
            if command
                .proposed_classification
                .combine(current_classification)
                == command.proposed_classification
            {
                return Err(domain_conflict(
                    &context,
                    "kpi.observation.classification_lowering_not_a_lowering",
                ));
            }
            let WorkManagementOperation::LowerKpiObservationClassification {
                observation_id,
                observation_version,
                current_classification: op_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(domain_conflict(
                    &context,
                    "kpi.observation.classification_lowering_invalid",
                ));
            };
            if observation_id != &command.observation_id
                || observation_version != &command.expected_version
                || op_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(domain_conflict(
                    &context,
                    "kpi.observation.classification_lowering_invalid",
                ));
            }
            let ordinal = next_kpi_observation_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "lower_kpi_observation_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'kpi_observation',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.observation_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO kpi_observation_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO kpi_observation_h2a_command_prepare_lower_classifications (idempotency_id,observation_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.observation_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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

    /// H2a step 2 for KpiObservation. See
    /// `approve_and_execute_lower_portfolio_classification` for the shared
    /// rationale, and `seed_kpi_observation_service_for_lowering`'s doc
    /// comment for why this seeds two records, not one.
    pub fn approve_and_execute_lower_kpi_observation_classification(
        &mut self,
        command: ApproveAndExecuteLowerKpiObservationClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<KpiObservationRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM kpi_observation_h2a_command_execute_lower_classifications WHERE idempotency_id=?1",
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
                return replay_kpi_observation_lowered_outcome(tx, &context);
            }
            let (
                stored_observation_id,
                stored_expected_version,
                stored_proposed_classification,
                stored_rationale,
                stored_created_at,
            ): (String, i64, String, String, i64) = tx
                .query_row(
                    "SELECT command.observation_id,command.expected_version,command.proposed_classification,command.rationale,intent.created_at FROM kpi_observation_h2a_prepare_replay_operations replay JOIN kpi_observation_h2a_command_prepare_lower_classifications command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference WHERE replay.result_reference=?1",
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
                    domain_conflict(&context, "kpi.observation.classification_lowering_preview_changed")
                })?;
            let observation_id =
                KpiObservationId::parse(stored_observation_id).map_err(|_| storage_error(&context))?;
            let expected_version = AggregateVersion::new(
                u64::try_from(stored_expected_version).map_err(|_| storage_error(&context))?,
            )
            .map_err(|_| storage_error(&context))?;
            let proposed_classification =
                DataClassification::from_persisted(&stored_proposed_classification)
                    .map_err(|_| storage_error(&context))?;
            let rationale = pmc_domain::work_management::WorkManagementRationale::parse(
                stored_rationale,
            )
            .map_err(|_| storage_error(&context))?;

            let clock = PortfolioReplayThenNowClock::new(
                UtcTimestamp::from_unix_millis(stored_created_at),
                occurred_at,
            );
            let mut ids = PortfolioReplayIds::new(
                command.approval.prepared_id().clone(),
                approval_receipt_id.clone(),
            );
            let mut service = seed_kpi_observation_service_for_lowering(
                tx,
                clock.clone(),
                audit_event_id.clone(),
                &observation_id,
                &context,
            )?;
            let replay_idempotency_id =
                IdempotencyId::parse(format!("replay-prepare-{}", context.idempotency_id.as_str()))
                    .map_err(|_| storage_error(&context))?;
            service
                .prepare_lower_kpi_observation_classification(
                    PrepareLowerKpiObservationClassification {
                        observation_id: observation_id.clone(),
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
                    domain_conflict(
                        &context,
                        "kpi.observation.classification_lowering_preview_changed",
                    )
                })?;
            clock.enter_execute_phase();
            let outcome = service.approve_and_execute_lower_kpi_observation_classification(
                ApproveAndExecuteLowerKpiObservationClassification {
                    approval: command.approval.clone(),
                    context: context.clone(),
                },
                &mut ids,
                &AllowPersistedPortfolioApproval,
            )?;
            let ordinal = next_kpi_observation_h2a_execute_operation_ordinal(tx, &context)?;
            persist_kpi_observation_lowered_bundle(
                tx,
                &command.approval,
                &outcome,
                &approval_receipt_id,
                ordinal,
                &context,
            )?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(outcome)
        })
    }
}

fn next_product_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM product_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_product_h2a_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM product_h2a_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// See `replay_lowered_outcome` (Portfolio) for the shared rationale.
fn replay_product_lowered_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<ProductRecord>, DomainError> {
    let product_id: String = tx
        .query_row(
            "SELECT product_id FROM product_h2a_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let row = tx
        .query_row(
            "SELECT products.id,products.name,products.details,registry.classification,products.provenance_kind,products.provenance_reference,registry.version FROM products JOIN aggregate_registry registry ON registry.id=products.id AND registry.aggregate_type='product' WHERE products.id=?1",
            [product_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = ProductRecord {
        id: ProductId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        details: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM product_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// See `persist_lowered_bundle` (Portfolio) for the shared rationale.
fn persist_product_lowered_bundle(
    tx: &Transaction<'_>,
    approval: &pmc_domain::work_management::WorkManagementApproval,
    outcome: &MutationOutcome<ProductRecord>,
    approval_receipt_id: &ApprovalReceiptId,
    ordinal: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let product_id = outcome.record.id.as_str();
    let audit = &outcome.audit_event;
    let AuditTarget::Product(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != product_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='product'",
        rusqlite::params![
            i64::try_from(outcome.record.version.get()).map_err(|_| storage_error(context))?,
            outcome.record.classification.as_persisted(),
            occurred_millis,
            product_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    persist_portfolio_audit(tx, audit, "product", product_id, context)?;
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
        "INSERT INTO product_h2a_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,product_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            product_id,
            approval.prepared_id().as_str(),
            approval_receipt_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO product_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO product_h2a_command_execute_lower_classifications(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// See `seed_portfolio_service_for_lowering` for the shared rationale.
fn seed_product_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    product_id: &ProductId,
    context: &OperationContext,
) -> Result<InMemoryPortfolioService<C, QueuedAuditIds>, DomainError> {
    let (name, details, classification, provenance_kind, provenance_reference, version): (
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT p.name,p.details,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM products p JOIN aggregate_registry r ON r.id=p.id AND r.aggregate_type='product' WHERE p.id=?1",
            [product_id.as_str()],
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
        .ok_or_else(|| domain_not_found(context, "product.not_found"))?;
    let name = pmc_domain::portfolio::ShortText::parse(name).map_err(|_| storage_error(context))?;
    let details =
        pmc_domain::portfolio::LongText::parse(details).map_err(|_| storage_error(context))?;
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!("replay-seed-audit-{}-{step}", product_id.as_str()))
                .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryPortfolioService::new(clock, QueuedAuditIds(synthetic_ids));
    service
        .create_product(CreateProduct {
            id: product_id.clone(),
            name: name.clone(),
            details: details.clone(),
            classification: Some(classification),
            provenance,
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    product_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_product_details(UpdateProductDetails {
                id: product_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                details: details.clone(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        product_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

fn next_roadmap_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM roadmap_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_roadmap_h2a_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM roadmap_h2a_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// See `replay_lowered_outcome` (Portfolio) for the shared rationale.
fn replay_roadmap_lowered_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<RoadmapRecord>, DomainError> {
    let roadmap_id: String = tx
        .query_row(
            "SELECT roadmap_id FROM roadmap_h2a_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let row = tx
        .query_row(
            "SELECT roadmaps.id,roadmaps.name,roadmaps.details,registry.classification,roadmaps.provenance_kind,roadmaps.provenance_reference,registry.version FROM roadmaps JOIN aggregate_registry registry ON registry.id=roadmaps.id AND registry.aggregate_type='roadmap' WHERE roadmaps.id=?1",
            [roadmap_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = RoadmapRecord {
        id: RoadmapId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        details: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM roadmap_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// See `persist_lowered_bundle` (Portfolio) for the shared rationale.
fn persist_roadmap_lowered_bundle(
    tx: &Transaction<'_>,
    approval: &pmc_domain::work_management::WorkManagementApproval,
    outcome: &MutationOutcome<RoadmapRecord>,
    approval_receipt_id: &ApprovalReceiptId,
    ordinal: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let roadmap_id = outcome.record.id.as_str();
    let audit = &outcome.audit_event;
    let AuditTarget::Roadmap(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != roadmap_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='roadmap'",
        rusqlite::params![
            i64::try_from(outcome.record.version.get()).map_err(|_| storage_error(context))?,
            outcome.record.classification.as_persisted(),
            occurred_millis,
            roadmap_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    persist_portfolio_audit(tx, audit, "roadmap", roadmap_id, context)?;
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
        "INSERT INTO roadmap_h2a_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,roadmap_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            roadmap_id,
            approval.prepared_id().as_str(),
            approval_receipt_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO roadmap_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO roadmap_h2a_command_execute_lower_classifications(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// See `seed_portfolio_service_for_lowering` for the shared rationale.
fn seed_roadmap_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    roadmap_id: &RoadmapId,
    context: &OperationContext,
) -> Result<InMemoryPortfolioService<C, QueuedAuditIds>, DomainError> {
    let (name, details, classification, provenance_kind, provenance_reference, version): (
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT p.name,p.details,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM roadmaps p JOIN aggregate_registry r ON r.id=p.id AND r.aggregate_type='roadmap' WHERE p.id=?1",
            [roadmap_id.as_str()],
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
        .ok_or_else(|| domain_not_found(context, "roadmap.not_found"))?;
    let name = pmc_domain::portfolio::ShortText::parse(name).map_err(|_| storage_error(context))?;
    let details =
        pmc_domain::portfolio::LongText::parse(details).map_err(|_| storage_error(context))?;
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!("replay-seed-audit-{}-{step}", roadmap_id.as_str()))
                .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryPortfolioService::new(clock, QueuedAuditIds(synthetic_ids));
    service
        .create_roadmap(CreateRoadmap {
            id: roadmap_id.clone(),
            name: name.clone(),
            details: details.clone(),
            classification: Some(classification),
            provenance,
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    roadmap_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_roadmap_details(UpdateRoadmapDetails {
                id: roadmap_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                details: details.clone(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        roadmap_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

fn next_kpi_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM kpi_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_kpi_h2a_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM kpi_h2a_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// See `replay_lowered_outcome` (Portfolio) for the shared rationale.
fn replay_kpi_lowered_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<KpiDefinitionRecord>, DomainError> {
    let kpi_id: String = tx
        .query_row(
            "SELECT kpi_id FROM kpi_h2a_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let row = tx
        .query_row(
            "SELECT definitions.id,definitions.name,definitions.definition,definitions.owner,definitions.target,definitions.cadence,definitions.source,registry.classification,definitions.provenance_kind,definitions.provenance_reference,registry.version FROM kpi_definitions definitions JOIN aggregate_registry registry ON registry.id=definitions.id AND registry.aggregate_type='kpi_definition' WHERE definitions.id=?1",
            [kpi_id.as_str()],
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
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = KpiDefinitionRecord {
        id: KpiId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        definition: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        owner: pmc_domain::portfolio::ShortText::parse(row.3)
            .map_err(|_| storage_error(context))?,
        target: pmc_domain::portfolio::ShortText::parse(row.4)
            .map_err(|_| storage_error(context))?,
        cadence: pmc_domain::portfolio::ShortText::parse(row.5)
            .map_err(|_| storage_error(context))?,
        source: pmc_domain::portfolio::LongText::parse(row.6)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.7)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.8, row.9, context)?,
        version: AggregateVersion::new(u64::try_from(row.10).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM kpi_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// See `persist_lowered_bundle` (Portfolio) for the shared rationale.
fn persist_kpi_lowered_bundle(
    tx: &Transaction<'_>,
    approval: &pmc_domain::work_management::WorkManagementApproval,
    outcome: &MutationOutcome<KpiDefinitionRecord>,
    approval_receipt_id: &ApprovalReceiptId,
    ordinal: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let kpi_id = outcome.record.id.as_str();
    let audit = &outcome.audit_event;
    let AuditTarget::Kpi(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != kpi_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='kpi_definition'",
        rusqlite::params![
            i64::try_from(outcome.record.version.get()).map_err(|_| storage_error(context))?,
            outcome.record.classification.as_persisted(),
            occurred_millis,
            kpi_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    persist_portfolio_audit(tx, audit, "kpi_definition", kpi_id, context)?;
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
        "INSERT INTO kpi_h2a_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,kpi_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            kpi_id,
            approval.prepared_id().as_str(),
            approval_receipt_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO kpi_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO kpi_h2a_command_execute_lower_classifications(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// See `seed_portfolio_service_for_lowering` for the shared rationale.
fn seed_kpi_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    kpi_id: &KpiId,
    context: &OperationContext,
) -> Result<InMemoryPortfolioService<C, QueuedAuditIds>, DomainError> {
    let (
        name,
        definition,
        owner,
        target,
        cadence,
        source,
        classification,
        provenance_kind,
        provenance_reference,
        version,
    ): (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT k.name,k.definition,k.owner,k.target,k.cadence,k.source,r.classification,k.provenance_kind,k.provenance_reference,r.version FROM kpi_definitions k JOIN aggregate_registry r ON r.id=k.id AND r.aggregate_type='kpi_definition' WHERE k.id=?1",
            [kpi_id.as_str()],
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
                    row.get(8)?,
                    row.get(9)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| domain_not_found(context, "kpi.not_found"))?;
    let name = pmc_domain::portfolio::ShortText::parse(name).map_err(|_| storage_error(context))?;
    let definition =
        pmc_domain::portfolio::LongText::parse(definition).map_err(|_| storage_error(context))?;
    let owner =
        pmc_domain::portfolio::ShortText::parse(owner).map_err(|_| storage_error(context))?;
    let target =
        pmc_domain::portfolio::ShortText::parse(target).map_err(|_| storage_error(context))?;
    let cadence =
        pmc_domain::portfolio::ShortText::parse(cadence).map_err(|_| storage_error(context))?;
    let source =
        pmc_domain::portfolio::LongText::parse(source).map_err(|_| storage_error(context))?;
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!("replay-seed-audit-{}-{step}", kpi_id.as_str()))
                .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryPortfolioService::new(clock, QueuedAuditIds(synthetic_ids));
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi_id.clone(),
            name: name.clone(),
            definition: definition.clone(),
            owner: owner.clone(),
            target: target.clone(),
            cadence: cadence.clone(),
            source: source.clone(),
            classification: Some(classification),
            provenance,
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    kpi_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_kpi_definition_details(UpdateKpiDefinitionDetails {
                id: kpi_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                definition: definition.clone(),
                owner: owner.clone(),
                target: target.clone(),
                cadence: cadence.clone(),
                source: source.clone(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        kpi_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

fn next_kpi_observation_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM kpi_observation_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_kpi_observation_h2a_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM kpi_observation_h2a_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// See `replay_lowered_outcome` (Portfolio) for the shared rationale.
fn replay_kpi_observation_lowered_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<KpiObservationRecord>, DomainError> {
    let observation_id: String = tx
        .query_row(
            "SELECT observation_id FROM kpi_observation_h2a_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let row = tx
        .query_row(
            "SELECT o.id,o.kpi_id,o.value,o.observed_at,o.source,r.classification,o.provenance_kind,o.provenance_reference,r.version,r.created_at,r.updated_at FROM kpi_observations o JOIN aggregate_registry r ON r.id=o.id AND r.aggregate_type='kpi_observation' WHERE o.id=?1",
            [observation_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
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
    let record = KpiObservationRecord {
        id: KpiObservationId::parse(row.0).map_err(|_| storage_error(context))?,
        kpi_id: KpiId::parse(row.1).map_err(|_| storage_error(context))?,
        value: pmc_domain::portfolio::ShortText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        observed_at: UtcTimestamp::from_unix_millis(row.3),
        source: pmc_domain::portfolio::LongText::parse(row.4)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.5)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.6, row.7, context)?,
        version: AggregateVersion::new(u64::try_from(row.8).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.9),
        updated_at: UtcTimestamp::from_unix_millis(row.10),
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM kpi_observation_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// See `persist_lowered_bundle` (Portfolio) for the shared rationale. Never
/// writes `created_at` back -- the seeded reconstruction's `created_at` is
/// a synthetic prepare-time stamp (see `seed_kpi_observation_service_for_
/// lowering`), not the observation's true historical creation time, so it
/// must never leak into durable state.
fn persist_kpi_observation_lowered_bundle(
    tx: &Transaction<'_>,
    approval: &pmc_domain::work_management::WorkManagementApproval,
    outcome: &MutationOutcome<KpiObservationRecord>,
    approval_receipt_id: &ApprovalReceiptId,
    ordinal: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let observation_id = outcome.record.id.as_str();
    let audit = &outcome.audit_event;
    let AuditTarget::KpiObservation(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != observation_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='kpi_observation'",
        rusqlite::params![
            i64::try_from(outcome.record.version.get()).map_err(|_| storage_error(context))?,
            outcome.record.classification.as_persisted(),
            occurred_millis,
            observation_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    persist_portfolio_audit(tx, audit, "kpi_observation", observation_id, context)?;
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
        "INSERT INTO kpi_observation_h2a_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,observation_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            observation_id,
            approval.prepared_id().as_str(),
            approval_receipt_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO kpi_observation_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO kpi_observation_h2a_command_execute_lower_classifications(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// See `seed_portfolio_service_for_lowering` for the shared rationale on
/// why this hand-seeds through the public H1 API rather than rehydrating a
/// full `PortfolioPersistenceSnapshot`. KpiObservation adds one wrinkle the
/// other four Portfolio-family types don't have: `InMemoryPortfolioService
/// ::create_kpi_observation` requires its *parent* Kpi Definition to
/// already exist in `self.kpis` (`kpi.not_found` otherwise) and folds the
/// parent's classification into the observation's own via `combine`
/// internally -- confirmed by reading `create_kpi_observation`'s and
/// `update_kpi_observation_details`'s source before writing this, not
/// assumed by analogy. So this seeds the parent Kpi first (exactly like
/// `seed_kpi_service_for_lowering`), then the Observation on top of the
/// same instance. The observation's synthetic `create`/`update` calls pass
/// its own *current* (already-combined) stored classification through as
/// the explicit value each time; combining an already-combined value with
/// the just-seeded, current parent classification is idempotent as long as
/// the parent's classification has not changed since the observation was
/// last touched -- the one case where that assumption could be stale is
/// exactly the kind of change `record.version != observation_version`
/// (checked by the domain execute method itself) or the payload-digest
/// check (checked by the H2a approval crypto) would already fail closed
/// on, never silently commit wrong state.
fn seed_kpi_observation_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    observation_id: &KpiObservationId,
    context: &OperationContext,
) -> Result<InMemoryPortfolioService<C, QueuedAuditIds>, DomainError> {
    let (
        kpi_id,
        value,
        observed_at,
        source,
        observation_classification,
        observation_provenance_kind,
        observation_provenance_reference,
        observation_version,
    ): (String, String, i64, String, String, String, Option<String>, i64) = tx
        .query_row(
            "SELECT o.kpi_id,o.value,o.observed_at,o.source,r.classification,o.provenance_kind,o.provenance_reference,r.version FROM kpi_observations o JOIN aggregate_registry r ON r.id=o.id AND r.aggregate_type='kpi_observation' WHERE o.id=?1",
            [observation_id.as_str()],
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
        .ok_or_else(|| domain_not_found(context, "kpi.observation.not_found"))?;
    let kpi_id = KpiId::parse(kpi_id).map_err(|_| storage_error(context))?;
    let value =
        pmc_domain::portfolio::ShortText::parse(value).map_err(|_| storage_error(context))?;
    let source =
        pmc_domain::portfolio::LongText::parse(source).map_err(|_| storage_error(context))?;
    let observation_classification =
        DataClassification::from_persisted(&observation_classification)
            .map_err(|_| storage_error(context))?;
    let observation_provenance = decode_provenance(
        &observation_provenance_kind,
        observation_provenance_reference,
        context,
    )?;
    let observation_version =
        u64::try_from(observation_version).map_err(|_| storage_error(context))?;

    let (
        kpi_name,
        kpi_definition,
        kpi_owner,
        kpi_target,
        kpi_cadence,
        kpi_source,
        kpi_classification,
        kpi_provenance_kind,
        kpi_provenance_reference,
        kpi_version,
    ): (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT k.name,k.definition,k.owner,k.target,k.cadence,k.source,r.classification,k.provenance_kind,k.provenance_reference,r.version FROM kpi_definitions k JOIN aggregate_registry r ON r.id=k.id AND r.aggregate_type='kpi_definition' WHERE k.id=?1",
            [kpi_id.as_str()],
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
                    row.get(8)?,
                    row.get(9)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| domain_not_found(context, "kpi.not_found"))?;
    let kpi_name =
        pmc_domain::portfolio::ShortText::parse(kpi_name).map_err(|_| storage_error(context))?;
    let kpi_definition = pmc_domain::portfolio::LongText::parse(kpi_definition)
        .map_err(|_| storage_error(context))?;
    let kpi_owner =
        pmc_domain::portfolio::ShortText::parse(kpi_owner).map_err(|_| storage_error(context))?;
    let kpi_target =
        pmc_domain::portfolio::ShortText::parse(kpi_target).map_err(|_| storage_error(context))?;
    let kpi_cadence =
        pmc_domain::portfolio::ShortText::parse(kpi_cadence).map_err(|_| storage_error(context))?;
    let kpi_source =
        pmc_domain::portfolio::LongText::parse(kpi_source).map_err(|_| storage_error(context))?;
    let kpi_classification = DataClassification::from_persisted(&kpi_classification)
        .map_err(|_| storage_error(context))?;
    let kpi_provenance =
        decode_provenance(&kpi_provenance_kind, kpi_provenance_reference, context)?;
    let kpi_version = u64::try_from(kpi_version).map_err(|_| storage_error(context))?;

    let total_synthetic = kpi_version + observation_version;
    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(total_synthetic).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..total_synthetic {
        synthetic_ids.push_back(
            AuditEventId::parse(format!(
                "replay-seed-audit-{}-{step}",
                observation_id.as_str()
            ))
            .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryPortfolioService::new(clock, QueuedAuditIds(synthetic_ids));

    // Seed the parent Kpi Definition first.
    service
        .create_kpi_definition(CreateKpiDefinition {
            id: kpi_id.clone(),
            name: kpi_name.clone(),
            definition: kpi_definition.clone(),
            owner: kpi_owner.clone(),
            target: kpi_target.clone(),
            cadence: kpi_cadence.clone(),
            source: kpi_source.clone(),
            classification: Some(kpi_classification),
            provenance: kpi_provenance,
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-kpi-{}",
                    kpi_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..kpi_version {
        service
            .update_kpi_definition_details(UpdateKpiDefinitionDetails {
                id: kpi_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: kpi_name.clone(),
                definition: kpi_definition.clone(),
                owner: kpi_owner.clone(),
                target: kpi_target.clone(),
                cadence: kpi_cadence.clone(),
                source: kpi_source.clone(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-kpi-{}-{step}",
                        kpi_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
            })
            .map_err(|_| storage_error(context))?;
    }

    // Now seed the Observation on top.
    service
        .create_kpi_observation(CreateKpiObservation {
            id: observation_id.clone(),
            kpi_id: kpi_id.clone(),
            value: value.clone(),
            observed_at: UtcTimestamp::from_unix_millis(observed_at),
            source: source.clone(),
            classification: Some(observation_classification),
            provenance: observation_provenance,
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    observation_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..observation_version {
        service
            .update_kpi_observation_details(UpdateKpiObservationDetails {
                id: observation_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                value: value.clone(),
                observed_at: UtcTimestamp::from_unix_millis(observed_at),
                source: source.clone(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        observation_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
}

fn decode_portfolio_provenance(
    tx: &Transaction<'_>,
    id: &PortfolioId,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    let (kind, reference): (String, Option<String>) = tx
        .query_row(
            "SELECT provenance_kind,provenance_reference FROM portfolios WHERE id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    decode_provenance(&kind, reference, context)
}

fn decode_product_provenance(
    tx: &Transaction<'_>,
    id: &ProductId,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    let (kind, reference): (String, Option<String>) = tx
        .query_row(
            "SELECT provenance_kind,provenance_reference FROM products WHERE id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    decode_provenance(&kind, reference, context)
}

fn decode_roadmap_provenance(
    tx: &Transaction<'_>,
    id: &RoadmapId,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    let (kind, reference): (String, Option<String>) = tx
        .query_row(
            "SELECT provenance_kind,provenance_reference FROM roadmaps WHERE id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    decode_provenance(&kind, reference, context)
}

fn decode_kpi_definition_provenance(
    tx: &Transaction<'_>,
    id: &KpiId,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    let (kind, reference): (String, Option<String>) = tx
        .query_row(
            "SELECT provenance_kind,provenance_reference FROM kpi_definitions WHERE id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    decode_provenance(&kind, reference, context)
}

fn decode_kpi_observation_provenance(
    tx: &Transaction<'_>,
    id: &KpiObservationId,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    let (kind, reference): (String, Option<String>) = tx
        .query_row(
            "SELECT provenance_kind,provenance_reference FROM kpi_observations WHERE id=?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    decode_provenance(&kind, reference, context)
}

fn decode_provenance(
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

fn decode_portfolio_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<MutationOutcome<PortfolioRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version FROM portfolio_command_results WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = PortfolioRecord {
        id: PortfolioId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        details: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM portfolio_idempotency_outcome_audits WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn decode_product_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<MutationOutcome<ProductRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version FROM portfolio_command_results WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = ProductRecord {
        id: ProductId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        details: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM portfolio_idempotency_outcome_audits WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn decode_roadmap_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<MutationOutcome<RoadmapRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_details,result_classification,result_provenance_kind,result_provenance_reference,result_version FROM portfolio_command_results WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = RoadmapRecord {
        id: RoadmapId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        details: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM portfolio_idempotency_outcome_audits WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn decode_kpi_definition_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<MutationOutcome<KpiDefinitionRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_name,result_definition,result_owner,result_target,result_cadence,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version FROM portfolio_command_results WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
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
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = KpiDefinitionRecord {
        id: KpiId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        definition: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        owner: pmc_domain::portfolio::ShortText::parse(row.3)
            .map_err(|_| storage_error(context))?,
        target: pmc_domain::portfolio::ShortText::parse(row.4)
            .map_err(|_| storage_error(context))?,
        cadence: pmc_domain::portfolio::ShortText::parse(row.5)
            .map_err(|_| storage_error(context))?,
        source: pmc_domain::portfolio::LongText::parse(row.6)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.7)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.8, row.9, context)?,
        version: AggregateVersion::new(u64::try_from(row.10).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM portfolio_idempotency_outcome_audits WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn decode_kpi_observation_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<MutationOutcome<KpiObservationRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT result_id,result_kpi_id,result_value,result_observed_at,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at FROM portfolio_command_results WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
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
    let record = KpiObservationRecord {
        id: KpiObservationId::parse(row.0).map_err(|_| storage_error(context))?,
        kpi_id: KpiId::parse(row.1).map_err(|_| storage_error(context))?,
        value: pmc_domain::portfolio::ShortText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        observed_at: UtcTimestamp::from_unix_millis(row.3),
        source: pmc_domain::portfolio::LongText::parse(row.4)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.5)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.6, row.7, context)?,
        version: AggregateVersion::new(u64::try_from(row.8).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.9),
        updated_at: UtcTimestamp::from_unix_millis(row.10),
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM portfolio_idempotency_outcome_audits WHERE namespace='portfolio' AND operation=?1 AND idempotency_id=?2 AND ordinal=0",
            rusqlite::params![operation, context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn decode_audit(
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
        "portfolio" => {
            AuditTarget::Portfolio(PortfolioId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        "product" => {
            AuditTarget::Product(ProductId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        "roadmap" => {
            AuditTarget::Roadmap(RoadmapId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        "kpi_definition" => {
            AuditTarget::Kpi(KpiId::parse(row.3).map_err(|_| storage_error(context))?)
        }
        "kpi_observation" => AuditTarget::KpiObservation(
            KpiObservationId::parse(row.3).map_err(|_| storage_error(context))?,
        ),
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
            vec![AuditEffectCode::parse(PORTFOLIO_EFFECT_CODE)
                .map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn build_audit(
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
            vec![AuditEffectCode::parse(PORTFOLIO_EFFECT_CODE)
                .map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn persist_portfolio_audit(
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
        rusqlite::params![audit.id().as_str(), PORTFOLIO_EFFECT_CODE, target_type, target_id],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn next_portfolio_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM portfolio_idempotency_outcomes WHERE namespace='portfolio'",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Persists the shared generic `idempotency_outcomes` row plus the
/// portfolio-namespace `portfolio_idempotency_outcomes` row. `payload_digest`
/// has no cross-checked cryptographic meaning in this schema (unlike the H2a
/// families' `WorkManagementPayloadDigest`) -- only `CHECK(length(...)>0)` --
/// so the idempotency ID itself is a sufficient, deterministic value.
fn persist_idempotency_claim(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
    ordinal: i64,
    target_id: &str,
    occurred_millis: i64,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO idempotency_outcomes VALUES('portfolio',?1,?2,?3,'succeeded',?4,?5)",
        rusqlite::params![
            operation,
            context.idempotency_id.as_str(),
            context.idempotency_id.as_str(),
            target_id,
            occurred_millis,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio',?1,?2,?1,?3,?4,'succeeded',?5)",
        rusqlite::params![
            operation,
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            target_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn persist_idempotency_outcome_audit(
    tx: &Transaction<'_>,
    context: &OperationContext,
    ordinal: i64,
    audit_event_id: &str,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO portfolio_idempotency_outcome_audits VALUES('portfolio',(SELECT operation FROM portfolio_idempotency_outcomes WHERE namespace='portfolio' AND idempotency_id=?1),?1,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), ordinal, audit_event_id],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn idempotency_claimed_by_other_operation(
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

/// See `risk_repository.rs::prepared_intent_digest_matches` for the full
/// rationale: a prepare replay must bind to the exact previously persisted
/// preview's cryptographic digest, not merely to matching command scalars.
fn prepared_intent_digest_matches(
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
fn persist_prepared_intent(
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

fn next_portfolio_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM portfolio_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_portfolio_h2a_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM portfolio_h2a_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Re-reads a previously committed classification lowering by its current
/// (already-mutated) durable state -- the portfolio row already reflects
/// the post-lowering result, exactly like `decode_portfolio_outcome` does
/// for the ordinary H1 replay path, just sourced from `portfolios` /
/// `aggregate_registry` directly instead of `portfolio_command_results`
/// (this operation never writes that H1-only table).
fn replay_lowered_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<PortfolioRecord>, DomainError> {
    let portfolio_id: String = tx
        .query_row(
            "SELECT portfolio_id FROM portfolio_h2a_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let row = tx
        .query_row(
            "SELECT portfolios.id,portfolios.name,portfolios.details,registry.classification,portfolios.provenance_kind,portfolios.provenance_reference,registry.version FROM portfolios JOIN aggregate_registry registry ON registry.id=portfolios.id AND registry.aggregate_type='portfolio' WHERE portfolios.id=?1",
            [portfolio_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = PortfolioRecord {
        id: PortfolioId::parse(row.0).map_err(|_| storage_error(context))?,
        name: pmc_domain::portfolio::ShortText::parse(row.1).map_err(|_| storage_error(context))?,
        details: pmc_domain::portfolio::LongText::parse(row.2)
            .map_err(|_| storage_error(context))?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        version: AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
    };
    let audit_event_id: String = tx
        .query_row(
            "SELECT audit_event_id FROM portfolio_h2a_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// Persists one successful Portfolio classification-lowering execution:
/// the mutation (classification + version bump on the existing row), its
/// audit event, the consumed prepared intent, the minted approval receipt,
/// and this adapter's own typed H2a execute replay bookkeeping. Mirrors
/// `risk_repository.rs::persist_closed_bundle`.
fn persist_lowered_bundle(
    tx: &Transaction<'_>,
    approval: &pmc_domain::work_management::WorkManagementApproval,
    outcome: &MutationOutcome<PortfolioRecord>,
    approval_receipt_id: &ApprovalReceiptId,
    ordinal: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let portfolio_id = outcome.record.id.as_str();
    let audit = &outcome.audit_event;
    let AuditTarget::Portfolio(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != portfolio_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='portfolio'",
        rusqlite::params![
            i64::try_from(outcome.record.version.get()).map_err(|_| storage_error(context))?,
            outcome.record.classification.as_persisted(),
            occurred_millis,
            portfolio_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    persist_portfolio_audit(tx, audit, "portfolio", portfolio_id, context)?;
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
        "INSERT INTO portfolio_h2a_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,portfolio_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            portfolio_id,
            approval.prepared_id().as_str(),
            approval_receipt_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO portfolio_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO portfolio_h2a_command_execute_lower_classifications(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Seeds a throwaway `InMemoryPortfolioService` with exactly the one
/// Portfolio record an H2a lowering touches, reconstructed to its exact
/// current (name, details, classification, provenance, version) via the
/// service's own public H1 API -- one synthetic `create_portfolio` plus
/// `version - 1` synthetic `update_portfolio_details` calls, all using the
/// record's CURRENT field values (so every intermediate synthetic state is
/// harmless: only the final state is ever observed). This sidesteps needing
/// a full historical replay-capsule decoder entirely: `InMemoryPortfolioService
/// ::rehydrate`'s `PortfolioPersistenceSnapshot::validate` replays and
/// verifies the *complete* durable history of every Portfolio-family record
/// (all five record types), which this adapter has no need to reconstruct
/// -- Portfolio's own lowering only ever reads `self.portfolios`. Every
/// audit id these synthetic calls consume is throwaway (never persisted);
/// `audit_ids` supplies exactly `version` of them, one per seeding call,
/// followed by the one real caller-supplied id for the actual execute.
fn seed_portfolio_service_for_lowering<C: Clock>(
    tx: &Transaction<'_>,
    clock: C,
    real_audit_event_id: AuditEventId,
    portfolio_id: &PortfolioId,
    context: &OperationContext,
) -> Result<InMemoryPortfolioService<C, QueuedAuditIds>, DomainError> {
    let (name, details, classification, provenance_kind, provenance_reference, version): (
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT p.name,p.details,r.classification,p.provenance_kind,p.provenance_reference,r.version FROM portfolios p JOIN aggregate_registry r ON r.id=p.id AND r.aggregate_type='portfolio' WHERE p.id=?1",
            [portfolio_id.as_str()],
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
        .ok_or_else(|| domain_not_found(context, "portfolio.not_found"))?;
    let name = pmc_domain::portfolio::ShortText::parse(name).map_err(|_| storage_error(context))?;
    let details =
        pmc_domain::portfolio::LongText::parse(details).map_err(|_| storage_error(context))?;
    let classification =
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?;
    let provenance = decode_provenance(&provenance_kind, provenance_reference, context)?;
    let version = u64::try_from(version).map_err(|_| storage_error(context))?;

    let mut synthetic_ids = std::collections::VecDeque::with_capacity(
        usize::try_from(version).unwrap_or(usize::MAX) + 1,
    );
    for step in 0..version {
        synthetic_ids.push_back(
            AuditEventId::parse(format!(
                "replay-seed-audit-{}-{step}",
                portfolio_id.as_str()
            ))
            .map_err(|_| storage_error(context))?,
        );
    }
    synthetic_ids.push_back(real_audit_event_id);

    let mut service = InMemoryPortfolioService::new(clock, QueuedAuditIds(synthetic_ids));
    service
        .create_portfolio(CreatePortfolio {
            id: portfolio_id.clone(),
            name: name.clone(),
            details: details.clone(),
            classification: Some(classification),
            provenance,
            context: OperationContext {
                idempotency_id: IdempotencyId::parse(format!(
                    "replay-seed-create-{}",
                    portfolio_id.as_str()
                ))
                .map_err(|_| storage_error(context))?,
                correlation_id: context.correlation_id.clone(),
            },
        })
        .map_err(|_| storage_error(context))?;
    for step in 1..version {
        service
            .update_portfolio_details(UpdatePortfolioDetails {
                id: portfolio_id.clone(),
                expected_version: AggregateVersion::new(step)
                    .map_err(|_| storage_error(context))?,
                name: name.clone(),
                details: details.clone(),
                classification: None,
                context: OperationContext {
                    idempotency_id: IdempotencyId::parse(format!(
                        "replay-seed-update-{}-{step}",
                        portfolio_id.as_str()
                    ))
                    .map_err(|_| storage_error(context))?,
                    correlation_id: context.correlation_id.clone(),
                },
            })
            .map_err(|_| storage_error(context))?;
    }
    Ok(service)
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
        MessageKey::parse("portfolio.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
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
