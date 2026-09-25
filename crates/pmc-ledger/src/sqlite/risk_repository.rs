//! Typed SQLite persistence seam for the H1 Risk creation lifecycle.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
        AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    identity::{
        ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, IssueId, PreparedIntentId,
        RiskId, StakeholderId,
    },
    issues::{
        DenyIssueEvidenceAuthority, IssueExecutionPolicy, IssueExecutionPolicyPort,
        IssueServiceIdSource, RecordedIssueClassification,
    },
    risks::{
        AllowRiskEvidence, ApproveAndExecuteCloseRisk, ApproveAndExecuteLowerRiskClassification,
        ApproveAndExecuteRecordRiskOccurrence, CreateRisk, InMemoryRiskService,
        OccurredRiskOutcome, PrepareCloseRisk, PrepareLowerRiskClassification,
        PrepareRecordRiskOccurrence, RecordedRiskClassification, RejectRiskPreparedIntent,
        ResidualExposure, RiskClassificationAuthorityPort, RiskDetails, RiskEvidenceAuthorityPort,
        RiskExecutionPolicy, RiskExecutionPolicyPort, RiskH2aPersistenceDecodeInput,
        RiskH2aRejectionReplay, RiskH2aRuntimeSnapshot, RiskH2aTerminalOperation,
        RiskH2aTerminalReplay, RiskMutationOutcome, RiskOperationContext, RiskPersistenceSnapshot,
        RiskRationale, RiskRecord, RiskServiceIdSource, RiskTitle, UpdateRiskResponse,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        prepared_intent_rejection_audit, ApprovalAuthorizationPort, ApprovalConfirmation,
        RejectedPreparedIntentOutcome, RiskResponseType, RiskState, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
        RISK_PREPARED_REJECTED_AUDIT_CODE,
    },
    work_management_runtime::WorkManagementRuntimeComposition,
};
use rusqlite::{OptionalExtension, Transaction};

use super::reservation_repository::{
    reservation_matches, ReservationRequest, ReservedEntityKind, ReservedId,
};
use super::{LedgerTransactionError, SqliteProductLedger, CURRENT_SCHEMA_VERSION};

/// One `risk_response_replay_operations` result: response, owner, rationale,
/// residual exposure, next review, exception-queue flag, version.
type ResponseResultRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    i64,
    i64,
);

#[derive(Clone, Copy)]
struct FixedClock(UtcTimestamp);
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

#[derive(Clone)]
struct ExecuteRiskIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_ids: [Option<AuditEventId>; 3],
    next_audit: usize,
}
impl RiskServiceIdSource for ExecuteRiskIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_execute_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.receipt_id.take().ok_or_else(missing_execute_id)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        let slot = self
            .audit_ids
            .get_mut(self.next_audit)
            .ok_or_else(missing_execute_id)?;
        self.next_audit += 1;
        slot.take().ok_or_else(missing_execute_id)
    }
}

/// Never actually invoked: composition-level rehydration requires an Issue
/// id source, but neither Risk H2a execute path calls into the Issue
/// service directly (occurrence stages its Issue through the shared
/// authority inside the Risk service itself).
#[derive(Clone, Copy, Default)]
struct UnusedIssueIds;
impl IssueServiceIdSource for UnusedIssueIds {
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_execute_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        Err(missing_execute_id())
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        Err(missing_execute_id())
    }
}

fn missing_execute_id() -> pmc_domain::DomainValueError {
    match RiskId::parse("") {
        Err(error) => error,
        Ok(_) => unreachable!("empty opaque identifiers must be rejected"),
    }
}

/// The Issue side of the shared composition is purely auxiliary here --
/// occurrence stages its Issue through the shared authority inside the Risk
/// service itself, never by calling into the Issue service -- so, mirroring
/// how `decision_repository.rs` hardcodes its own auxiliary Action-side
/// ports instead of exposing them generically, this stays internal rather
/// than becoming a caller-supplied generic parameter.
#[derive(Clone, Copy, Default)]
struct AllowIssueExecution;
impl IssueExecutionPolicyPort for AllowIssueExecution {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

/// A rejection never consults the execution policy (nothing executes), but
/// composition-level rehydration still needs one; mirrors
/// `action_repository.rs`'s `PersistedRejectPolicy`.
#[derive(Clone, Copy, Default)]
struct RejectRiskPolicy;
impl RiskExecutionPolicyPort for RejectRiskPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Allowed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskPersistenceLoadError {
    UnsupportedSchema { found: u32 },
    StorageUnavailable,
    InvalidRiskSnapshot,
}

impl SqliteProductLedger {
    pub fn load_risk_persistence_snapshot(
        &self,
    ) -> Result<RiskPersistenceSnapshot, RiskPersistenceLoadError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(RiskPersistenceLoadError::UnsupportedSchema {
                found: self.schema_version,
            });
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| RiskPersistenceLoadError::StorageUnavailable)?;
        let snapshot = decode_namespace(&transaction, true)?;
        transaction
            .commit()
            .map_err(|_| RiskPersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }

    /// Reconstructs only the typed durable Risk H2a policy-denial replay
    /// authority. Callers that do not opt into this boundary cannot ignore it.
    pub fn load_risk_h2a_runtime_snapshot(
        &self,
    ) -> Result<RiskH2aRuntimeSnapshot, RiskPersistenceLoadError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(RiskPersistenceLoadError::UnsupportedSchema {
                found: self.schema_version,
            });
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| RiskPersistenceLoadError::StorageUnavailable)?;
        let base = decode_namespace(&transaction, false)?;
        let terminals = decode_v11_terminal_denials(&transaction)?;
        let prepared = decode_v13_prepared_previews(&transaction)?;
        let rejections = decode_risk_rejections(&transaction)?;
        let snapshot = RiskH2aPersistenceDecodeInput::new(base, terminals)
            .with_prepared(prepared)
            .with_rejections(rejections)
            .decode()
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        transaction
            .commit()
            .map_err(|_| RiskPersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }

    /// The preview a Risk H2a PREPARE already produced for this client
    /// request, if it produced one.
    ///
    /// A facade must ask this before minting anything: the host-minted
    /// prepared-intent id and, for an occurrence, the id of the Issue the
    /// occurrence will create are both compared against what the first
    /// attempt stored, and a preview cannot be rebuilt at a later instant
    /// because its expiry is part of the payload digest. `None` means this
    /// client request has not prepared anything here.
    pub fn risk_prepared_intent_for_client_request(
        &self,
        idempotency: &IdempotencyId,
    ) -> Result<Option<PreparedIntentId>, RiskPersistenceLoadError> {
        self.connection
            .query_row(
                "SELECT prepared_result_id FROM risk_h2a_v13_replay_operations WHERE idempotency_id=?1",
                [idempotency.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|_| RiskPersistenceLoadError::StorageUnavailable)?
            .flatten()
            .map(PreparedIntentId::parse)
            .transpose()
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)
    }

    pub fn create_risk(
        &mut self,
        command: CreateRisk,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<RiskMutationOutcome<RiskRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        self.with_immediate_transaction(|transaction| {
            Self::insert_created_risk(
                &mut transaction.transaction,
                &command,
                audit_event_id,
                occurred_at,
                expected_revision,
            )
        })
    }

    /// The record-entry create (schema v47): the Risk's id is the one the
    /// Ledger reserved for this idempotency id, checked again here inside the
    /// create transaction, so a create can only be reached through a
    /// reservation and a retry always names the same Risk. Everything else is
    /// [`Self::create_risk`].
    #[allow(clippy::too_many_arguments)]
    pub fn create_risk_from_reservation(
        &mut self,
        reserved: &ReservedId<RiskId>,
        title: RiskTitle,
        details: RiskDetails,
        classification: DataClassification,
        context: RiskOperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<RiskMutationOutcome<RiskRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&context)))?;
        if reserved.idempotency_id() != &context.idempotency_id
            || reserved.request()
                != (ReservationRequest {
                    kind: ReservedEntityKind::Risk,
                    operation: "create_risk",
                })
        {
            return Err(LedgerTransactionError::Operation(idempotency_conflict(
                &context,
            )));
        }
        let command = CreateRisk {
            id: reserved.id().clone(),
            title,
            details,
            classification,
            context: context.clone(),
        };
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if !reservation_matches(tx, reserved).map_err(|_| storage_error(&context))? {
                return Err(idempotency_conflict(&context));
            }
            Self::insert_created_risk(tx, &command, audit_event_id, occurred_at, expected_revision)
        })
    }

    /// H1 `UpdateRiskResponse` (schema v47): the same command as the domain's,
    /// persisted with its own typed command / result / audit replay rows. A
    /// retry with the same idempotency id and the same command returns the
    /// outcome the first attempt committed — the response it produced, its
    /// audit and correlation — however the Risk has moved since.
    pub fn update_risk_response(
        &mut self,
        command: UpdateRiskResponse,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<RiskMutationOutcome<RiskRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT risk_id,expected_version,response,owner_id,rationale,residual_exposure,next_review_at FROM risk_response_command_updates WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<i64>>(6)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let same = existing.0 == command.risk_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.response.as_persisted()
                    && existing.3.as_deref() == command.owner.as_ref().map(StakeholderId::as_str)
                    && existing.4.as_deref() == command.rationale.as_ref().map(RiskRationale::as_str)
                    && existing.5.as_deref() == command.residual_exposure.as_ref().map(ResidualExposure::as_str)
                    && existing.6 == command.next_review_at.map(UtcTimestamp::unix_millis);
                if same {
                    return risk_response_outcome_from_replay(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)", [context.idempotency_id.as_str()], |row| row.get::<_, i64>(0)).map_err(|_| storage_error(&context))? != 0 {
                return Err(idempotency_conflict(&context));
            }
            let Some(current) = tx
                .query_row(
                    "SELECT registry.version,risks.state,registry.classification FROM risks JOIN aggregate_registry registry ON registry.id=risks.id AND registry.aggregate_type='risk' WHERE risks.id=?1",
                    [command.risk_id.as_str()],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            else {
                return Err(not_found(&context));
            };
            if current.0 != i64::try_from(command.expected_version.get()).unwrap_or(-1) || current.1 != "open" {
                return Err(transition_conflict(&context));
            }
            // The column refuses an instant before the epoch; say so as a
            // field refusal rather than a storage failure.
            if command.next_review_at.is_some_and(|at| at.unix_millis() < 0) {
                return Err(DomainError::new(
                    ErrorCode::ValidationInvalidField,
                    MessageKey::parse("risk.next_review_before_epoch").unwrap_or_else(|_| unreachable!()),
                    context.correlation_id.clone(),
                    false,
                ));
            }
            let settled = matches!(command.response, RiskResponseType::Accept | RiskResponseType::Transfer);
            if settled
                && (command.owner.is_none()
                    || command.rationale.is_none()
                    || command.residual_exposure.is_none()
                    || command.next_review_at.is_none())
            {
                return Err(DomainError::new(
                    ErrorCode::ValidationInvalidField,
                    MessageKey::parse("risk.accepted_fields_required").unwrap_or_else(|_| unreachable!()),
                    context.correlation_id.clone(),
                    false,
                ));
            }
            if let Some(owner) = &command.owner {
                let known: i64 = tx.query_row("SELECT EXISTS(SELECT 1 FROM stakeholders WHERE id=?1)", [owner.as_str()], |row| row.get(0)).map_err(|_| storage_error(&context))?;
                if known == 0 {
                    return Err(DomainError::new(
                        ErrorCode::DomainNotFound,
                        MessageKey::parse("risk.owner_not_found").unwrap_or_else(|_| unreachable!()),
                        context.correlation_id.clone(),
                        false,
                    ));
                }
            }
            let in_exception_queue = i64::from(!settled);
            let new_version = current.0 + 1;
            let owner = command.owner.as_ref().map(StakeholderId::as_str);
            let rationale = command.rationale.as_ref().map(RiskRationale::as_str);
            let residual = command.residual_exposure.as_ref().map(ResidualExposure::as_str);
            let next_review = command.next_review_at.map(UtcTimestamp::unix_millis);
            tx.execute(
                "UPDATE risks SET response=?2,owner_id=?3,rationale=?4,residual_exposure=?5,next_review_at=?6,in_exception_queue=?7 WHERE id=?1",
                rusqlite::params![command.risk_id.as_str(), command.response.as_persisted(), owner, rationale, residual, next_review, in_exception_queue],
            ).map_err(|_| storage_error(&context))?;
            if tx.execute(
                "UPDATE aggregate_registry SET version=?2,updated_at=?3 WHERE id=?1 AND aggregate_type='risk' AND version=?4",
                rusqlite::params![command.risk_id.as_str(), new_version, occurred_at.unix_millis(), current.0],
            ).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            let audit = response_audit(audit_event_id, occurred_at, &command.risk_id, context.correlation_id.clone()).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management','risk.response_updated','risk',?3,?4,'allowed','not_required','succeeded','complete')", rusqlite::params![audit.id().as_str(), occurred_at.unix_millis(), command.risk_id.as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,'risk.response_updated','complete','risk',?2)", rusqlite::params![audit.id().as_str(), command.risk_id.as_str()]).map_err(|_| storage_error(&context))?;
            let ordinal: i64 = tx.query_row("SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_response_replay_operations", [], |row| row.get(0)).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_response_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_reference,result_classification,result_response,result_owner_id,result_rationale,result_residual_exposure,result_next_review_at,result_in_exception_queue,result_version) VALUES(?1,'update_risk_response',?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, command.risk_id.as_str(), current.2, command.response.as_persisted(), owner, rationale, residual, next_review, in_exception_queue, new_version],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_response_command_updates(idempotency_id,risk_id,expected_version,response,owner_id,rationale,residual_exposure,next_review_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![context.idempotency_id.as_str(), command.risk_id.as_str(), current.0, command.response.as_persisted(), owner, rationale, residual, next_review],
            ).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO risk_response_replay_audits(idempotency_id,audit_event_id,correlation_id) VALUES(?1,?2,?3)", rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![i64::try_from(expected_revision + 1).map_err(|_| storage_error(&context))?, i64::try_from(expected_revision).map_err(|_| storage_error(&context))?]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            risk_response_outcome_from_replay(tx, &context)
        })
    }

    /// The H1 create, inside the caller's transaction: the shared body of
    /// [`Self::create_risk`] and [`Self::create_risk_from_reservation`].
    fn insert_created_risk(
        tx: &mut Transaction<'_>,
        command: &CreateRisk,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
        expected_revision: u64,
    ) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
        let context = &command.context;
        if let Some(existing) = tx
                .query_row(
                    "SELECT command.risk_id,command.title,command.details,command.classification,replay.result_reference,replay.correlation_id FROM risk_replay_operations replay JOIN risk_command_creates command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?)),
                )
                .optional()
                .map_err(|_| storage_error(context))?
            {
                let (risk_id, title, details, classification, reference, correlation) = existing;
                if (risk_id, title, details, classification, reference) == (command.id.as_str().to_owned(), command.title.as_str().to_owned(), command.details.as_str().to_owned(), command.classification.as_persisted().to_owned(), command.id.as_str().to_owned()) {
                    // The outcome the first attempt committed, under its own
                    // correlation: a retry may carry a fresh one.
                    let original = RiskOperationContext {
                        idempotency_id: context.idempotency_id.clone(),
                        correlation_id: CorrelationId::parse(correlation).map_err(|_| storage_error(context))?,
                    };
                    return risk_outcome_from_snapshot(tx, &command.id, &original);
                }                return Err(idempotency_conflict(context));
            }
        if tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)",
                [context.idempotency_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| storage_error(context))?
            != 0
        {
            return Err(idempotency_conflict(context));
        }
        if command.classification == DataClassification::Unclassified {
            return Err(storage_error(context));
        }
        let exists: i64 = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
                [command.id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| storage_error(context))?;
        if exists != 0 {
            return Err(idempotency_conflict(context));
        }
        tx.execute("INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'risk',1,?2,?3,?3)", rusqlite::params![command.id.as_str(), command.classification.as_persisted(), occurred_at.unix_millis()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO risks(id,title,details,state,response,owner_id,rationale,residual_exposure,next_review_at,in_exception_queue) VALUES(?1,?2,?3,'open',NULL,NULL,NULL,NULL,NULL,1)", rusqlite::params![command.id.as_str(), command.title.as_str(), command.details.as_str()]).map_err(|_| storage_error(context))?;
        let audit = create_audit(audit_event_id, occurred_at, &command.id, context)?;
        tx.execute("INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management','risk.created','risk',?3,?4,'allowed','not_required','succeeded','complete')", rusqlite::params![audit.id().as_str(), occurred_at.unix_millis(), command.id.as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,'risk.created','complete','risk',?2)", rusqlite::params![audit.id().as_str(), command.id.as_str()]).map_err(|_| storage_error(context))?;
        let ordinal: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_replay_operations",
                [],
                |row| row.get(0),
            )
            .map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO risk_replay_operations(idempotency_id,correlation_id,operation_ordinal,result_reference) VALUES(?1,?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, command.id.as_str()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO risk_command_creates(idempotency_id,risk_id,title,details,classification) VALUES(?1,?2,?3,?4,?5)", rusqlite::params![context.idempotency_id.as_str(), command.id.as_str(), command.title.as_str(), command.details.as_str(), command.classification.as_persisted()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO risk_replay_audits(idempotency_id,audit_event_id,correlation_id) VALUES(?1,?2,?3)", rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
        if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![i64::try_from(expected_revision + 1).map_err(|_| storage_error(context))?, i64::try_from(expected_revision).map_err(|_| storage_error(context))?]).map_err(|_| storage_error(context))? != 1 { return Err(storage_error(context)); }
        risk_outcome_from_snapshot(tx, &command.id, context)
    }

    /// Persist one already-canonical H2a Risk occurrence preview.
    ///
    /// The domain service is the only authority that may create `prepared`.
    /// This adapter verifies its exact topology before atomically storing the
    /// typed command and replay records under V13. V9's own prepare tracking
    /// remains quarantined in full, so this is a fresh root, not a repair.
    pub fn prepare_record_risk_occurrence(
        &mut self,
        command: PrepareRecordRiskOccurrence,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_occurrence")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT risk_id,expected_version,issue_id,result_reference FROM risk_h2a_v13_replay_operations replay JOIN risk_h2a_v13_command_prepare_occurrences command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.risk_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.issue_id.as_str()
                    && existing.3.as_deref() == Some(prepared.id().as_str())
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let snapshot = decode_namespace(tx, true).map_err(|_| storage_error(&context))?;
            let Some(risk) = snapshot
                .risks()
                .iter()
                .find(|risk| risk.id() == &command.risk_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if risk.version() != command.expected_version || risk.state() != RiskState::Open {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::RecordRiskOccurrence {
                risk_id,
                risk_version,
                issue_id,
                issue_classification,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if risk_id != &command.risk_id
                || risk_version != &command.expected_version
                || issue_id != &command.issue_id
                || issue_classification != &risk.classification()
            {
                return Err(transition_conflict(&context));
            }
            let ordinal = next_risk_h2a_v13_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "record_risk_occurrence", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'risk',?2,?3)",
                rusqlite::params![prepared.id().as_str(), command.risk_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,created_id,created_classification) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![prepared.id().as_str(), command.risk_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.issue_id.as_str(), issue_classification.as_persisted()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_h2a_v13_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_occurrence',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_h2a_v13_command_prepare_occurrences (idempotency_id,risk_id,expected_version,issue_id) VALUES (?1,?2,?3,?4)",
                rusqlite::params![context.idempotency_id.as_str(), command.risk_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.issue_id.as_str()],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// Persist one already-canonical H2a Risk close preview. See
    /// [`Self::prepare_record_risk_occurrence`] for the shared V13 rationale.
    pub fn prepare_close_risk(
        &mut self,
        command: PrepareCloseRisk,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_close")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT risk_id,expected_version,rationale,result_reference FROM risk_h2a_v13_replay_operations replay JOIN risk_h2a_v13_command_prepare_closes command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.risk_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.rationale.as_str()
                    && existing.3.as_deref() == Some(prepared.id().as_str())
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let snapshot = decode_namespace(tx, true).map_err(|_| storage_error(&context))?;
            let Some(risk) = snapshot
                .risks()
                .iter()
                .find(|risk| risk.id() == &command.risk_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if risk.version() != command.expected_version || risk.state() != RiskState::Open {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::CloseRisk {
                risk_id,
                risk_version,
                rationale,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if risk_id != &command.risk_id
                || risk_version != &command.expected_version
                || rationale != &command.rationale
            {
                return Err(transition_conflict(&context));
            }
            let ordinal = next_risk_h2a_v13_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "close_risk", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'risk',?2,?3)",
                rusqlite::params![prepared.id().as_str(), command.risk_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,rationale) VALUES (?1,?2,?3,?4)",
                rusqlite::params![prepared.id().as_str(), command.risk_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.rationale.as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_h2a_v13_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_close',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_h2a_v13_command_prepare_closes (idempotency_id,risk_id,expected_version,rationale) VALUES (?1,?2,?3,?4)",
                rusqlite::params![context.idempotency_id.as_str(), command.risk_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.rationale.as_str()],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// v46: durable, audited rejection of an outstanding Risk H2a preview
    /// (record-occurrence or close). Same contract as
    /// `action_repository.rs`'s `reject_action_prepared_intent`: the intent
    /// is consumed at the rejection instant, one zero-effect audit is
    /// recorded, no receipt is minted, no aggregate changes, and the
    /// `risk_reject_prepared_command_results` row claims the idempotency id
    /// on its own per-table ordinal stream (see the v46 schema note).
    ///
    /// A preview consumed by anything other than a rejection -- an execute
    /// or a terminal denial -- is refused as a lifecycle conflict here,
    /// before the domain sees it, because the outstanding-preview decoders
    /// deliberately forget consumed intents.
    pub fn reject_risk_prepared_intent<Z>(
        &mut self,
        command: RejectRiskPreparedIntent,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
        authorization: Z,
    ) -> Result<RejectedPreparedIntentOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let replaying = tx
                .query_row(
                    "SELECT prepared_intent_id,actor FROM risk_reject_prepared_command_results WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            match &replaying {
                Some((stored_prepared, stored_actor)) => {
                    if stored_prepared != command.prepared_id.as_str()
                        || stored_actor != command.actor.as_persisted()
                    {
                        return Err(idempotency_conflict(&context));
                    }
                }
                None => {
                    if tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)",
                            [context.idempotency_id.as_str()],
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(|_| storage_error(&context))?
                        != 0
                    {
                        return Err(idempotency_conflict(&context));
                    }
                    if consumed_other_than_by_rejection(
                        tx,
                        &command.prepared_id,
                        "('record_risk_occurrence','close_risk')",
                        "risk_reject_prepared_command_results",
                    )
                    .map_err(|_| storage_error(&context))?
                    {
                        return Err(transition_conflict(&context));
                    }
                }
            }
            let risk_snapshot =
                decode_risk_h2a_runtime_snapshot(tx).map_err(|_| storage_error(&context))?;
            // Risk-only rehydration on purpose: a rejection stages nothing,
            // so it needs no standalone Issue authority -- and taking the
            // Issue H1 snapshot would make this operation unavailable for
            // good the moment any standalone Issue leaves its pristine
            // create state (a classification lowering is enough).
            let mut service = InMemoryRiskService::rehydrate_with_h2a(
                FixedClock(occurred_at),
                ExecuteRiskIds {
                    receipt_id: None,
                    audit_ids: [Some(audit_event_id.clone()), None, None],
                    next_audit: 0,
                },
                authorization,
                RejectRiskPolicy,
                AllowRiskEvidence,
                RecordedRiskClassification,
                risk_snapshot,
            )
            .map_err(|_| storage_error(&context))?;
            let outcome = service.reject_risk_prepared_intent(command.clone())?;
            if replaying.is_some() {
                return Ok(outcome);
            }
            if outcome.audit_event().id() != &audit_event_id {
                return Err(storage_error(&context));
            }
            let ordinal = next_risk_reject_prepared_operation_ordinal(tx, &context)?;
            persist_risk_prepared_intent_rejection(tx, &context, ordinal, &outcome)?;
            decode_risk_h2a_runtime_snapshot(tx).map_err(|_| storage_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| storage_error(&context))?;
            Ok(outcome)
        })
    }

    /// Execute one explicit H2a approval against an exact persisted V13 Risk
    /// occurrence preview. The canonical domain service (rehydrated through
    /// the shared Risk+Issue authority so occurrence's Issue creation cannot
    /// collide with an existing identity) validates the human binding before
    /// this adapter writes its normalized commit bundle -- or, on the one
    /// accepted durable post-start denial, a V11 terminal replay row.
    ///
    /// `audit_event_ids` supplies exactly the ids the domain service may
    /// consume: 3 for a successful Occurred outcome (`risk.occurred`,
    /// `issue.created_from_risk`, `risk.issue_linked`), of which only the
    /// first is ever drawn on the single-audit denial path.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_record_risk_occurrence<Z, RP, RE, RC>(
        &mut self,
        command: ApproveAndExecuteRecordRiskOccurrence,
        audit_event_ids: [AuditEventId; 3],
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
    ) -> Result<OccurredRiskOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        RP: RiskExecutionPolicyPort,
        RE: RiskEvidenceAuthorityPort,
        RC: RiskClassificationAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        // The inner Result carries the domain outcome; the outer Result controls
        // whether this transaction commits. A durable denial must still commit
        // (its V11 row is the whole point), so it is `Ok(Err(..))`, not `Err(..)`
        // -- returning `Err` here would roll back everything just written.
        match self.with_immediate_transaction(|transaction| {
                let tx = &mut transaction.transaction;
                if idempotency_claimed_by_other_operation(tx, &context, "execute_occurrence")? {
                    return Err(idempotency_conflict(&context));
                }
                if let Some(existing) = tx
                    .query_row(
                        "SELECT prepared_id,actor,acknowledged_digest FROM risk_h2a_v14_command_execute_occurrences WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                    )
                    .optional()
                    .map_err(|_| storage_error(&context))?
                {
                    if existing.0 != command.approval.prepared_id().as_str()
                        || existing.1 != command.approval.actor().as_persisted()
                        || existing.2 != command.approval.acknowledged_payload_digest().as_str()
                    {
                        return Err(idempotency_conflict(&context));
                    }
                    return replay_occurred_outcome(tx, &context).map(Ok);
                }
                let risk_snapshot =
                    decode_risk_h2a_runtime_snapshot(tx).map_err(|_| storage_error(&context))?;
                // Every Issue at its current state, not only the pristine
                // standalone ones: occurrence stages a new Issue and the
                // identity authority must be able to see every existing one,
                // whatever lifecycle state it has reached.
                let issue_snapshot =
                    super::issue_repository::decode_issue_h2a_runtime_snapshot(tx)
                        .map_err(|_| storage_error(&context))?;
                let mut composition = WorkManagementRuntimeComposition::rehydrate_with_risk_h2a_and_issue_h2a(
                    FixedClock(occurred_at),
                    ExecuteRiskIds {
                        receipt_id: Some(approval_receipt_id.clone()),
                        audit_ids: audit_event_ids.clone().map(Some),
                        next_audit: 0,
                    },
                    authorization.clone(),
                    risk_policy,
                    risk_evidence,
                    risk_classification,
                    FixedClock(occurred_at),
                    UnusedIssueIds,
                    authorization,
                    AllowIssueExecution,
                    DenyIssueEvidenceAuthority,
                    RecordedIssueClassification,
                    risk_snapshot,
                    issue_snapshot,
                )
                .map_err(|_| storage_error(&context))?;
                let (result, durable) = composition
                    .approve_and_execute_record_risk_occurrence_with_durability(command.clone());
                match result {
                    Ok(outcome) => {
                        if outcome.audit_events.len() != 3
                            || outcome.approval_receipt_id != approval_receipt_id
                        {
                            return Err(storage_error(&context));
                        }
                        let ordinal = next_risk_h2a_v14_operation_ordinal(tx, &context)?;
                        persist_occurred_bundle(tx, &command.approval, &outcome, ordinal, &context)?;
                        let expected_revision = i64::try_from(expected_revision)
                            .map_err(|_| storage_error(&context))?;
                        if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                            return Err(storage_error(&context));
                        }
                        Ok(Ok(outcome))
                    }
                    Err(error) => {
                        if durable {
                            persist_v11_terminal_denial(
                                tx,
                                &command.approval,
                                "record_occurrence",
                                "risk.occurrence_denied",
                                &audit_event_ids[0],
                                occurred_at,
                                &error,
                                &context,
                            )?;
                        }
                        Ok(Err(error))
                    }
                }
            }) {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(domain_error)) => Err(LedgerTransactionError::Operation(domain_error)),
            Err(e) => Err(e),
        }
    }

    /// Execute one explicit H2a approval against an exact persisted V13 Risk
    /// close preview. See
    /// [`Self::approve_and_execute_record_risk_occurrence`] for the shared
    /// rationale; close never touches the Issue side.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_close_risk<Z, RP, RE, RC>(
        &mut self,
        command: ApproveAndExecuteCloseRisk,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
    ) -> Result<RiskMutationOutcome<RiskRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        RP: RiskExecutionPolicyPort,
        RE: RiskEvidenceAuthorityPort,
        RC: RiskClassificationAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        // See approve_and_execute_record_risk_occurrence: the inner Result
        // carries the domain outcome, the outer Result controls the commit. A
        // durable denial is `Ok(Err(..))` so its V11 row still commits.
        match self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_close")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM risk_h2a_v14_command_execute_closes WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 != command.approval.prepared_id().as_str()
                    || existing.1 != command.approval.actor().as_persisted()
                    || existing.2 != command.approval.acknowledged_payload_digest().as_str()
                {
                    return Err(idempotency_conflict(&context));
                }
                return replay_closed_outcome(tx, &context).map(Ok);
            }
            let risk_snapshot = decode_risk_h2a_runtime_snapshot(tx).map_err(|_| storage_error(&context))?;
            // Risk-only rehydration on purpose: closing a Risk creates no
            // Issue, so it needs no standalone Issue authority -- and taking
            // the Issue H1 snapshot would make Close unavailable for good the
            // moment any standalone Issue leaves its pristine create state.
            let mut service = InMemoryRiskService::rehydrate_with_h2a(
                FixedClock(occurred_at),
                ExecuteRiskIds {
                    receipt_id: Some(approval_receipt_id.clone()),
                    audit_ids: [Some(audit_event_id.clone()), None, None],
                    next_audit: 0,
                },
                authorization,
                risk_policy,
                risk_evidence,
                risk_classification,
                risk_snapshot,
            )
            .map_err(|_| storage_error(&context))?;
            let (result, durable) =
                service.approve_and_execute_close_risk_with_durability(command.clone());
            match result {
                Ok(outcome) => {
                    if outcome.audit_events.len() != 1
                        || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
                    {
                        return Err(storage_error(&context));
                    }
                    let ordinal = next_risk_h2a_v14_operation_ordinal(tx, &context)?;
                    persist_closed_bundle(tx, &command.approval, &outcome, ordinal, &context)?;
                    let expected_revision =
                        i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
                    if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                        return Err(storage_error(&context));
                    }
                    Ok(Ok(outcome))
                }
                Err(error) => {
                    if durable {
                        persist_v11_terminal_denial(
                            tx,
                            &command.approval,
                            "close",
                            "risk.close_denied",
                            &audit_event_id,
                            occurred_at,
                            &error,
                            &context,
                        )?;
                    }
                    Ok(Err(error))
                }
            }
        }) {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(domain_error)) => Err(LedgerTransactionError::Operation(domain_error)),
            Err(e) => Err(e),
        }
    }

    /// H2a step 1: persist one already-canonical Risk
    /// classification-lowering preview. Mirrors `prepare_close_risk` exactly
    /// (no service reconstruction -- the domain service is the only
    /// authority that may create `prepared`, this adapter re-verifies its
    /// exact topology against durable state before storing it).
    pub fn prepare_lower_risk_classification(
        &mut self,
        command: PrepareLowerRiskClassification,
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
                    "SELECT risk_id,expected_version,proposed_classification,rationale,result_reference FROM risk_h2a_lower_classification_prepare_replay_operations replay JOIN risk_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.risk_id.as_str()
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
            let snapshot = decode_namespace(tx, true).map_err(|_| storage_error(&context))?;
            let Some(risk) = snapshot
                .risks()
                .iter()
                .find(|risk| risk.id() == &command.risk_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if risk.version() != command.expected_version {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::LowerRiskClassification {
                risk_id,
                risk_version,
                current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if risk_id != &command.risk_id
                || risk_version != &command.expected_version
                || current_classification != &risk.classification()
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(transition_conflict(&context));
            }
            let ordinal = next_risk_h2a_lower_classification_prepare_operation_ordinal(tx, &context)?;
            persist_prepared_intent(tx, &prepared, "lower_risk_classification", &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'risk',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.risk_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_h2a_lower_classification_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO risk_h2a_lower_classification_command_prepares (idempotency_id,risk_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.risk_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// H2a step 2 for Risk. See `approve_and_execute_close_risk`
    /// for the shared rehydrate/durability-tracking rationale. Constructs
    /// `InMemoryRiskService` directly via `rehydrate_with_h2a` rather than
    /// going through `WorkManagementRuntimeComposition` -- unlike Close and
    /// RecordRiskOccurrence, this operation never touches Issue, and the
    /// composition exposes no forwarding method for it (private `risks`
    /// field, no wrapper), so there is nothing to gain from routing through
    /// it.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_lower_risk_classification<Z, RP, RE, RC>(
        &mut self,
        command: ApproveAndExecuteLowerRiskClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        risk_policy: RP,
        risk_evidence: RE,
        risk_classification: RC,
    ) -> Result<RiskMutationOutcome<RiskRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        RP: RiskExecutionPolicyPort,
        RE: RiskEvidenceAuthorityPort,
        RC: RiskClassificationAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        match self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "execute_lower_classification")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM risk_h2a_lower_classification_command_executes WHERE idempotency_id=?1",
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
                return replay_lowered_risk_outcome(tx, &context).map(Ok);
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_intent_id,acknowledged_digest FROM risk_h2a_lower_classification_terminal_denials WHERE execute_idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 != command.approval.prepared_id().as_str()
                    || existing.1 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                return replay_lower_classification_terminal_denial(tx, &context).map(Err);
            }
            let risk_snapshot =
                decode_risk_h2a_runtime_snapshot(tx).map_err(|_| storage_error(&context))?;
            let mut service = InMemoryRiskService::rehydrate_with_h2a(
                FixedClock(occurred_at),
                ExecuteRiskIds {
                    receipt_id: Some(approval_receipt_id.clone()),
                    audit_ids: [Some(audit_event_id.clone()), None, None],
                    next_audit: 0,
                },
                authorization,
                risk_policy,
                risk_evidence,
                risk_classification,
                risk_snapshot,
            )
            .map_err(|_| storage_error(&context))?;
            let (result, durable) =
                service.approve_and_execute_lower_risk_classification_with_durability(command.clone());
            match result {
                Ok(outcome) => {
                    if outcome.audit_events.len() != 1
                        || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
                    {
                        return Err(storage_error(&context));
                    }
                    let ordinal =
                        next_risk_h2a_lower_classification_execute_operation_ordinal(tx, &context)?;
                    persist_lowered_risk_bundle(tx, &command.approval, &outcome, ordinal, &context)?;
                    let expected_revision =
                        i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
                    let rows_changed = tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))?;
                    if rows_changed != 1 {
                        return Err(storage_error(&context));
                    }
                    Ok(Ok(outcome))
                }
                Err(error) => {
                    if durable {
                        persist_lower_classification_terminal_denial(
                            tx,
                            &command.approval,
                            &audit_event_id,
                            occurred_at,
                            &error,
                            &context,
                        )?;
                    }
                    Ok(Err(error))
                }
            }
        }) {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(domain_error)) => Err(LedgerTransactionError::Operation(domain_error)),
            Err(e) => Err(e),
        }
    }
}

fn decode_namespace(
    tx: &Transaction<'_>,
    reject_v11_terminal_denials: bool,
) -> Result<RiskPersistenceSnapshot, RiskPersistenceLoadError> {
    // v9 created the initial Risk H2a tables, but its replay authority is
    // deliberately quarantined until the forward-only binding repair lands.
    // Never infer a terminal Risk state from an unbound preview/receipt row.
    let untrusted_h2a_rows: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM risk_h2a_replay_operations)",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    if untrusted_h2a_rows != 0 {
        return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
    }
    // V11 has a separately typed restart loader under construction.  The
    // ordinary Risk snapshot must not silently omit a durable terminal denial
    // while that loader is unavailable.
    let v11_terminal_rows: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM risk_h2a_v11_terminal_denials)",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    if reject_v11_terminal_denials && v11_terminal_rows != 0 {
        return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
    }
    let records = tx.prepare("SELECT risks.id,risks.title,risks.details,registry.classification,registry.version,risks.state,risks.response,risks.owner_id,risks.rationale,risks.residual_exposure,risks.next_review_at,risks.in_exception_queue FROM risks JOIN aggregate_registry registry ON registry.id=risks.id AND registry.aggregate_type='risk' ORDER BY risks.id").map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, i64>(4)?,row.get::<_, String>(5)?,row.get::<_, Option<String>>(6)?,row.get::<_, Option<String>>(7)?,row.get::<_, Option<String>>(8)?,row.get::<_, Option<String>>(9)?,row.get::<_, Option<i64>>(10)?,row.get::<_, i64>(11)?)))
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?.collect::<Result<Vec<_>, _>>().map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    let mut risks = Vec::with_capacity(records.len());
    for row in records {
        let id = RiskId::parse(row.0).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        let title =
            RiskTitle::parse(row.1).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        let details =
            RiskDetails::parse(row.2).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        let classification = DataClassification::from_persisted(&row.3)
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        let version = pmc_domain::identity::AggregateVersion::new(
            u64::try_from(row.4).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
        )
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        // A Risk whose response was recorded (v47 `update_risk_response`) is
        // rehydrated from its typed replay rows: the attributes on the row
        // must be exactly what the last update stored, the version must
        // account for every update, lowering and transition, and a
        // transition must still have its execute proof. Never from the
        // mutable columns alone.
        // Each update's whole topology must be there and agree with itself:
        // the result row, its typed command (naming this Risk, producing
        // `expected_version + 1`, with the same attributes), its audit
        // binding to a `risk.response_updated` event on this Risk under the
        // same correlation, and its idempotency claim. Every `result_*`
        // column is mutable, so a result that lost its command or was
        // renumbered is refused rather than trusted.
        let response_updates: Vec<ResponseResultRow> = tx
            .prepare("SELECT replay.result_response,replay.result_owner_id,replay.result_rationale,replay.result_residual_exposure,replay.result_next_review_at,replay.result_in_exception_queue,replay.result_version FROM risk_response_replay_operations replay JOIN risk_response_command_updates command ON command.idempotency_id=replay.idempotency_id AND command.risk_id=replay.result_reference AND command.expected_version+1=replay.result_version AND command.response=replay.result_response AND command.owner_id IS replay.result_owner_id AND command.rationale IS replay.result_rationale AND command.residual_exposure IS replay.result_residual_exposure AND command.next_review_at IS replay.result_next_review_at JOIN risk_response_replay_audits replay_audit ON replay_audit.idempotency_id=replay.idempotency_id AND replay_audit.correlation_id=replay.correlation_id JOIN audit_events audit ON audit.id=replay_audit.audit_event_id AND audit.correlation_id=replay.correlation_id AND audit.event_code='risk.response_updated' AND audit.target_type='risk' AND audit.target_id=replay.result_reference JOIN ledger_idempotency_claims claim ON claim.idempotency_id=replay.idempotency_id AND claim.namespace='risk' AND claim.operation='update_risk_response' WHERE replay.result_reference=?1 AND EXISTS(SELECT 1 FROM audit_effects effect WHERE effect.audit_event_id=audit.id AND effect.ordinal=0 AND effect.effect_code='risk.response_updated' AND effect.target_id=replay.result_reference) ORDER BY replay.result_version")
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
            .query_map([id.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        // A result row without its full topology is not a partial update: it
        // is an invalid Ledger.
        let result_rows: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM risk_response_replay_operations WHERE result_reference=?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        if usize::try_from(result_rows)
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
            != response_updates.len()
        {
            return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
        }
        if let Some(last) = response_updates.last() {
            let state = RiskState::from_persisted(&row.5)
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            if row.6.as_deref() != Some(last.0.as_str())
                || row.7 != last.1
                || row.8 != last.2
                || row.9 != last.3
                || row.10 != last.4
            {
                return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
            }
            let expected_in_exception_queue = if state == RiskState::Closed {
                0
            } else {
                last.5
            };
            if row.11 != expected_in_exception_queue {
                return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
            }
            let lowering_rows: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM risk_h2a_lower_classification_execute_replay_operations WHERE risk_id=?1",
                    [id.as_str()],
                    |r| r.get(0),
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            let transitions: i64 = if state == RiskState::Open {
                0
            } else {
                tx.query_row(
                    "SELECT COUNT(*) FROM risk_h2a_v14_replay_operations WHERE risk_id=?1 AND result_kind=?2",
                    rusqlite::params![id.as_str(), row.5.as_str()],
                    |r| r.get(0),
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
            };
            // Versions form a chain: every update's result version is greater
            // than the last, and the newest is at most the Risk's own version
            // (a transition after it may have moved it once more).
            let mut previous = 1_i64;
            for update in &response_updates {
                if update.6 <= previous {
                    return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
                }
                previous = update.6;
            }
            if (state != RiskState::Open && transitions != 1)
                || last.6 + transitions
                    != i64::try_from(version.get())
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
                || i64::try_from(version.get())
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
                    != 1 + lowering_rows
                        + i64::try_from(response_updates.len())
                            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
                        + transitions
            {
                return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
            }
            let risk = RiskRecord::from_persisted_responded(
                id,
                title,
                details,
                classification,
                version,
                state,
                RiskResponseType::from_persisted(&last.0)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                last.1
                    .clone()
                    .map(StakeholderId::parse)
                    .transpose()
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                last.2
                    .clone()
                    .map(RiskRationale::parse)
                    .transpose()
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                last.3
                    .clone()
                    .map(ResidualExposure::parse)
                    .transpose()
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                last.4.map(UtcTimestamp::from_unix_millis),
            )
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            risks.push(risk);
            continue;
        }
        if row.6.is_some()
            || row.7.is_some()
            || row.8.is_some()
            || row.9.is_some()
            || row.10.is_some()
        {
            return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
        }
        // `from_persisted_occurred_from_created_open`/`_closed_from_created_open` are the
        // narrow shape this reader accepts: a fresh Open Risk transitioned exactly once, with
        // no accumulated response attributes (checked above). Occurred keeps
        // `in_exception_queue=true`; Closed clears it -- unlike Open/Occurred, a Closed Risk is
        // no longer live review work.
        let expected_in_exception_queue = i64::from(row.5 != "closed");
        if row.11 != expected_in_exception_queue {
            return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
        }
        // An occurred or closed Risk needs its typed V14 execute-replay row as durable proof of
        // a legitimate H2a transition -- never infer occurrence/closure from the mutable
        // `risks.state` column alone, which a tampered row could set directly.
        let risk = match row.5.as_str() {
            "open" if version == pmc_domain::identity::AggregateVersion::initial() => {
                RiskRecord::from_persisted_created_open(id, title, details, classification, version)
            }
            // An Open Risk past its initial version has had its
            // classification governed-lowered one or more times (H2a
            // `LowerRiskClassification` is the only operation that
            // advances an Open Risk's version without a lifecycle
            // transition) -- never infer that from the mutable `risks.state`
            // column alone; require exactly one durable typed execute row
            // per version advance, matching the occurred/closed proof below.
            "open" => {
                let lowering_rows: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM risk_h2a_lower_classification_execute_replay_operations WHERE risk_id=?1",
                        [id.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let expected_lowerings = version.get() - 1;
                if u64::try_from(lowering_rows).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
                    != expected_lowerings
                {
                    Err(pmc_domain::risks::RiskRehydrationError::InvalidCreatedRisk)
                } else {
                    RiskRecord::from_persisted_open_with_lowered_classification(
                        id,
                        title,
                        details,
                        classification,
                        version,
                    )
                }
            }
            "occurred" | "closed" => {
                let result_kind = if row.5 == "occurred" { "occurred" } else { "closed" };
                let execute_rows: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM risk_h2a_v14_replay_operations WHERE risk_id=?1 AND result_kind=?2",
                        rusqlite::params![id.as_str(), result_kind],
                        |r| r.get(0),
                    )
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                if execute_rows != 1 {
                    Err(pmc_domain::risks::RiskRehydrationError::InvalidCreatedRisk)
                } else if row.5 == "occurred" {
                    RiskRecord::from_persisted_occurred_from_created_open(
                        id,
                        title,
                        details,
                        classification,
                        version,
                    )
                } else {
                    RiskRecord::from_persisted_closed_from_created_open(
                        id,
                        title,
                        details,
                        classification,
                        version,
                    )
                }
            }
            _ => Err(pmc_domain::risks::RiskRehydrationError::InvalidCreatedRisk),
        }
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
        risks.push(risk);
    }
    let orphaned_risk_issue: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM issues issue WHERE issue.source_risk_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM risk_issue_links link WHERE link.issue_id=issue.id))",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    if orphaned_risk_issue != 0 {
        return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
    }
    let issue_rows = tx.prepare("SELECT issue.id,issue.source_risk_id,issue.title,issue.details,registry.classification,registry.version,issue.state,issue.recurrence_of_id,issue.resolution_type,issue.resolution_rationale FROM issues issue JOIN risk_issue_links link ON link.issue_id=issue.id JOIN aggregate_registry registry ON registry.id=issue.id AND registry.aggregate_type='issue' ORDER BY issue.id").map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, Option<String>>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, String>(4)?,row.get::<_, i64>(5)?,row.get::<_, String>(6)?,row.get::<_, Option<String>>(7)?,row.get::<_, Option<String>>(8)?,row.get::<_, Option<String>>(9)?)))
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?.collect::<Result<Vec<_>, _>>().map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    let mut issues = Vec::with_capacity(issue_rows.len());
    for row in issue_rows {
        if row.6 != "open" || row.7.is_some() || row.8.is_some() || row.9.is_some() {
            return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
        }
        issues.push(
            pmc_domain::issues::IssueRecord::from_persisted_created_from_risk(
                pmc_domain::identity::IssueId::parse(row.0)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                RiskId::parse(row.1.ok_or(RiskPersistenceLoadError::InvalidRiskSnapshot)?)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                pmc_domain::issues::IssueTitle::parse(row.2)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                pmc_domain::issues::IssueDetails::parse(row.3)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                DataClassification::from_persisted(&row.4)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                pmc_domain::identity::AggregateVersion::new(
                    u64::try_from(row.5)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
            )
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
        );
    }
    let links = tx.prepare("SELECT risk_id,issue_id,risk_version,issue_version,classification FROM risk_issue_links ORDER BY risk_id,issue_id").map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?,row.get::<_, i64>(3)?,row.get::<_, String>(4)?)))
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?.collect::<Result<Vec<_>, _>>().map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .into_iter().map(|row| pmc_domain::risks::RiskIssueLink::from_persisted(RiskId::parse(row.0).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?, pmc_domain::identity::IssueId::parse(row.1).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?, pmc_domain::identity::AggregateVersion::new(u64::try_from(row.2).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?, pmc_domain::identity::AggregateVersion::new(u64::try_from(row.3).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?, DataClassification::from_persisted(&row.4).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)).collect::<Result<Vec<_>,_>>()?;
    RiskPersistenceSnapshot::try_new(risks, issues, links)
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)
}

fn decode_v11_terminal_denials(
    tx: &Transaction<'_>,
) -> Result<Vec<RiskH2aTerminalReplay>, RiskPersistenceLoadError> {
    let rows = tx
        .prepare("SELECT denial.execute_idempotency_id,denial.prepared_intent_id,denial.risk_id,denial.risk_version,denial.operation,denial.acknowledged_digest,denial.correlation_id,denial.error_code,denial.error_message_key,denial.error_retryable,prepared.payload_digest,prepared.classification,prepared.expires_at,prepared.created_at,payload.primary_id,payload.primary_version,payload.created_id,payload.created_classification,payload.rationale,audit.id,audit.occurred_at,audit.event_code FROM risk_h2a_v11_terminal_denials denial JOIN prepared_intents prepared ON prepared.id=denial.prepared_intent_id JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=denial.prepared_intent_id JOIN audit_events audit ON audit.id=denial.policy_denial_audit_id ORDER BY denial.execute_idempotency_id")
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?, row.get::<_, i64>(9)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?, row.get::<_, i64>(12)?, row.get::<_, i64>(13)?, row.get::<_, String>(14)?, row.get::<_, i64>(15)?, row.get::<_, Option<String>>(16)?, row.get::<_, Option<String>>(17)?, row.get::<_, Option<String>>(18)?, row.get::<_, String>(19)?, row.get::<_, i64>(20)?, row.get::<_, String>(21)?
        )))
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    rows.into_iter()
        .map(|row| {
            if row.7 != "SECURITY_POLICY_DENIED"
                || row.9 != 0
                || row.14 != row.2
                || row.15 != row.3
                || row.5 != row.10
            {
                return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
            }
            let risk_id =
                RiskId::parse(row.2).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            let version = pmc_domain::identity::AggregateVersion::new(
                u64::try_from(row.3).map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
            )
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            let classification = DataClassification::from_persisted(&row.11)
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            let operation = match row.4.as_str() {
                "record_occurrence" => WorkManagementOperation::RecordRiskOccurrence {
                    risk_id: risk_id.clone(),
                    risk_version: version,
                    issue_id: pmc_domain::identity::IssueId::parse(
                        row.16
                            .ok_or(RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                    )
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                    issue_classification: DataClassification::from_persisted(
                        &row.17
                            .ok_or(RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                    )
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                },
                "close" => WorkManagementOperation::CloseRisk {
                    risk_id: risk_id.clone(),
                    risk_version: version,
                    rationale: WorkManagementRationale::parse(
                        row.18
                            .ok_or(RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                    )
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                },
                _ => return Err(RiskPersistenceLoadError::InvalidRiskSnapshot),
            };
            let prepared = WorkManagementPreparedIntent::prepare(
                PreparedIntentId::parse(row.1)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                operation,
                classification,
                None,
                UtcTimestamp::from_unix_millis(row.13),
            )
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            if prepared.payload_digest().as_str() != row.10
                || prepared.preview().expires_at().unix_millis() != row.12
            {
                return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
            }
            let context = RiskOperationContext {
                idempotency_id: IdempotencyId::parse(row.0)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                correlation_id: CorrelationId::parse(row.6)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
            };
            let approval = WorkManagementApproval::new(
                prepared.id().clone(),
                AuditActor::HeadOfProducts,
                prepared.payload_digest().clone(),
                context.idempotency_id.clone(),
                Some(ApprovalConfirmation::Confirmed),
            )
            .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
            let audit = AuditEvent::new(
                AuditEventId::parse(row.19)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                UtcTimestamp::from_unix_millis(row.20),
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    AuditEventCode::parse(row.21)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                    AuditTarget::Risk(risk_id.clone()),
                ),
                context.correlation_id.clone(),
                AuditDisposition::new(
                    AuditPolicyOutcome::Denied,
                    AuditApprovalOutcome::NotRequired,
                    AuditExecutionOutcome::NotAttempted,
                    AuditEffectScope::None,
                    vec![],
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
            );
            let error = DomainError::new(
                ErrorCode::SecurityPolicyDenied,
                MessageKey::parse(row.8)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                context.correlation_id.clone(),
                false,
            );
            Ok(RiskH2aTerminalReplay::new(
                if row.4 == "record_occurrence" {
                    RiskH2aTerminalOperation::RecordOccurrence
                } else {
                    RiskH2aTerminalOperation::Close
                },
                risk_id,
                prepared,
                approval,
                context,
                error,
                audit,
            ))
        })
        .collect()
}

/// Reconstructs outstanding, not-yet-consumed V13 Risk H2a previews so they
/// survive a restart. A prepared intent that has since been consumed (denied
/// and terminalized under V11, or executed) is excluded via the shared
/// `prepared_intents.consumed_at` marker rather than a Risk-specific check.
fn decode_v13_prepared_previews(
    tx: &Transaction<'_>,
) -> Result<Vec<WorkManagementPreparedIntent>, RiskPersistenceLoadError> {
    let mut prepared = Vec::new();
    prepared.extend(decode_v13_prepared_occurrences(tx)?);
    prepared.extend(decode_v13_prepared_closes(tx)?);
    Ok(prepared)
}

fn decode_v13_prepared_occurrences(
    tx: &Transaction<'_>,
) -> Result<Vec<WorkManagementPreparedIntent>, RiskPersistenceLoadError> {
    let rows = tx
        .prepare("SELECT intent.id,intent.contract_version,intent.payload_digest,intent.classification,intent.expires_at,target.target_id,target.expected_version,payload.created_id,payload.created_classification FROM risk_h2a_v13_replay_operations replay JOIN risk_h2a_v13_command_prepare_occurrences command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference JOIN prepared_intent_targets target ON target.prepared_intent_id=intent.id AND target.ordinal=0 JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id WHERE replay.operation='prepare_occurrence' AND replay.result_kind='prepared' AND intent.consumed_at IS NULL ORDER BY intent.id")
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    rows.into_iter()
        .map(
            |(
                intent_id,
                contract_version,
                payload_digest,
                classification,
                expires_at,
                target_risk_id,
                target_expected_version,
                created_issue_id,
                created_issue_classification,
            )| {
                let risk_id = RiskId::parse(target_risk_id)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let risk_version = pmc_domain::identity::AggregateVersion::new(
                    u64::try_from(target_expected_version)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let issue_id = pmc_domain::identity::IssueId::parse(created_issue_id)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let issue_classification =
                    DataClassification::from_persisted(&created_issue_classification)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let classification = DataClassification::from_persisted(&classification)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let operation = WorkManagementOperation::RecordRiskOccurrence {
                    risk_id,
                    risk_version,
                    issue_id,
                    issue_classification,
                };
                build_and_verify_prepared_intent(
                    intent_id,
                    operation,
                    classification,
                    contract_version,
                    payload_digest,
                    expires_at,
                )
            },
        )
        .collect()
}

fn decode_v13_prepared_closes(
    tx: &Transaction<'_>,
) -> Result<Vec<WorkManagementPreparedIntent>, RiskPersistenceLoadError> {
    let rows = tx
        .prepare("SELECT intent.id,intent.contract_version,intent.payload_digest,intent.classification,intent.expires_at,target.target_id,target.expected_version,payload.rationale FROM risk_h2a_v13_replay_operations replay JOIN risk_h2a_v13_command_prepare_closes command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference JOIN prepared_intent_targets target ON target.prepared_intent_id=intent.id AND target.ordinal=0 JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id WHERE replay.operation='prepare_close' AND replay.result_kind='prepared' AND intent.consumed_at IS NULL ORDER BY intent.id")
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    rows.into_iter()
        .map(
            |(
                intent_id,
                contract_version,
                payload_digest,
                classification,
                expires_at,
                target_risk_id,
                target_expected_version,
                rationale,
            )| {
                let risk_id = RiskId::parse(target_risk_id)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let risk_version = pmc_domain::identity::AggregateVersion::new(
                    u64::try_from(target_expected_version)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let rationale =
                    pmc_domain::work_management::WorkManagementRationale::parse(rationale)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let classification = DataClassification::from_persisted(&classification)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let operation = WorkManagementOperation::CloseRisk {
                    risk_id,
                    risk_version,
                    rationale,
                };
                build_and_verify_prepared_intent(
                    intent_id,
                    operation,
                    classification,
                    contract_version,
                    payload_digest,
                    expires_at,
                )
            },
        )
        .collect()
}

fn build_and_verify_prepared_intent(
    intent_id: String,
    operation: WorkManagementOperation,
    classification: DataClassification,
    contract_version: i64,
    payload_digest: String,
    expires_at: i64,
) -> Result<WorkManagementPreparedIntent, RiskPersistenceLoadError> {
    let id = PreparedIntentId::parse(intent_id)
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    let created_at = expires_at - pmc_domain::work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS;
    let intent = WorkManagementPreparedIntent::prepare(
        id,
        operation,
        classification,
        None,
        UtcTimestamp::from_unix_millis(created_at),
    )
    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    if intent.payload_digest().as_str() != payload_digest
        || intent.preview().expires_at().unix_millis() != expires_at
        || i64::from(intent.preview().contract_version()) != contract_version
    {
        return Err(RiskPersistenceLoadError::InvalidRiskSnapshot);
    }
    Ok(intent)
}

fn risk_outcome_from_snapshot(
    tx: &Transaction<'_>,
    risk_id: &RiskId,
    context: &RiskOperationContext,
) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
    let snapshot = decode_namespace(tx, true).map_err(|_| storage_error(context))?;
    let record = snapshot
        .risks()
        .iter()
        .find(|risk| risk.id() == risk_id)
        .cloned()
        .ok_or_else(|| storage_error(context))?;
    let audit_row = tx.query_row("SELECT audit.id,audit.occurred_at FROM audit_events audit JOIN risk_replay_audits replay ON replay.audit_event_id=audit.id WHERE replay.idempotency_id=?1 AND audit.actor='head_of_products' AND audit.module='work_management' AND audit.event_code='risk.created' AND audit.target_type='risk' AND audit.target_id=?2 AND audit.correlation_id=?3 AND audit.policy_outcome='allowed' AND audit.approval_outcome='not_required' AND audit.execution_outcome='succeeded' AND audit.effect_scope='complete'", rusqlite::params![context.idempotency_id.as_str(), risk_id.as_str(), context.correlation_id.as_str()], |row| Ok((row.get::<_, String>(0)?,row.get::<_, i64>(1)?))).map_err(|_| storage_error(context))?;
    let effects: i64 = tx.query_row("SELECT count(*) FROM audit_effects WHERE audit_event_id=?1 AND ordinal=0 AND effect_code='risk.created' AND scope='complete' AND target_type='risk' AND target_id=?2", rusqlite::params![audit_row.0.as_str(), risk_id.as_str()], |row| row.get(0)).map_err(|_| storage_error(context))?;
    if effects != 1 {
        return Err(storage_error(context));
    }
    Ok(RiskMutationOutcome {
        record,
        audit_events: vec![create_audit(
            AuditEventId::parse(audit_row.0).map_err(|_| storage_error(context))?,
            UtcTimestamp::from_unix_millis(audit_row.1),
            risk_id,
            context,
        )?],
        approval_receipt_id: None,
    })
}

fn create_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    risk_id: &RiskId,
    context: &RiskOperationContext,
) -> Result<AuditEvent, DomainError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("risk.created").map_err(|_| storage_error(context))?,
            AuditTarget::Risk(risk_id.clone()),
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse("risk.created").map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

/// The outcome an `update_risk_response` committed, rebuilt from its own
/// replay rows: the response result it stored (not the Risk as it is now),
/// its audit and the correlation of that first attempt.
fn risk_response_outcome_from_replay(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
    let row = tx.query_row(
        "SELECT replay.correlation_id,replay.result_reference,replay.result_classification,replay.result_response,replay.result_owner_id,replay.result_rationale,replay.result_residual_exposure,replay.result_next_review_at,replay.result_version,risks.title,risks.details,audit.id,audit.occurred_at FROM risk_response_replay_operations replay JOIN risks ON risks.id=replay.result_reference JOIN risk_response_command_updates command ON command.idempotency_id=replay.idempotency_id AND command.risk_id=replay.result_reference AND command.expected_version+1=replay.result_version AND command.response=replay.result_response AND command.owner_id IS replay.result_owner_id AND command.rationale IS replay.result_rationale AND command.residual_exposure IS replay.result_residual_exposure AND command.next_review_at IS replay.result_next_review_at JOIN risk_response_replay_audits replay_audit ON replay_audit.idempotency_id=replay.idempotency_id AND replay_audit.correlation_id=replay.correlation_id JOIN audit_events audit ON audit.id=replay_audit.audit_event_id AND audit.correlation_id=replay.correlation_id AND audit.event_code='risk.response_updated' AND audit.target_type='risk' AND audit.target_id=replay.result_reference JOIN ledger_idempotency_claims claim ON claim.idempotency_id=replay.idempotency_id AND claim.namespace='risk' AND claim.operation='update_risk_response' WHERE replay.idempotency_id=?1 AND EXISTS(SELECT 1 FROM audit_effects effect WHERE effect.audit_event_id=audit.id AND effect.ordinal=0 AND effect.effect_code='risk.response_updated' AND effect.target_id=replay.result_reference) AND replay.result_version<=(SELECT version FROM aggregate_registry WHERE id=replay.result_reference)",
        [context.idempotency_id.as_str()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<i64>>(7)?, row.get::<_, i64>(8)?, row.get::<_, String>(9)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?, row.get::<_, i64>(12)?)),
    ).map_err(|_| storage_error(context))?;
    let risk_id = RiskId::parse(row.1).map_err(|_| storage_error(context))?;
    let record = RiskRecord::from_persisted_responded(
        risk_id.clone(),
        RiskTitle::parse(row.9).map_err(|_| storage_error(context))?,
        RiskDetails::parse(row.10).map_err(|_| storage_error(context))?,
        DataClassification::from_persisted(&row.2).map_err(|_| storage_error(context))?,
        pmc_domain::identity::AggregateVersion::new(
            u64::try_from(row.8).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
        RiskState::Open,
        RiskResponseType::from_persisted(&row.3).map_err(|_| storage_error(context))?,
        row.4
            .map(StakeholderId::parse)
            .transpose()
            .map_err(|_| storage_error(context))?,
        row.5
            .map(RiskRationale::parse)
            .transpose()
            .map_err(|_| storage_error(context))?,
        row.6
            .map(ResidualExposure::parse)
            .transpose()
            .map_err(|_| storage_error(context))?,
        row.7.map(UtcTimestamp::from_unix_millis),
    )
    .map_err(|_| storage_error(context))?;
    let audit = response_audit(
        AuditEventId::parse(row.11).map_err(|_| storage_error(context))?,
        UtcTimestamp::from_unix_millis(row.12),
        &risk_id,
        CorrelationId::parse(row.0).map_err(|_| storage_error(context))?,
    )
    .map_err(|_| storage_error(context))?;
    Ok(RiskMutationOutcome {
        record,
        audit_events: vec![audit],
        approval_receipt_id: None,
    })
}

fn response_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    risk_id: &RiskId,
    correlation_id: CorrelationId,
) -> Result<AuditEvent, pmc_domain::DomainValueError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("risk.response_updated")?,
            AuditTarget::Risk(risk_id.clone()),
        ),
        correlation_id,
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse("risk.response_updated")?],
        )
        .map_err(|_| {
            pmc_domain::DomainValueError::new(pmc_domain::ValueErrorKind::InvalidCharacter)
        })?,
    ))
}
fn storage_error(context: &RiskOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("ledger.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}
fn idempotency_conflict(context: &RiskOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("ledger.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn not_found(context: &RiskOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("risk.not_found").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn transition_conflict(context: &RiskOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("risk.stale_or_illegal").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

/// A claims row for this idempotency identifier under any operation other
/// than `operation` means it was already spent by a different command
/// (another Risk H2a preview, the H1 `create_risk` seam, or another
/// aggregate's namespace entirely).
fn idempotency_claimed_by_other_operation(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
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

/// A prepare replay must bind to the exact previously persisted preview, not
/// merely to matching command scalars and a matching prepared-intent ID: the
/// payload digest is a cryptographic hash of the full canonical preview
/// (operation, classification, expiry, support), so comparing it catches a
/// caller-supplied `prepared` whose evidence/support genuinely differs from
/// what was actually durably recorded, which scalar-only comparison cannot.
fn prepared_intent_digest_matches(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &RiskOperationContext,
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
/// family. Callers still separately insert the target and typed payload rows
/// this row's foreign keys require.
fn persist_prepared_intent(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    intent_kind: &str,
    context: &RiskOperationContext,
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

fn next_risk_h2a_v13_operation_ordinal(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_h2a_v13_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn decode_risk_h2a_runtime_snapshot(
    tx: &Transaction<'_>,
) -> Result<RiskH2aRuntimeSnapshot, RiskPersistenceLoadError> {
    let base = decode_namespace(tx, false)?;
    let terminals = decode_v11_terminal_denials(tx)?;
    let mut prepared = decode_v13_prepared_previews(tx)?;
    prepared.extend(decode_lower_risk_classification_prepared_previews(tx)?);
    let rejections = decode_risk_rejections(tx)?;
    RiskH2aPersistenceDecodeInput::new(base, terminals)
        .with_prepared(prepared)
        .with_rejections(rejections)
        .decode()
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)
}

/// True when `prepared_id` names a prepared intent of `kinds` that has been
/// consumed by something other than a row in `rejection_table` -- an
/// execute or a terminal denial -- and is therefore no longer refusable.
/// Unknown ids and outstanding previews are `false`; the domain decides
/// those. Shared by the Risk and Issue rejection writers.
pub(super) fn consumed_other_than_by_rejection(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    kinds: &str,
    rejection_table: &str,
) -> Result<bool, rusqlite::Error> {
    tx.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM prepared_intents WHERE id=?1 AND intent_kind IN {kinds} AND consumed_at IS NOT NULL AND NOT EXISTS(SELECT 1 FROM {rejection_table} WHERE prepared_intent_id=prepared_intents.id))"
        ),
        [prepared_id.as_str()],
        |row| row.get::<_, i64>(0),
    )
    .map(|found| found != 0)
}

/// Whether the stored `audit_events` row for `audit` says, column by
/// column, exactly what a prepared-intent rejection audit says -- and
/// carries no `audit_effects`. Any read failure is a mismatch. Shared by the
/// Risk and Issue rejection decoders.
pub(super) fn rejection_audit_row_matches(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
    target_type: &str,
    target_id: &str,
) -> bool {
    let row = tx
        .query_row(
            "SELECT occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope,(SELECT count(*) FROM audit_effects WHERE audit_effects.audit_event_id=audit_events.id) FROM audit_events WHERE id=?1",
            [audit.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .optional();
    let Ok(Some(row)) = row else {
        return false;
    };
    row.0 == audit.occurred_at().unix_millis()
        && row.1 == "head_of_products"
        && row.2 == "work_management"
        && row.3 == audit.code().as_str()
        && row.4 == target_type
        && row.5 == target_id
        && row.6 == audit.correlation_id().as_str()
        && row.7 == "allowed"
        && row.8 == "rejected"
        && row.9 == "not_attempted"
        && row.10 == "none"
        && row.11 == 0
}

fn next_risk_reject_prepared_operation_ordinal(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_reject_prepared_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Persists one Risk rejection: consumes the intent at the rejection
/// instant (it must still be outstanding), records the zero-effect audit,
/// and appends the v46 result row, whose triggers claim the idempotency id
/// and refuse anything that did not consume a Risk intent at that instant.
fn persist_risk_prepared_intent_rejection(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
    operation_ordinal: i64,
    outcome: &RejectedPreparedIntentOutcome,
) -> Result<(), DomainError> {
    let audit = outcome.audit_event();
    let AuditTarget::Risk(risk_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if tx
        .execute(
            "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
            rusqlite::params![
                outcome.rejected_at().unix_millis(),
                outcome.prepared_intent_id().as_str()
            ],
        )
        .map_err(|_| storage_error(context))?
        != 1
    {
        return Err(storage_error(context));
    }
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'risk',?4,?5,'allowed','rejected','not_attempted','none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            risk_id.as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared',?1,?2,?3,?4,'head_of_products',?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            operation_ordinal,
            outcome.prepared_intent_id().as_str(),
            outcome.rejected_at().unix_millis(),
            audit.id().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

struct RiskRejectionRow {
    idempotency_id: String,
    correlation_id: String,
    rejected_at: i64,
    audit_event_id: String,
    intent_id: String,
    intent_kind: String,
    contract_version: i64,
    payload_digest: String,
    classification: String,
    expires_at: i64,
    consumed_at: Option<i64>,
    target_id: String,
    expected_version: i64,
    created_id: Option<String>,
    created_classification: Option<String>,
    rationale: Option<String>,
}

/// Rebuilds every durable v46 Risk rejection as the domain's replay input.
/// The consumed preview is re-derived and digest-verified exactly like an
/// outstanding one (`build_and_verify_prepared_intent`), its consumption
/// instant must equal the recorded rejection instant, and the zero-effect
/// audit is re-derived wholesale and compared against the stored row rather
/// than trusted from it.
fn decode_risk_rejections(
    tx: &Transaction<'_>,
) -> Result<Vec<RiskH2aRejectionReplay>, RiskPersistenceLoadError> {
    let invalid = || RiskPersistenceLoadError::InvalidRiskSnapshot;
    let rows = tx
        .prepare("SELECT rejection.idempotency_id,rejection.correlation_id,rejection.rejected_at,rejection.audit_event_id,intent.id,intent.intent_kind,intent.contract_version,intent.payload_digest,intent.classification,intent.expires_at,intent.consumed_at,target.target_id,target.expected_version,payload.created_id,payload.created_classification,payload.rationale FROM risk_reject_prepared_command_results rejection JOIN prepared_intents intent ON intent.id=rejection.prepared_intent_id JOIN prepared_intent_targets target ON target.prepared_intent_id=intent.id AND target.ordinal=0 JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id ORDER BY rejection.operation_ordinal")
        .map_err(|_| invalid())?
        .query_map([], |row| {
            Ok(RiskRejectionRow {
                idempotency_id: row.get(0)?,
                correlation_id: row.get(1)?,
                rejected_at: row.get(2)?,
                audit_event_id: row.get(3)?,
                intent_id: row.get(4)?,
                intent_kind: row.get(5)?,
                contract_version: row.get(6)?,
                payload_digest: row.get(7)?,
                classification: row.get(8)?,
                expires_at: row.get(9)?,
                consumed_at: row.get(10)?,
                target_id: row.get(11)?,
                expected_version: row.get(12)?,
                created_id: row.get(13)?,
                created_classification: row.get(14)?,
                rationale: row.get(15)?,
            })
        })
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    rows.into_iter()
        .map(|row| {
            let risk_id = RiskId::parse(row.target_id).map_err(|_| invalid())?;
            let risk_version = pmc_domain::identity::AggregateVersion::new(
                u64::try_from(row.expected_version).map_err(|_| invalid())?,
            )
            .map_err(|_| invalid())?;
            let operation = match row.intent_kind.as_str() {
                "record_risk_occurrence" => WorkManagementOperation::RecordRiskOccurrence {
                    risk_id: risk_id.clone(),
                    risk_version,
                    issue_id: IssueId::parse(row.created_id.ok_or_else(invalid)?)
                        .map_err(|_| invalid())?,
                    issue_classification: DataClassification::from_persisted(
                        &row.created_classification.ok_or_else(invalid)?,
                    )
                    .map_err(|_| invalid())?,
                },
                "close_risk" => WorkManagementOperation::CloseRisk {
                    risk_id: risk_id.clone(),
                    risk_version,
                    rationale: WorkManagementRationale::parse(row.rationale.ok_or_else(invalid)?)
                        .map_err(|_| invalid())?,
                },
                _ => return Err(invalid()),
            };
            let classification =
                DataClassification::from_persisted(&row.classification).map_err(|_| invalid())?;
            let prepared = build_and_verify_prepared_intent(
                row.intent_id,
                operation,
                classification,
                row.contract_version,
                row.payload_digest,
                row.expires_at,
            )?;
            if row.consumed_at != Some(row.rejected_at) {
                return Err(invalid());
            }
            let rejected_at = UtcTimestamp::from_unix_millis(row.rejected_at);
            let correlation_id = CorrelationId::parse(row.correlation_id).map_err(|_| invalid())?;
            let audit = prepared_intent_rejection_audit(
                AuditEventId::parse(row.audit_event_id).map_err(|_| invalid())?,
                rejected_at,
                RISK_PREPARED_REJECTED_AUDIT_CODE,
                AuditTarget::Risk(risk_id.clone()),
                correlation_id.clone(),
            )
            .ok_or_else(invalid)?;
            if !rejection_audit_row_matches(tx, &audit, "risk", risk_id.as_str()) {
                return Err(invalid());
            }
            let outcome = RejectedPreparedIntentOutcome::new(
                prepared.id().clone(),
                rejected_at,
                rejected_at >= prepared.preview().expires_at(),
                audit,
            );
            Ok(RiskH2aRejectionReplay::new(
                prepared,
                RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(row.idempotency_id)
                        .map_err(|_| invalid())?,
                    correlation_id,
                },
                outcome,
            ))
        })
        .collect()
}

fn next_risk_h2a_v14_operation_ordinal(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_h2a_v14_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Persists the full Occurred success bundle: the new Issue, the Risk
/// transition, the link, all 3 audits, the Approval Receipt, prepared-intent
/// consumption, and the V14 idempotent-replay row.
fn persist_occurred_bundle(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    outcome: &OccurredRiskOutcome,
    ordinal: i64,
    context: &RiskOperationContext,
) -> Result<(), DomainError> {
    let risk_id = outcome.risk.id().as_str();
    let issue_id = outcome.issue.id().as_str();
    let occurred_millis = outcome
        .audit_events
        .first()
        .ok_or_else(|| storage_error(context))?
        .occurred_at()
        .unix_millis();
    tx.execute(
        "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'issue',1,?2,?3,?3)",
        rusqlite::params![issue_id, outcome.issue.classification().as_persisted(), occurred_millis],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO issues(id,source_risk_id,recurrence_of_id,title,details,state,resolution_type,resolution_rationale) VALUES(?1,?2,NULL,?3,?4,'open',NULL,NULL)",
        rusqlite::params![issue_id, risk_id, outcome.issue.title().as_str(), outcome.issue.details().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute("UPDATE risks SET state='occurred' WHERE id=?1", [risk_id])
        .map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='risk'",
        rusqlite::params![
            i64::try_from(outcome.risk.version().get()).map_err(|_| storage_error(context))?,
            occurred_millis,
            risk_id
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_issue_links(risk_id,issue_id,risk_version,issue_version,classification) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![
            risk_id,
            issue_id,
            i64::try_from(outcome.risk.version().get()).map_err(|_| storage_error(context))?,
            i64::try_from(outcome.issue.version().get()).map_err(|_| storage_error(context))?,
            outcome.risk.classification().as_persisted()
        ],
    )
    .map_err(|_| storage_error(context))?;
    for audit in &outcome.audit_events {
        let (target_type, target_id) = match audit.target() {
            AuditTarget::Risk(id) => ("risk", id.as_str()),
            AuditTarget::Issue(id) => ("issue", id.as_str()),
            _ => return Err(storage_error(context)),
        };
        tx.execute(
            "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')",
            rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, context.correlation_id.as_str()],
        ).map_err(|_| storage_error(context))?;
        let effect_code = audit
            .actual_effects()
            .first()
            .ok_or_else(|| storage_error(context))?
            .as_str();
        tx.execute(
            "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete',?3,?4)",
            rusqlite::params![audit.id().as_str(), effect_code, target_type, target_id],
        )
        .map_err(|_| storage_error(context))?;
    }
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![outcome.approval_receipt_id.as_str(), context.idempotency_id.as_str(), occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_v14_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,risk_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_occurrence',?2,?3,'occurred',?4,?5,?6)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, risk_id, approval.prepared_id().as_str(), outcome.approval_receipt_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    for (index, audit) in outcome.audit_events.iter().enumerate() {
        tx.execute(
            "INSERT INTO risk_h2a_v14_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,?2,?3,?4)",
            rusqlite::params![context.idempotency_id.as_str(), i64::try_from(index).map_err(|_| storage_error(context))?, audit.id().as_str(), context.correlation_id.as_str()],
        ).map_err(|_| storage_error(context))?;
    }
    tx.execute(
        "INSERT INTO risk_h2a_v14_command_execute_occurrences(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), approval.prepared_id().as_str(), approval.acknowledged_payload_digest().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Persists the Closed success bundle: the Risk transition, its single
/// audit, the Approval Receipt, prepared-intent consumption, and the V14
/// idempotent-replay row.
fn persist_closed_bundle(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    outcome: &RiskMutationOutcome<RiskRecord>,
    ordinal: i64,
    context: &RiskOperationContext,
) -> Result<(), DomainError> {
    let risk_id = outcome.record.id().as_str();
    let audit = outcome
        .audit_events
        .first()
        .ok_or_else(|| storage_error(context))?;
    let AuditTarget::Risk(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != risk_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE risks SET state='closed',in_exception_queue=0 WHERE id=?1",
        [risk_id],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='risk'",
        rusqlite::params![
            i64::try_from(outcome.record.version().get()).map_err(|_| storage_error(context))?,
            occurred_millis,
            risk_id
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'risk',?4,?5,'allowed','approved','succeeded','complete')",
        rusqlite::params![audit.id().as_str(), occurred_millis, audit.code().as_str(), risk_id, context.correlation_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    let effect_code = audit
        .actual_effects()
        .first()
        .ok_or_else(|| storage_error(context))?
        .as_str();
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete','risk',?3)",
        rusqlite::params![audit.id().as_str(), effect_code, risk_id],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    let approval_receipt_id = outcome
        .approval_receipt_id
        .clone()
        .ok_or_else(|| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![approval_receipt_id.as_str(), context.idempotency_id.as_str(), occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_v14_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,risk_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_close',?2,?3,'closed',?4,?5,?6)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, risk_id, approval.prepared_id().as_str(), approval_receipt_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_v14_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_v14_command_execute_closes(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), approval.prepared_id().as_str(), approval.acknowledged_payload_digest().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Persists the one accepted durable post-start terminal: a policy denial
/// discovered after the Approval Receipt would otherwise have been consumed.
/// The prepared intent's own row already exists from the V13 prepare step;
/// this only adds the denial audit, the V11 terminal row, and consumption.
#[allow(clippy::too_many_arguments)]
fn persist_v11_terminal_denial(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    operation: &str,
    audit_event_code: &str,
    audit_event_id: &AuditEventId,
    occurred_at: UtcTimestamp,
    error: &DomainError,
    context: &RiskOperationContext,
) -> Result<(), DomainError> {
    let (risk_id, risk_version, payload_digest): (String, i64, String) = tx
        .query_row(
            "SELECT target.target_id,target.expected_version,intent.payload_digest FROM prepared_intent_targets target JOIN prepared_intents intent ON intent.id=target.prepared_intent_id WHERE target.prepared_intent_id=?1 AND target.ordinal=0",
            [approval.prepared_id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| storage_error(context))?;
    let occurred_millis = occurred_at.unix_millis();
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'risk',?4,?5,'denied','not_required','not_attempted','none')",
        rusqlite::params![audit_event_id.as_str(), occurred_millis, audit_event_code, risk_id, context.correlation_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_v11_terminal_denials(execute_idempotency_id,prepared_intent_id,risk_id,risk_version,operation,acknowledged_digest,correlation_id,policy_denial_audit_id,error_code,error_message_key,error_retryable) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'SECURITY_POLICY_DENIED',?9,0)",
        rusqlite::params![context.idempotency_id.as_str(), approval.prepared_id().as_str(), risk_id, risk_version, operation, payload_digest, context.correlation_id.as_str(), audit_event_id.as_str(), error.message_key().as_str()],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn decode_v14_replay_audits(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<Vec<AuditEvent>, DomainError> {
    let rows = tx
        .prepare("SELECT audit.id,audit.occurred_at,audit.event_code,audit.target_type,audit.target_id,audit.correlation_id FROM risk_h2a_v14_replay_audits replay JOIN audit_events audit ON audit.id=replay.audit_event_id WHERE replay.idempotency_id=?1 ORDER BY replay.ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([context.idempotency_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    rows.into_iter()
        .map(|(id, at, code, target_type, target_id, correlation_id)| {
            let target = match target_type.as_str() {
                "risk" => {
                    AuditTarget::Risk(RiskId::parse(target_id).map_err(|_| storage_error(context))?)
                }
                "issue" => AuditTarget::Issue(
                    IssueId::parse(target_id).map_err(|_| storage_error(context))?,
                ),
                _ => return Err(storage_error(context)),
            };
            let code = AuditEventCode::parse(&code).map_err(|_| storage_error(context))?;
            let effect_code = pmc_domain::audit::AuditEffectCode::parse(code.as_str())
                .map_err(|_| storage_error(context))?;
            Ok(AuditEvent::new(
                AuditEventId::parse(id).map_err(|_| storage_error(context))?,
                UtcTimestamp::from_unix_millis(at),
                AuditActor::HeadOfProducts,
                AuditAction::new(AuditModule::WorkManagement, code, target),
                CorrelationId::parse(correlation_id).map_err(|_| storage_error(context))?,
                AuditDisposition::new(
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Succeeded,
                    AuditEffectScope::Complete,
                    vec![effect_code],
                )
                .map_err(|_| storage_error(context))?,
            ))
        })
        .collect()
}

fn replay_occurred_outcome(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<OccurredRiskOutcome, DomainError> {
    let (risk_id, receipt_id): (String, String) = tx
        .query_row(
            "SELECT risk_id,approval_receipt_id FROM risk_h2a_v14_replay_operations WHERE idempotency_id=?1 AND operation='execute_occurrence'",
            [context.idempotency_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let (title, details, classification, version): (String, String, String, i64) = tx
        .query_row(
            "SELECT risks.title,risks.details,registry.classification,registry.version FROM risks JOIN aggregate_registry registry ON registry.id=risks.id AND registry.aggregate_type='risk' WHERE risks.id=?1",
            [risk_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| storage_error(context))?;
    let risk = RiskRecord::from_persisted_occurred_from_created_open(
        RiskId::parse(risk_id.clone()).map_err(|_| storage_error(context))?,
        RiskTitle::parse(title).map_err(|_| storage_error(context))?,
        RiskDetails::parse(details).map_err(|_| storage_error(context))?,
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?,
        pmc_domain::identity::AggregateVersion::new(
            u64::try_from(version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
    )
    .map_err(|_| storage_error(context))?;
    let (issue_id, issue_title, issue_details, issue_classification, issue_version): (
        String,
        String,
        String,
        String,
        i64,
    ) = tx
        .query_row(
            "SELECT issues.id,issues.title,issues.details,registry.classification,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.source_risk_id=?1",
            [risk_id.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let issue = pmc_domain::issues::IssueRecord::from_persisted_created_from_risk(
        IssueId::parse(issue_id).map_err(|_| storage_error(context))?,
        RiskId::parse(risk_id).map_err(|_| storage_error(context))?,
        pmc_domain::issues::IssueTitle::parse(issue_title).map_err(|_| storage_error(context))?,
        pmc_domain::issues::IssueDetails::parse(issue_details)
            .map_err(|_| storage_error(context))?,
        DataClassification::from_persisted(&issue_classification)
            .map_err(|_| storage_error(context))?,
        pmc_domain::identity::AggregateVersion::new(
            u64::try_from(issue_version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
    )
    .map_err(|_| storage_error(context))?;
    let audit_events = decode_v14_replay_audits(tx, context)?;
    Ok(OccurredRiskOutcome {
        risk,
        issue,
        audit_events,
        approval_receipt_id: ApprovalReceiptId::parse(receipt_id)
            .map_err(|_| storage_error(context))?,
    })
}

fn replay_closed_outcome(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
    let (risk_id, receipt_id): (String, String) = tx
        .query_row(
            "SELECT risk_id,approval_receipt_id FROM risk_h2a_v14_replay_operations WHERE idempotency_id=?1 AND operation='execute_close'",
            [context.idempotency_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let (title, details, classification, version): (String, String, String, i64) = tx
        .query_row(
            "SELECT risks.title,risks.details,registry.classification,registry.version FROM risks JOIN aggregate_registry registry ON registry.id=risks.id AND registry.aggregate_type='risk' WHERE risks.id=?1",
            [risk_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| storage_error(context))?;
    let record = RiskRecord::from_persisted_closed_from_created_open(
        RiskId::parse(risk_id).map_err(|_| storage_error(context))?,
        RiskTitle::parse(title).map_err(|_| storage_error(context))?,
        RiskDetails::parse(details).map_err(|_| storage_error(context))?,
        DataClassification::from_persisted(&classification).map_err(|_| storage_error(context))?,
        pmc_domain::identity::AggregateVersion::new(
            u64::try_from(version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
    )
    .map_err(|_| storage_error(context))?;
    let audit_events = decode_v14_replay_audits(tx, context)?;
    Ok(RiskMutationOutcome {
        record,
        audit_events,
        approval_receipt_id: Some(
            ApprovalReceiptId::parse(receipt_id).map_err(|_| storage_error(context))?,
        ),
    })
}

fn next_risk_h2a_lower_classification_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_h2a_lower_classification_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_risk_h2a_lower_classification_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM risk_h2a_lower_classification_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Decodes still-outstanding (`consumed_at IS NULL`) Risk classification-
/// lowering previews so they survive a restart, mirroring
/// `decode_v13_prepared_closes` -- a fresh root of its own typed tables (this
/// table already carries `rationale`/`proposed_classification` inline, so
/// unlike V13 there is no separate `prepared_work_management_payloads` join).
fn decode_lower_risk_classification_prepared_previews(
    tx: &Transaction<'_>,
) -> Result<Vec<WorkManagementPreparedIntent>, RiskPersistenceLoadError> {
    let rows = tx
        .prepare("SELECT intent.id,intent.contract_version,intent.payload_digest,intent.classification,intent.expires_at,target.target_id,target.expected_version,command.proposed_classification,command.rationale FROM risk_h2a_lower_classification_prepare_replay_operations replay JOIN risk_h2a_lower_classification_command_prepares command USING(idempotency_id) JOIN prepared_intents intent ON intent.id=replay.result_reference JOIN prepared_intent_targets target ON target.prepared_intent_id=intent.id AND target.ordinal=0 WHERE replay.operation='prepare_lower_classification' AND replay.result_kind='prepared' AND intent.consumed_at IS NULL ORDER BY intent.id")
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
    rows.into_iter()
        .map(
            |(
                intent_id,
                contract_version,
                payload_digest,
                classification,
                expires_at,
                target_risk_id,
                target_expected_version,
                proposed_classification,
                rationale,
            )| {
                let risk_id = RiskId::parse(target_risk_id)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let risk_version = pmc_domain::identity::AggregateVersion::new(
                    u64::try_from(target_expected_version)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?,
                )
                .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let rationale =
                    pmc_domain::work_management::WorkManagementRationale::parse(rationale)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let current_classification = DataClassification::from_persisted(&classification)
                    .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let proposed_classification =
                    DataClassification::from_persisted(&proposed_classification)
                        .map_err(|_| RiskPersistenceLoadError::InvalidRiskSnapshot)?;
                let operation = WorkManagementOperation::LowerRiskClassification {
                    risk_id,
                    risk_version,
                    current_classification,
                    proposed_classification,
                    rationale,
                };
                build_and_verify_prepared_intent(
                    intent_id,
                    operation,
                    current_classification,
                    contract_version,
                    payload_digest,
                    expires_at,
                )
            },
        )
        .collect()
}

/// See `decode_v14_replay_audits` -- identical shape, pointed at this
/// operation's own execute-side replay-audit table.
fn decode_lower_classification_replay_audits(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<Vec<AuditEvent>, DomainError> {
    let rows = tx
        .prepare("SELECT audit.id,audit.occurred_at,audit.event_code,audit.target_type,audit.target_id,audit.correlation_id FROM risk_h2a_lower_classification_execute_replay_audits replay JOIN audit_events audit ON audit.id=replay.audit_event_id WHERE replay.idempotency_id=?1 ORDER BY replay.ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([context.idempotency_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    rows.into_iter()
        .map(|(id, at, code, target_type, target_id, correlation_id)| {
            let target = match target_type.as_str() {
                "risk" => {
                    AuditTarget::Risk(RiskId::parse(target_id).map_err(|_| storage_error(context))?)
                }
                "issue" => AuditTarget::Issue(
                    IssueId::parse(target_id).map_err(|_| storage_error(context))?,
                ),
                _ => return Err(storage_error(context)),
            };
            let code = AuditEventCode::parse(&code).map_err(|_| storage_error(context))?;
            let effect_code = pmc_domain::audit::AuditEffectCode::parse(code.as_str())
                .map_err(|_| storage_error(context))?;
            Ok(AuditEvent::new(
                AuditEventId::parse(id).map_err(|_| storage_error(context))?,
                UtcTimestamp::from_unix_millis(at),
                AuditActor::HeadOfProducts,
                AuditAction::new(AuditModule::WorkManagement, code, target),
                CorrelationId::parse(correlation_id).map_err(|_| storage_error(context))?,
                AuditDisposition::new(
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Succeeded,
                    AuditEffectScope::Complete,
                    vec![effect_code],
                )
                .map_err(|_| storage_error(context))?,
            ))
        })
        .collect()
}

/// Re-reads a completed Risk classification-lowering execution for replay.
/// The risk row already reflects the post-lowering result (classification
/// lives solely in `aggregate_registry`, not in `risks`), so this reads it
/// back the same way `risk_outcome_from_snapshot` does for H1 create,
/// combined with this operation's own audit replay trail.
fn replay_lowered_risk_outcome(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<RiskMutationOutcome<RiskRecord>, DomainError> {
    let (risk_id, receipt_id): (String, String) = tx
        .query_row(
            "SELECT risk_id,approval_receipt_id FROM risk_h2a_lower_classification_execute_replay_operations WHERE idempotency_id=?1 AND operation='execute_lower_classification'",
            [context.idempotency_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let risk_id = RiskId::parse(risk_id).map_err(|_| storage_error(context))?;
    let snapshot = decode_namespace(tx, true).map_err(|_| storage_error(context))?;
    let record = snapshot
        .risks()
        .iter()
        .find(|risk| risk.id() == &risk_id)
        .cloned()
        .ok_or_else(|| storage_error(context))?;
    let audit_events = decode_lower_classification_replay_audits(tx, context)?;
    Ok(RiskMutationOutcome {
        record,
        audit_events,
        approval_receipt_id: Some(
            ApprovalReceiptId::parse(receipt_id).map_err(|_| storage_error(context))?,
        ),
    })
}

/// Re-reads a previously persisted durable policy-denial terminal for Risk
/// classification lowering and returns it as the original `DomainError`,
/// mirroring the V11 terminal-denial replay path. `error_retryable` is
/// always 0 by CHECK constraint (the one accepted durable terminal here is
/// never retryable), so it is not re-read.
fn replay_lower_classification_terminal_denial(
    tx: &Transaction<'_>,
    context: &RiskOperationContext,
) -> Result<DomainError, DomainError> {
    let error_message_key: String = tx
        .query_row(
            "SELECT error_message_key FROM risk_h2a_lower_classification_terminal_denials WHERE execute_idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    Ok(DomainError::new(
        ErrorCode::SecurityPolicyDenied,
        MessageKey::parse(&error_message_key).map_err(|_| storage_error(context))?,
        context.correlation_id.clone(),
        false,
    ))
}

/// Persists one successful Risk classification-lowering execution. Mirrors
/// `persist_closed_bundle`; unlike Close, no `risks` column changes (state
/// is untouched by this operation), and `aggregate_registry.classification`
/// is updated alongside `version`.
fn persist_lowered_risk_bundle(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    outcome: &RiskMutationOutcome<RiskRecord>,
    ordinal: i64,
    context: &RiskOperationContext,
) -> Result<(), DomainError> {
    let risk_id = outcome.record.id().as_str();
    let audit = outcome
        .audit_events
        .first()
        .ok_or_else(|| storage_error(context))?;
    let AuditTarget::Risk(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if target_id.as_str() != risk_id {
        return Err(storage_error(context));
    }
    let occurred_millis = audit.occurred_at().unix_millis();
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='risk'",
        rusqlite::params![
            i64::try_from(outcome.record.version().get()).map_err(|_| storage_error(context))?,
            outcome.record.classification().as_persisted(),
            occurred_millis,
            risk_id
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'risk',?4,?5,'allowed','approved','succeeded','complete')",
        rusqlite::params![audit.id().as_str(), occurred_millis, audit.code().as_str(), risk_id, context.correlation_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    let effect_code = audit
        .actual_effects()
        .first()
        .ok_or_else(|| storage_error(context))?
        .as_str();
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete','risk',?3)",
        rusqlite::params![audit.id().as_str(), effect_code, risk_id],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    let approval_receipt_id = outcome
        .approval_receipt_id
        .clone()
        .ok_or_else(|| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![approval_receipt_id.as_str(), context.idempotency_id.as_str(), occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_lower_classification_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,risk_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, risk_id, approval.prepared_id().as_str(), approval_receipt_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_lower_classification_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_lower_classification_command_executes(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), approval.prepared_id().as_str(), approval.acknowledged_payload_digest().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Persists the one accepted durable post-start terminal for Risk
/// classification lowering: a policy denial discovered after the Approval
/// Receipt would otherwise have been consumed. Mirrors
/// `persist_v11_terminal_denial`, but as its own dedicated table (this
/// operation was never part of V11's `operation IN ('record_occurrence',
/// 'close')` CHECK, and extending that CHECK would mean yet another
/// recreate of an already-released table for no real benefit over a fresh
/// root).
fn persist_lower_classification_terminal_denial(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    audit_event_id: &AuditEventId,
    occurred_at: UtcTimestamp,
    error: &DomainError,
    context: &RiskOperationContext,
) -> Result<(), DomainError> {
    let (risk_id, risk_version, payload_digest): (String, i64, String) = tx
        .query_row(
            "SELECT target.target_id,target.expected_version,intent.payload_digest FROM prepared_intent_targets target JOIN prepared_intents intent ON intent.id=target.prepared_intent_id WHERE target.prepared_intent_id=?1 AND target.ordinal=0",
            [approval.prepared_id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| storage_error(context))?;
    let occurred_millis = occurred_at.unix_millis();
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management','risk.classification_lowering_denied',?3,?4,?5,'denied','not_required','not_attempted','none')",
        rusqlite::params![audit_event_id.as_str(), occurred_millis, "risk", risk_id, context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO risk_h2a_lower_classification_terminal_denials(execute_idempotency_id,prepared_intent_id,risk_id,risk_version,acknowledged_digest,correlation_id,policy_denial_audit_id,error_code,error_message_key,error_retryable) VALUES(?1,?2,?3,?4,?5,?6,?7,'SECURITY_POLICY_DENIED',?8,0)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            risk_id,
            risk_version,
            payload_digest,
            context.correlation_id.as_str(),
            audit_event_id.as_str(),
            error.message_key().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}
