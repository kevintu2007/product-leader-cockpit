//! Typed SQLite persistence seam for the Decision Request H1 create lifecycle.

use pmc_domain::{
    actions::{
        ActionExecutionPolicy, ActionExecutionPolicyPort, ActionServiceIdSource,
        DenyActionEvidenceAuthority, InMemoryActionService,
    },
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
        AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    decisions::{
        ApproveAndExecuteLowerDecisionClassification, ApproveAndExecuteResolveDecisionRequest,
        ApproveAndExecuteSupersedeDecision, CreateDecisionRequestDraft,
        DecisionEvidenceAuthorityPort, DecisionExecutionPolicyPort, DecisionMutationOutcome,
        DecisionOperationContext, DecisionPersistenceCommand, DecisionPersistenceDecodeInput,
        DecisionPersistenceResult, DecisionPersistenceSnapshot, DecisionRecord,
        DecisionReplayCapsule, DecisionRequestRecord, DecisionServiceIdSource, DecisionSubject,
        DecisionText, InMemoryDecisionService, PrepareLowerDecisionClassification,
        PrepareSupersedeDecision, RejectDecisionPreparedIntent, ResolvedDecisionOutcome,
        SubmitDecisionRequest, SupersededDecisionOutcome, WithdrawDecisionRequest,
    },
    error::{DomainError, ErrorCode, MessageKey},
    identity::{
        ActionId, ApprovalReceiptId, AuditEventId, CorrelationId, DecisionId, DecisionRequestId,
        EvidenceReferenceId, IdempotencyId, PreparedIntentId, StakeholderId,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        prepared_intent_rejection_audit, ApprovalAuthorizationPort, ApprovalConfirmation,
        DecisionRequestState, DenyWorkManagementApproval, EvidenceOrJudgment,
        EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification, IntegrityDigest,
        RejectedPreparedIntentOutcome, WorkManagementApproval, WorkManagementOperation,
        WorkManagementPayloadDigest, WorkManagementPreparedIntent,
        DECISION_PREPARED_REJECTED_AUDIT_CODE,
    },
};
use rusqlite::{OptionalExtension, Transaction};

use super::{LedgerTransactionError, SqliteProductLedger, CURRENT_SCHEMA_VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionPersistenceLoadError {
    UnsupportedSchema { found: u32 },
    StorageUnavailable,
    InvalidDecisionSnapshot,
}

#[derive(Clone, Copy)]
struct PersistedResolveClock(UtcTimestamp);

impl Clock for PersistedResolveClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

struct PersistedResolveIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_ids: std::array::IntoIter<AuditEventId, 3>,
}

fn missing_persisted_resolve_id() -> pmc_domain::DomainValueError {
    match DecisionId::parse("") {
        Err(error) => error,
        Ok(_) => unreachable!("empty opaque identifiers must be rejected"),
    }
}

impl DecisionServiceIdSource for PersistedResolveIds {
    fn next_decision_id(&mut self) -> Result<DecisionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.receipt_id
            .take()
            .ok_or_else(missing_persisted_resolve_id)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_ids
            .next()
            .ok_or_else(missing_persisted_resolve_id)
    }
}

/// Verification-only id source for `decode_namespace`'s own reconstruction
/// pass: it re-runs the domain resolve purely to cross-check its Decision-
/// side outcome against already-durable rows, then discards the throwaway
/// Action service entirely (nothing here is persisted), so synthesized,
/// non-durable audit ids are fine -- unlike
/// `PersistedResolveActionExecuteIds` below, which backs the real EXECUTE
/// write path.
struct PersistedResolveActionIds {
    next_audit: u64,
}

impl ActionServiceIdSource for PersistedResolveActionIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        let value = format!("decision-stage-audit-{}", self.next_audit);
        self.next_audit = self
            .next_audit
            .checked_add(1)
            .ok_or_else(missing_persisted_resolve_id)?;
        AuditEventId::parse(value)
    }
}

/// The Decision-triggered Action provenance gap fix (2026-09): `actions` is
/// now the REAL, rehydrated `InMemoryActionService` (see
/// `approve_and_execute_resolve_decision_request`'s body), not a fresh
/// throwaway one -- its exported capsules are actually persisted, so their
/// audit event ids must be real, caller-supplied, ledger-durable ids like
/// every other `ActionServiceIdSource` in this crate, not synthesized
/// strings. The caller pre-generates exactly as many as
/// `create_resulting_action_request_from_decision_transition`/
/// `mark_action_request_superseded_premise_from_decision`/
/// `mark_action_superseded_premise_from_decision` will need (one audit per
/// call); this ledger validates the count matches before consuming any.
struct PersistedResolveActionExecuteIds {
    audit_ids: std::vec::IntoIter<AuditEventId>,
}

impl ActionServiceIdSource for PersistedResolveActionExecuteIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_ids
            .next()
            .ok_or_else(missing_persisted_resolve_id)
    }
}

/// H2a "Lower Data Classification" execute id source: a single
/// audit event, unlike Resolve's fixed three.
struct PersistedLowerClassificationIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_id: Option<AuditEventId>,
}

impl DecisionServiceIdSource for PersistedLowerClassificationIds {
    fn next_decision_id(&mut self) -> Result<DecisionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.receipt_id
            .take()
            .ok_or_else(missing_persisted_resolve_id)
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_id
            .take()
            .ok_or_else(missing_persisted_resolve_id)
    }
}

#[derive(Clone, Copy)]
struct AllowDecisionResultingActionPolicy;

impl ActionExecutionPolicyPort for AllowDecisionResultingActionPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}

#[derive(Clone, Copy)]
struct AllowPersistedDecisionApproval;
impl ApprovalAuthorizationPort for AllowPersistedDecisionApproval {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}
/// v45 rejection mints exactly one audit event and nothing else.
struct PersistedRejectIds {
    audit_id: Option<AuditEventId>,
}

impl DecisionServiceIdSource for PersistedRejectIds {
    fn next_decision_id(&mut self) -> Result<DecisionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        Err(missing_persisted_resolve_id())
    }
    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_id
            .take()
            .ok_or_else(missing_persisted_resolve_id)
    }
}

/// Rejection never consults the execution policy (nothing is executed).
struct PersistedRejectPolicy;

impl DecisionExecutionPolicyPort for PersistedRejectPolicy {
    fn current_policy(
        &self,
        _: &WorkManagementOperation,
    ) -> pmc_domain::decisions::DecisionExecutionPolicy {
        pmc_domain::decisions::DecisionExecutionPolicy::Allowed
    }
}

#[derive(Clone, Copy)]
struct AllowPersistedDecisionPolicy;
impl DecisionExecutionPolicyPort for AllowPersistedDecisionPolicy {
    fn current_policy(
        &self,
        _: &WorkManagementOperation,
    ) -> pmc_domain::decisions::DecisionExecutionPolicy {
        pmc_domain::decisions::DecisionExecutionPolicy::Allowed
    }
}

impl SqliteProductLedger {
    pub fn load_decision_persistence_snapshot(
        &self,
    ) -> Result<DecisionPersistenceSnapshot, DecisionPersistenceLoadError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(DecisionPersistenceLoadError::UnsupportedSchema {
                found: self.schema_version,
            });
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| DecisionPersistenceLoadError::StorageUnavailable)?;
        let snapshot = decode_namespace(&transaction)?;
        transaction
            .commit()
            .map_err(|_| DecisionPersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }

    pub fn create_decision_request_draft(
        &mut self,
        command: CreateDecisionRequestDraft,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? { return Err(idempotency_conflict(&context)); }
            if let Some(operation) = tx.query_row("SELECT operation FROM decision_replay_operations WHERE idempotency_id=?1", [context.idempotency_id.as_str()], |row| row.get::<_, String>(0)).optional().map_err(|_| storage_error(&context))? {
                if operation != "create_request" { return Err(idempotency_conflict(&context)); }
            }
            if let Some(existing) = tx.query_row(
                "SELECT request_id,subject,details,intended_owner_id,classification FROM decision_command_create_requests WHERE idempotency_id=?1",
                [context.idempotency_id.as_str()],
                |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,String>(4)?)),
            ).optional().map_err(|_| storage_error(&context))? {
                if existing.0 == command.id.as_str() && existing.1 == command.subject.as_str() && existing.2 == command.details.as_str() && existing.3.as_deref() == command.intended_owner.as_ref().map(StakeholderId::as_str) && existing.4 == command.classification.as_persisted() {
                    return snapshot_create_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            if tx.query_row("SELECT 1 FROM decision_requests WHERE id=?1", [command.id.as_str()], |_| Ok(())).optional().map_err(|_| storage_error(&context))?.is_some() {
                return Err(already_exists(&context));
            }
            let ordinal = next_decision_operation_ordinal(tx, &context)?;
            let request = DecisionRequestRecord::from_persisted_created_draft(command.id.clone(), command.subject.clone(), command.details.clone(), command.intended_owner.clone(), command.classification);
            let audit = create_audit(audit_event_id, occurred_at, &request, &context)?;
            tx.execute("INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'decision_request',1,?2,?3,?3)", rusqlite::params![request.id().as_str(), request.classification().as_persisted(), occurred_at.unix_millis()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_requests (id,subject,details,intended_owner_id,state,withdrawal_rationale,linked_decision_id) VALUES (?1,?2,?3,?4,'draft',NULL,NULL)", rusqlite::params![request.id().as_str(),request.subject().as_str(),request.details().as_str(),request.intended_owner().map(StakeholderId::as_str)]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_reference) VALUES (?1,'create_request',?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(),context.correlation_id.as_str(),ordinal,request.id().as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_command_create_requests (idempotency_id,request_id,subject,details,intended_owner_id,classification) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![context.idempotency_id.as_str(),request.id().as_str(),request.subject().as_str(),request.details().as_str(),request.intended_owner().map(StakeholderId::as_str),request.classification().as_persisted()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management','decision_request.created','decision_request',?3,?4,'allowed','not_required','succeeded','complete')", rusqlite::params![audit.id().as_str(),audit.occurred_at().unix_millis(),request.id().as_str(),context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,'decision_request.created','complete','decision_request',?2)", rusqlite::params![audit.id().as_str(),request.id().as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)", rusqlite::params![context.idempotency_id.as_str(),audit.id().as_str(),context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            let revised = tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))?;
            if revised != 1 { return Err(storage_error(&context)); }
            let snapshot = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            snapshot_create_outcome(snapshot, &context)
        })
    }

    /// Submit one persisted Draft Decision Request through the typed H1 seam.
    pub fn submit_decision_request(
        &mut self,
        command: SubmitDecisionRequest,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            if idempotency_claimed_by_other_namespace(tx, &context)? { return Err(idempotency_conflict(&context)); }
            if let Some(operation) = tx.query_row("SELECT operation FROM decision_replay_operations WHERE idempotency_id=?1", [context.idempotency_id.as_str()], |row| row.get::<_, String>(0)).optional().map_err(|_| storage_error(&context))? {
                if operation != "transition_request" { return Err(idempotency_conflict(&context)); }
            }
            if let Some(existing) = tx.query_row("SELECT request_id,expected_version,target_state,rationale FROM decision_command_transition_requests WHERE idempotency_id=?1", [context.idempotency_id.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?))).optional().map_err(|_| storage_error(&context))? {
                if existing.0 == command.request_id.as_str() && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1) && existing.2 == "open" && existing.3.is_none() { return snapshot_create_outcome(before, &context); }
                return Err(idempotency_conflict(&context));
            }
            let Some(previous) = before.requests().iter().find(|request| request.id() == &command.request_id).cloned() else { return Err(not_found(&context)); };
            if previous.state() != DecisionRequestState::Draft || previous.version() != command.expected_version { return Err(transition_conflict(&context)); }
            let next = DecisionRequestRecord::from_persisted_submitted_open(previous).ok_or_else(|| storage_error(&context))?;
            let ordinal = next_decision_operation_ordinal(tx, &context)?;
            let audit = create_transition_audit(audit_event_id, occurred_at, &next, &context)?;
            tx.execute("UPDATE decision_requests SET state='open' WHERE id=?1", [command.request_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("UPDATE aggregate_registry SET version=2,updated_at=?1 WHERE id=?2 AND aggregate_type='decision_request' AND version=1", rusqlite::params![occurred_at.unix_millis(), command.request_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_request_transitions (request_id,ordinal,from_state,to_state,rationale,occurred_at) VALUES (?1,0,'draft','open',NULL,?2)", rusqlite::params![command.request_id.as_str(), occurred_at.unix_millis()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_reference) VALUES (?1,'transition_request',?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, command.request_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_command_transition_requests (idempotency_id,request_id,expected_version,target_state,rationale) VALUES (?1,?2,?3,'open',NULL)", rusqlite::params![context.idempotency_id.as_str(), command.request_id.as_str(), i64::try_from(command.expected_version.get()).unwrap_or(-1)]).map_err(|_| storage_error(&context))?;
            insert_audit(tx, &audit, &context, "decision_request.submitted")?;
            tx.execute("INSERT INTO decision_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)", rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            snapshot_create_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context)
        })
    }

    /// Withdraw one persisted Open Decision Request through the typed H1 seam.
    pub fn withdraw_decision_request(
        &mut self,
        command: WithdrawDecisionRequest,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            if idempotency_claimed_by_other_namespace(tx, &context)? { return Err(idempotency_conflict(&context)); }
            if let Some(operation) = tx.query_row("SELECT operation FROM decision_replay_operations WHERE idempotency_id=?1", [context.idempotency_id.as_str()], |row| row.get::<_, String>(0)).optional().map_err(|_| storage_error(&context))? {
                if operation != "transition_request" { return Err(idempotency_conflict(&context)); }
            }
            if let Some(existing) = tx.query_row("SELECT request_id,expected_version,target_state,rationale FROM decision_command_transition_requests WHERE idempotency_id=?1", [context.idempotency_id.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?))).optional().map_err(|_| storage_error(&context))? {
                if existing.0 == command.request_id.as_str() && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1) && existing.2 == "withdrawn" && existing.3.as_deref() == Some(command.rationale.as_str()) { return snapshot_create_outcome(before, &context); }
                return Err(idempotency_conflict(&context));
            }
            let Some(previous) = before.requests().iter().find(|request| request.id() == &command.request_id).cloned() else { return Err(not_found(&context)); };
            if previous.state() != DecisionRequestState::Open || previous.version() != command.expected_version { return Err(transition_conflict(&context)); }
            let next = DecisionRequestRecord::from_persisted_open_to_withdrawn(previous, command.rationale.clone()).ok_or_else(|| storage_error(&context))?;
            let ordinal = next_decision_operation_ordinal(tx, &context)?;
            let audit = create_withdraw_audit(audit_event_id, occurred_at, &next, &context)?;
            tx.execute("UPDATE decision_requests SET state='withdrawn',withdrawal_rationale=?1 WHERE id=?2", rusqlite::params![command.rationale.as_str(), command.request_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("UPDATE aggregate_registry SET version=3,updated_at=?1 WHERE id=?2 AND aggregate_type='decision_request' AND version=2", rusqlite::params![occurred_at.unix_millis(), command.request_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_request_transitions (request_id,ordinal,from_state,to_state,rationale,occurred_at) VALUES (?1,1,'open','withdrawn',?2,?3)", rusqlite::params![command.request_id.as_str(), command.rationale.as_str(), occurred_at.unix_millis()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_reference) VALUES (?1,'transition_request',?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, command.request_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO decision_command_transition_requests (idempotency_id,request_id,expected_version,target_state,rationale) VALUES (?1,?2,?3,'withdrawn',?4)", rusqlite::params![context.idempotency_id.as_str(), command.request_id.as_str(), i64::try_from(command.expected_version.get()).unwrap_or(-1), command.rationale.as_str()]).map_err(|_| storage_error(&context))?;
            insert_audit(tx, &audit, &context, "decision_request.withdrawn")?;
            tx.execute("INSERT INTO decision_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)", rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            snapshot_create_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context)
        })
    }

    /// Persist one already-canonical H2a Decision Request resolution preview.
    ///
    /// The domain service is the only authority that may create `prepared`.
    /// This adapter verifies its exact topology before atomically storing the
    /// typed command and normalized preview records.
    pub fn prepare_resolve_decision_request(
        &mut self,
        command: pmc_domain::decisions::PrepareResolveDecisionRequest,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT request_id,expected_version,statement,rationale,impact,result_reference FROM decision_h2a_replay_operations replay JOIN decision_h2a_command_prepare_resolves command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let existing_evidence_ids = tx
                    .prepare(
                        "SELECT evidence_id FROM decision_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal",
                    )
                    .map_err(|_| storage_error(&context))?
                    .query_map([existing.5.as_deref().unwrap_or_default()], |row| row.get::<_, String>(0))
                    .map_err(|_| storage_error(&context))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| storage_error(&context))?;
                let command_evidence_ids = command
                    .evidence_ids
                    .iter()
                    .map(|evidence_id| evidence_id.as_str().to_owned())
                    .collect::<Vec<_>>();
                if existing.0 == command.request_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.statement.as_str()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == command.impact.as_str()
                    && existing.5.as_deref() == Some(prepared.id().as_str())
                    && existing_evidence_ids == command_evidence_ids
                {
                    return snapshot_prepared_outcome(
                        decode_namespace(tx).map_err(|_| storage_error(&context))?,
                        &context,
                    );
                }
                return Err(idempotency_conflict(&context));
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            let Some(request) = before.requests().iter().find(|item| item.id() == &command.request_id) else {
                return Err(not_found(&context));
            };
            if request.state() != DecisionRequestState::Open || request.version() != command.expected_version {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::ResolveDecisionRequest {
                request_id, request_version, decision_id: _, decision_classification, statement,
                rationale, impact, decision_owner, decided_at: _, resulting_action_requests: _,
            } = prepared.operation() else { return Err(transition_conflict(&context)); };
            if request_id != &command.request_id || request_version != &command.expected_version
                || decision_classification != &request.classification()
                || statement != &command.statement || rationale != &command.rationale || impact != &command.impact
                || request.intended_owner() != Some(decision_owner)
                || prepared.preview().support().is_none()
            { return Err(transition_conflict(&context)); }
            let ordinal = next_decision_operation_ordinal(tx, &context)?;
            persist_resolve_prepared(tx, &command, &prepared, request.classification(), &context)?;
            tx.execute(
                "INSERT INTO decision_h2a_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_resolve',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            insert_correlation_anchor(tx, &context)?;
            tx.execute(
                "INSERT INTO decision_h2a_command_prepare_resolves (idempotency_id,request_id,expected_version,statement,rationale,impact) VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![context.idempotency_id.as_str(), request_id.as_str(), i64::try_from(request_version.get()).unwrap_or(-1), statement.as_str(), rationale.as_str(), impact.as_str()],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            snapshot_prepared_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context)
        })
    }

    /// Execute one explicit H2a approval against the exact persisted Decision
    /// Request resolution preview.  The canonical domain service validates the
    /// human binding before this adapter writes its normalized commit bundle.
    /// H2a rejection (v45): the Head of Products refuses a pending Resolve
    /// preview. Mirrors the Action writer: one immediate transaction, the
    /// domain service rejects, then `consumed_at`, one zero-effect audit
    /// and the `decision_reject_prepared_command_results` row are written
    /// together with an ordinal from the global Decision operation stream.
    pub fn reject_decision_prepared_intent<Z>(
        &mut self,
        command: RejectDecisionPreparedIntent,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
        authorization: Z,
    ) -> Result<RejectedPreparedIntentOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? {
                return Err(idempotency_conflict(&context));
            }
            if let Some((stored_prepared, stored_actor)) = tx
                .query_row(
                    "SELECT prepared_intent_id,actor FROM decision_reject_prepared_command_results WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if stored_prepared != command.prepared_id.as_str()
                    || stored_actor != command.actor.as_persisted()
                {
                    return Err(idempotency_conflict(&context));
                }
                return decode_namespace(tx)
                    .map_err(|_| storage_error(&context))?
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .and_then(|capsule| match capsule.result() {
                        DecisionPersistenceResult::Rejected(outcome) => Some(outcome.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| storage_error(&context));
            }
            let claimed: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)",
                    [context.idempotency_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if claimed != 0 {
                return Err(idempotency_conflict(&context));
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            let mut service = InMemoryDecisionService::rehydrate(
                PersistedResolveClock(occurred_at),
                PersistedRejectIds {
                    audit_id: Some(audit_event_id.clone()),
                },
                authorization,
                PersistedRejectPolicy,
                PersistedDecisionEvidenceAuthority {
                    evidence: Vec::new(),
                },
                before,
            );
            let outcome = service.reject_decision_prepared_intent(command.clone())?;
            let ordinal = next_decision_operation_ordinal(tx, &context)?;
            persist_decision_prepared_intent_rejection(tx, &context, ordinal, &outcome)?;
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
            decode_namespace(tx).map_err(|_| storage_error(&context))?;
            Ok(outcome)
        })
    }

    #[allow(clippy::too_many_arguments)] // H2a's public seam mirrors its exact approval bundle.
    pub fn approve_and_execute_resolve_decision_request<Z, P, E>(
        &mut self,
        command: ApproveAndExecuteResolveDecisionRequest,
        audit_event_ids: [AuditEventId; 3],
        action_audit_event_ids: Vec<AuditEventId>,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
        evidence_authority: E,
    ) -> Result<ResolvedDecisionOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: DecisionExecutionPolicyPort,
        E: DecisionEvidenceAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? { return Err(idempotency_conflict(&context)); }
            if command.approval.idempotency_id() != &context.idempotency_id { return Err(idempotency_conflict(&context)); }
            if let Some(existing) = tx.query_row(
                "SELECT prepared_id,actor,acknowledged_digest FROM decision_h2a_command_execute_resolves WHERE idempotency_id=?1",
                [context.idempotency_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            ).optional().map_err(|_| storage_error(&context))? {
                if existing.0 != command.approval.prepared_id().as_str() || existing.1 != command.approval.actor().as_persisted() || existing.2 != command.approval.acknowledged_payload_digest().as_str() { return Err(idempotency_conflict(&context)); }
                return snapshot_resolved_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context);
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            // Decision-triggered Action provenance gap fix (2026-09): `actions`
            // must be the REAL rehydrated Action namespace, not a fresh
            // throwaway one -- its exported capsules (see
            // `persist_action_decision_capsules` below) carry an
            // `operation_ordinal` that must continue the real, globally
            // contiguous Action ordinal sequence (V36's trigger enforces this
            // at commit time), which only a real rehydrate produces correctly.
            let action_before =
                super::action_repository::decode_action_namespace(tx).map_err(|_| storage_error(&context))?;
            let already_known_action_capsules: std::collections::HashSet<String> = action_before
                .replay()
                .iter()
                .map(|capsule| capsule.idempotency_id().as_str().to_owned())
                .collect();
            let action_audit_count = action_audit_event_ids.len();
            let ids = PersistedResolveIds { receipt_id: Some(approval_receipt_id.clone()), audit_ids: audit_event_ids.into_iter() };
            let mut decisions = InMemoryDecisionService::rehydrate(
                PersistedResolveClock(occurred_at), ids, authorization, policy, evidence_authority, before,
            );
            let mut actions = InMemoryActionService::rehydrate(
                PersistedResolveClock(occurred_at),
                PersistedResolveActionExecuteIds { audit_ids: action_audit_event_ids.into_iter() },
                DenyWorkManagementApproval,
                AllowDecisionResultingActionPolicy,
                DenyActionEvidenceAuthority,
                action_before,
            );
            let outcome = decisions
                .approve_and_execute_resolve_decision_request(command.clone(), &mut actions)?;
            if outcome.audit_events.len() != 3 || outcome.approval_receipt_id != approval_receipt_id { return Err(storage_error(&context)); }
            if action_audit_count != outcome.resulting_action_request_ids.len() { return Err(storage_error(&context)); }
            persist_resolved_decision_bundle(tx, &command, &outcome, occurred_at, &context)?;
            let action_snapshot = actions
                .persistence_snapshot()
                .map_err(|_| storage_error(&context))?;
            persist_action_decision_capsules(tx, &already_known_action_capsules, &action_snapshot, &context)?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            decode_namespace(tx).map_err(|_| storage_error(&context))?;
            super::action_repository::decode_action_namespace(tx).map_err(|_| storage_error(&context))?;
            Ok(outcome)
        })
    }

    /// H2a step 1: persist one already-canonical Decision
    /// classification-lowering preview. Mirrors
    /// `prepare_resolve_decision_request` structurally, but simpler: no
    /// evidence/judgment support, and its own fresh-root V26 replay
    /// authority rather than V4's resolve-specific
    /// `decision_h2a_replay_operations`.
    pub fn prepare_lower_decision_classification(
        &mut self,
        command: PrepareLowerDecisionClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT decision_id,expected_version,proposed_classification,rationale,result_reference FROM decision_h2a_lower_classification_prepare_replay_operations replay JOIN decision_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.decision_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                {
                    return snapshot_prepared_outcome(
                        decode_namespace(tx).map_err(|_| storage_error(&context))?,
                        &context,
                    );
                }
                return Err(idempotency_conflict(&context));
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            let Some(decision) = before
                .decisions()
                .iter()
                .find(|item| item.id() == &command.decision_id)
            else {
                return Err(not_found(&context));
            };
            if decision.version() != command.expected_version {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::LowerDecisionClassification {
                decision_id,
                decision_version,
                current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if decision_id != &command.decision_id
                || decision_version != &command.expected_version
                || current_classification != &decision.classification()
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(transition_conflict(&context));
            }
            let ordinal =
                next_decision_operation_ordinal(tx, &context)?;
            persist_lower_decision_prepared(tx, &prepared, &context)?;
            tx.execute(
                "INSERT INTO decision_h2a_lower_classification_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO decision_h2a_lower_classification_command_prepares (idempotency_id,decision_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.decision_id.as_str(),
                    i64::try_from(command.expected_version.get())
                        .map_err(|_| storage_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            snapshot_prepared_outcome(
                decode_namespace(tx).map_err(|_| storage_error(&context))?,
                &context,
            )
        })
    }

    /// H2a step 2 for Decision. Mirrors
    /// `approve_and_execute_resolve_decision_request`'s rehydrate/execute/
    /// persist shape, but calls the plain (non-`_with_durability`) domain
    /// method: Decision has no durable post-start terminal for any H2a
    /// operation, so a domain failure here is an ordinary rollback via `?`.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_lower_decision_classification<Z, P, E>(
        &mut self,
        command: ApproveAndExecuteLowerDecisionClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
        evidence_authority: E,
    ) -> Result<DecisionMutationOutcome<DecisionRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: DecisionExecutionPolicyPort,
        E: DecisionEvidenceAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? {
                return Err(idempotency_conflict(&context));
            }
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM decision_h2a_lower_classification_command_executes WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
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
                return snapshot_lowered_outcome(
                    decode_namespace(tx).map_err(|_| storage_error(&context))?,
                    &context,
                );
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            let ids = PersistedLowerClassificationIds {
                receipt_id: Some(approval_receipt_id.clone()),
                audit_id: Some(audit_event_id.clone()),
            };
            let mut decisions = InMemoryDecisionService::rehydrate(
                PersistedResolveClock(occurred_at),
                ids,
                authorization,
                policy,
                evidence_authority,
                before,
            );
            let outcome =
                decisions.approve_and_execute_lower_decision_classification(command.clone())?;
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
            {
                return Err(storage_error(&context));
            }
            persist_lowered_decision_bundle(tx, &command, &outcome, occurred_at, &context)?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            decode_namespace(tx).map_err(|_| storage_error(&context))?;
            Ok(outcome)
        })
    }

    /// Persist one already-canonical Decision Supersede PREPARE preview.
    /// Mirrors `prepare_resolve_decision_request`'s "light, caller-supplied-
    /// canonical" shape exactly: the domain service is the only authority
    /// that may create `prepared` (which the caller computes by calling
    /// `InMemoryDecisionService::prepare_supersede_decision` against a real,
    /// already-rehydrated `InMemoryActionService` -- this adapter does not
    /// touch Action state itself, since `incomplete_downstream` is already
    /// baked into `prepared`'s operation by the time it reaches here). This
    /// adapter verifies its exact topology before atomically storing the
    /// typed command and normalized preview records.
    pub fn prepare_supersede_decision(
        &mut self,
        command: PrepareSupersedeDecision,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT decision_id,expected_version,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,result_reference FROM decision_h2a_replay_operations replay JOIN decision_h2a_command_prepare_supersedes command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, Option<String>>(6)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let existing_evidence_ids = tx
                    .prepare(
                        "SELECT evidence_id FROM decision_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal",
                    )
                    .map_err(|_| storage_error(&context))?
                    .query_map([existing.6.as_deref().unwrap_or_default()], |row| row.get::<_, String>(0))
                    .map_err(|_| storage_error(&context))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| storage_error(&context))?;
                let command_evidence_ids = command
                    .evidence_ids
                    .iter()
                    .map(|evidence_id| evidence_id.as_str().to_owned())
                    .collect::<Vec<_>>();
                if existing.0 == command.decision_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.replacement_statement.as_str()
                    && existing.3 == command.replacement_rationale.as_str()
                    && existing.4 == command.replacement_impact.as_str()
                    && existing.5 == command.replacement_owner.as_str()
                    && existing.6.as_deref() == Some(prepared.id().as_str())
                    && existing_evidence_ids == command_evidence_ids
                {
                    return snapshot_prepared_outcome(
                        decode_namespace(tx).map_err(|_| storage_error(&context))?,
                        &context,
                    );
                }
                return Err(idempotency_conflict(&context));
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            let Some(decision) = before.decisions().iter().find(|item| item.id() == &command.decision_id) else {
                return Err(not_found(&context));
            };
            if decision.state() != pmc_domain::work_management::DecisionState::Effective || decision.version() != command.expected_version {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::SupersedeDecision {
                decision_id, decision_version, replacement_statement, replacement_rationale,
                replacement_impact, replacement_owner, ..
            } = prepared.operation() else { return Err(transition_conflict(&context)); };
            if decision_id != &command.decision_id || decision_version != &command.expected_version
                || replacement_statement != &command.replacement_statement
                || replacement_rationale != &command.replacement_rationale
                || replacement_impact != &command.replacement_impact
                || replacement_owner != &command.replacement_owner
                || prepared.preview().support().is_none()
            { return Err(transition_conflict(&context)); }
            let ordinal = next_decision_operation_ordinal(tx, &context)?;
            persist_supersede_prepared(tx, &prepared, &context)?;
            tx.execute(
                "INSERT INTO decision_h2a_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_supersede',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            insert_correlation_anchor(tx, &context)?;
            tx.execute(
                "INSERT INTO decision_h2a_command_prepare_supersedes (idempotency_id,decision_id,expected_version,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![context.idempotency_id.as_str(), decision_id.as_str(), i64::try_from(decision_version.get()).unwrap_or(-1), replacement_statement.as_str(), replacement_rationale.as_str(), replacement_impact.as_str(), replacement_owner.as_str()],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            snapshot_prepared_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context)
        })
    }

    /// Execute one explicit approval against the exact persisted Decision
    /// Supersede preview. Mirrors
    /// `approve_and_execute_resolve_decision_request`'s rehydrate/execute/
    /// persist shape (real Action rehydration is mandatory here too --
    /// domain `execute_supersede` recomputes current downstream Action state
    /// and rejects if it has drifted from the prepared snapshot).
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_supersede_decision<Z, P, E>(
        &mut self,
        command: ApproveAndExecuteSupersedeDecision,
        audit_event_ids: [AuditEventId; 3],
        action_audit_event_ids: Vec<AuditEventId>,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
        evidence_authority: E,
    ) -> Result<SupersededDecisionOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: DecisionExecutionPolicyPort,
        E: DecisionEvidenceAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_namespace(tx, &context)? { return Err(idempotency_conflict(&context)); }
            if command.approval.idempotency_id() != &context.idempotency_id { return Err(idempotency_conflict(&context)); }
            if let Some(existing) = tx.query_row(
                "SELECT prepared_id,actor,acknowledged_digest FROM decision_h2a_command_execute_supersedes WHERE idempotency_id=?1",
                [context.idempotency_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            ).optional().map_err(|_| storage_error(&context))? {
                if existing.0 != command.approval.prepared_id().as_str() || existing.1 != command.approval.actor().as_persisted() || existing.2 != command.approval.acknowledged_payload_digest().as_str() { return Err(idempotency_conflict(&context)); }
                return snapshot_superseded_outcome(decode_namespace(tx).map_err(|_| storage_error(&context))?, &context);
            }
            let before = decode_namespace(tx).map_err(|_| storage_error(&context))?;
            let action_before =
                super::action_repository::decode_action_namespace(tx).map_err(|_| storage_error(&context))?;
            let already_known_action_capsules: std::collections::HashSet<String> = action_before
                .replay()
                .iter()
                .map(|capsule| capsule.idempotency_id().as_str().to_owned())
                .collect();
            let action_audit_count = action_audit_event_ids.len();
            let ids = PersistedResolveIds { receipt_id: Some(approval_receipt_id.clone()), audit_ids: audit_event_ids.into_iter() };
            let mut decisions = InMemoryDecisionService::rehydrate(
                PersistedResolveClock(occurred_at), ids, authorization, policy, evidence_authority, before,
            );
            let mut actions = InMemoryActionService::rehydrate(
                PersistedResolveClock(occurred_at),
                PersistedResolveActionExecuteIds { audit_ids: action_audit_event_ids.into_iter() },
                DenyWorkManagementApproval,
                AllowDecisionResultingActionPolicy,
                DenyActionEvidenceAuthority,
                action_before,
            );
            let outcome = decisions
                .approve_and_execute_supersede_decision(command.clone(), &mut actions)?;
            if outcome.audit_events.len() != 3 || outcome.approval_receipt_id != approval_receipt_id { return Err(storage_error(&context)); }
            if action_audit_count != outcome.resulting_action_request_ids.len() + outcome.flagged_downstream.len() { return Err(storage_error(&context)); }
            persist_superseded_decision_bundle(tx, &command, &outcome, occurred_at, &context)?;
            persist_supersede_downstream_action_rows(tx, &actions, &outcome, occurred_at, &context)?;
            let action_snapshot = actions
                .persistence_snapshot()
                .map_err(|_| storage_error(&context))?;
            persist_action_decision_capsules(tx, &already_known_action_capsules, &action_snapshot, &context)?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 { return Err(storage_error(&context)); }
            decode_namespace(tx).map_err(|_| storage_error(&context))?;
            super::action_repository::decode_action_namespace(tx).map_err(|_| storage_error(&context))?;
            Ok(outcome)
        })
    }
}

/// The single global ordinal source for every Decision capsule -- H1,
/// Resolve, and H2a Lower Data Classification alike. Decision's
/// restart decode is a full event-sourced replay
/// (`request_before_ordinal`/`decision_before_ordinal`/
/// `snapshot_before_ordinal` all reconstruct state "as of ordinal N" by
/// filtering the shared capsule timeline), which only works if every
/// capsule-producing table shares one gapless sequence -- see V26/V27's
/// matching global contiguous-insert triggers.
fn next_decision_operation_ordinal(
    tx: &Transaction<'_>,
    context: &DecisionOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM decision_replay_operations UNION ALL SELECT operation_ordinal FROM decision_h2a_replay_operations UNION ALL SELECT operation_ordinal FROM decision_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM decision_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM decision_reject_prepared_command_results)",
        [], |row| row.get(0),
    ).map_err(|_| storage_error(context))
}

/// Persist one already-canonical H2a Decision classification-lowering
/// preview. Mirrors `persist_resolve_prepared`'s `prepared_intents`/
/// `prepared_intent_targets`/`prepared_intent_effects` writes, but skips
/// everything support/evidence-specific (this operation needs none) and its
/// own fresh-root command table carries every scalar inline, so there is no
/// `prepared_work_management_payloads` row either.
fn persist_lower_decision_prepared(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    let WorkManagementOperation::LowerDecisionClassification {
        decision_id,
        decision_version,
        ..
    } = prepared.operation()
    else {
        return Err(storage_error(context));
    };
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'lower_decision_classification',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(prepared.preview().contract_version()),
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            prepared.preview().expires_at().unix_millis(),
            prepared.preview().expires_at().unix_millis()
                - pmc_domain::work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'decision',?2,?3)",
        rusqlite::params![
            prepared.id().as_str(),
            decision_id.as_str(),
            i64::try_from(decision_version.get()).map_err(|_| storage_error(context))?,
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal, effect) in prepared.effects().iter().enumerate() {
        let pmc_domain::work_management::WorkManagementEffect::LowerDecisionClassification(id) =
            effect
        else {
            return Err(storage_error(context));
        };
        tx.execute(
            "INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,?2,'decision.classification_lowered','decision',?3,NULL,NULL)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| storage_error(context))?,
                id.as_str(),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

/// Persist one successful Decision classification-lowering execution.
/// Mirrors `persist_resolved_decision_bundle`'s shape, but simpler: no
/// resulting Action Requests, and `aggregate_registry` is an UPDATE (the
/// decision already exists) rather than the resolve path's INSERT.
fn persist_lowered_decision_bundle(
    tx: &Transaction<'_>,
    command: &ApproveAndExecuteLowerDecisionClassification,
    outcome: &DecisionMutationOutcome<DecisionRecord>,
    occurred_at: UtcTimestamp,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    let prepared_id = command.approval.prepared_id().as_str();
    let ordinal = next_decision_operation_ordinal(tx, context)?;
    if tx
        .execute(
            "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='decision'",
            rusqlite::params![
                i64::try_from(outcome.record.version().get())
                    .map_err(|_| storage_error(context))?,
                outcome.record.classification().as_persisted(),
                occurred_at.unix_millis(),
                outcome.record.id().as_str(),
            ],
        )
        .map_err(|_| storage_error(context))?
        != 1
    {
        return Err(storage_error(context));
    }
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_at.unix_millis(), prepared_id],
    )
    .map_err(|_| storage_error(context))?;
    let approval_receipt_id = outcome
        .approval_receipt_id
        .clone()
        .ok_or_else(|| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![
            approval_receipt_id.as_str(),
            context.idempotency_id.as_str(),
            occurred_at.unix_millis(),
            prepared_id
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decision_h2a_lower_classification_execute_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,decision_id,prepared_intent_id,approval_receipt_id) VALUES (?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            outcome.record.id().as_str(),
            prepared_id,
            approval_receipt_id.as_str()
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decision_h2a_lower_classification_command_executes (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            prepared_id,
            command.approval.acknowledged_payload_digest().as_str()
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (index, audit) in outcome.audit_events.iter().enumerate() {
        let (target_type, target_id) = match audit.target() {
            AuditTarget::Decision(id) => ("decision", id.as_str()),
            _ => return Err(storage_error(context)),
        };
        tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')", rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete',?3,?4)", rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_type, target_id]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_h2a_lower_classification_execute_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), i64::try_from(index).map_err(|_| storage_error(context))?, audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn persist_resolved_decision_bundle(
    tx: &Transaction<'_>,
    command: &ApproveAndExecuteResolveDecisionRequest,
    outcome: &ResolvedDecisionOutcome,
    occurred_at: UtcTimestamp,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    let prepared_id = command.approval.prepared_id().as_str();
    let support_id: String = tx.query_row(
        "SELECT support_id FROM prepared_intents WHERE id=?1 AND intent_kind='resolve_decision_request' AND consumed_at IS NULL",
        [prepared_id], |row| row.get(0),
    ).map_err(|_| storage_error(context))?;
    let ordinal = next_decision_operation_ordinal(tx, context)?;
    tx.execute(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'decision',1,?2,?3,?3)",
        rusqlite::params![outcome.decision.id().as_str(), outcome.decision.classification().as_persisted(), occurred_at.unix_millis()],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decisions (id,source_request_id,statement,rationale,impact,owner_id,decided_at,state,support_id,supersedes_decision_id,superseded_by_decision_id) VALUES (?1,?2,?3,?4,?5,?6,?7,'effective',?8,NULL,NULL)",
        rusqlite::params![outcome.decision.id().as_str(), outcome.request.id().as_str(), outcome.decision.statement().as_str(), outcome.decision.rationale().as_str(), outcome.decision.impact().as_str(), outcome.decision.owner().as_str(), outcome.decision.decided_at().unix_millis(), support_id],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE decision_requests SET state='resolved',linked_decision_id=?1 WHERE id=?2 AND state='open'",
        rusqlite::params![outcome.decision.id().as_str(), outcome.request.id().as_str()],
    ).map_err(|_| storage_error(context))?;
    if tx.execute(
        "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='decision_request'",
        rusqlite::params![i64::try_from(outcome.request.version().get()).map_err(|_| storage_error(context))?, occurred_at.unix_millis(), outcome.request.id().as_str()],
    ).map_err(|_| storage_error(context))? != 1 { return Err(storage_error(context)); }
    for action_request_id in &outcome.resulting_action_request_ids {
        let item = tx.query_row(
            "SELECT subject,details,intended_owner_id,due_at,classification FROM prepared_resulting_action_requests WHERE prepared_intent_id=?1 AND action_request_id=?2",
            rusqlite::params![prepared_id, action_request_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, String>(4)?)),
        ).map_err(|_| storage_error(context))?;
        let item_classification =
            DataClassification::from_persisted(&item.4).map_err(|_| storage_error(context))?;
        let classification = outcome
            .decision
            .classification()
            .combine(item_classification);
        tx.execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'action_request',1,?2,?3,?3)",
            rusqlite::params![action_request_id.as_str(), classification.as_persisted(), occurred_at.unix_millis()],
        ).map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO action_requests (id,title,details,intended_owner_id,response_due_at,intended_action_due_at,state,terminal_rationale,linked_action_id,source_decision_id,superseded_premise) VALUES (?1,?2,?3,?4,NULL,?5,'open',NULL,NULL,?6,0)",
            rusqlite::params![action_request_id.as_str(), item.0, item.1, item.2, item.3, outcome.decision.id().as_str()],
        ).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_resulting_action_requests (decision_id,action_request_id) VALUES (?1,?2)", rusqlite::params![outcome.decision.id().as_str(), action_request_id.as_str()]).map_err(|_| storage_error(context))?;
    }
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_at.unix_millis(), prepared_id],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![outcome.approval_receipt_id.as_str(), context.idempotency_id.as_str(), occurred_at.unix_millis(), prepared_id],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decision_h2a_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,approval_receipt_id) VALUES (?1,'execute_resolve',?2,?3,'resolved',?4,?5,?6)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, outcome.decision.id().as_str(), prepared_id, outcome.approval_receipt_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    insert_correlation_anchor(tx, context)?;
    tx.execute(
        "INSERT INTO decision_h2a_command_execute_resolves (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), prepared_id, command.approval.acknowledged_payload_digest().as_str()],
    ).map_err(|_| storage_error(context))?;
    for (index, audit) in outcome.audit_events.iter().enumerate() {
        let (target_type, target_id) = match audit.target() {
            AuditTarget::DecisionRequest(id) => ("decision_request", id.as_str()),
            AuditTarget::Decision(id) => ("decision", id.as_str()),
            _ => return Err(storage_error(context)),
        };
        tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')", rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete',?3,?4)", rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_type, target_id]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_h2a_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), i64::try_from(index).map_err(|_| storage_error(context))?, audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn persist_superseded_decision_bundle(
    tx: &Transaction<'_>,
    command: &ApproveAndExecuteSupersedeDecision,
    outcome: &SupersededDecisionOutcome,
    occurred_at: UtcTimestamp,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    let prepared_id = command.approval.prepared_id().as_str();
    let support_id: String = tx.query_row(
        "SELECT support_id FROM prepared_intents WHERE id=?1 AND intent_kind='supersede_decision' AND consumed_at IS NULL",
        [prepared_id], |row| row.get(0),
    ).map_err(|_| storage_error(context))?;
    let ordinal = next_decision_operation_ordinal(tx, context)?;
    // The replacement decision row must exist BEFORE the old decision's
    // `superseded_by_decision_id` can reference it (`decisions.superseded_by_decision_id`
    // is an immediate, non-deferrable FK to `decisions(id)`).
    tx.execute(
        "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'decision',1,?2,?3,?3)",
        rusqlite::params![outcome.replacement.id().as_str(), outcome.replacement.classification().as_persisted(), occurred_at.unix_millis()],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decisions (id,source_request_id,statement,rationale,impact,owner_id,decided_at,state,support_id,supersedes_decision_id,superseded_by_decision_id) VALUES (?1,NULL,?2,?3,?4,?5,?6,'effective',?7,?8,NULL)",
        rusqlite::params![outcome.replacement.id().as_str(), outcome.replacement.statement().as_str(), outcome.replacement.rationale().as_str(), outcome.replacement.impact().as_str(), outcome.replacement.owner().as_str(), outcome.replacement.decided_at().unix_millis(), support_id, outcome.superseded.id().as_str()],
    ).map_err(|_| storage_error(context))?;
    if tx.execute(
        "UPDATE decisions SET state='superseded',superseded_by_decision_id=?1 WHERE id=?2 AND state='effective'",
        rusqlite::params![outcome.replacement.id().as_str(), outcome.superseded.id().as_str()],
    ).map_err(|_| storage_error(context))? != 1 {
        return Err(storage_error(context));
    }
    if tx.execute(
        "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='decision'",
        rusqlite::params![i64::try_from(outcome.superseded.version().get()).map_err(|_| storage_error(context))?, occurred_at.unix_millis(), outcome.superseded.id().as_str()],
    ).map_err(|_| storage_error(context))? != 1 {
        return Err(storage_error(context));
    }
    for action_request_id in &outcome.resulting_action_request_ids {
        let item = tx.query_row(
            "SELECT subject,details,intended_owner_id,due_at,classification FROM prepared_resulting_action_requests WHERE prepared_intent_id=?1 AND action_request_id=?2",
            rusqlite::params![prepared_id, action_request_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, String>(4)?)),
        ).map_err(|_| storage_error(context))?;
        let item_classification =
            DataClassification::from_persisted(&item.4).map_err(|_| storage_error(context))?;
        let classification = outcome
            .replacement
            .classification()
            .combine(item_classification);
        tx.execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'action_request',1,?2,?3,?3)",
            rusqlite::params![action_request_id.as_str(), classification.as_persisted(), occurred_at.unix_millis()],
        ).map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO action_requests (id,title,details,intended_owner_id,response_due_at,intended_action_due_at,state,terminal_rationale,linked_action_id,source_decision_id,superseded_premise) VALUES (?1,?2,?3,?4,NULL,?5,'open',NULL,NULL,?6,0)",
            rusqlite::params![action_request_id.as_str(), item.0, item.1, item.2, item.3, outcome.replacement.id().as_str()],
        ).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_resulting_action_requests (decision_id,action_request_id) VALUES (?1,?2)", rusqlite::params![outcome.replacement.id().as_str(), action_request_id.as_str()]).map_err(|_| storage_error(context))?;
    }
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_at.unix_millis(), prepared_id],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![outcome.approval_receipt_id.as_str(), context.idempotency_id.as_str(), occurred_at.unix_millis(), prepared_id],
    ).map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decision_h2a_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,approval_receipt_id) VALUES (?1,'execute_supersede',?2,?3,'superseded',?4,?5,?6)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, outcome.superseded.id().as_str(), prepared_id, outcome.approval_receipt_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    insert_correlation_anchor(tx, context)?;
    tx.execute(
        "INSERT INTO decision_h2a_command_execute_supersedes (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), prepared_id, command.approval.acknowledged_payload_digest().as_str()],
    ).map_err(|_| storage_error(context))?;
    for (index, audit) in outcome.audit_events.iter().enumerate() {
        let target_id = match audit.target() {
            AuditTarget::Decision(id) => id.as_str(),
            _ => return Err(storage_error(context)),
        };
        tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,'decision',?4,?5,'allowed','approved','succeeded','complete')", rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_id, context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete','decision',?3)", rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_id]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_h2a_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), i64::try_from(index).map_err(|_| storage_error(context))?, audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
    }
    Ok(())
}

/// The NEW authoritative Action writes Decision Supersede's downstream
/// marking needs, which `persist_action_decision_capsules` deliberately does
/// NOT perform (see its own doc comment: it only ever wrote the *replay
/// authority* -- V36's `action_decision_replay_operations` and friends --
/// plus the real Action-targeted audit event, reusing `action_requests`/
/// `actions`/`aggregate_registry` content rows some OTHER write already
/// made). For Resolve, that other write is `persist_resolved_decision_bundle`'s
/// own resulting-action-request INSERT loop. Supersede's downstream marks
/// are not INSERTs though -- they are UPDATEs of already-durable Action
/// Request/Action rows, and nothing else in this transaction performs them,
/// so this function must. `actions` is the REAL, already-mutated
/// `InMemoryActionService` `execute_supersede` just ran against, so the
/// final (already-`combine`d) classification and version are read directly
/// off it rather than recomputed by hand here.
fn persist_supersede_downstream_action_rows<C, I, Z, P, E>(
    tx: &Transaction<'_>,
    actions: &InMemoryActionService<C, I, Z, P, E>,
    outcome: &SupersededDecisionOutcome,
    occurred_at: UtcTimestamp,
    context: &DecisionOperationContext,
) -> Result<(), DomainError>
where
    C: Clock,
    I: pmc_domain::actions::ActionServiceIdSource,
    Z: ApprovalAuthorizationPort,
    P: ActionExecutionPolicyPort,
    E: pmc_domain::actions::ActionEvidenceAuthorityPort,
{
    for item in &outcome.flagged_downstream {
        match item {
            pmc_domain::work_management::IncompleteDownstreamWork::ActionRequest(id, _, _) => {
                let marked = actions.request(id).ok_or_else(|| storage_error(context))?;
                if tx.execute(
                    "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='action_request'",
                    rusqlite::params![i64::try_from(marked.version().get()).map_err(|_| storage_error(context))?, marked.classification().as_persisted(), occurred_at.unix_millis(), id.as_str()],
                ).map_err(|_| storage_error(context))? != 1 {
                    return Err(storage_error(context));
                }
                if tx
                    .execute(
                        "UPDATE action_requests SET superseded_premise=1 WHERE id=?1",
                        [id.as_str()],
                    )
                    .map_err(|_| storage_error(context))?
                    != 1
                {
                    return Err(storage_error(context));
                }
            }
            pmc_domain::work_management::IncompleteDownstreamWork::Action(id, _, _) => {
                let marked = actions.action(id).ok_or_else(|| storage_error(context))?;
                if tx.execute(
                    "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='action'",
                    rusqlite::params![i64::try_from(marked.version().get()).map_err(|_| storage_error(context))?, marked.classification().as_persisted(), occurred_at.unix_millis(), id.as_str()],
                ).map_err(|_| storage_error(context))? != 1 {
                    return Err(storage_error(context));
                }
                if tx.execute(
                    "UPDATE actions SET superseded_premise=1,commitment_classification=?1 WHERE id=?2",
                    rusqlite::params![marked.commitment_classification().as_persisted(), id.as_str()],
                ).map_err(|_| storage_error(context))? != 1 {
                    return Err(storage_error(context));
                }
            }
        }
    }
    Ok(())
}

/// Persist every Action-side capsule a Decision execute (Resolve today,
/// Supersede once it lands) newly staged on the real, rehydrated
/// `InMemoryActionService` -- `already_known` is the idempotency-id set
/// already durable before this call (from decoding the real Action
/// namespace up front), so this only writes what is genuinely new,
/// regardless of how many decision-triggered mutations one execute call
/// produces. Reuses `action_requests`/`aggregate_registry`'s own content
/// rows, already written by `persist_resolved_decision_bundle`'s resulting-
/// action-request loop above -- this only adds the missing *replay
/// authority* (V36's `action_decision_replay_operations` and friends) plus
/// the real Action-targeted audit event these mutations always produce
/// in-domain but this ledger used to silently discard.
fn persist_action_decision_capsules(
    tx: &Transaction<'_>,
    already_known: &std::collections::HashSet<String>,
    snapshot: &pmc_domain::actions::ActionPersistenceSnapshot,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    for capsule in snapshot.replay() {
        let idempotency_id = capsule.idempotency_id().as_str();
        if already_known.contains(idempotency_id) {
            continue;
        }
        let ordinal =
            i64::try_from(capsule.operation_ordinal()).map_err(|_| storage_error(context))?;
        let correlation_id = capsule.original_correlation_id().as_str();
        let audit_events: &[AuditEvent] = match (capsule.command(), capsule.result()) {
            (
                pmc_domain::actions::ActionPersistenceCommand::CreateRequestFromDecision {
                    request,
                    source_decision_id,
                    classification,
                },
                pmc_domain::actions::ActionPersistenceResult::Request(outcome),
            ) => {
                tx.execute(
                    "INSERT INTO action_decision_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,source_decision_id) VALUES (?1,'create_from_decision',?2,?3,'request',?4,?5)",
                    rusqlite::params![idempotency_id, correlation_id, ordinal, request.id.as_str(), source_decision_id.as_str()],
                ).map_err(|_| storage_error(context))?;
                tx.execute(
                    "INSERT INTO action_command_create_from_decisions (idempotency_id,request_id,title,details,intended_owner_id,intended_action_due_at,classification,source_decision_id) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    rusqlite::params![idempotency_id, request.id.as_str(), request.subject.as_str(), request.details.as_str(), request.intended_owner.as_str(), request.due_at.unix_millis(), classification.as_persisted(), source_decision_id.as_str()],
                ).map_err(|_| storage_error(context))?;
                &outcome.audit_events
            }
            (
                pmc_domain::actions::ActionPersistenceCommand::MarkRequestSupersededPremise {
                    request_id,
                    expected_version,
                    source_decision_id,
                    classification,
                },
                pmc_domain::actions::ActionPersistenceResult::Request(outcome),
            ) => {
                tx.execute(
                    "INSERT INTO action_decision_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,source_decision_id) VALUES (?1,'mark_request_superseded_premise',?2,?3,'request',?4,?5)",
                    rusqlite::params![idempotency_id, correlation_id, ordinal, request_id.as_str(), source_decision_id.as_str()],
                ).map_err(|_| storage_error(context))?;
                tx.execute(
                    "INSERT INTO action_command_mark_request_superseded_premises (idempotency_id,request_id,expected_version,source_decision_id,classification) VALUES (?1,?2,?3,?4,?5)",
                    rusqlite::params![idempotency_id, request_id.as_str(), i64::try_from(expected_version.get()).map_err(|_| storage_error(context))?, source_decision_id.as_str(), classification.as_persisted()],
                ).map_err(|_| storage_error(context))?;
                &outcome.audit_events
            }
            (
                pmc_domain::actions::ActionPersistenceCommand::MarkActionSupersededPremise {
                    action_id,
                    expected_version,
                    source_decision_id,
                    classification,
                },
                pmc_domain::actions::ActionPersistenceResult::Action(outcome),
            ) => {
                tx.execute(
                    "INSERT INTO action_decision_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,source_decision_id) VALUES (?1,'mark_action_superseded_premise',?2,?3,'action',?4,?5)",
                    rusqlite::params![idempotency_id, correlation_id, ordinal, action_id.as_str(), source_decision_id.as_str()],
                ).map_err(|_| storage_error(context))?;
                tx.execute(
                    "INSERT INTO action_command_mark_action_superseded_premises (idempotency_id,action_id,expected_version,source_decision_id,classification) VALUES (?1,?2,?3,?4,?5)",
                    rusqlite::params![idempotency_id, action_id.as_str(), i64::try_from(expected_version.get()).map_err(|_| storage_error(context))?, source_decision_id.as_str(), classification.as_persisted()],
                ).map_err(|_| storage_error(context))?;
                &outcome.audit_events
            }
            // Every other Action command this crate does not (yet) route
            // through the Decision execute seam -- fail closed rather than
            // silently drop an unexpected capsule.
            _ => return Err(storage_error(context)),
        };
        if audit_events.len() != 1 {
            return Err(storage_error(context));
        }
        let audit = &audit_events[0];
        let (target_type, target_id) = match audit.target() {
            AuditTarget::ActionRequest(id) => ("action_request", id.as_str()),
            AuditTarget::Action(id) => ("action", id.as_str()),
            _ => return Err(storage_error(context)),
        };
        tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')", rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, correlation_id]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete',?3,?4)", rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_type, target_id]).map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO action_decision_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)", rusqlite::params![idempotency_id, audit.id().as_str(), correlation_id]).map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn persist_resolve_prepared(
    tx: &Transaction<'_>,
    command: &pmc_domain::decisions::PrepareResolveDecisionRequest,
    prepared: &WorkManagementPreparedIntent,
    target_classification: DataClassification,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    let WorkManagementOperation::ResolveDecisionRequest {
        request_id,
        request_version,
        decision_id,
        decision_classification,
        statement,
        rationale,
        impact,
        decision_owner,
        decided_at,
        resulting_action_requests,
    } = prepared.operation()
    else {
        return Err(storage_error(context));
    };
    let support = prepared
        .preview()
        .support()
        .ok_or_else(|| storage_error(context))?;
    let support_id = format!("decision-h2a-support-{}", prepared.id().as_str());
    let (disposition, judgments) = match support.disposition() {
        pmc_domain::work_management::SupportDisposition::EvidenceSatisfied => {
            ("evidence_satisfied", support.judgments())
        }
        pmc_domain::work_management::SupportDisposition::JudgmentSatisfied => {
            ("judgment_satisfied", support.judgments())
        }
        pmc_domain::work_management::SupportDisposition::VerificationPending => {
            ("verification_pending", support.judgments())
        }
    };
    tx.execute("INSERT INTO support_witnesses (id,requirement,disposition,classification) VALUES (?1,'evidence_or_judgment',?2,?3)", rusqlite::params![support_id, disposition, support.classification().as_persisted()]).map_err(|_| storage_error(context))?;
    tx.execute("INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'resolve_decision_request',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',?5,NULL,?6,?7)", rusqlite::params![prepared.id().as_str(), i64::from(prepared.preview().contract_version()), prepared.payload_digest().as_str(), prepared.classification().as_persisted(), support_id, prepared.preview().expires_at().unix_millis(), prepared.preview().expires_at().unix_millis() - 300_000]).map_err(|_| storage_error(context))?;
    for (ordinal, judgment) in judgments.iter().enumerate() {
        tx.execute("INSERT INTO support_judgments (support_id,ordinal,actor,disposition,rationale,classification) VALUES (?1,?2,'head_of_products','proceed_with_documented_rationale',?3,?4)", rusqlite::params![support_id, i64::try_from(ordinal).map_err(|_| storage_error(context))?, judgment.rationale(), judgment.classification().as_persisted()]).map_err(|_| storage_error(context))?;
    }
    for (ordinal, evidence) in support.evidence().iter().enumerate() {
        let (verification, last_verified_at, integrity_digest) = match evidence.verification() {
            EvidenceVerification::Verified {
                verified_at,
                integrity_digest,
            } => (
                "verified",
                Some(verified_at.unix_millis()),
                Some(integrity_digest.as_str()),
            ),
            EvidenceVerification::ObservedUnpinned {
                observed_at,
                integrity_digest,
            } => (
                "observed_unpinned",
                Some(observed_at.unix_millis()),
                Some(integrity_digest.as_str()),
            ),
            EvidenceVerification::DegradedLastVerified {
                last_verified_at,
                integrity_digest,
            } => (
                "degraded_last_verified",
                Some(last_verified_at.unix_millis()),
                Some(integrity_digest.as_str()),
            ),
            EvidenceVerification::Unverified => ("unverified", None, None),
            EvidenceVerification::IntegrityMismatch => ("integrity_mismatch", None, None),
        };
        if evidence.role() != EvidenceRole::DecisionResolution {
            return Err(storage_error(context));
        }
        tx.execute(
            "INSERT INTO support_evidence (support_id,evidence_id) VALUES (?1,?2)",
            rusqlite::params![support_id, evidence.id().as_str()],
        )
        .map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_h2a_support_evidence_snapshots (prepared_intent_id,ordinal,evidence_id,evidence_version,classification,role,verification,last_verified_at,integrity_digest) VALUES (?1,?2,?3,?8,?4,'decision_resolution',?5,?6,?7)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, evidence.id().as_str(), evidence.classification().as_persisted(), verification, last_verified_at, integrity_digest, i64::try_from(evidence.source_version().get()).map_err(|_| storage_error(context))?]).map_err(|_| storage_error(context))?;
    }
    tx.execute("INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,created_id,created_classification,statement,rationale,impact,decision_owner_id,decided_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", rusqlite::params![prepared.id().as_str(), request_id.as_str(), i64::try_from(request_version.get()).map_err(|_| storage_error(context))?, decision_id.as_str(), decision_classification.as_persisted(), statement.as_str(), rationale.as_str(), impact.as_str(), decision_owner.as_str(), decided_at.unix_millis()]).map_err(|_| storage_error(context))?;
    tx.execute("INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'decision_request',?2,?3)", rusqlite::params![prepared.id().as_str(), request_id.as_str(), i64::try_from(request_version.get()).map_err(|_| storage_error(context))?]).map_err(|_| storage_error(context))?;
    let effects = prepared.effects();
    for (ordinal, effect) in effects.iter().enumerate() {
        let (code, target_type, target_id, secondary_type, secondary_id) = match effect {
            pmc_domain::work_management::WorkManagementEffect::ResolveDecisionRequest(id) => (
                "decision_request.resolved",
                "decision_request",
                id.as_str(),
                None,
                None,
            ),
            pmc_domain::work_management::WorkManagementEffect::CreateDecision(id) => {
                ("decision.created", "decision", id.as_str(), None, None)
            }
            pmc_domain::work_management::WorkManagementEffect::LinkDecisionRequestToDecision(
                left,
                right,
            ) => (
                "decision_request.decision_linked",
                "decision_request",
                left.as_str(),
                Some("decision"),
                Some(right.as_str()),
            ),
            pmc_domain::work_management::WorkManagementEffect::CreateResultingActionRequest(id) => {
                (
                    "action_request.created_from_decision",
                    "action_request",
                    id.as_str(),
                    None,
                    None,
                )
            }
            pmc_domain::work_management::WorkManagementEffect::LinkDecisionToActionRequest(
                left,
                right,
            ) => (
                "decision.action_request_linked",
                "decision",
                left.as_str(),
                Some("action_request"),
                Some(right.as_str()),
            ),
            _ => return Err(storage_error(context)),
        };
        tx.execute("INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, code, target_type, target_id, secondary_type, secondary_id]).map_err(|_| storage_error(context))?;
    }
    for (ordinal, item) in resulting_action_requests.iter().enumerate() {
        tx.execute("INSERT INTO prepared_resulting_action_requests (prepared_intent_id,ordinal,action_request_id,subject,details,intended_owner_id,due_at,classification) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, item.id.as_str(), item.subject.as_str(), item.details.as_str(), item.intended_owner.as_str(), item.due_at.unix_millis(), item.classification.as_persisted()]).map_err(|_| storage_error(context))?;
    }
    let preview_evidence_ids = support
        .evidence()
        .iter()
        .map(|evidence| evidence.id())
        .collect::<Vec<_>>();
    if command.evidence_ids.iter().collect::<Vec<_>>() != preview_evidence_ids
        || target_classification != *decision_classification
    {
        return Err(storage_error(context));
    }
    Ok(())
}

/// Persist one already-canonical Decision Supersede PREPARE preview. Mirrors
/// `persist_resolve_prepared`'s support/evidence/judgment writes exactly
/// (reused as-is -- Supersede's own `support()` call goes through the same
/// `EvidenceRole::DecisionResolution` seam Resolve uses), but targets the
/// OLD decision (not a request) at ordinal 0 in `prepared_intent_targets`,
/// uses `prepared_work_management_payloads`'s dedicated `replacement_*`
/// columns (not the generic `created_*`/`statement` columns Resolve uses:
/// reusing those would erase the stored operation-shape discriminator), and
/// additionally persists
/// `incomplete_downstream` into the pre-existing V1 `prepared_incomplete_downstream`
/// table (already shaped exactly for `IncompleteDownstreamWork`, no new
/// migration needed).
fn persist_supersede_prepared(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    let WorkManagementOperation::SupersedeDecision {
        decision_id,
        decision_version,
        replacement_decision_id,
        replacement_decision_version,
        replacement_decision_classification,
        replacement_statement,
        replacement_rationale,
        replacement_impact,
        replacement_owner,
        replacement_decided_at,
        resulting_action_requests,
        incomplete_downstream,
    } = prepared.operation()
    else {
        return Err(storage_error(context));
    };
    let support = prepared
        .preview()
        .support()
        .ok_or_else(|| storage_error(context))?;
    let support_id = format!("decision-h2a-support-{}", prepared.id().as_str());
    let (disposition, judgments) = match support.disposition() {
        pmc_domain::work_management::SupportDisposition::EvidenceSatisfied => {
            ("evidence_satisfied", support.judgments())
        }
        pmc_domain::work_management::SupportDisposition::JudgmentSatisfied => {
            ("judgment_satisfied", support.judgments())
        }
        pmc_domain::work_management::SupportDisposition::VerificationPending => {
            ("verification_pending", support.judgments())
        }
    };
    tx.execute("INSERT INTO support_witnesses (id,requirement,disposition,classification) VALUES (?1,'evidence_or_judgment',?2,?3)", rusqlite::params![support_id, disposition, support.classification().as_persisted()]).map_err(|_| storage_error(context))?;
    tx.execute("INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'supersede_decision',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',?5,NULL,?6,?7)", rusqlite::params![prepared.id().as_str(), i64::from(prepared.preview().contract_version()), prepared.payload_digest().as_str(), prepared.classification().as_persisted(), support_id, prepared.preview().expires_at().unix_millis(), prepared.preview().expires_at().unix_millis() - pmc_domain::work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS]).map_err(|_| storage_error(context))?;
    for (ordinal, judgment) in judgments.iter().enumerate() {
        tx.execute("INSERT INTO support_judgments (support_id,ordinal,actor,disposition,rationale,classification) VALUES (?1,?2,'head_of_products','proceed_with_documented_rationale',?3,?4)", rusqlite::params![support_id, i64::try_from(ordinal).map_err(|_| storage_error(context))?, judgment.rationale(), judgment.classification().as_persisted()]).map_err(|_| storage_error(context))?;
    }
    for (ordinal, evidence) in support.evidence().iter().enumerate() {
        let (verification, last_verified_at, integrity_digest) = match evidence.verification() {
            EvidenceVerification::Verified {
                verified_at,
                integrity_digest,
            } => (
                "verified",
                Some(verified_at.unix_millis()),
                Some(integrity_digest.as_str()),
            ),
            EvidenceVerification::ObservedUnpinned {
                observed_at,
                integrity_digest,
            } => (
                "observed_unpinned",
                Some(observed_at.unix_millis()),
                Some(integrity_digest.as_str()),
            ),
            EvidenceVerification::DegradedLastVerified {
                last_verified_at,
                integrity_digest,
            } => (
                "degraded_last_verified",
                Some(last_verified_at.unix_millis()),
                Some(integrity_digest.as_str()),
            ),
            EvidenceVerification::Unverified => ("unverified", None, None),
            EvidenceVerification::IntegrityMismatch => ("integrity_mismatch", None, None),
        };
        if evidence.role() != EvidenceRole::DecisionResolution {
            return Err(storage_error(context));
        }
        tx.execute(
            "INSERT INTO support_evidence (support_id,evidence_id) VALUES (?1,?2)",
            rusqlite::params![support_id, evidence.id().as_str()],
        )
        .map_err(|_| storage_error(context))?;
        tx.execute("INSERT INTO decision_h2a_support_evidence_snapshots (prepared_intent_id,ordinal,evidence_id,evidence_version,classification,role,verification,last_verified_at,integrity_digest) VALUES (?1,?2,?3,?8,?4,'decision_resolution',?5,?6,?7)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, evidence.id().as_str(), evidence.classification().as_persisted(), verification, last_verified_at, integrity_digest, i64::try_from(evidence.source_version().get()).map_err(|_| storage_error(context))?]).map_err(|_| storage_error(context))?;
    }
    tx.execute("INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", rusqlite::params![prepared.id().as_str(), decision_id.as_str(), i64::try_from(decision_version.get()).map_err(|_| storage_error(context))?, replacement_decision_id.as_str(), i64::try_from(replacement_decision_version.get()).map_err(|_| storage_error(context))?, replacement_decision_classification.as_persisted(), replacement_statement.as_str(), replacement_rationale.as_str(), replacement_impact.as_str(), replacement_owner.as_str(), replacement_decided_at.unix_millis()]).map_err(|_| storage_error(context))?;
    tx.execute("INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'decision',?2,?3)", rusqlite::params![prepared.id().as_str(), decision_id.as_str(), i64::try_from(decision_version.get()).map_err(|_| storage_error(context))?]).map_err(|_| storage_error(context))?;
    for (ordinal, item) in resulting_action_requests.iter().enumerate() {
        tx.execute("INSERT INTO prepared_resulting_action_requests (prepared_intent_id,ordinal,action_request_id,subject,details,intended_owner_id,due_at,classification) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, item.id.as_str(), item.subject.as_str(), item.details.as_str(), item.intended_owner.as_str(), item.due_at.unix_millis(), item.classification.as_persisted()]).map_err(|_| storage_error(context))?;
    }
    for (ordinal, item) in incomplete_downstream.iter().enumerate() {
        let (target_type, target_id, version, classification) = match item {
            pmc_domain::work_management::IncompleteDownstreamWork::ActionRequest(id, v, c) => {
                ("action_request", id.as_str(), *v, *c)
            }
            pmc_domain::work_management::IncompleteDownstreamWork::Action(id, v, c) => {
                ("action", id.as_str(), *v, *c)
            }
        };
        tx.execute("INSERT INTO prepared_incomplete_downstream (prepared_intent_id,ordinal,target_type,target_id,target_version,classification) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, target_type, target_id, i64::try_from(version.get()).map_err(|_| storage_error(context))?, classification.as_persisted()]).map_err(|_| storage_error(context))?;
    }
    let effects = prepared.effects();
    for (ordinal, effect) in effects.iter().enumerate() {
        let (code, target_type, target_id, secondary_type, secondary_id) = match effect {
            pmc_domain::work_management::WorkManagementEffect::SupersedeDecision(id) => {
                ("decision.superseded", "decision", id.as_str(), None, None)
            }
            pmc_domain::work_management::WorkManagementEffect::CreateDecision(id) => (
                "decision.replacement_created",
                "decision",
                id.as_str(),
                None,
                None,
            ),
            pmc_domain::work_management::WorkManagementEffect::LinkReplacementDecision(
                left,
                right,
            ) => (
                "decision.replacement_linked",
                "decision",
                left.as_str(),
                Some("decision"),
                Some(right.as_str()),
            ),
            pmc_domain::work_management::WorkManagementEffect::CreateResultingActionRequest(id) => {
                (
                    "action_request.created_from_decision",
                    "action_request",
                    id.as_str(),
                    None,
                    None,
                )
            }
            pmc_domain::work_management::WorkManagementEffect::LinkDecisionToActionRequest(
                left,
                right,
            ) => (
                "decision.action_request_linked",
                "decision",
                left.as_str(),
                Some("action_request"),
                Some(right.as_str()),
            ),
            pmc_domain::work_management::WorkManagementEffect::FlagSupersededPremiseActionRequest(
                id,
            ) => (
                "action_request.superseded_premise_flagged",
                "action_request",
                id.as_str(),
                None,
                None,
            ),
            pmc_domain::work_management::WorkManagementEffect::FlagSupersededPremiseAction(id) => (
                "action.superseded_premise_flagged",
                "action",
                id.as_str(),
                None,
                None,
            ),
            _ => return Err(storage_error(context)),
        };
        tx.execute("INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, code, target_type, target_id, secondary_type, secondary_id]).map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn request_before_ordinal(
    replay: &[DecisionReplayCapsule],
    ordinal: u64,
) -> Vec<DecisionRequestRecord> {
    let mut requests = std::collections::BTreeMap::new();
    for capsule in replay
        .iter()
        .filter(|capsule| capsule.operation_ordinal() < ordinal)
    {
        match capsule.result() {
            DecisionPersistenceResult::Request(outcome) => {
                requests.insert(outcome.record.id().clone(), outcome.record.clone());
            }
            DecisionPersistenceResult::Resolved(outcome) => {
                requests.insert(outcome.request.id().clone(), outcome.request.clone());
            }
            DecisionPersistenceResult::Prepared(_)
            | DecisionPersistenceResult::Lowered(_)
            | DecisionPersistenceResult::Superseded(_)
            | DecisionPersistenceResult::Rejected(_) => {}
        }
    }
    requests.into_values().collect()
}

/// See `request_before_ordinal` -- same technique, for the Decision side of
/// the timeline. A decision can be mutated by `Resolved` (its creation),
/// `Lowered` (H2a Lower Data Classification), or `Superseded` (which
/// yields two records at once: the now-`Superseded` original and its
/// freshly created replacement), so all three feed the same map.
fn decision_before_ordinal(replay: &[DecisionReplayCapsule], ordinal: u64) -> Vec<DecisionRecord> {
    let mut decisions = std::collections::BTreeMap::new();
    for capsule in replay
        .iter()
        .filter(|capsule| capsule.operation_ordinal() < ordinal)
    {
        match capsule.result() {
            DecisionPersistenceResult::Resolved(outcome) => {
                decisions.insert(outcome.decision.id().clone(), outcome.decision.clone());
            }
            DecisionPersistenceResult::Lowered(outcome) => {
                decisions.insert(outcome.record.id().clone(), outcome.record.clone());
            }
            DecisionPersistenceResult::Superseded(outcome) => {
                decisions.insert(outcome.superseded.id().clone(), outcome.superseded.clone());
                decisions.insert(
                    outcome.replacement.id().clone(),
                    outcome.replacement.clone(),
                );
            }
            DecisionPersistenceResult::Request(_)
            | DecisionPersistenceResult::Prepared(_)
            | DecisionPersistenceResult::Rejected(_) => {}
        }
    }
    decisions.into_values().collect()
}

fn snapshot_before_ordinal(
    replay: &[DecisionReplayCapsule],
    audits: &[AuditEvent],
    ordinal: u64,
) -> Result<DecisionPersistenceSnapshot, DecisionPersistenceLoadError> {
    let mut requests = std::collections::BTreeMap::new();
    let mut decisions = std::collections::BTreeMap::new();
    let mut prepared = std::collections::BTreeMap::new();
    let mut prefix = replay
        .iter()
        .filter(|capsule| capsule.operation_ordinal() < ordinal)
        .cloned()
        .collect::<Vec<_>>();
    prefix.sort_by_key(DecisionReplayCapsule::operation_ordinal);

    for capsule in &prefix {
        match capsule.result() {
            DecisionPersistenceResult::Request(outcome) => {
                requests.insert(outcome.record.id().clone(), outcome.record.clone());
            }
            DecisionPersistenceResult::Prepared(intent) => {
                prepared.insert(intent.id().clone(), intent.clone());
            }
            DecisionPersistenceResult::Resolved(outcome) => {
                requests.insert(outcome.request.id().clone(), outcome.request.clone());
                decisions.insert(outcome.decision.id().clone(), outcome.decision.clone());
                let DecisionPersistenceCommand::ExecuteResolve { approval } = capsule.command()
                else {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                };
                prepared.remove(approval.prepared_id());
            }
            DecisionPersistenceResult::Lowered(outcome) => {
                decisions.insert(outcome.record.id().clone(), outcome.record.clone());
                let DecisionPersistenceCommand::ExecuteLowerClassification { approval } =
                    capsule.command()
                else {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                };
                prepared.remove(approval.prepared_id());
            }
            DecisionPersistenceResult::Superseded(outcome) => {
                decisions.insert(outcome.superseded.id().clone(), outcome.superseded.clone());
                decisions.insert(
                    outcome.replacement.id().clone(),
                    outcome.replacement.clone(),
                );
                let DecisionPersistenceCommand::ExecuteSupersede { approval } = capsule.command()
                else {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                };
                prepared.remove(approval.prepared_id());
            }
            DecisionPersistenceResult::Rejected(outcome) => {
                prepared.remove(outcome.prepared_intent_id());
            }
        }
    }

    let audit_by_id = audits
        .iter()
        .map(|audit| (audit.id().clone(), audit.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let prefix_audits = prefix
        .iter()
        .flat_map(DecisionReplayCapsule::audit_event_ids)
        .map(|id| {
            audit_by_id
                .get(id)
                .cloned()
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)
        })
        .collect::<Result<Vec<_>, _>>()?;

    DecisionPersistenceSnapshot::try_new(
        requests.into_values().collect(),
        decisions.into_values().collect(),
        prepared.into_values().collect(),
        prefix,
        prefix_audits,
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)
}

fn decode_namespace(
    tx: &Transaction<'_>,
) -> Result<DecisionPersistenceSnapshot, DecisionPersistenceLoadError> {
    let rows = tx.prepare("SELECT request.id,request.subject,request.details,request.intended_owner_id,registry.classification,registry.version,request.state,request.withdrawal_rationale,request.linked_decision_id FROM decision_requests request JOIN aggregate_registry registry ON registry.id=request.id AND registry.aggregate_type='decision_request' ORDER BY request.id").map_err(|_| DecisionPersistenceLoadError::StorageUnavailable)?.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,String>(4)?,row.get::<_,i64>(5)?,row.get::<_,String>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,Option<String>>(8)?))).map_err(|_| DecisionPersistenceLoadError::StorageUnavailable)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::StorageUnavailable)?;
    let mut requests = Vec::new();
    let mut decisions = Vec::new();
    let mut replay = Vec::new();
    let mut audits = Vec::new();
    for row in rows {
        if !matches!(
            (row.5, row.6.as_str(), row.7.is_some(), row.8.is_some()),
            (1, "draft", false, false)
                | (2, "open", false, false)
                | (3, "withdrawn", true, false)
                | (3, "resolved", false, true)
        ) {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let request = DecisionRequestRecord::from_persisted_created_draft(
            DecisionRequestId::parse(row.0.clone())
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            DecisionSubject::parse(row.1)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            DecisionText::parse(row.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            row.3
                .map(StakeholderId::parse)
                .transpose()
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            DataClassification::from_persisted(&row.4)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        );
        let expected_history_count = match row.6.as_str() {
            "draft" => 0,
            "open" => 1,
            "withdrawn" => 2,
            "resolved" => 1,
            _ => return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot),
        };
        let history_count = tx
            .query_row(
                "SELECT count(*) FROM decision_request_transitions WHERE request_id=?1",
                [request.id().as_str()],
                |record| record.get::<_, i64>(0),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if history_count != expected_history_count {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let command=tx.query_row("SELECT operation,correlation_id,operation_ordinal,result_reference FROM decision_replay_operations WHERE result_reference=?1 AND operation='create_request'",[request.id().as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let child=tx.query_row("SELECT idempotency_id,subject,details,intended_owner_id,classification FROM decision_command_create_requests WHERE request_id=?1",[request.id().as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if command.0 != "create_request"
            || command.3 != request.id().as_str()
            || child.1 != request.subject().as_str()
            || child.2 != request.details().as_str()
            || child.3.as_deref() != request.intended_owner().map(StakeholderId::as_str)
            || child.4 != request.classification().as_persisted()
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let audit_row=tx.query_row("SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN decision_replay_audits replay ON replay.audit_event_id=audit.id WHERE replay.idempotency_id=?1",[child.0.as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let correlation = CorrelationId::parse(command.1)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if audit_row.2 != correlation.as_str() {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let audit = create_audit(
            AuditEventId::parse(audit_row.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            UtcTimestamp::from_unix_millis(audit_row.1),
            &request,
            &DecisionOperationContext {
                idempotency_id: IdempotencyId::parse(child.0.clone())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                correlation_id: correlation.clone(),
            },
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        validate_persisted_audit(tx, &audit)?;
        let outcome = DecisionMutationOutcome {
            record: request.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        };
        replay.push(DecisionReplayCapsule::new(
            IdempotencyId::parse(child.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            correlation,
            u64::try_from(command.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            DecisionPersistenceCommand::CreateRequest {
                id: request.id().clone(),
                subject: request.subject().clone(),
                details: request.details().clone(),
                intended_owner: request.intended_owner().cloned(),
                classification: request.classification(),
            },
            DecisionPersistenceResult::Request(outcome),
            vec![audit.id().clone()],
        ));
        audits.push(audit);
        let request = if row.6 != "draft" {
            let transition = tx.query_row("SELECT idempotency_id,expected_version,target_state,rationale FROM decision_command_transition_requests WHERE request_id=?1 AND target_state='open'", [request.id().as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            if transition.1 != 1 || transition.2 != "open" || transition.3.is_some() {
                return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
            }
            let transition_id = IdempotencyId::parse(transition.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            let replay_row = tx.query_row("SELECT operation,correlation_id,operation_ordinal,result_reference FROM decision_replay_operations WHERE idempotency_id=?1", [transition_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, i64>(2)?,r.get::<_, String>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            if replay_row.0 != "transition_request" || replay_row.3 != request.id().as_str() {
                return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
            }
            let context = DecisionOperationContext {
                idempotency_id: transition_id.clone(),
                correlation_id: CorrelationId::parse(replay_row.1)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            };
            let open = DecisionRequestRecord::from_persisted_submitted_open(request.clone())
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            let audit_row = tx.query_row("SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN decision_replay_audits replay ON replay.audit_event_id=audit.id WHERE replay.idempotency_id=?1", [transition_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, i64>(1)?,r.get::<_, String>(2)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            if audit_row.2 != context.correlation_id.as_str() {
                return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
            }
            let transition_audit = create_transition_audit(
                AuditEventId::parse(audit_row.0)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                UtcTimestamp::from_unix_millis(audit_row.1),
                &open,
                &context,
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            validate_persisted_audit(tx, &transition_audit)?;
            let transition_history = tx.query_row("SELECT from_state,to_state,rationale,occurred_at FROM decision_request_transitions WHERE request_id=?1 AND ordinal=0", [request.id().as_str()], |record| Ok((record.get::<_, String>(0)?, record.get::<_, String>(1)?, record.get::<_, Option<String>>(2)?, record.get::<_, i64>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            if transition_history.0 != "draft"
                || transition_history.1 != "open"
                || transition_history.2.is_some()
                || transition_history.3 != transition_audit.occurred_at().unix_millis()
            {
                return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
            }
            replay.push(DecisionReplayCapsule::new(
                transition_id,
                context.correlation_id,
                u64::try_from(replay_row.2)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                DecisionPersistenceCommand::SubmitRequest {
                    request_id: request.id().clone(),
                    expected_version: pmc_domain::identity::AggregateVersion::initial(),
                },
                DecisionPersistenceResult::Request(DecisionMutationOutcome {
                    record: open.clone(),
                    audit_events: vec![transition_audit.clone()],
                    approval_receipt_id: None,
                }),
                vec![transition_audit.id().clone()],
            ));
            audits.push(transition_audit);
            if row.6 == "withdrawn" {
                let transition = tx.query_row("SELECT idempotency_id,expected_version,target_state,rationale FROM decision_command_transition_requests WHERE request_id=?1 AND target_state='withdrawn'", [request.id().as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                let Some(rationale) = transition.3 else {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                };
                if transition.1 != 2
                    || transition.2 != "withdrawn"
                    || row.7.as_deref() != Some(rationale.as_str())
                {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                }
                let withdrawal_id = IdempotencyId::parse(transition.0)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                let replay_row = tx.query_row("SELECT operation,correlation_id,operation_ordinal,result_reference FROM decision_replay_operations WHERE idempotency_id=?1", [withdrawal_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, i64>(2)?,r.get::<_, String>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                if replay_row.0 != "transition_request" || replay_row.3 != request.id().as_str() {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                }
                let context = DecisionOperationContext {
                    idempotency_id: withdrawal_id.clone(),
                    correlation_id: CorrelationId::parse(replay_row.1)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                };
                let rationale =
                    pmc_domain::decisions::DecisionWithdrawalRationale::parse(rationale)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                let withdrawn = DecisionRequestRecord::from_persisted_open_to_withdrawn(
                    open,
                    rationale.clone(),
                )
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                let audit_row = tx.query_row("SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN decision_replay_audits replay ON replay.audit_event_id=audit.id WHERE replay.idempotency_id=?1", [withdrawal_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, i64>(1)?,r.get::<_, String>(2)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                if audit_row.2 != context.correlation_id.as_str() {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                }
                let withdrawal_audit = create_withdraw_audit(
                    AuditEventId::parse(audit_row.0)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    UtcTimestamp::from_unix_millis(audit_row.1),
                    &withdrawn,
                    &context,
                )
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                validate_persisted_audit(tx, &withdrawal_audit)?;
                let withdrawal_history = tx.query_row("SELECT from_state,to_state,rationale,occurred_at FROM decision_request_transitions WHERE request_id=?1 AND ordinal=1", [request.id().as_str()], |record| Ok((record.get::<_, String>(0)?, record.get::<_, String>(1)?, record.get::<_, Option<String>>(2)?, record.get::<_, i64>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
                if withdrawal_history.0 != "open"
                    || withdrawal_history.1 != "withdrawn"
                    || withdrawal_history.2.as_deref() != Some(rationale.as_str())
                    || withdrawal_history.3 != withdrawal_audit.occurred_at().unix_millis()
                {
                    return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
                }
                replay.push(DecisionReplayCapsule::new(
                    withdrawal_id,
                    context.correlation_id,
                    u64::try_from(replay_row.2)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    DecisionPersistenceCommand::WithdrawRequest {
                        request_id: request.id().clone(),
                        expected_version: pmc_domain::identity::AggregateVersion::initial()
                            .next()
                            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                        rationale,
                    },
                    DecisionPersistenceResult::Request(DecisionMutationOutcome {
                        record: withdrawn.clone(),
                        audit_events: vec![withdrawal_audit.clone()],
                        approval_receipt_id: None,
                    }),
                    vec![withdrawal_audit.id().clone()],
                ));
                audits.push(withdrawal_audit);
                withdrawn
            } else {
                open
            }
        } else {
            request
        };
        requests.push(request);
    }
    // The persisted namespace is one chronological command stream, even though H1
    // and H2a use separate typed tables.  Build every capsule against the request
    // state visible at its own ordinal; never group by table or aggregate here.
    replay.sort_by_key(DecisionReplayCapsule::operation_ordinal);
    let mut prepared = Vec::new();
    let h2a = tx.prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.result_reference,command.request_id,command.expected_version,command.statement,command.rationale,command.impact FROM decision_h2a_replay_operations replay JOIN decision_h2a_command_prepare_resolves command USING(idempotency_id) WHERE replay.operation='prepare_resolve' AND replay.result_kind='prepared' ORDER BY replay.operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?,row.get::<_, String>(3)?,row.get::<_, String>(4)?,row.get::<_, i64>(5)?,row.get::<_, String>(6)?,row.get::<_, String>(7)?,row.get::<_, String>(8)?)))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in h2a {
        let request_id = DecisionRequestId::parse(row.4)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let request = request_before_ordinal(
            &replay,
            u64::try_from(row.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .into_iter()
        .find(|item| item.id() == &request_id)
        .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let expected_version = pmc_domain::identity::AggregateVersion::new(
            u64::try_from(row.5)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if request.state() != DecisionRequestState::Open || request.version() != expected_version {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let prepared_id = pmc_domain::identity::PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let intent = decode_resolve_prepared(tx, &prepared_id, &request)?;
        let WorkManagementOperation::ResolveDecisionRequest {
            statement,
            rationale,
            impact,
            resulting_action_requests,
            ..
        } = intent.operation()
        else {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        };
        if statement.as_str() != row.6 || rationale.as_str() != row.7 || impact.as_str() != row.8 {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let mut expected_actions = resulting_action_requests.clone();
        expected_actions.sort_by(|left, right| left.id.cmp(&right.id));
        let command = DecisionPersistenceCommand::PrepareResolve {
            request_id,
            expected_version,
            statement: statement.clone(),
            rationale: rationale.clone(),
            impact: impact.clone(),
            evidence_ids: intent
                .preview()
                .support()
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
                .evidence()
                .iter()
                .map(|evidence| evidence.id().clone())
                .collect(),
            judgments: intent
                .preview()
                .support()
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
                .judgments()
                .to_vec(),
            resulting_action_requests: expected_actions,
        };
        replay.push(DecisionReplayCapsule::new(
            IdempotencyId::parse(row.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            CorrelationId::parse(row.1)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            u64::try_from(row.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            command,
            DecisionPersistenceResult::Prepared(intent.clone()),
            Vec::new(),
        ));
        prepared.push(intent);
    }
    // v45 rejections of Resolve previews: decoded from their own rows (never
    // by re-running the service through the rehydration-only Allow*
    // adapters), each against the still-pending intent it consumed.
    let rejections = tx
        .prepare("SELECT idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,rejected_at,audit_event_id,actor FROM decision_reject_prepared_command_results ORDER BY operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?)))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in rejections {
        if row.6 != "head_of_products" || row.2 < 0 {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let prepared_id = PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let intent = prepared
            .iter()
            .find(|intent| intent.id() == &prepared_id)
            .cloned()
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let WorkManagementOperation::ResolveDecisionRequest { request_id, .. } = intent.operation()
        else {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        };
        let correlation = CorrelationId::parse(row.1)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let rejected_at = UtcTimestamp::from_unix_millis(row.4);
        let audit_id = AuditEventId::parse(row.5)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let audit = prepared_intent_rejection_audit(
            audit_id.clone(),
            rejected_at,
            DECISION_PREPARED_REJECTED_AUDIT_CODE,
            AuditTarget::DecisionRequest(request_id.clone()),
            correlation.clone(),
        )
        .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let agreeing_rows: i64 = tx
            .query_row(
                "SELECT count(*) FROM audit_events WHERE id=?1 AND occurred_at=?2 AND actor='head_of_products' AND module='work_management' AND event_code=?3 AND target_type='decision_request' AND target_id=?4 AND correlation_id=?5 AND policy_outcome='allowed' AND approval_outcome='rejected' AND execution_outcome='not_attempted' AND effect_scope='none' AND NOT EXISTS(SELECT 1 FROM audit_effects WHERE audit_event_id=?1)",
                rusqlite::params![
                    audit_id.as_str(),
                    rejected_at.unix_millis(),
                    DECISION_PREPARED_REJECTED_AUDIT_CODE,
                    request_id.as_str(),
                    correlation.as_str(),
                ],
                |row| row.get(0),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let consumed_at: Option<i64> = tx
            .query_row(
                "SELECT consumed_at FROM prepared_intents WHERE id=?1",
                [prepared_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if agreeing_rows != 1 || consumed_at != Some(row.4) {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let outcome = RejectedPreparedIntentOutcome::new(
            prepared_id.clone(),
            rejected_at,
            rejected_at >= intent.preview().expires_at(),
            audit.clone(),
        );
        replay.push(DecisionReplayCapsule::new(
            IdempotencyId::parse(row.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            correlation,
            u64::try_from(row.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            DecisionPersistenceCommand::RejectPrepared {
                prepared_id: prepared_id.clone(),
                actor: AuditActor::HeadOfProducts,
            },
            DecisionPersistenceResult::Rejected(outcome),
            vec![audit_id],
        ));
        audits.push(audit);
        prepared.retain(|intent| intent.id() != &prepared_id);
    }
    let executes = tx.prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.prepared_intent_id,replay.approval_receipt_id,command.actor,command.acknowledged_digest FROM decision_h2a_replay_operations replay JOIN decision_h2a_command_execute_resolves command USING(idempotency_id) WHERE replay.operation='execute_resolve' AND replay.result_kind='resolved' ORDER BY replay.operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?)))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in executes {
        if row.5 != "head_of_products" {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let prepared_id = PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let receipt_id = ApprovalReceiptId::parse(row.4)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let audit_rows = tx.prepare("SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN decision_h2a_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 ORDER BY link.ordinal")
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            .query_map([row.0.as_str()], |record| Ok((record.get::<_, String>(0)?, record.get::<_, i64>(1)?, record.get::<_, String>(2)?)))
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            .collect::<Result<Vec<_>, _>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if audit_rows.len() != 3 || audit_rows.iter().any(|audit| audit.2 != row.1) {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let at = UtcTimestamp::from_unix_millis(audit_rows[0].1);
        if audit_rows.iter().any(|audit| audit.1 != at.unix_millis()) {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let snapshot = snapshot_before_ordinal(
            &replay,
            &audits,
            u64::try_from(row.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )?;
        let ids = PersistedResolveIds {
            receipt_id: Some(receipt_id.clone()),
            audit_ids: [
                AuditEventId::parse(audit_rows[0].0.clone())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                AuditEventId::parse(audit_rows[1].0.clone())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                AuditEventId::parse(audit_rows[2].0.clone())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            ]
            .into_iter(),
        };
        let approval = WorkManagementApproval::new(
            prepared_id.clone(),
            AuditActor::HeadOfProducts,
            WorkManagementPayloadDigest::from_persisted(row.6)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            IdempotencyId::parse(row.0.clone())
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            Some(ApprovalConfirmation::Confirmed),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let context = DecisionOperationContext {
            idempotency_id: approval.idempotency_id().clone(),
            correlation_id: CorrelationId::parse(row.1)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        };
        let evidence_authority = PersistedDecisionEvidenceAuthority {
            evidence: prepared
                .iter()
                .find(|intent| intent.id() == &prepared_id)
                .and_then(|intent| intent.preview().support())
                .map(|support| support.evidence().to_vec())
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        };
        let mut service = InMemoryDecisionService::rehydrate(
            PersistedResolveClock(at),
            ids,
            AllowPersistedDecisionApproval,
            AllowPersistedDecisionPolicy,
            evidence_authority,
            snapshot,
        );
        let mut action_service = InMemoryActionService::new(
            PersistedResolveClock(at),
            PersistedResolveActionIds { next_audit: 0 },
            DenyWorkManagementApproval,
            AllowDecisionResultingActionPolicy,
            DenyActionEvidenceAuthority,
        );
        let outcome = service
            .approve_and_execute_resolve_decision_request(
                ApproveAndExecuteResolveDecisionRequest {
                    approval: approval.clone(),
                    context: context.clone(),
                },
                &mut action_service,
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str())
            .collect::<Vec<_>>()
            != audit_rows
                .iter()
                .map(|audit| audit.0.as_str())
                .collect::<Vec<_>>()
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        for audit in &outcome.audit_events {
            validate_persisted_h2a_audit(tx, audit)?;
        }
        let persisted = tx
            .query_row(
                "SELECT state,linked_decision_id FROM decision_requests WHERE id=?1",
                [outcome.request.id().as_str()],
                |record| {
                    Ok((
                        record.get::<_, String>(0)?,
                        record.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if persisted.0 != "resolved"
            || persisted.1.as_deref() != Some(outcome.decision.id().as_str())
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        // `state` is deliberately NOT pinned to 'effective' here (unlike the
        // rest of this row's fields, which never change again): Decision
        // Supersede (2026-09) can move this same decision to 'superseded'
        // sometime after this Resolve capsule was recorded. That later
        // transition is this row's own history to verify -- see the
        // Supersede EXECUTE decode loop's own `persisted.0 != "superseded"`
        // cross-check below -- not something this Resolve-capsule check
        // should re-litigate.
        let decision_count: i64 = tx.query_row("SELECT count(*) FROM decisions WHERE id=?1 AND source_request_id=?2 AND statement=?3 AND rationale=?4 AND impact=?5 AND owner_id=?6 AND decided_at=?7 AND state IN ('effective','superseded')", rusqlite::params![outcome.decision.id().as_str(), outcome.request.id().as_str(), outcome.decision.statement().as_str(), outcome.decision.rationale().as_str(), outcome.decision.impact().as_str(), outcome.decision.owner().as_str(), outcome.decision.decided_at().unix_millis()], |record| record.get(0)).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let action_count: i64 = tx
            .query_row(
                "SELECT count(*) FROM decision_resulting_action_requests WHERE decision_id=?1",
                [outcome.decision.id().as_str()],
                |record| record.get(0),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let receipt_count: i64 = tx.query_row("SELECT count(*) FROM approval_receipts WHERE id=?1 AND prepared_intent_id=?2 AND actor='head_of_products' AND acknowledged_payload_digest=?3 AND idempotency_id=?4", rusqlite::params![receipt_id.as_str(), prepared_id.as_str(), approval.acknowledged_payload_digest().as_str(), context.idempotency_id.as_str()], |record| record.get(0)).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if decision_count != 1
            || action_count
                != i64::try_from(outcome.resulting_action_request_ids.len())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            || receipt_count != 1
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        replay.push(DecisionReplayCapsule::new(
            context.idempotency_id,
            context.correlation_id,
            u64::try_from(row.2)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            DecisionPersistenceCommand::ExecuteResolve { approval },
            DecisionPersistenceResult::Resolved(outcome.clone()),
            outcome
                .audit_events
                .iter()
                .map(|audit| audit.id().clone())
                .collect(),
        ));
        audits.extend(outcome.audit_events);
        prepared.retain(|intent| intent.id() != &prepared_id);
        requests.retain(|request| request.id() != outcome.request.id());
        requests.push(outcome.request);
        decisions.push(outcome.decision);
    }

    // H2a "Lower Data Classification" for Decision -- its own
    // fresh-root V26/V27 replay authority (see the module-level rationale on
    // `decode_lower_decision_prepared`). Both loops run after the Resolve
    // decode above so `decisions`/`requests`/`prepared` already reflect every
    // H1 and Resolve capsule; `decision_before_ordinal` reconstructs a
    // decision's state as of a given ordinal from `replay` alone, mirroring
    // `request_before_ordinal`.
    let h2a_lower_prepares = tx.prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.result_reference,command.decision_id,command.expected_version,command.proposed_classification,command.rationale FROM decision_h2a_lower_classification_prepare_replay_operations replay JOIN decision_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.operation='prepare_lower_classification' AND replay.result_kind='prepared' ORDER BY replay.operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?,
            row.get::<_, String>(4)?, row.get::<_, i64>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?,
        )))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in h2a_lower_prepares {
        let decision_id = DecisionId::parse(row.4)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let ordinal = u64::try_from(row.2)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let decision = decision_before_ordinal(&replay, ordinal)
            .into_iter()
            .find(|item| item.id() == &decision_id)
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let expected_version = pmc_domain::identity::AggregateVersion::new(
            u64::try_from(row.5)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if decision.version() != expected_version {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let proposed_classification = DataClassification::from_persisted(&row.6)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let rationale = pmc_domain::work_management::WorkManagementRationale::parse(row.7)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let prepared_id = pmc_domain::identity::PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let intent = decode_lower_decision_prepared(
            tx,
            &prepared_id,
            &decision,
            proposed_classification,
            &rationale,
        )?;
        let command = DecisionPersistenceCommand::PrepareLowerClassification {
            decision_id,
            expected_version,
            proposed_classification,
            rationale,
        };
        replay.push(DecisionReplayCapsule::new(
            IdempotencyId::parse(row.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            CorrelationId::parse(row.1)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            ordinal,
            command,
            DecisionPersistenceResult::Prepared(intent.clone()),
            Vec::new(),
        ));
        prepared.push(intent);
    }

    let lower_executes = tx.prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.prepared_intent_id,replay.approval_receipt_id,command.actor,command.acknowledged_digest FROM decision_h2a_lower_classification_execute_replay_operations replay JOIN decision_h2a_lower_classification_command_executes command USING(idempotency_id) WHERE replay.operation='execute_lower_classification' AND replay.result_kind='lowered' ORDER BY replay.operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?,
            row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?,
        )))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in lower_executes {
        if row.5 != "head_of_products" {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let prepared_id = pmc_domain::identity::PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let receipt_id = ApprovalReceiptId::parse(row.4)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let audit_row = tx.query_row("SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN decision_h2a_lower_classification_execute_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0", [row.0.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if audit_row.2 != row.1 {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let at = UtcTimestamp::from_unix_millis(audit_row.1);
        let ordinal = u64::try_from(row.2)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let snapshot = snapshot_before_ordinal(&replay, &audits, ordinal)?;
        let audit_event_id = AuditEventId::parse(audit_row.0.clone())
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let ids = PersistedLowerClassificationIds {
            receipt_id: Some(receipt_id.clone()),
            audit_id: Some(audit_event_id),
        };
        let approval = WorkManagementApproval::new(
            prepared_id.clone(),
            AuditActor::HeadOfProducts,
            WorkManagementPayloadDigest::from_persisted(row.6)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            IdempotencyId::parse(row.0.clone())
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            Some(ApprovalConfirmation::Confirmed),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let context = DecisionOperationContext {
            idempotency_id: approval.idempotency_id().clone(),
            correlation_id: CorrelationId::parse(row.1)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        };
        let mut service = InMemoryDecisionService::rehydrate(
            PersistedResolveClock(at),
            ids,
            AllowPersistedDecisionApproval,
            AllowPersistedDecisionPolicy,
            PersistedDecisionEvidenceAuthority {
                evidence: Vec::new(),
            },
            snapshot,
        );
        let outcome = service
            .approve_and_execute_lower_decision_classification(
                ApproveAndExecuteLowerDecisionClassification {
                    approval: approval.clone(),
                    context: context.clone(),
                },
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if outcome.audit_events.len() != 1 || outcome.audit_events[0].id().as_str() != audit_row.0 {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        validate_persisted_h2a_audit(tx, &outcome.audit_events[0])?;
        let registry_count: i64 = tx
            .query_row(
                "SELECT count(*) FROM aggregate_registry WHERE id=?1 AND aggregate_type='decision' AND version=?2 AND classification=?3",
                rusqlite::params![
                    outcome.record.id().as_str(),
                    i64::try_from(outcome.record.version().get())
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    outcome.record.classification().as_persisted()
                ],
                |r| r.get(0),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let receipt_count: i64 = tx.query_row("SELECT count(*) FROM approval_receipts WHERE id=?1 AND prepared_intent_id=?2 AND actor='head_of_products' AND acknowledged_payload_digest=?3 AND idempotency_id=?4", rusqlite::params![receipt_id.as_str(), prepared_id.as_str(), approval.acknowledged_payload_digest().as_str(), context.idempotency_id.as_str()], |r| r.get(0)).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if registry_count != 1 || receipt_count != 1 {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        replay.push(DecisionReplayCapsule::new(
            context.idempotency_id,
            context.correlation_id,
            ordinal,
            DecisionPersistenceCommand::ExecuteLowerClassification { approval },
            DecisionPersistenceResult::Lowered(outcome.clone()),
            vec![outcome.audit_events[0].id().clone()],
        ));
        audits.extend(outcome.audit_events);
        prepared.retain(|intent| intent.id() != &prepared_id);
        decisions.retain(|decision| decision.id() != outcome.record.id());
        decisions.push(outcome.record);
    }

    // Decision Supersede PREPARE: mirrors the Resolve `h2a` loop above --
    // `decode_supersede_prepared` rebuilds and digest-verifies the exact
    // stored preview, this loop just cross-checks it against its own
    // scalar command columns and pushes the resulting capsule/prepared
    // intent.
    let supersede_prepares = tx.prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.result_reference,command.decision_id,command.expected_version,command.replacement_statement,command.replacement_rationale,command.replacement_impact,command.replacement_owner_id FROM decision_h2a_replay_operations replay JOIN decision_h2a_command_prepare_supersedes command USING(idempotency_id) WHERE replay.operation='prepare_supersede' AND replay.result_kind='prepared' ORDER BY replay.operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, i64>(2)?,row.get::<_, String>(3)?,row.get::<_, String>(4)?,row.get::<_, i64>(5)?,row.get::<_, String>(6)?,row.get::<_, String>(7)?,row.get::<_, String>(8)?,row.get::<_, String>(9)?)))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in supersede_prepares {
        let decision_id = DecisionId::parse(row.4)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let ordinal = u64::try_from(row.2)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let decision = decision_before_ordinal(&replay, ordinal)
            .into_iter()
            .find(|item| item.id() == &decision_id)
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let expected_version = pmc_domain::identity::AggregateVersion::new(
            u64::try_from(row.5)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if decision.version() != expected_version {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let prepared_id = pmc_domain::identity::PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let intent = decode_supersede_prepared(tx, &prepared_id, &decision)?;
        let WorkManagementOperation::SupersedeDecision {
            replacement_statement,
            replacement_rationale,
            replacement_impact,
            replacement_owner,
            resulting_action_requests,
            ..
        } = intent.operation()
        else {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        };
        if replacement_statement.as_str() != row.6
            || replacement_rationale.as_str() != row.7
            || replacement_impact.as_str() != row.8
            || replacement_owner.as_str() != row.9
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let mut expected_actions = resulting_action_requests.clone();
        expected_actions.sort_by(|left, right| left.id.cmp(&right.id));
        let support = intent
            .preview()
            .support()
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let command = DecisionPersistenceCommand::PrepareSupersede {
            decision_id,
            expected_version,
            replacement_statement: replacement_statement.clone(),
            replacement_rationale: replacement_rationale.clone(),
            replacement_impact: replacement_impact.clone(),
            replacement_owner: replacement_owner.clone(),
            evidence_ids: support
                .evidence()
                .iter()
                .map(|evidence| evidence.id().clone())
                .collect(),
            judgments: support.judgments().to_vec(),
            resulting_action_requests: expected_actions,
        };
        replay.push(DecisionReplayCapsule::new(
            IdempotencyId::parse(row.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            CorrelationId::parse(row.1)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            ordinal,
            command,
            DecisionPersistenceResult::Prepared(intent.clone()),
            Vec::new(),
        ));
        prepared.push(intent);
    }

    // Decision Supersede EXECUTE: deliberately does NOT re-run domain
    // `execute_supersede` the way Resolve/Lower's own execute decode does
    // above -- that domain call recomputes `current_downstream` from the
    // REAL, live Action state, which after a successful Supersede already
    // carries the post-mutation (marked/versioned) records, not the ones
    // the historical PREPARE captured. Comparing those would spuriously
    // reject perfectly valid history. Instead this reconstructs
    // `SupersededDecisionOutcome` directly from already-durable rows (the
    // exact same "rebuild, then let the frozen validator prove it" contract
    // `decode_supersede_prepared` uses for the PREPARE side) and lets
    // `DecisionPersistenceSnapshot::try_new`'s `validate_decision_capsule`
    // catch anything inconsistent.
    let supersede_executes = tx.prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.prepared_intent_id,replay.approval_receipt_id,command.actor,command.acknowledged_digest FROM decision_h2a_replay_operations replay JOIN decision_h2a_command_execute_supersedes command USING(idempotency_id) WHERE replay.operation='execute_supersede' AND replay.result_kind='superseded' ORDER BY replay.operation_ordinal")
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?)))
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .collect::<Result<Vec<_>, _>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for row in supersede_executes {
        if row.5 != "head_of_products" {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let prepared_id = PreparedIntentId::parse(row.3)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let receipt_id = ApprovalReceiptId::parse(row.4)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let intent = prepared
            .iter()
            .find(|item| item.id() == &prepared_id)
            .cloned()
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let WorkManagementOperation::SupersedeDecision {
            decision_id,
            decision_version,
            replacement_decision_id,
            replacement_decision_version,
            replacement_statement,
            replacement_rationale,
            replacement_impact,
            replacement_owner,
            replacement_decided_at,
            resulting_action_requests,
            incomplete_downstream,
            ..
        } = intent.operation().clone()
        else {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        };
        let ordinal = u64::try_from(row.2)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let previous = decision_before_ordinal(&replay, ordinal)
            .into_iter()
            .find(|item| item.id() == &decision_id)
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if previous.version() != decision_version {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let superseded =
            DecisionRecord::from_persisted_superseded(previous, replacement_decision_id.clone())
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let support = intent
            .preview()
            .support()
            .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            .clone();
        let replacement = DecisionRecord::from_persisted_replacement(
            replacement_decision_id.clone(),
            replacement_statement,
            replacement_rationale,
            replacement_impact,
            replacement_owner,
            replacement_decided_at,
            intent.classification(),
            support,
            resulting_action_requests
                .iter()
                .map(|item| item.id.clone())
                .collect(),
            decision_id.clone(),
        );
        if replacement.version() != replacement_decision_version {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let audit_rows = tx.prepare("SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN decision_h2a_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 ORDER BY link.ordinal")
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            .query_map([row.0.as_str()], |record| Ok((record.get::<_, String>(0)?, record.get::<_, i64>(1)?, record.get::<_, String>(2)?)))
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            .collect::<Result<Vec<_>, _>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if audit_rows.len() != 3 || audit_rows.iter().any(|audit| audit.2 != row.1) {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let at = UtcTimestamp::from_unix_millis(audit_rows[0].1);
        if audit_rows.iter().any(|audit| audit.1 != at.unix_millis()) {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let correlation_id = CorrelationId::parse(row.1.clone())
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let expected_audits = [
            (
                "decision.superseded",
                AuditTarget::Decision(decision_id.clone()),
            ),
            (
                "decision.replacement_created",
                AuditTarget::Decision(replacement_decision_id.clone()),
            ),
            (
                "decision.replacement_linked",
                AuditTarget::Decision(decision_id.clone()),
            ),
        ];
        let mut audit_events = Vec::with_capacity(3);
        for ((audit_id, occurred_at, _), (code, target)) in audit_rows.iter().zip(expected_audits) {
            let audit = decode_h2a_audit(
                AuditEventId::parse(audit_id.clone())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                UtcTimestamp::from_unix_millis(*occurred_at),
                code,
                target,
                correlation_id.clone(),
            )?;
            validate_persisted_h2a_audit(tx, &audit)?;
            audit_events.push(audit);
        }
        let approval = WorkManagementApproval::new(
            prepared_id.clone(),
            AuditActor::HeadOfProducts,
            WorkManagementPayloadDigest::from_persisted(row.6)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            IdempotencyId::parse(row.0.clone())
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            Some(ApprovalConfirmation::Confirmed),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if approval.acknowledged_payload_digest() != intent.payload_digest() {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let outcome = SupersededDecisionOutcome {
            superseded: superseded.clone(),
            replacement: replacement.clone(),
            resulting_action_request_ids: replacement.resulting_action_request_ids().to_vec(),
            flagged_downstream: incomplete_downstream,
            audit_events,
            approval_receipt_id: receipt_id.clone(),
        };
        let persisted = tx
            .query_row(
                "SELECT state,superseded_by_decision_id FROM decisions WHERE id=?1",
                [decision_id.as_str()],
                |record| {
                    Ok((
                        record.get::<_, String>(0)?,
                        record.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if persisted.0 != "superseded"
            || persisted.1.as_deref() != Some(replacement_decision_id.as_str())
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        let decision_count: i64 = tx.query_row("SELECT count(*) FROM decisions WHERE id=?1 AND source_request_id IS NULL AND statement=?2 AND rationale=?3 AND impact=?4 AND owner_id=?5 AND decided_at=?6 AND state='effective' AND supersedes_decision_id=?7", rusqlite::params![replacement.id().as_str(), replacement.statement().as_str(), replacement.rationale().as_str(), replacement.impact().as_str(), replacement.owner().as_str(), replacement.decided_at().unix_millis(), decision_id.as_str()], |record| record.get(0)).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let action_count: i64 = tx
            .query_row(
                "SELECT count(*) FROM decision_resulting_action_requests WHERE decision_id=?1",
                [replacement.id().as_str()],
                |record| record.get(0),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        let receipt_count: i64 = tx.query_row("SELECT count(*) FROM approval_receipts WHERE id=?1 AND prepared_intent_id=?2 AND actor='head_of_products' AND acknowledged_payload_digest=?3 AND idempotency_id=?4", rusqlite::params![receipt_id.as_str(), prepared_id.as_str(), approval.acknowledged_payload_digest().as_str(), row.0.as_str()], |record| record.get(0)).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if decision_count != 1
            || action_count
                != i64::try_from(outcome.resulting_action_request_ids.len())
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
            || receipt_count != 1
        {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
        replay.push(DecisionReplayCapsule::new(
            IdempotencyId::parse(row.0)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            correlation_id,
            ordinal,
            DecisionPersistenceCommand::ExecuteSupersede { approval },
            DecisionPersistenceResult::Superseded(outcome.clone()),
            outcome
                .audit_events
                .iter()
                .map(|audit| audit.id().clone())
                .collect(),
        ));
        audits.extend(outcome.audit_events);
        prepared.retain(|intent| intent.id() != &prepared_id);
        decisions.retain(|item| item.id() != &decision_id && item.id() != replacement.id());
        decisions.push(outcome.superseded);
        decisions.push(outcome.replacement);
    }

    replay.sort_by_key(DecisionReplayCapsule::operation_ordinal);
    let audit_by_id = audits
        .into_iter()
        .map(|audit| (audit.id().clone(), audit))
        .collect::<std::collections::HashMap<_, _>>();
    let audits = replay
        .iter()
        .flat_map(DecisionReplayCapsule::audit_event_ids)
        .map(|audit_id| {
            audit_by_id
                .get(audit_id)
                .cloned()
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected_h1_replay_count = i64::try_from(
        replay
            .iter()
            .filter(|capsule| matches!(capsule.result(), DecisionPersistenceResult::Request(_)))
            .count(),
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let expected_transition_count = i64::try_from(
        replay
            .iter()
            .filter(|capsule| {
                matches!(
                    capsule.command(),
                    DecisionPersistenceCommand::SubmitRequest { .. }
                        | DecisionPersistenceCommand::WithdrawRequest { .. }
                )
            })
            .count(),
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    for (sql, expected) in [
        (
            "SELECT count(*) FROM decision_replay_operations",
            expected_h1_replay_count,
        ),
        (
            "SELECT count(*) FROM decision_command_create_requests",
            i64::try_from(requests.len())
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        (
            "SELECT count(*) FROM decision_command_transition_requests",
            expected_transition_count,
        ),
        (
            "SELECT count(*) FROM decision_replay_audits",
            expected_h1_replay_count,
        ),
        (
            "SELECT count(*) FROM decision_h2a_replay_audits",
            i64::try_from(
                replay
                    .iter()
                    .filter(|capsule| {
                        matches!(
                            capsule.result(),
                            DecisionPersistenceResult::Resolved(_)
                                | DecisionPersistenceResult::Superseded(_)
                        )
                    })
                    .map(|capsule| capsule.audit_event_ids().len())
                    .sum::<usize>(),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        (
            "SELECT count(*) FROM decision_h2a_command_prepare_supersedes",
            i64::try_from(
                replay
                    .iter()
                    .filter(|capsule| {
                        matches!(
                            capsule.command(),
                            DecisionPersistenceCommand::PrepareSupersede { .. }
                        )
                    })
                    .count(),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        (
            "SELECT count(*) FROM decision_h2a_command_execute_supersedes",
            i64::try_from(
                replay
                    .iter()
                    .filter(|capsule| {
                        matches!(capsule.result(), DecisionPersistenceResult::Superseded(_))
                    })
                    .count(),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        (
            "SELECT count(*) FROM decision_h2a_lower_classification_command_prepares",
            i64::try_from(
                replay
                    .iter()
                    .filter(|capsule| {
                        matches!(
                            capsule.command(),
                            DecisionPersistenceCommand::PrepareLowerClassification { .. }
                        )
                    })
                    .count(),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        (
            "SELECT count(*) FROM decision_reject_prepared_command_results",
            i64::try_from(
                replay
                    .iter()
                    .filter(|capsule| {
                        matches!(capsule.result(), DecisionPersistenceResult::Rejected(_))
                    })
                    .count(),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        (
            "SELECT count(*) FROM decision_h2a_lower_classification_execute_replay_audits",
            i64::try_from(
                replay
                    .iter()
                    .filter(|capsule| {
                        matches!(capsule.result(), DecisionPersistenceResult::Lowered(_))
                    })
                    .map(|capsule| capsule.audit_event_ids().len())
                    .sum::<usize>(),
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
    ] {
        let actual = tx
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
        if actual != expected {
            return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
        }
    }
    DecisionPersistenceDecodeInput::new(requests, decisions, prepared, replay, audits)
        .decode()
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)
}

fn create_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    request: &DecisionRequestRecord,
    context: &DecisionOperationContext,
) -> Result<AuditEvent, DomainError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("decision_request.created")
                .map_err(|_| storage_error(context))?,
            AuditTarget::DecisionRequest(request.id().clone()),
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse("decision_request.created")
                .map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn decode_resolve_prepared(
    tx: &Transaction<'_>,
    prepared_id: &pmc_domain::identity::PreparedIntentId,
    request: &DecisionRequestRecord,
) -> Result<WorkManagementPreparedIntent, DecisionPersistenceLoadError> {
    let intent = tx.query_row("SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND contract_version=1 AND intent_kind='resolve_decision_request' AND policy='allowed' AND cancellation_policy='not_cancellable_after_submit' AND authority='head_of_products'", [prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if intent.2 < intent.3 || intent.4.is_none() {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    let payload = tx.query_row("SELECT primary_id,primary_version,created_id,created_classification,statement,rationale,impact,decision_owner_id,decided_at FROM prepared_work_management_payloads WHERE prepared_intent_id=?1", [prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, i64>(1)?,r.get::<_, Option<String>>(2)?,r.get::<_, Option<String>>(3)?,r.get::<_, Option<String>>(4)?,r.get::<_, Option<String>>(5)?,r.get::<_, Option<String>>(6)?,r.get::<_, Option<String>>(7)?,r.get::<_, Option<i64>>(8)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let version = pmc_domain::identity::AggregateVersion::new(
        u64::try_from(payload.1)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if payload.0 != request.id().as_str() || version != request.version() {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    let actions = tx.prepare("SELECT action_request_id,subject,details,intended_owner_id,due_at,classification FROM prepared_resulting_action_requests WHERE prepared_intent_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, String>(3)?,r.get::<_, i64>(4)?,r.get::<_, String>(5)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let resulting = actions
        .into_iter()
        .map(|row| {
            Ok(
                pmc_domain::work_management::DecisionResultingActionRequest {
                    id: pmc_domain::identity::ActionRequestId::parse(row.0)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    subject: pmc_domain::actions::ActionTitle::parse(row.1)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    details: pmc_domain::actions::ActionDetails::parse(row.2)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    intended_owner: StakeholderId::parse(row.3)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    due_at: UtcTimestamp::from_unix_millis(row.4),
                    classification: DataClassification::from_persisted(&row.5)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                },
            )
        })
        .collect::<Result<Vec<_>, DecisionPersistenceLoadError>>()?;
    let support_id = intent.4.unwrap_or_default();
    let judgments = tx.prepare("SELECT rationale,classification FROM support_judgments WHERE support_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([support_id], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let judgments = judgments.into_iter().map(|row| pmc_domain::work_management::HumanJudgment::new(pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale, row.0, DataClassification::from_persisted(&row.1).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)).collect::<Result<Vec<_>,_>>()?;
    let evidence = tx.prepare("SELECT evidence_id,classification,role,verification,last_verified_at,integrity_digest,evidence_version FROM decision_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, String>(3)?,r.get::<_, Option<i64>>(4)?,r.get::<_, Option<String>>(5)?,r.get::<_, i64>(6)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let evidence = evidence
        .into_iter()
        .map(|row| {
            let verification = match (row.3.as_str(), row.4, row.5) {
                ("verified", Some(at), Some(digest)) => EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(at),
                    integrity_digest: IntegrityDigest::parse(digest)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                },
                ("observed_unpinned", Some(at), Some(digest)) => {
                    EvidenceVerification::ObservedUnpinned {
                        observed_at: UtcTimestamp::from_unix_millis(at),
                        integrity_digest: IntegrityDigest::parse(digest)
                            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    }
                }
                ("degraded_last_verified", Some(at), Some(digest)) => {
                    EvidenceVerification::DegradedLastVerified {
                        last_verified_at: UtcTimestamp::from_unix_millis(at),
                        integrity_digest: IntegrityDigest::parse(digest)
                            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    }
                }
                ("unverified", None, None) => EvidenceVerification::Unverified,
                ("integrity_mismatch", None, None) => EvidenceVerification::IntegrityMismatch,
                _ => return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot),
            };
            if row.2 != "decision_resolution" {
                return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
            }
            let source_version = pmc_domain::identity::AggregateVersion::new(
                u64::try_from(row.6)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            Ok(EvidenceReferenceMetadata::new(
                EvidenceReferenceId::parse(row.0)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                source_version,
                DataClassification::from_persisted(&row.1)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                EvidenceRole::DecisionResolution,
                verification,
            ))
        })
        .collect::<Result<Vec<_>, DecisionPersistenceLoadError>>()?;
    let support = EvidenceOrJudgment::new(evidence, judgments)
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .evaluate_evidence_or_judgment()
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let operation = WorkManagementOperation::ResolveDecisionRequest {
        request_id: request.id().clone(),
        request_version: version,
        decision_id: pmc_domain::identity::DecisionId::parse(
            payload
                .2
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        decision_classification: DataClassification::from_persisted(
            &payload
                .3
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        statement: DecisionText::parse(
            payload
                .4
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        rationale: DecisionText::parse(
            payload
                .5
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        impact: DecisionText::parse(
            payload
                .6
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        decision_owner: StakeholderId::parse(
            payload
                .7
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        decided_at: UtcTimestamp::from_unix_millis(
            payload
                .8
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        resulting_action_requests: resulting,
    };
    let rebuilt = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        request.classification(),
        Some(support),
        UtcTimestamp::from_unix_millis(intent.3),
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if rebuilt.payload_digest().as_str() != intent.0
        || rebuilt.classification().as_persisted() != intent.1
        || rebuilt.preview().expires_at().unix_millis() != intent.2
    {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    Ok(rebuilt)
}

/// See `decode_resolve_prepared` -- same "rebuild and verify the exact
/// stored digest" technique, but for `SupersedeDecision`: reads the
/// dedicated `replacement_*` payload columns (not `created_*`/`statement`),
/// the pre-existing V1 `prepared_incomplete_downstream` table for
/// `incomplete_downstream`, and targets the OLD decision (not a request).
fn decode_supersede_prepared(
    tx: &Transaction<'_>,
    prepared_id: &pmc_domain::identity::PreparedIntentId,
    decision: &DecisionRecord,
) -> Result<WorkManagementPreparedIntent, DecisionPersistenceLoadError> {
    let intent = tx.query_row("SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND contract_version=1 AND intent_kind='supersede_decision' AND policy='allowed' AND cancellation_policy='not_cancellable_after_submit' AND authority='head_of_products'", [prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if intent.2 < intent.3 || intent.4.is_none() {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    let payload = tx.query_row("SELECT primary_id,primary_version,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at FROM prepared_work_management_payloads WHERE prepared_intent_id=?1", [prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, Option<String>>(7)?, r.get::<_, Option<String>>(8)?, r.get::<_, Option<i64>>(9)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let version = pmc_domain::identity::AggregateVersion::new(
        u64::try_from(payload.1)
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if payload.0 != decision.id().as_str() || version != decision.version() {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    let actions = tx.prepare("SELECT action_request_id,subject,details,intended_owner_id,due_at,classification FROM prepared_resulting_action_requests WHERE prepared_intent_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, String>(3)?,r.get::<_, i64>(4)?,r.get::<_, String>(5)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let resulting = actions
        .into_iter()
        .map(|row| {
            Ok(
                pmc_domain::work_management::DecisionResultingActionRequest {
                    id: pmc_domain::identity::ActionRequestId::parse(row.0)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    subject: pmc_domain::actions::ActionTitle::parse(row.1)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    details: pmc_domain::actions::ActionDetails::parse(row.2)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    intended_owner: StakeholderId::parse(row.3)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    due_at: UtcTimestamp::from_unix_millis(row.4),
                    classification: DataClassification::from_persisted(&row.5)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                },
            )
        })
        .collect::<Result<Vec<_>, DecisionPersistenceLoadError>>()?;
    let downstream_rows = tx.prepare("SELECT target_type,target_id,target_version,classification FROM prepared_incomplete_downstream WHERE prepared_intent_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let incomplete_downstream = downstream_rows
        .into_iter()
        .map(|row| {
            let target_version = pmc_domain::identity::AggregateVersion::new(
                u64::try_from(row.2)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            let classification = DataClassification::from_persisted(&row.3)
                .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            match row.0.as_str() {
                "action_request" => Ok(
                    pmc_domain::work_management::IncompleteDownstreamWork::ActionRequest(
                        pmc_domain::identity::ActionRequestId::parse(row.1)
                            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                        target_version,
                        classification,
                    ),
                ),
                "action" => Ok(
                    pmc_domain::work_management::IncompleteDownstreamWork::Action(
                        pmc_domain::identity::ActionId::parse(row.1)
                            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                        target_version,
                        classification,
                    ),
                ),
                _ => Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot),
            }
        })
        .collect::<Result<Vec<_>, DecisionPersistenceLoadError>>()?;
    let support_id = intent.4.unwrap_or_default();
    let judgments = tx.prepare("SELECT rationale,classification FROM support_judgments WHERE support_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([support_id], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let judgments = judgments.into_iter().map(|row| pmc_domain::work_management::HumanJudgment::new(pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale, row.0, DataClassification::from_persisted(&row.1).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)).collect::<Result<Vec<_>,_>>()?;
    let evidence = tx.prepare("SELECT evidence_id,classification,role,verification,last_verified_at,integrity_digest,evidence_version FROM decision_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal").map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.query_map([prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, String>(3)?,r.get::<_, Option<i64>>(4)?,r.get::<_, Option<String>>(5)?,r.get::<_, i64>(6)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?.collect::<Result<Vec<_>,_>>().map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let evidence = evidence
        .into_iter()
        .map(|row| {
            let verification = match (row.3.as_str(), row.4, row.5) {
                ("verified", Some(at), Some(digest)) => EvidenceVerification::Verified {
                    verified_at: UtcTimestamp::from_unix_millis(at),
                    integrity_digest: IntegrityDigest::parse(digest)
                        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                },
                ("observed_unpinned", Some(at), Some(digest)) => {
                    EvidenceVerification::ObservedUnpinned {
                        observed_at: UtcTimestamp::from_unix_millis(at),
                        integrity_digest: IntegrityDigest::parse(digest)
                            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    }
                }
                ("degraded_last_verified", Some(at), Some(digest)) => {
                    EvidenceVerification::DegradedLastVerified {
                        last_verified_at: UtcTimestamp::from_unix_millis(at),
                        integrity_digest: IntegrityDigest::parse(digest)
                            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                    }
                }
                ("unverified", None, None) => EvidenceVerification::Unverified,
                ("integrity_mismatch", None, None) => EvidenceVerification::IntegrityMismatch,
                _ => return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot),
            };
            if row.2 != "decision_resolution" {
                return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
            }
            let source_version = pmc_domain::identity::AggregateVersion::new(
                u64::try_from(row.6)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
            Ok(EvidenceReferenceMetadata::new(
                EvidenceReferenceId::parse(row.0)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                source_version,
                DataClassification::from_persisted(&row.1)
                    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
                EvidenceRole::DecisionResolution,
                verification,
            ))
        })
        .collect::<Result<Vec<_>, DecisionPersistenceLoadError>>()?;
    let support = EvidenceOrJudgment::new(evidence, judgments)
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?
        .evaluate_evidence_or_judgment()
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let operation = WorkManagementOperation::SupersedeDecision {
        decision_id: decision.id().clone(),
        decision_version: version,
        replacement_decision_id: pmc_domain::identity::DecisionId::parse(
            payload
                .2
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_decision_version: pmc_domain::identity::AggregateVersion::new(
            u64::try_from(
                payload
                    .3
                    .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
            )
            .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_decision_classification: DataClassification::from_persisted(
            &payload
                .4
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_statement: DecisionText::parse(
            payload
                .5
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_rationale: DecisionText::parse(
            payload
                .6
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_impact: DecisionText::parse(
            payload
                .7
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_owner: StakeholderId::parse(
            payload
                .8
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        replacement_decided_at: UtcTimestamp::from_unix_millis(
            payload
                .9
                .ok_or(DecisionPersistenceLoadError::InvalidDecisionSnapshot)?,
        ),
        resulting_action_requests: resulting,
        incomplete_downstream,
    };
    let rebuilt = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        decision.classification(),
        Some(support),
        UtcTimestamp::from_unix_millis(intent.3),
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if rebuilt.payload_digest().as_str() != intent.0
        || rebuilt.classification().as_persisted() != intent.1
        || rebuilt.preview().expires_at().unix_millis() != intent.2
    {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    Ok(rebuilt)
}

/// See `decode_resolve_prepared` -- same "rebuild and verify the exact
/// stored digest" technique, but for `LowerDecisionClassification`'s much
/// simpler shape (no support, no separate `prepared_work_management_payloads`
/// row: `decision_h2a_lower_classification_command_prepares` already carries
/// every scalar this operation needs).
fn decode_lower_decision_prepared(
    tx: &Transaction<'_>,
    prepared_id: &pmc_domain::identity::PreparedIntentId,
    decision: &DecisionRecord,
    proposed_classification: DataClassification,
    rationale: &pmc_domain::work_management::WorkManagementRationale,
) -> Result<WorkManagementPreparedIntent, DecisionPersistenceLoadError> {
    let intent = tx.query_row("SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND contract_version=1 AND intent_kind='lower_decision_classification' AND policy='allowed' AND cancellation_policy='not_cancellable_after_submit' AND authority='head_of_products'", [prepared_id.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, Option<String>>(4)?))).map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if intent.2 < intent.3 || intent.4.is_some() {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    let operation = WorkManagementOperation::LowerDecisionClassification {
        decision_id: decision.id().clone(),
        decision_version: decision.version(),
        current_classification: decision.classification(),
        proposed_classification,
        rationale: rationale.clone(),
    };
    let rebuilt = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        decision.classification(),
        None,
        UtcTimestamp::from_unix_millis(intent.3),
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if rebuilt.payload_digest().as_str() != intent.0
        || rebuilt.classification().as_persisted() != intent.1
        || rebuilt.preview().expires_at().unix_millis() != intent.2
    {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    Ok(rebuilt)
}

#[derive(Clone)]
struct PersistedDecisionEvidenceAuthority {
    evidence: Vec<EvidenceReferenceMetadata>,
}

impl DecisionEvidenceAuthorityPort for PersistedDecisionEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, pmc_domain::decisions::DecisionEvidenceAuthorityError>
    {
        self.evidence
            .iter()
            .find(|item| item.id() == id)
            .cloned()
            .ok_or(pmc_domain::decisions::DecisionEvidenceAuthorityError::NotFound)
    }
}

fn create_transition_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    request: &DecisionRequestRecord,
    context: &DecisionOperationContext,
) -> Result<AuditEvent, DomainError> {
    let code =
        AuditEventCode::parse("decision_request.submitted").map_err(|_| storage_error(context))?;
    let effect =
        AuditEffectCode::parse("decision_request.submitted").map_err(|_| storage_error(context))?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect],
    )
    .map_err(|_| storage_error(context))?;
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            code,
            AuditTarget::DecisionRequest(request.id().clone()),
        ),
        context.correlation_id.clone(),
        disposition,
    ))
}

fn create_withdraw_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    request: &DecisionRequestRecord,
    context: &DecisionOperationContext,
) -> Result<AuditEvent, DomainError> {
    let code =
        AuditEventCode::parse("decision_request.withdrawn").map_err(|_| storage_error(context))?;
    let effect =
        AuditEffectCode::parse("decision_request.withdrawn").map_err(|_| storage_error(context))?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect],
    )
    .map_err(|_| storage_error(context))?;
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            code,
            AuditTarget::DecisionRequest(request.id().clone()),
        ),
        context.correlation_id.clone(),
        disposition,
    ))
}

fn insert_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
    context: &DecisionOperationContext,
    effect: &str,
) -> Result<(), DomainError> {
    let AuditTarget::DecisionRequest(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,'decision_request',?4,?5,'allowed','not_required','succeeded','complete')", rusqlite::params![audit.id().as_str(),audit.occurred_at().unix_millis(),audit.code().as_str(),target_id.as_str(),context.correlation_id.as_str()]).map_err(|_| storage_error(context))?;
    tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete','decision_request',?3)", rusqlite::params![audit.id().as_str(),effect,target_id.as_str()]).map_err(|_| storage_error(context))?;
    Ok(())
}

fn validate_persisted_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
) -> Result<(), DecisionPersistenceLoadError> {
    let AuditTarget::DecisionRequest(target_id) = audit.target() else {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    };
    let exact_event = tx
        .query_row(
            "SELECT count(*) FROM audit_events WHERE id=?1 AND occurred_at=?2 AND actor='head_of_products' AND module='work_management' AND event_code=?3 AND target_type='decision_request' AND target_id=?4 AND correlation_id=?5 AND policy_outcome='allowed' AND approval_outcome='not_required' AND execution_outcome='succeeded' AND effect_scope='complete'",
            rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_id.as_str(), audit.correlation_id().as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let exact_effect = tx
        .query_row(
            "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1 AND ordinal=0 AND effect_code=?2 AND scope='complete' AND target_type='decision_request' AND target_id=?3",
            rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let effect_count = tx
        .query_row(
            "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1",
            [audit.id().as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if exact_event != 1 || exact_effect != 1 || effect_count != 1 {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    Ok(())
}

fn validate_persisted_h2a_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
) -> Result<(), DecisionPersistenceLoadError> {
    let (target_type, target_id) = match audit.target() {
        AuditTarget::DecisionRequest(id) => ("decision_request", id.as_str()),
        AuditTarget::Decision(id) => ("decision", id.as_str()),
        _ => return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot),
    };
    let exact_event = tx
        .query_row(
            "SELECT count(*) FROM audit_events WHERE id=?1 AND occurred_at=?2 AND actor='head_of_products' AND module='work_management' AND event_code=?3 AND target_type=?4 AND target_id=?5 AND correlation_id=?6 AND policy_outcome='allowed' AND approval_outcome='approved' AND execution_outcome='succeeded' AND effect_scope='complete'",
            rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, audit.correlation_id().as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let exact_effect = tx
        .query_row(
            "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1 AND ordinal=0 AND effect_code=?2 AND scope='complete' AND target_type=?3 AND target_id=?4",
            rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_type, target_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let effect_count = tx
        .query_row(
            "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1",
            [audit.id().as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    if exact_event != 1 || exact_effect != 1 || effect_count != 1 {
        return Err(DecisionPersistenceLoadError::InvalidDecisionSnapshot);
    }
    Ok(())
}

/// Construct one Decision Supersede EXECUTE audit event directly from its
/// persisted id/timestamp plus the expected event code/target -- used only
/// by Supersede's own decode, which reconstructs `SupersededDecisionOutcome`
/// from persisted rows rather than re-running domain `execute_supersede`
/// (see the Supersede EXECUTE decode loop's own comment in `decode_namespace`
/// for why). `validate_persisted_h2a_audit` cross-checks the result against
/// the real `audit_events`/`audit_effects` rows immediately after
/// construction, so this constructor does not need to itself be trusted.
fn decode_h2a_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    code: &str,
    target: AuditTarget,
    correlation_id: CorrelationId,
) -> Result<AuditEvent, DecisionPersistenceLoadError> {
    let event_code = AuditEventCode::parse(code)
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let effect = AuditEffectCode::parse(code)
        .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect],
    )
    .map_err(|_| DecisionPersistenceLoadError::InvalidDecisionSnapshot)?;
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(AuditModule::WorkManagement, event_code, target),
        correlation_id,
        disposition,
    ))
}

fn idempotency_claimed_by_other_namespace(
    tx: &Transaction<'_>,
    context: &DecisionOperationContext,
) -> Result<bool, DomainError> {
    // Includes V36's `action_decision_replay_operations`: Decision
    // Supersede's execute now stages Action
    // capsules through the very same idempotency-id space as ordinary
    // Action operations (via `persist_action_decision_capsules`), so a
    // collision there is a real cross-namespace conflict this check must
    // catch too, not just the pre-existing five namespaces.
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM action_replay_operations WHERE idempotency_id=?1 UNION ALL SELECT 1 FROM action_decision_replay_operations WHERE idempotency_id=?1 UNION ALL SELECT 1 FROM relationship_replay_operations WHERE idempotency_id=?1 UNION ALL SELECT 1 FROM delivery_idempotency_outcomes WHERE idempotency_id=?1 UNION ALL SELECT 1 FROM portfolio_idempotency_outcomes WHERE idempotency_id=?1 UNION ALL SELECT 1 FROM operations WHERE idempotency_id=?1)",
        [context.idempotency_id.as_str()],
        |row| row.get::<_, i64>(0),
    )
    .map(|found| found != 0)
    .map_err(|_| storage_error(context))
}

fn snapshot_create_outcome(
    snapshot: DecisionPersistenceSnapshot,
    context: &DecisionOperationContext,
) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DomainError> {
    snapshot
        .replay()
        .iter()
        .find_map(|capsule| {
            if capsule.idempotency_id() == &context.idempotency_id {
                match capsule.result() {
                    DecisionPersistenceResult::Request(outcome) => Some(outcome.clone()),
                    DecisionPersistenceResult::Prepared(_)
                    | DecisionPersistenceResult::Resolved(_)
                    | DecisionPersistenceResult::Lowered(_)
                    | DecisionPersistenceResult::Superseded(_)
                    | DecisionPersistenceResult::Rejected(_) => None,
                }
            } else {
                None
            }
        })
        .ok_or_else(|| storage_error(context))
}

fn snapshot_prepared_outcome(
    snapshot: DecisionPersistenceSnapshot,
    context: &DecisionOperationContext,
) -> Result<WorkManagementPreparedIntent, DomainError> {
    snapshot
        .replay()
        .iter()
        .find_map(|capsule| {
            (capsule.idempotency_id() == &context.idempotency_id)
                .then(|| match capsule.result() {
                    DecisionPersistenceResult::Prepared(prepared) => Some(prepared.clone()),
                    _ => None,
                })
                .flatten()
        })
        .ok_or_else(|| storage_error(context))
}

fn snapshot_resolved_outcome(
    snapshot: DecisionPersistenceSnapshot,
    context: &DecisionOperationContext,
) -> Result<ResolvedDecisionOutcome, DomainError> {
    snapshot
        .replay()
        .iter()
        .find_map(|capsule| {
            (capsule.idempotency_id() == &context.idempotency_id)
                .then(|| match capsule.result() {
                    DecisionPersistenceResult::Resolved(outcome) => Some(outcome.clone()),
                    _ => None,
                })
                .flatten()
        })
        .ok_or_else(|| storage_error(context))
}
fn snapshot_superseded_outcome(
    snapshot: DecisionPersistenceSnapshot,
    context: &DecisionOperationContext,
) -> Result<SupersededDecisionOutcome, DomainError> {
    snapshot
        .replay()
        .iter()
        .find_map(|capsule| {
            (capsule.idempotency_id() == &context.idempotency_id)
                .then(|| match capsule.result() {
                    DecisionPersistenceResult::Superseded(outcome) => Some(outcome.clone()),
                    _ => None,
                })
                .flatten()
        })
        .ok_or_else(|| storage_error(context))
}
fn snapshot_lowered_outcome(
    snapshot: DecisionPersistenceSnapshot,
    context: &DecisionOperationContext,
) -> Result<DecisionMutationOutcome<DecisionRecord>, DomainError> {
    snapshot
        .replay()
        .iter()
        .find_map(|capsule| {
            (capsule.idempotency_id() == &context.idempotency_id)
                .then(|| match capsule.result() {
                    DecisionPersistenceResult::Lowered(outcome) => Some(outcome.clone()),
                    _ => None,
                })
                .flatten()
        })
        .ok_or_else(|| storage_error(context))
}

/// Writes the three rows a rejection is made of, in the order the binding
/// trigger needs: the intent is consumed first, then the zero-effect audit,
/// then the result row that references both.
fn persist_decision_prepared_intent_rejection(
    tx: &Transaction<'_>,
    context: &DecisionOperationContext,
    operation_ordinal: i64,
    outcome: &RejectedPreparedIntentOutcome,
) -> Result<(), DomainError> {
    let audit = outcome.audit_event();
    let AuditTarget::DecisionRequest(target_id) = audit.target() else {
        return Err(storage_error(context));
    };
    if tx
        .execute(
            "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND intent_kind='resolve_decision_request' AND consumed_at IS NULL",
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
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,'decision_request',?4,?5,'allowed','rejected','not_attempted','none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_id.as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO decision_reject_prepared_command_results (operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES ('reject_prepared',?1,?2,?3,?4,'head_of_products',?5,?6)",
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

fn insert_correlation_anchor(
    tx: &Transaction<'_>,
    context: &DecisionOperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO decision_h2a_correlation_anchors (idempotency_id,correlation_id) VALUES (?1,?2)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn storage_error(context: &DecisionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("ledger.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}
fn already_exists(context: &DecisionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("decision.already_exists").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn idempotency_conflict(context: &DecisionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("ledger.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn not_found(context: &DecisionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("decision.not_found").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn transition_conflict(context: &DecisionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("decision.request_transition_conflict")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
