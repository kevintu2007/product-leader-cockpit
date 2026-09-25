//! Typed SQLite persistence seam for the independent Issue H1 creation lifecycle
//! and the Issue H2a Resolve/Close/Reopen prepare and execute lifecycle.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
        AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, EvidenceReferenceId,
        IdempotencyId, IssueId, PreparedIntentId, RiskId,
    },
    issues::{
        ApproveAndExecuteIssueTransition, ApproveAndExecuteLowerIssueClassification, CreateIssue,
        DenyIssueEvidenceAuthority, IssueClassificationAuthorityPort, IssueEvidenceAuthorityPort,
        IssueExecutionPolicy, IssueExecutionPolicyPort, IssueH1RuntimeSnapshot,
        IssueH2aRejectionReplay, IssueH2aRuntimeSnapshot, IssueMutationOutcome,
        IssueOperationContext, IssueRecord, IssueServiceIdSource, PrepareCloseIssue,
        PrepareLowerIssueClassification, PrepareReopenIssue, PrepareResolveIssue,
        RecordedIssueClassification, RejectIssuePreparedIntent,
    },
    risks::{
        AllowRiskEvidence, RecordedRiskClassification, RiskExecutionPolicy,
        RiskExecutionPolicyPort, RiskServiceIdSource,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        prepared_intent_rejection_audit, ApprovalAuthorizationPort, EvidenceOrJudgment,
        EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification, HumanJudgment,
        HumanJudgmentDisposition, IntegrityDigest, IssueResolutionType, IssueState,
        RejectedPreparedIntentOutcome, SupportDisposition, SupportWitness, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
        ISSUE_PREPARED_REJECTED_AUDIT_CODE,
    },
    work_management_runtime::WorkManagementRuntimeComposition,
};
use rusqlite::{OptionalExtension, Transaction};

use super::{LedgerTransactionError, SqliteProductLedger};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IssuePersistenceLoadError {
    StorageUnavailable,
    InvalidIssueSnapshot,
}

#[derive(Clone, Copy)]
struct FixedClock(UtcTimestamp);
impl Clock for FixedClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

#[derive(Clone)]
struct ExecuteIssueIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_id: Option<AuditEventId>,
}
impl IssueServiceIdSource for ExecuteIssueIds {
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
        self.audit_id.take().ok_or_else(missing_execute_id)
    }
}

/// Never actually invoked: composition-level rehydration requires a Risk id
/// source, but an Issue's own resolve/close/reopen execute never touches the
/// Risk side of the shared composition.
#[derive(Clone, Copy, Default)]
struct UnusedRiskIds;
impl RiskServiceIdSource for UnusedRiskIds {
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
    match IssueId::parse("") {
        Err(error) => error,
        Ok(_) => unreachable!("empty opaque identifiers must be rejected"),
    }
}

/// The Risk side of the shared composition is purely auxiliary here -- an
/// Issue's own resolve/close/reopen execute never reads or mutates a Risk --
/// so, mirroring how `risk_repository.rs` hardcodes its own auxiliary
/// Issue-side execution policy port instead of exposing it generically, this
/// stays internal rather than becoming a caller-supplied generic parameter.
#[derive(Clone, Copy, Default)]
struct DenyRiskExecution;
impl RiskExecutionPolicyPort for DenyRiskExecution {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        RiskExecutionPolicy::Denied
    }
}

/// A rejection never consults the execution policy (nothing executes), but
/// composition-level rehydration still needs one; mirrors
/// `action_repository.rs`'s `PersistedRejectPolicy`.
#[derive(Clone, Copy, Default)]
struct RejectIssuePolicy;
impl IssueExecutionPolicyPort for RejectIssuePolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}

/// Which of the three unified Issue H2a execute operations is being run.
/// The domain layer collapses Resolve/Close/Reopen into one `execute` path
/// and one `IssueMutationOutcome`; this adapter still needs the distinction
/// to select the right typed V16 command table and topology check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IssueH2aExecuteKind {
    Resolve,
    Close,
    Reopen,
}
impl IssueH2aExecuteKind {
    const fn operation_str(self) -> &'static str {
        match self {
            Self::Resolve => "execute_resolve",
            Self::Close => "execute_close",
            Self::Reopen => "execute_reopen",
        }
    }
    const fn result_kind(self) -> &'static str {
        match self {
            Self::Resolve => "resolved",
            Self::Close => "closed",
            Self::Reopen => "reopened",
        }
    }
    const fn intent_kind(self) -> &'static str {
        match self {
            Self::Resolve => "resolve_issue",
            Self::Close => "close_issue",
            Self::Reopen => "reopen_issue",
        }
    }
    fn from_intent_kind(kind: &str) -> Option<Self> {
        match kind {
            "resolve_issue" => Some(Self::Resolve),
            "close_issue" => Some(Self::Close),
            "reopen_issue" => Some(Self::Reopen),
            _ => None,
        }
    }
    const fn command_table(self) -> &'static str {
        match self {
            Self::Resolve => "issue_h2a_command_execute_resolves",
            Self::Close => "issue_h2a_command_execute_closes",
            Self::Reopen => "issue_h2a_command_execute_reopens",
        }
    }
    /// Local convention used by `issue_evidence`/`issue_h2a_support_evidence_snapshots`;
    /// distinct from `EvidenceRole`'s own `evidence_references.role` strings.
    const fn evidence_role_column(self) -> &'static str {
        match self {
            Self::Resolve => "resolution",
            Self::Close => "closure_verification",
            Self::Reopen => "failed_verification",
        }
    }
    const fn evidence_role(self) -> EvidenceRole {
        match self {
            Self::Resolve => EvidenceRole::IssueResolution,
            Self::Close => EvidenceRole::IssueClosureVerification,
            Self::Reopen => EvidenceRole::IssueFailedVerification,
        }
    }
    fn matches_operation(self, operation: &WorkManagementOperation) -> bool {
        matches!(
            (self, operation),
            (Self::Resolve, WorkManagementOperation::ResolveIssue { .. })
                | (Self::Close, WorkManagementOperation::CloseIssue { .. })
                | (Self::Reopen, WorkManagementOperation::ReopenIssue { .. })
        )
    }
}

impl SqliteProductLedger {
    /// The preview an Issue H2a PREPARE already produced for this client
    /// request, if it produced one. See
    /// [`SqliteProductLedger::risk_prepared_intent_for_client_request`] for
    /// why a facade must ask before minting.
    pub fn issue_prepared_intent_for_client_request(
        &self,
        idempotency: &IdempotencyId,
    ) -> Result<Option<PreparedIntentId>, IssuePersistenceLoadError> {
        self.connection
            .query_row(
                "SELECT result_reference FROM issue_h2a_prepare_replay_operations WHERE idempotency_id=?1",
                [idempotency.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| IssuePersistenceLoadError::StorageUnavailable)?
            .map(PreparedIntentId::parse)
            .transpose()
            .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)
    }

    /// Reconstructs every Issue at its current lifecycle state, with every
    /// outstanding H2a preview and every durable v46 rejection.
    ///
    /// [`Self::load_issue_h1_runtime_snapshot`] answers a narrower question --
    /// standalone Issues still at their create state -- and is the wrong
    /// authority for anything that must not miss an Issue. This one misses
    /// none, which is what a Risk occurrence needs to refuse a duplicate
    /// identity, and what the application's Issue H2a facade needs to build a
    /// preview through the domain service rather than by hand.
    pub fn load_issue_h2a_runtime_snapshot(
        &self,
    ) -> Result<IssueH2aRuntimeSnapshot, IssuePersistenceLoadError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| IssuePersistenceLoadError::StorageUnavailable)?;
        let snapshot = decode_issue_h2a_runtime_snapshot(&transaction)?;
        transaction
            .commit()
            .map_err(|_| IssuePersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }

    /// Reconstructs every standalone (not Risk-derived) independent Issue at
    /// its untouched H1 create state, which `IssueH1RuntimeSnapshot::try_new`
    /// enforces: Open, version 1, no lifecycle history.
    ///
    /// That shape is narrower than "every standalone Issue". The query below
    /// already excludes a resolved, closed or reopened Issue, but a
    /// *classification lowering* leaves state and the resolution columns
    /// untouched while bumping the version, so such an Issue still matches
    /// the query and is then refused by the constructor -- and the whole
    /// snapshot with it. Callers that need every Issue at its current state
    /// (Risk occurrence needs exactly that, for its identity authority) must
    /// not build it from here.
    pub fn load_issue_h1_runtime_snapshot(
        &self,
    ) -> Result<IssueH1RuntimeSnapshot, IssuePersistenceLoadError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| IssuePersistenceLoadError::StorageUnavailable)?;
        let records = decode_issue_h1_records(&transaction)?;
        let snapshot = IssueH1RuntimeSnapshot::try_new(records)
            .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?;
        transaction
            .commit()
            .map_err(|_| IssuePersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }

    pub fn create_issue(
        &mut self,
        command: CreateIssue,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some((id, title, details, classification, recurrence)) = tx.query_row(
                "SELECT command.issue_id,command.title,command.details,command.classification,command.recurrence_of_id FROM issue_replay_operations replay JOIN issue_command_creates command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                [context.idempotency_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?)),
            ).optional().map_err(|_| storage_error(&context))? {
                if id == command.id.as_str() && title == command.title.as_str() && details == command.details.as_str() && classification == command.classification.as_persisted() && recurrence.is_none() && command.recurrence_of.is_none() {
                    return outcome(tx, &command.id, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            // Recurrence is not implemented in this writer (the column is
            // always NULL below). Say so as a validation refusal, not as a
            // retryable storage failure.
            if command.recurrence_of.is_some() {
                return Err(recurrence_not_supported(&context));
            }
            if command.classification == pmc_domain::classification::DataClassification::Unclassified {
                return Err(storage_error(&context));
            }
            let claimed: i64 = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)", [context.idempotency_id.as_str()], |row| row.get(0)).map_err(|_| storage_error(&context))?;
            if claimed != 0 { return Err(idempotency_conflict(&context)); }
            let exists: i64 = tx.query_row("SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)", [command.id.as_str()], |row| row.get(0)).map_err(|_| storage_error(&context))?;
            if exists != 0 { return Err(idempotency_conflict(&context)); }
            tx.execute("INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'issue',1,?2,?3,?3)", rusqlite::params![command.id.as_str(), command.classification.as_persisted(), occurred_at.unix_millis()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO issues(id,source_risk_id,recurrence_of_id,title,details,state,resolution_type,resolution_rationale) VALUES(?1,NULL,NULL,?2,?3,'open',NULL,NULL)", rusqlite::params![command.id.as_str(), command.title.as_str(), command.details.as_str()]).map_err(|_| storage_error(&context))?;
            let audit = audit(audit_event_id, occurred_at, &command.id, &context)?;
            tx.execute("INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management','issue.created','issue',?3,?4,'allowed','not_required','succeeded','complete')", rusqlite::params![audit.id().as_str(), occurred_at.unix_millis(), command.id.as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,'issue.created','complete','issue',?2)", rusqlite::params![audit.id().as_str(), command.id.as_str()]).map_err(|_| storage_error(&context))?;
            let ordinal: i64 = tx.query_row("SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM issue_replay_operations", [], |row| row.get(0)).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO ledger_idempotency_claims(idempotency_id,namespace,operation) VALUES(?1,'issue','create')", [context.idempotency_id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO issue_replay_operations(idempotency_id,correlation_id,operation_ordinal,result_reference) VALUES(?1,?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, command.id.as_str()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO issue_command_creates(idempotency_id,issue_id,title,details,classification,recurrence_of_id) VALUES(?1,?2,?3,?4,?5,NULL)", rusqlite::params![context.idempotency_id.as_str(), command.id.as_str(), command.title.as_str(), command.details.as_str(), command.classification.as_persisted()]).map_err(|_| storage_error(&context))?;
            tx.execute("INSERT INTO issue_replay_audits(idempotency_id,audit_event_id,correlation_id) VALUES(?1,?2,?3)", rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| storage_error(&context))?;
            let mutation = outcome(tx, &command.id, &context)?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| storage_error(&context))?;
            Ok(mutation)
        })
    }

    /// Persist one already-canonical H2a Issue resolve preview. The domain
    /// service is the only authority that may create `prepared`; this adapter
    /// re-verifies its exact topology before atomically storing the typed
    /// command, evidence/judgment support, and replay records.
    pub fn prepare_resolve_issue(
        &mut self,
        command: PrepareResolveIssue,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_resolve")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT issue_id,expected_version,resolution_type,rationale,result_reference FROM issue_h2a_prepare_replay_operations replay JOIN issue_h2a_command_prepare_resolves command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.issue_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.resolution_type.as_persisted()
                    && existing.3 == command.rationale.as_str()
                    && existing.4 == prepared.id().as_str()
                    && support_matches_command(&prepared, &command.evidence_ids, command.judgment.as_ref())
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (state, _classification, version): (String, String, i64) = tx
                .query_row(
                    "SELECT issues.state,registry.classification,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.id=?1",
                    [command.issue_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| not_found(&context))?;
            if state != "open" || version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::ResolveIssue {
                issue_id,
                issue_version,
                resolution_type,
                rationale,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if issue_id != &command.issue_id
                || issue_version != &command.expected_version
                || *resolution_type != command.resolution_type
                || rationale != &command.rationale
                || !support_matches_command(&prepared, &command.evidence_ids, command.judgment.as_ref())
            {
                return Err(transition_conflict(&context));
            }
            let ordinal = next_issue_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_issue_h2a_support_witness(tx, &prepared, &context)?;
            persist_prepared_intent(tx, &prepared, "resolve_issue", &context)?;
            persist_issue_h2a_support_detail(tx, &prepared, EvidenceRole::IssueResolution, &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'issue',?2,?3)",
                rusqlite::params![prepared.id().as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,rationale,resolution_type) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![prepared.id().as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.rationale.as_str(), command.resolution_type.as_persisted()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_resolve',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_command_prepare_resolves (idempotency_id,issue_id,expected_version,resolution_type,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![context.idempotency_id.as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.resolution_type.as_persisted(), command.rationale.as_str()],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// Persist one already-canonical H2a Issue close preview. See
    /// [`Self::prepare_resolve_issue`] for the shared rationale.
    pub fn prepare_close_issue(
        &mut self,
        command: PrepareCloseIssue,
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
                    "SELECT issue_id,expected_version,result_reference FROM issue_h2a_prepare_replay_operations replay JOIN issue_h2a_command_prepare_closes command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.issue_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == prepared.id().as_str()
                    && support_matches_command(&prepared, &command.evidence_ids, command.judgment.as_ref())
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (state, _classification, version): (String, String, i64) = tx
                .query_row(
                    "SELECT issues.state,registry.classification,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.id=?1",
                    [command.issue_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| not_found(&context))?;
            if state != "resolved" || version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::CloseIssue {
                issue_id,
                issue_version,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if issue_id != &command.issue_id
                || issue_version != &command.expected_version
                || !support_matches_command(&prepared, &command.evidence_ids, command.judgment.as_ref())
            {
                return Err(transition_conflict(&context));
            }
            let ordinal = next_issue_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_issue_h2a_support_witness(tx, &prepared, &context)?;
            persist_prepared_intent(tx, &prepared, "close_issue", &context)?;
            persist_issue_h2a_support_detail(tx, &prepared, EvidenceRole::IssueClosureVerification, &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'issue',?2,?3)",
                rusqlite::params![prepared.id().as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version) VALUES (?1,?2,?3)",
                rusqlite::params![prepared.id().as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_close',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_command_prepare_closes (idempotency_id,issue_id,expected_version) VALUES (?1,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// Persist one already-canonical H2a Issue reopen preview. See
    /// [`Self::prepare_resolve_issue`] for the shared rationale.
    pub fn prepare_reopen_issue(
        &mut self,
        command: PrepareReopenIssue,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, "prepare_reopen")? {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT issue_id,expected_version,rationale,result_reference FROM issue_h2a_prepare_replay_operations replay JOIN issue_h2a_command_prepare_reopens command USING(idempotency_id) WHERE replay.idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing.0 == command.issue_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.rationale.as_str()
                    && existing.3 == prepared.id().as_str()
                    && support_matches_command(&prepared, &command.evidence_ids, command.judgment.as_ref())
                    && prepared_intent_digest_matches(tx, &prepared, &context)?
                {
                    return Ok(prepared);
                }
                return Err(idempotency_conflict(&context));
            }
            let (state, _classification, version): (String, String, i64) = tx
                .query_row(
                    "SELECT issues.state,registry.classification,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.id=?1",
                    [command.issue_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| not_found(&context))?;
            if state != "resolved" || version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(transition_conflict(&context));
            }
            let WorkManagementOperation::ReopenIssue {
                issue_id,
                issue_version,
                rationale,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if issue_id != &command.issue_id
                || issue_version != &command.expected_version
                || rationale != &command.rationale
                || !support_matches_command(&prepared, &command.evidence_ids, command.judgment.as_ref())
            {
                return Err(transition_conflict(&context));
            }
            let ordinal = next_issue_h2a_prepare_operation_ordinal(tx, &context)?;
            persist_issue_h2a_support_witness(tx, &prepared, &context)?;
            persist_prepared_intent(tx, &prepared, "reopen_issue", &context)?;
            persist_issue_h2a_support_detail(tx, &prepared, EvidenceRole::IssueFailedVerification, &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'issue',?2,?3)",
                rusqlite::params![prepared.id().as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,rationale) VALUES (?1,?2,?3,?4)",
                rusqlite::params![prepared.id().as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.rationale.as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_reopen',?2,?3,'prepared',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, prepared.id().as_str()],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_command_prepare_reopens (idempotency_id,issue_id,expected_version,rationale) VALUES (?1,?2,?3,?4)",
                rusqlite::params![context.idempotency_id.as_str(), command.issue_id.as_str(), i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?, command.rationale.as_str()],
            ).map_err(|_| storage_error(&context))?;
            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx.execute("UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2", rusqlite::params![expected_revision + 1, expected_revision]).map_err(|_| storage_error(&context))? != 1 {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// v46: durable, audited rejection of an outstanding Issue H2a preview
    /// (resolve, close or reopen). Same contract as
    /// `risk_repository.rs`'s `reject_risk_prepared_intent`. The snapshot is
    /// the narrow per-Issue one every Issue execute writer rehydrates from,
    /// now including that Issue's earlier rejections so the domain replays
    /// exactly and refuses a second rejection of the same preview.
    pub fn reject_issue_prepared_intent<Z>(
        &mut self,
        command: RejectIssuePreparedIntent,
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
                    "SELECT prepared_intent_id,actor FROM issue_reject_prepared_command_results WHERE idempotency_id=?1",
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
                    if super::risk_repository::consumed_other_than_by_rejection(
                        tx,
                        &command.prepared_id,
                        "('resolve_issue','close_issue','reopen_issue')",
                        "issue_reject_prepared_command_results",
                    )
                    .map_err(|_| storage_error(&context))?
                    {
                        return Err(transition_conflict(&context));
                    }
                }
            }
            // Locate the preview among the three rejectable Issue kinds; an
            // unknown id is the domain's NotFound, not a storage failure.
            let Some((intent_kind, consumed_at, primary_id)) = tx
                .query_row(
                    "SELECT intent.intent_kind,intent.consumed_at,payload.primary_id FROM prepared_intents intent JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id WHERE intent.id=?1 AND intent.intent_kind IN ('resolve_issue','close_issue','reopen_issue')",
                    [command.prepared_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            else {
                return Err(not_found(&context));
            };
            let kind = IssueH2aExecuteKind::from_intent_kind(&intent_kind)
                .ok_or_else(|| storage_error(&context))?;
            let issue_id = IssueId::parse(primary_id).map_err(|_| storage_error(&context))?;
            let record = decode_issue_h2a_record(tx, &issue_id, &context)?;
            let issue_classification = record.classification();
            let rejections = decode_issue_rejections(tx, &issue_id, issue_classification, &context)?;
            let prepared = if consumed_at.is_none() {
                vec![decode_outstanding_prepared_intent(
                    tx,
                    kind,
                    &command.prepared_id,
                    issue_classification,
                    &context,
                )?]
            } else {
                Vec::new()
            };
            let issue_snapshot =
                IssueH2aRuntimeSnapshot::try_new_with_rejections(vec![record], prepared, rejections)
                    .map_err(|_| storage_error(&context))?;
            let mut composition = WorkManagementRuntimeComposition::rehydrate_with_issue_h2a(
                FixedClock(occurred_at),
                UnusedRiskIds,
                authorization.clone(),
                DenyRiskExecution,
                AllowRiskEvidence,
                RecordedRiskClassification,
                FixedClock(occurred_at),
                ExecuteIssueIds {
                    receipt_id: None,
                    audit_id: Some(audit_event_id.clone()),
                },
                authorization,
                RejectIssuePolicy,
                DenyIssueEvidenceAuthority,
                RecordedIssueClassification,
                issue_snapshot,
            )
            .map_err(|_| storage_error(&context))?;
            let outcome = composition.reject_issue_prepared_intent(command.clone())?;
            if replaying.is_some() {
                return Ok(outcome);
            }
            if outcome.audit_event().id() != &audit_event_id {
                return Err(storage_error(&context));
            }
            let ordinal = next_issue_reject_prepared_operation_ordinal(tx, &context)?;
            persist_issue_prepared_intent_rejection(tx, &context, ordinal, &outcome)?;
            decode_issue_rejections(tx, &issue_id, issue_classification, &context)?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| storage_error(&context))?;
            Ok(outcome)
        })
    }

    /// Execute one explicit H2a approval against an exact persisted V15
    /// resolve preview. See [`Self::approve_and_execute_issue_h2a`] for the
    /// shared rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_resolve_issue<Z, IP, IE, IC>(
        &mut self,
        command: ApproveAndExecuteIssueTransition,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        IP: IssueExecutionPolicyPort,
        IE: IssueEvidenceAuthorityPort,
        IC: IssueClassificationAuthorityPort,
    {
        self.approve_and_execute_issue_h2a(
            IssueH2aExecuteKind::Resolve,
            command,
            audit_event_id,
            approval_receipt_id,
            occurred_at,
            authorization,
            issue_policy,
            issue_evidence,
            issue_classification,
        )
    }

    /// Execute one explicit H2a approval against an exact persisted V15
    /// close preview. See [`Self::approve_and_execute_issue_h2a`] for the
    /// shared rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_close_issue<Z, IP, IE, IC>(
        &mut self,
        command: ApproveAndExecuteIssueTransition,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        IP: IssueExecutionPolicyPort,
        IE: IssueEvidenceAuthorityPort,
        IC: IssueClassificationAuthorityPort,
    {
        self.approve_and_execute_issue_h2a(
            IssueH2aExecuteKind::Close,
            command,
            audit_event_id,
            approval_receipt_id,
            occurred_at,
            authorization,
            issue_policy,
            issue_evidence,
            issue_classification,
        )
    }

    /// Execute one explicit H2a approval against an exact persisted V15
    /// reopen preview. See [`Self::approve_and_execute_issue_h2a`] for the
    /// shared rationale.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_reopen_issue<Z, IP, IE, IC>(
        &mut self,
        command: ApproveAndExecuteIssueTransition,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        IP: IssueExecutionPolicyPort,
        IE: IssueEvidenceAuthorityPort,
        IC: IssueClassificationAuthorityPort,
    {
        self.approve_and_execute_issue_h2a(
            IssueH2aExecuteKind::Reopen,
            command,
            audit_event_id,
            approval_receipt_id,
            occurred_at,
            authorization,
            issue_policy,
            issue_evidence,
            issue_classification,
        )
    }

    /// Shared driver for the unified Issue H2a execute family. Unlike Risk's
    /// occurrence/close execute writers, this never needs the nested-Result
    /// transaction pattern: `issues.rs` does not yet classify any execute
    /// failure as a durable post-start terminal (see V16's own doc comment),
    /// so a plain `Err` here rolls back exactly as every other H2a writer in
    /// this codebase already does.
    #[allow(clippy::too_many_arguments)]
    fn approve_and_execute_issue_h2a<Z, IP, IE, IC>(
        &mut self,
        kind: IssueH2aExecuteKind,
        command: ApproveAndExecuteIssueTransition,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        IP: IssueExecutionPolicyPort,
        IE: IssueEvidenceAuthorityPort,
        IC: IssueClassificationAuthorityPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if idempotency_claimed_by_other_operation(tx, &context, kind.operation_str())? {
                return Err(idempotency_conflict(&context));
            }
            let command_table = kind.command_table();
            if let Some(existing) = tx
                .query_row(
                    &format!("SELECT prepared_id,actor,acknowledged_digest FROM {command_table} WHERE idempotency_id=?1"),
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
                return replay_issue_execute_outcome(tx, kind, &context);
            }
            let primary_id: String = tx
                .query_row(
                    "SELECT payload.primary_id FROM prepared_intents intent JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id WHERE intent.id=?1 AND intent.intent_kind=?2 AND intent.consumed_at IS NULL",
                    rusqlite::params![command.approval.prepared_id().as_str(), kind.intent_kind()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| transition_conflict(&context))?;
            let issue_id = IssueId::parse(primary_id).map_err(|_| storage_error(&context))?;
            let record = decode_issue_h2a_record(tx, &issue_id, &context)?;
            let prepared = decode_outstanding_prepared_intent(
                tx,
                kind,
                command.approval.prepared_id(),
                record.classification(),
                &context,
            )?;
            if !kind.matches_operation(prepared.operation()) {
                return Err(transition_conflict(&context));
            }
            let rejections =
                decode_issue_rejections(tx, &issue_id, record.classification(), &context)?;
            let issue_snapshot = IssueH2aRuntimeSnapshot::try_new_with_rejections(
                vec![record],
                vec![prepared],
                rejections,
            )
            .map_err(|_| storage_error(&context))?;
            let mut composition = WorkManagementRuntimeComposition::rehydrate_with_issue_h2a(
                FixedClock(occurred_at),
                UnusedRiskIds,
                authorization.clone(),
                DenyRiskExecution,
                AllowRiskEvidence,
                RecordedRiskClassification,
                FixedClock(occurred_at),
                ExecuteIssueIds {
                    receipt_id: Some(approval_receipt_id.clone()),
                    audit_id: Some(audit_event_id.clone()),
                },
                authorization,
                issue_policy,
                issue_evidence,
                issue_classification,
                issue_snapshot,
            )
            .map_err(|_| storage_error(&context))?;
            let outcome = match kind {
                IssueH2aExecuteKind::Resolve => {
                    composition.approve_and_execute_resolve_issue(command.clone())
                }
                IssueH2aExecuteKind::Close => {
                    composition.approve_and_execute_close_issue(command.clone())
                }
                IssueH2aExecuteKind::Reopen => {
                    composition.approve_and_execute_reopen_issue(command.clone())
                }
            }?;
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id)
            {
                return Err(storage_error(&context));
            }
            let ordinal = next_issue_h2a_execute_operation_ordinal(tx, &context)?;
            persist_issue_execute_bundle(tx, kind, &command.approval, &outcome, ordinal, &context)?;
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

    /// H2a step 1: persist one already-canonical Issue
    /// classification-lowering preview. Mirrors `prepare_resolve_issue`'s
    /// hand-rolled shape, but simpler: no Evidence-or-Judgment support (see
    /// `PrepareLowerIssueClassification`'s own doc comment), and its own
    /// fresh-root V28 replay authority rather than V15's shared, support-
    /// requiring `issue_h2a_prepare_replay_operations`.
    pub fn prepare_lower_issue_classification(
        &mut self,
        command: PrepareLowerIssueClassification,
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
                    "SELECT issue_id,expected_version,proposed_classification,rationale,result_reference FROM issue_h2a_lower_classification_prepare_replay_operations replay JOIN issue_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.idempotency_id=?1",
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
                if existing.0 == command.issue_id.as_str()
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
            let (classification, version): (String, i64) = tx
                .query_row(
                    "SELECT registry.classification,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.id=?1",
                    [command.issue_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| not_found(&context))?;
            if version != i64::try_from(command.expected_version.get()).unwrap_or(-1) {
                return Err(transition_conflict(&context));
            }
            let current_classification =
                DataClassification::from_persisted(&classification).map_err(|_| storage_error(&context))?;
            let WorkManagementOperation::LowerIssueClassification {
                issue_id,
                issue_version,
                current_classification: prepared_current_classification,
                proposed_classification,
                rationale,
            } = prepared.operation()
            else {
                return Err(transition_conflict(&context));
            };
            if issue_id != &command.issue_id
                || issue_version != &command.expected_version
                || prepared_current_classification != &current_classification
                || proposed_classification != &command.proposed_classification
                || rationale != &command.rationale
            {
                return Err(transition_conflict(&context));
            }
            let ordinal =
                next_issue_h2a_lower_classification_prepare_operation_ordinal(tx, &context)?;
            persist_lower_issue_classification_prepared(tx, &prepared, &context)?;
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'issue',?2,?3)",
                rusqlite::params![
                    prepared.id().as_str(),
                    command.issue_id.as_str(),
                    i64::try_from(command.expected_version.get()).map_err(|_| storage_error(&context))?,
                ],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_lower_classification_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            ).map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO issue_h2a_lower_classification_command_prepares (idempotency_id,issue_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.issue_id.as_str(),
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

    /// H2a step 2 for Issue. Rehydrates through
    /// `WorkManagementRuntimeComposition::rehydrate_with_issue_h2a` (the only
    /// public entry point into `InMemoryIssueService`: `SharedIssueAuthority`
    /// is `pub(crate)`, so there is no bypass constructor analogous to Risk's
    /// `rehydrate_with_h2a`), then calls the small additive
    /// `approve_and_execute_lower_issue_classification` forwarding method.
    /// `issues.rs` classifies no H2a execute failure as a durable post-start
    /// terminal for any operation, so a domain failure here is an ordinary
    /// rollback via `?`, matching `approve_and_execute_issue_h2a`.
    #[allow(clippy::too_many_arguments)]
    pub fn approve_and_execute_lower_issue_classification<Z, IP, IE, IC>(
        &mut self,
        command: ApproveAndExecuteLowerIssueClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        issue_policy: IP,
        issue_evidence: IE,
        issue_classification: IC,
    ) -> Result<IssueMutationOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort + Clone,
        IP: IssueExecutionPolicyPort,
        IE: IssueEvidenceAuthorityPort,
        IC: IssueClassificationAuthorityPort,
    {
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
                    "SELECT prepared_id,actor,acknowledged_digest FROM issue_h2a_lower_classification_command_executes WHERE idempotency_id=?1",
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
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                return replay_lowered_issue_outcome(tx, &context);
            }
            let primary_id: String = tx
                .query_row(
                    "SELECT target.target_id FROM prepared_intents intent JOIN prepared_intent_targets target ON target.prepared_intent_id=intent.id AND target.ordinal=0 WHERE intent.id=?1 AND intent.intent_kind='lower_issue_classification' AND intent.consumed_at IS NULL",
                    [command.approval.prepared_id().as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| transition_conflict(&context))?;
            let issue_id = IssueId::parse(primary_id).map_err(|_| storage_error(&context))?;
            let record = decode_issue_h2a_record(tx, &issue_id, &context)?;
            let prepared = decode_lower_issue_classification_prepared(
                tx,
                command.approval.prepared_id(),
                &record,
                &context,
            )?;
            let issue_snapshot = IssueH2aRuntimeSnapshot::try_new(vec![record], vec![prepared])
                .map_err(|_| storage_error(&context))?;
            let mut composition = WorkManagementRuntimeComposition::rehydrate_with_issue_h2a(
                FixedClock(occurred_at),
                UnusedRiskIds,
                authorization.clone(),
                DenyRiskExecution,
                AllowRiskEvidence,
                RecordedRiskClassification,
                FixedClock(occurred_at),
                ExecuteIssueIds {
                    receipt_id: Some(approval_receipt_id.clone()),
                    audit_id: Some(audit_event_id.clone()),
                },
                authorization,
                issue_policy,
                issue_evidence,
                issue_classification,
                issue_snapshot,
            )
            .map_err(|_| storage_error(&context))?;
            let outcome =
                composition.approve_and_execute_lower_issue_classification(command.clone())?;
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id)
            {
                return Err(storage_error(&context));
            }
            let ordinal =
                next_issue_h2a_lower_classification_execute_operation_ordinal(tx, &context)?;
            persist_lowered_issue_bundle(tx, &command.approval, &outcome, ordinal, &context)?;
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

pub(super) fn decode_issue_h1_records(
    tx: &rusqlite::Transaction<'_>,
) -> Result<Vec<IssueRecord>, IssuePersistenceLoadError> {
    let rows = tx
        .prepare("SELECT issue.id,issue.title,issue.details,registry.classification,registry.version FROM issues issue JOIN aggregate_registry registry ON registry.id=issue.id AND registry.aggregate_type='issue' WHERE issue.source_risk_id IS NULL AND issue.recurrence_of_id IS NULL AND issue.state='open' AND issue.resolution_type IS NULL AND issue.resolution_rationale IS NULL ORDER BY issue.id")
        .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?;
    rows.into_iter()
        .map(|(id, title, details, classification, version)| {
            IssueRecord::from_persisted_created_open(
                pmc_domain::identity::IssueId::parse(id)
                    .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?,
                pmc_domain::issues::IssueTitle::parse(title)
                    .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?,
                pmc_domain::issues::IssueDetails::parse(details)
                    .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?,
                pmc_domain::classification::DataClassification::from_persisted(&classification)
                    .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?,
                pmc_domain::identity::AggregateVersion::new(
                    u64::try_from(version)
                        .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?,
                )
                .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)?,
            )
            .map_err(|_| IssuePersistenceLoadError::InvalidIssueSnapshot)
        })
        .collect()
}

fn outcome(
    tx: &rusqlite::Transaction<'_>,
    id: &pmc_domain::identity::IssueId,
    context: &IssueOperationContext,
) -> Result<IssueMutationOutcome, DomainError> {
    let replay_audit_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM issue_replay_audits WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let replay_effect_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM issue_replay_audits replay_audit JOIN audit_effects effect ON effect.audit_event_id=replay_audit.audit_event_id WHERE replay_audit.idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    if replay_audit_count != 1 || replay_effect_count != 1 {
        return Err(storage_error(context));
    }
    let matching_audits: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM issue_replay_audits replay_audit JOIN audit_events audit ON audit.id=replay_audit.audit_event_id JOIN audit_effects effect ON effect.audit_event_id=audit.id WHERE replay_audit.idempotency_id=?1 AND replay_audit.correlation_id=?2 AND audit.correlation_id=?2 AND audit.actor='head_of_products' AND audit.module='work_management' AND audit.event_code='issue.created' AND audit.target_type='issue' AND audit.target_id=?3 AND audit.policy_outcome='allowed' AND audit.approval_outcome='not_required' AND audit.execution_outcome='succeeded' AND audit.effect_scope='complete' AND effect.ordinal=0 AND effect.effect_code='issue.created' AND effect.scope='complete' AND effect.target_type='issue' AND effect.target_id=?3",
            rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    if matching_audits != 1 {
        return Err(storage_error(context));
    }
    let row = tx.query_row("SELECT issue.title,issue.details,registry.classification,registry.version,audit.id,audit.occurred_at FROM issues issue JOIN aggregate_registry registry ON registry.id=issue.id AND registry.aggregate_type='issue' JOIN issue_replay_operations replay ON replay.result_reference=issue.id JOIN issue_replay_audits replay_audit ON replay_audit.idempotency_id=replay.idempotency_id JOIN audit_events audit ON audit.id=replay_audit.audit_event_id WHERE issue.id=?1 AND replay.idempotency_id=?2 AND replay.correlation_id=?3 AND replay_audit.correlation_id=?3 AND audit.correlation_id=?3 AND issue.source_risk_id IS NULL AND issue.recurrence_of_id IS NULL AND issue.state='open' AND issue.resolution_type IS NULL AND issue.resolution_rationale IS NULL", rusqlite::params![id.as_str(),context.idempotency_id.as_str(),context.correlation_id.as_str()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).map_err(|_| storage_error(context))?;
    let record = IssueRecord::from_persisted_created_open(
        id.clone(),
        pmc_domain::issues::IssueTitle::parse(row.0).map_err(|_| storage_error(context))?,
        pmc_domain::issues::IssueDetails::parse(row.1).map_err(|_| storage_error(context))?,
        pmc_domain::classification::DataClassification::from_persisted(&row.2)
            .map_err(|_| storage_error(context))?,
        pmc_domain::identity::AggregateVersion::new(
            u64::try_from(row.3).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
    )
    .map_err(|_| storage_error(context))?;
    Ok(IssueMutationOutcome {
        record,
        audit_events: vec![audit(
            AuditEventId::parse(row.4).map_err(|_| storage_error(context))?,
            UtcTimestamp::from_unix_millis(row.5),
            id,
            context,
        )?],
        approval_receipt_id: None,
    })
}

fn audit(
    id: AuditEventId,
    at: UtcTimestamp,
    issue_id: &pmc_domain::identity::IssueId,
    context: &IssueOperationContext,
) -> Result<AuditEvent, DomainError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("issue.created").map_err(|_| storage_error(context))?,
            AuditTarget::Issue(issue_id.clone()),
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse("issue.created").map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}
fn recurrence_not_supported(context: &IssueOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::ValidationInvalidField,
        MessageKey::parse("issue.recurrence_not_supported").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn storage_error(context: &IssueOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("ledger.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}
fn idempotency_conflict(context: &IssueOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("ledger.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn not_found(context: &IssueOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("issue.not_found").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
fn transition_conflict(context: &IssueOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("issue.stale_or_illegal").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

/// A claims row for this idempotency identifier under any operation other
/// than `operation` means it was already spent by a different command.
fn idempotency_claimed_by_other_operation(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
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
    context: &IssueOperationContext,
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

/// The preview's support must be exactly what the command asked for: the
/// same Evidence in the same order and the same Judgment (none, or one with
/// the same rationale and classification). The operation carries neither,
/// so without this a caller could pair a command with a preview whose
/// support says something the person never wrote.
fn support_matches_command(
    prepared: &WorkManagementPreparedIntent,
    evidence_ids: &[EvidenceReferenceId],
    judgment: Option<&pmc_domain::work_management::HumanJudgment>,
) -> bool {
    let Some(support) = prepared.preview().support() else {
        return false;
    };
    // Every Issue transition is Evidence-required. Re-deriving the witness
    // from its own Evidence and Judgment and comparing it whole refuses a
    // preview whose support was built any other way, including one carried
    // by a Judgment alone.
    let rederived =
        EvidenceOrJudgment::new(support.evidence().to_vec(), support.judgments().to_vec())
            .ok()
            .and_then(|candidate| candidate.evaluate_evidence_required().ok());
    !evidence_ids.is_empty()
        && rederived.as_ref() == Some(support)
        && support
            .evidence()
            .iter()
            .map(|evidence| evidence.id())
            .eq(evidence_ids.iter())
        && support.judgments().iter().eq(judgment)
}

fn next_issue_h2a_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM issue_h2a_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Persists the generic `prepared_intents` row shared by every H2a preview
/// family. Callers still separately insert the target and typed payload rows
/// this row's foreign keys require.
fn persist_prepared_intent(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    intent_kind: &str,
    context: &IssueOperationContext,
) -> Result<(), DomainError> {
    let expires_at = prepared.preview().expires_at().unix_millis();
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,expires_at,created_at) VALUES (?1,?2,?3,?4,?5,'allowed','not_cancellable_after_submit','head_of_products',?6,?7,?8)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(prepared.preview().contract_version()),
            intent_kind,
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            support_id_for(prepared),
            expires_at,
            expires_at - pmc_domain::work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS,
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn support_id_for(prepared: &WorkManagementPreparedIntent) -> String {
    format!("issue-h2a-support-{}", prepared.id().as_str())
}

/// Persists the `support_witnesses` row an Issue H2a preview's Evidence-or-
/// Judgment support requires. This must run before [`persist_prepared_intent`]
/// -- `prepared_intents.support_id` references this row -- while the
/// judgments/evidence detail in [`persist_issue_h2a_support_detail`] must run
/// after it, since the typed evidence snapshot references `prepared_intents`
/// in turn. The two halves cannot be one function without breaking one FK.
fn persist_issue_h2a_support_witness(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &IssueOperationContext,
) -> Result<(), DomainError> {
    let support = prepared
        .preview()
        .support()
        .ok_or_else(|| storage_error(context))?;
    let support_id = support_id_for(prepared);
    let disposition = match support.disposition() {
        SupportDisposition::EvidenceSatisfied => "evidence_satisfied",
        SupportDisposition::JudgmentSatisfied => "judgment_satisfied",
        SupportDisposition::VerificationPending => "verification_pending",
    };
    tx.execute(
        // Every Issue transition is Evidence-required (a Judgment only
        // carries partly verified Evidence; it never stands alone).
        "INSERT INTO support_witnesses (id,requirement,disposition,classification) VALUES (?1,'evidence_required',?2,?3)",
        rusqlite::params![support_id, disposition, support.classification().as_persisted()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Persists the judgments and typed evidence snapshot for the support witness
/// [`persist_issue_h2a_support_witness`] already created. See that function's
/// doc comment for why this half must run after [`persist_prepared_intent`].
fn persist_issue_h2a_support_detail(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    required_role: EvidenceRole,
    context: &IssueOperationContext,
) -> Result<(), DomainError> {
    let support = prepared
        .preview()
        .support()
        .ok_or_else(|| storage_error(context))?;
    let support_id = support_id_for(prepared);
    for (ordinal, judgment) in support.judgments().iter().enumerate() {
        tx.execute(
            "INSERT INTO support_judgments (support_id,ordinal,actor,disposition,rationale,classification) VALUES (?1,?2,'head_of_products','proceed_with_documented_rationale',?3,?4)",
            rusqlite::params![support_id, i64::try_from(ordinal).map_err(|_| storage_error(context))?, judgment.rationale(), judgment.classification().as_persisted()],
        ).map_err(|_| storage_error(context))?;
    }
    let role_str = match required_role {
        EvidenceRole::IssueResolution => "resolution",
        EvidenceRole::IssueClosureVerification => "closure_verification",
        EvidenceRole::IssueFailedVerification => "failed_verification",
        _ => return Err(storage_error(context)),
    };
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
        if evidence.role() != required_role {
            return Err(storage_error(context));
        }
        tx.execute(
            "INSERT INTO support_evidence (support_id,evidence_id) VALUES (?1,?2)",
            rusqlite::params![support_id, evidence.id().as_str()],
        )
        .map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO issue_h2a_support_evidence_snapshots (prepared_intent_id,ordinal,evidence_id,evidence_version,classification,role,verification,last_verified_at,integrity_digest) VALUES (?1,?2,?3,?9,?4,?5,?6,?7,?8)",
            rusqlite::params![prepared.id().as_str(), i64::try_from(ordinal).map_err(|_| storage_error(context))?, evidence.id().as_str(), evidence.classification().as_persisted(), role_str, verification, last_verified_at, integrity_digest, i64::try_from(evidence.source_version().get()).map_err(|_| storage_error(context))?],
        ).map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn next_issue_h2a_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM issue_h2a_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// The context the whole-Ledger decoders carry. Every per-Issue decoder maps
/// its failures through an `IssueOperationContext`, which only a command has;
/// a read has no idempotency or correlation identity of its own. This one is
/// a reserved placeholder whose error envelopes are always discarded and
/// replaced by [`IssuePersistenceLoadError`] before anything can observe
/// them -- no read path writes, so it can never claim an idempotency key.
fn snapshot_load_context() -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse("ledger-issue-h2a-snapshot-load")
            .unwrap_or_else(|_| unreachable!("reserved snapshot-load idempotency id")),
        correlation_id: CorrelationId::parse("ledger-issue-h2a-snapshot-load")
            .unwrap_or_else(|_| unreachable!("reserved snapshot-load correlation id")),
    }
}

/// Every Issue at its current state, every outstanding preview, every
/// durable rejection -- decoded through the same per-Issue decoders the write
/// paths use, driven over the whole `issues` table.
pub(super) fn decode_issue_h2a_runtime_snapshot(
    tx: &Transaction<'_>,
) -> Result<IssueH2aRuntimeSnapshot, IssuePersistenceLoadError> {
    let invalid = || IssuePersistenceLoadError::InvalidIssueSnapshot;
    let context = snapshot_load_context();
    let ids = tx
        .prepare("SELECT id FROM issues ORDER BY id")
        .map_err(|_| invalid())?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    let mut records = Vec::with_capacity(ids.len());
    for id in ids {
        let issue_id = IssueId::parse(id).map_err(|_| invalid())?;
        records.push(decode_issue_h2a_record(tx, &issue_id, &context).map_err(|_| invalid())?);
    }

    let mut prepared = Vec::new();
    let outstanding = tx
        .prepare("SELECT intent.id,intent.intent_kind,payload.primary_id FROM prepared_intents intent JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id WHERE intent.intent_kind IN ('resolve_issue','close_issue','reopen_issue','lower_issue_classification') AND intent.consumed_at IS NULL ORDER BY intent.id")
        .map_err(|_| invalid())?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| invalid())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid())?;
    for (prepared_id, intent_kind, primary_id) in outstanding {
        let prepared_id = PreparedIntentId::parse(prepared_id).map_err(|_| invalid())?;
        let issue_id = IssueId::parse(primary_id).map_err(|_| invalid())?;
        let record = records
            .iter()
            .find(|record| record.id() == &issue_id)
            .ok_or_else(invalid)?;
        let intent = if intent_kind == "lower_issue_classification" {
            decode_lower_issue_classification_prepared(tx, &prepared_id, record, &context)
        } else {
            let kind = IssueH2aExecuteKind::from_intent_kind(&intent_kind).ok_or_else(invalid)?;
            decode_prepared_intent(
                tx,
                kind,
                &prepared_id,
                record.classification(),
                &context,
                true,
            )
        }
        .map_err(|_| invalid())?;
        prepared.push(intent);
    }

    let mut rejections = Vec::new();
    for record in &records {
        rejections.extend(
            decode_issue_rejections(tx, record.id(), record.classification(), &context)
                .map_err(|_| invalid())?,
        );
    }

    IssueH2aRuntimeSnapshot::try_new_with_rejections(records, prepared, rejections)
        .map_err(|_| invalid())
}

fn next_issue_reject_prepared_operation_ordinal(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM issue_reject_prepared_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Persists one Issue rejection: consumes the intent at the rejection
/// instant (it must still be outstanding), records the zero-effect audit,
/// and appends the v46 result row, whose triggers claim the idempotency id
/// and refuse anything that did not consume an Issue intent at that instant.
fn persist_issue_prepared_intent_rejection(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
    operation_ordinal: i64,
    outcome: &RejectedPreparedIntentOutcome,
) -> Result<(), DomainError> {
    let audit = outcome.audit_event();
    let AuditTarget::Issue(issue_id) = audit.target() else {
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
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'issue',?4,?5,'allowed','rejected','not_attempted','none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            issue_id.as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO issue_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared',?1,?2,?3,?4,'head_of_products',?5,?6)",
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

/// Rebuilds every durable v46 rejection whose preview targeted `issue_id`
/// as the domain's replay input. The consumed preview is re-derived and
/// digest-verified exactly like an outstanding one, its consumption instant
/// must equal the recorded rejection instant, and the zero-effect audit is
/// re-derived wholesale and compared against the stored row.
fn decode_issue_rejections(
    tx: &Transaction<'_>,
    issue_id: &IssueId,
    issue_classification: DataClassification,
    context: &IssueOperationContext,
) -> Result<Vec<IssueH2aRejectionReplay>, DomainError> {
    let rows = tx
        .prepare("SELECT rejection.idempotency_id,rejection.correlation_id,rejection.rejected_at,rejection.audit_event_id,intent.id,intent.intent_kind,intent.consumed_at FROM issue_reject_prepared_command_results rejection JOIN prepared_intents intent ON intent.id=rejection.prepared_intent_id JOIN prepared_work_management_payloads payload ON payload.prepared_intent_id=intent.id WHERE payload.primary_id=?1 ORDER BY rejection.operation_ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([issue_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    rows.into_iter()
        .map(
            |(
                idempotency_id,
                correlation_id,
                rejected_at,
                audit_event_id,
                intent_id,
                intent_kind,
                consumed_at,
            )| {
                let kind = IssueH2aExecuteKind::from_intent_kind(&intent_kind)
                    .ok_or_else(|| storage_error(context))?;
                let prepared_id =
                    PreparedIntentId::parse(intent_id).map_err(|_| storage_error(context))?;
                let prepared = decode_prepared_intent(
                    tx,
                    kind,
                    &prepared_id,
                    issue_classification,
                    context,
                    false,
                )?;
                if consumed_at != Some(rejected_at) {
                    return Err(storage_error(context));
                }
                let rejected_at = UtcTimestamp::from_unix_millis(rejected_at);
                let correlation_id =
                    CorrelationId::parse(correlation_id).map_err(|_| storage_error(context))?;
                let audit = prepared_intent_rejection_audit(
                    AuditEventId::parse(audit_event_id).map_err(|_| storage_error(context))?,
                    rejected_at,
                    ISSUE_PREPARED_REJECTED_AUDIT_CODE,
                    AuditTarget::Issue(issue_id.clone()),
                    correlation_id.clone(),
                )
                .ok_or_else(|| storage_error(context))?;
                if !super::risk_repository::rejection_audit_row_matches(
                    tx,
                    &audit,
                    "issue",
                    issue_id.as_str(),
                ) {
                    return Err(storage_error(context));
                }
                let outcome = RejectedPreparedIntentOutcome::new(
                    prepared.id().clone(),
                    rejected_at,
                    rejected_at >= prepared.preview().expires_at(),
                    audit,
                );
                Ok(IssueH2aRejectionReplay::new(
                    prepared,
                    IssueOperationContext {
                        idempotency_id: IdempotencyId::parse(idempotency_id)
                            .map_err(|_| storage_error(context))?,
                        correlation_id,
                    },
                    outcome,
                ))
            },
        )
        .collect()
}

/// Reconstructs the full current `IssueRecord` for `issue_id`, including
/// every accumulated Vec field, from the durable tables the V16 execute
/// bundle writes: `issue_evidence` (resolution/closure/failed-verification
/// evidence, grouped by role), `issue_reopen_history` (rationales, ordered),
/// and `issue_support_history` (one `SupportWitness` per prior execute,
/// ordered). Unlike `decode_issue_h1_records`, this accepts any lifecycle
/// state and any version, since Issue cycles between Open and Resolved an
/// unbounded number of times before a terminal Close.
fn decode_issue_h2a_record(
    tx: &Transaction<'_>,
    issue_id: &IssueId,
    context: &IssueOperationContext,
) -> Result<IssueRecord, DomainError> {
    let row = tx
        .query_row(
            "SELECT issues.source_risk_id,issues.recurrence_of_id,issues.title,issues.details,registry.classification,issues.state,issues.resolution_type,issues.resolution_rationale,registry.version FROM issues JOIN aggregate_registry registry ON registry.id=issues.id AND registry.aggregate_type='issue' WHERE issues.id=?1",
            [issue_id.as_str()],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| not_found(context))?;
    let source_risk_id = row
        .0
        .map(RiskId::parse)
        .transpose()
        .map_err(|_| storage_error(context))?;
    let recurrence_of = row
        .1
        .map(IssueId::parse)
        .transpose()
        .map_err(|_| storage_error(context))?;
    let title = pmc_domain::issues::IssueTitle::parse(row.2).map_err(|_| storage_error(context))?;
    let details =
        pmc_domain::issues::IssueDetails::parse(row.3).map_err(|_| storage_error(context))?;
    let classification =
        DataClassification::from_persisted(&row.4).map_err(|_| storage_error(context))?;
    let state = IssueState::from_persisted(&row.5).map_err(|_| storage_error(context))?;
    let resolution_type = row
        .6
        .map(|value| IssueResolutionType::from_persisted(&value))
        .transpose()
        .map_err(|_| storage_error(context))?;
    let resolution_rationale = row
        .7
        .map(WorkManagementRationale::parse)
        .transpose()
        .map_err(|_| storage_error(context))?;
    let version = AggregateVersion::new(u64::try_from(row.8).map_err(|_| storage_error(context))?)
        .map_err(|_| storage_error(context))?;

    let resolution_evidence = decode_issue_evidence(tx, issue_id, "resolution", context)?;
    let closure_verification_evidence =
        decode_issue_evidence(tx, issue_id, "closure_verification", context)?;
    let failed_verification_evidence =
        decode_issue_evidence(tx, issue_id, "failed_verification", context)?;

    let reopen_rationales = tx
        .prepare("SELECT rationale FROM issue_reopen_history WHERE issue_id=?1 ORDER BY ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([issue_id.as_str()], |r| r.get::<_, String>(0))
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?
        .into_iter()
        .map(|value| WorkManagementRationale::parse(value).map_err(|_| storage_error(context)))
        .collect::<Result<Vec<_>, DomainError>>()?;

    let support_ids = tx
        .prepare("SELECT support_id FROM issue_support_history WHERE issue_id=?1 ORDER BY ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([issue_id.as_str()], |r| r.get::<_, String>(0))
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    let support_history = support_ids
        .into_iter()
        .map(|support_id| decode_issue_support_history_entry(tx, &support_id, context))
        .collect::<Result<Vec<_>, DomainError>>()?;

    IssueRecord::from_persisted(
        issue_id.clone(),
        source_risk_id,
        recurrence_of,
        title,
        details,
        classification,
        state,
        resolution_type,
        resolution_rationale,
        resolution_evidence,
        closure_verification_evidence,
        failed_verification_evidence,
        reopen_rationales,
        support_history,
        version,
    )
    .map_err(|_| storage_error(context))
}

fn decode_issue_evidence(
    tx: &Transaction<'_>,
    issue_id: &IssueId,
    role_column: &str,
    context: &IssueOperationContext,
) -> Result<Vec<EvidenceReferenceId>, DomainError> {
    tx.prepare(
        "SELECT evidence_id FROM issue_evidence WHERE issue_id=?1 AND role=?2 ORDER BY evidence_id",
    )
    .map_err(|_| storage_error(context))?
    .query_map(rusqlite::params![issue_id.as_str(), role_column], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|_| storage_error(context))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| storage_error(context))?
    .into_iter()
    .map(|id| EvidenceReferenceId::parse(id).map_err(|_| storage_error(context)))
    .collect()
}

/// Reconstructs one prior execute's `SupportWitness` from the typed evidence
/// snapshot its Prepared Intent recorded, so history shows the Evidence as it
/// stood when the transition ran. Current Evidence may since have become
/// unverified; that must not rewrite, or break, what was already decided. The
/// rebuilt witness must still agree with its `support_witnesses` header.
fn decode_issue_support_history_entry(
    tx: &Transaction<'_>,
    support_id: &str,
    context: &IssueOperationContext,
) -> Result<SupportWitness, DomainError> {
    let prepared = tx
        .prepare("SELECT id,intent_kind FROM prepared_intents WHERE support_id=?1")
        .map_err(|_| storage_error(context))?
        .query_map([support_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    let [(prepared_id, intent_kind)] = prepared.as_slice() else {
        return Err(storage_error(context));
    };
    let kind =
        IssueH2aExecuteKind::from_intent_kind(intent_kind).ok_or_else(|| storage_error(context))?;
    let prepared_id =
        PreparedIntentId::parse(prepared_id.clone()).map_err(|_| storage_error(context))?;
    let witness =
        decode_issue_h2a_support(tx, support_id, &prepared_id, kind.evidence_role(), context)?;
    let (requirement, disposition, classification) = tx
        .query_row(
            "SELECT requirement,disposition,classification FROM support_witnesses WHERE id=?1",
            [support_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    // Issue transitions are Evidence-required, which is what the header now
    // records. Rows written before Issues took a Judgment recorded
    // 'evidence_or_judgment' for the same Evidence-only support; both are
    // named here so any other value is refused rather than passed over.
    if !matches!(
        requirement.as_str(),
        "evidence_required" | "evidence_or_judgment"
    ) {
        return Err(storage_error(context));
    }
    let expected_disposition = match witness.disposition() {
        SupportDisposition::EvidenceSatisfied => "evidence_satisfied",
        SupportDisposition::JudgmentSatisfied => "judgment_satisfied",
        SupportDisposition::VerificationPending => "verification_pending",
    };
    if disposition != expected_disposition
        || classification != witness.classification().as_persisted()
    {
        return Err(storage_error(context));
    }
    Ok(witness)
}

fn decode_support_judgments(
    tx: &Transaction<'_>,
    support_id: &str,
    context: &IssueOperationContext,
) -> Result<Vec<HumanJudgment>, DomainError> {
    tx.prepare("SELECT rationale,classification FROM support_judgments WHERE support_id=?1 ORDER BY ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([support_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?
        .into_iter()
        .map(|(rationale, classification)| {
            HumanJudgment::new(
                HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                rationale,
                DataClassification::from_persisted(&classification)
                    .map_err(|_| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))
        })
        .collect()
}

fn decode_evidence_verification(
    verification: &str,
    last_verified_at: Option<i64>,
    integrity_digest: Option<String>,
    context: &IssueOperationContext,
) -> Result<EvidenceVerification, DomainError> {
    match (verification, last_verified_at, integrity_digest) {
        ("verified", Some(at), Some(digest)) => Ok(EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(at),
            integrity_digest: IntegrityDigest::parse(digest).map_err(|_| storage_error(context))?,
        }),
        ("observed_unpinned", Some(at), Some(digest)) => {
            Ok(EvidenceVerification::ObservedUnpinned {
                observed_at: UtcTimestamp::from_unix_millis(at),
                integrity_digest: IntegrityDigest::parse(digest)
                    .map_err(|_| storage_error(context))?,
            })
        }
        ("degraded_last_verified", Some(at), Some(digest)) => {
            Ok(EvidenceVerification::DegradedLastVerified {
                last_verified_at: UtcTimestamp::from_unix_millis(at),
                integrity_digest: IntegrityDigest::parse(digest)
                    .map_err(|_| storage_error(context))?,
            })
        }
        ("unverified", None, None) => Ok(EvidenceVerification::Unverified),
        ("integrity_mismatch", None, None) => Ok(EvidenceVerification::IntegrityMismatch),
        _ => Err(storage_error(context)),
    }
}

/// Reconstructs the exact, still-unconsumed prepared intent an execute
/// approval targets, from `prepared_intents` + `prepared_work_management_payloads`
/// plus the V15 typed evidence snapshot, then re-verifies its digest,
/// classification, and expiry against the durable row. This mirrors
/// `decision_repository.rs`'s `decode_resolve_prepared` and doubles as
/// tamper detection: a rebuilt intent whose recomputed digest disagrees with
/// what was stored can never reach the domain layer.
fn decode_outstanding_prepared_intent(
    tx: &Transaction<'_>,
    kind: IssueH2aExecuteKind,
    prepared_id: &PreparedIntentId,
    issue_classification: DataClassification,
    context: &IssueOperationContext,
) -> Result<WorkManagementPreparedIntent, DomainError> {
    decode_prepared_intent(tx, kind, prepared_id, issue_classification, context, true)
}

/// The decoder behind `decode_outstanding_prepared_intent`; with
/// `outstanding == false` it also rebuilds an already-consumed preview, which
/// is what a durable v46 rejection replays against.
fn decode_prepared_intent(
    tx: &Transaction<'_>,
    kind: IssueH2aExecuteKind,
    prepared_id: &PreparedIntentId,
    issue_classification: DataClassification,
    context: &IssueOperationContext,
    outstanding: bool,
) -> Result<WorkManagementPreparedIntent, DomainError> {
    let sql = if outstanding {
        "SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND intent_kind=?2 AND consumed_at IS NULL"
    } else {
        "SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND intent_kind=?2"
    };
    let intent = tx
        .query_row(
            sql,
            rusqlite::params![prepared_id.as_str(), kind.intent_kind()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| transition_conflict(context))?;
    let support_id = intent.4.clone().ok_or_else(|| storage_error(context))?;
    let payload = tx
        .query_row(
            "SELECT primary_id,primary_version,resolution_type,rationale FROM prepared_work_management_payloads WHERE prepared_intent_id=?1",
            [prepared_id.as_str()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let issue_id = IssueId::parse(payload.0).map_err(|_| storage_error(context))?;
    let issue_version =
        AggregateVersion::new(u64::try_from(payload.1).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?;
    let operation = match kind {
        IssueH2aExecuteKind::Resolve => WorkManagementOperation::ResolveIssue {
            issue_id,
            issue_version,
            resolution_type: IssueResolutionType::from_persisted(
                &payload.2.ok_or_else(|| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?,
            rationale: WorkManagementRationale::parse(
                payload.3.ok_or_else(|| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?,
        },
        IssueH2aExecuteKind::Close => WorkManagementOperation::CloseIssue {
            issue_id,
            issue_version,
        },
        IssueH2aExecuteKind::Reopen => WorkManagementOperation::ReopenIssue {
            issue_id,
            issue_version,
            rationale: WorkManagementRationale::parse(
                payload.3.ok_or_else(|| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?,
        },
    };
    let support =
        decode_issue_h2a_support(tx, &support_id, prepared_id, kind.evidence_role(), context)?;
    let rebuilt = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        issue_classification,
        Some(support),
        UtcTimestamp::from_unix_millis(intent.3),
    )
    .map_err(|_| storage_error(context))?;
    if rebuilt.payload_digest().as_str() != intent.0
        || rebuilt.classification().as_persisted() != intent.1
        || rebuilt.preview().expires_at().unix_millis() != intent.2
    {
        return Err(storage_error(context));
    }
    Ok(rebuilt)
}

/// Reconstructs the `SupportWitness` embedded in one outstanding V15 prepare
/// preview, from the typed per-prepared-intent evidence snapshot (not the
/// shared history tables -- this is the most recent preview, not a prior
/// execute).
fn decode_issue_h2a_support(
    tx: &Transaction<'_>,
    support_id: &str,
    prepared_id: &PreparedIntentId,
    role: EvidenceRole,
    context: &IssueOperationContext,
) -> Result<SupportWitness, DomainError> {
    let judgments = decode_support_judgments(tx, support_id, context)?;
    let role_column = match role {
        EvidenceRole::IssueResolution => "resolution",
        EvidenceRole::IssueClosureVerification => "closure_verification",
        EvidenceRole::IssueFailedVerification => "failed_verification",
        _ => return Err(storage_error(context)),
    };
    let evidence_rows = tx
        .prepare("SELECT evidence_id,classification,role,verification,last_verified_at,integrity_digest,evidence_version FROM issue_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([prepared_id.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    let evidence = evidence_rows
        .into_iter()
        .map(|row| {
            if row.2 != role_column {
                return Err(storage_error(context));
            }
            let verification = decode_evidence_verification(&row.3, row.4, row.5, context)?;
            let source_version =
                AggregateVersion::new(u64::try_from(row.6).map_err(|_| storage_error(context))?)
                    .map_err(|_| storage_error(context))?;
            Ok(EvidenceReferenceMetadata::new(
                EvidenceReferenceId::parse(row.0).map_err(|_| storage_error(context))?,
                source_version,
                DataClassification::from_persisted(&row.1).map_err(|_| storage_error(context))?,
                role,
                verification,
            ))
        })
        .collect::<Result<Vec<_>, DomainError>>()?;
    EvidenceOrJudgment::new(evidence, judgments)
        .map_err(|_| storage_error(context))?
        .evaluate_evidence_required()
        .map_err(|_| storage_error(context))
}

/// Persists the execute success bundle: the mutable `issues` row, the
/// binding `issue_evidence`/`issue_reopen_history`/`issue_support_history`
/// rows this cycle contributes, the single audit, the Approval Receipt,
/// prepared-intent consumption, and the V16 idempotent-replay row.
fn persist_issue_execute_bundle(
    tx: &Transaction<'_>,
    kind: IssueH2aExecuteKind,
    approval: &WorkManagementApproval,
    outcome: &IssueMutationOutcome,
    ordinal: i64,
    context: &IssueOperationContext,
) -> Result<(), DomainError> {
    let issue_id = outcome.record.id().as_str();
    let audit = outcome
        .audit_events
        .first()
        .ok_or_else(|| storage_error(context))?;
    let occurred_millis = audit.occurred_at().unix_millis();
    let receipt_id = outcome
        .approval_receipt_id
        .as_ref()
        .ok_or_else(|| storage_error(context))?;

    tx.execute(
        "UPDATE issues SET state=?1,resolution_type=?2,resolution_rationale=?3 WHERE id=?4",
        rusqlite::params![
            outcome.record.state().as_persisted(),
            outcome.record.resolution_type().map(|t| t.as_persisted()),
            outcome.record.resolution_rationale().map(|r| r.as_str()),
            issue_id,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='issue'",
        rusqlite::params![
            i64::try_from(outcome.record.version().get()).map_err(|_| storage_error(context))?,
            occurred_millis,
            issue_id
        ],
    )
    .map_err(|_| storage_error(context))?;

    let role_column = kind.evidence_role_column();
    // `resolution`/`closure_verification` are overwritten by the domain layer
    // on each resolve/close (see IssueRecord::resolution_evidence's doc), so
    // the durable row set for that role must be replaced, not accumulated --
    // otherwise a second resolve after a reopen would rehydrate a
    // `resolution_evidence` Vec containing both the stale and current
    // evidence, diverging from the domain's own in-memory result. Reopen's
    // `failed_verification` role is genuinely accumulated and must not be
    // cleared here.
    if !matches!(kind, IssueH2aExecuteKind::Reopen) {
        tx.execute(
            "DELETE FROM issue_evidence WHERE issue_id=?1 AND role=?2",
            rusqlite::params![issue_id, role_column],
        )
        .map_err(|_| storage_error(context))?;
    }
    let evidence_ids: Vec<String> = tx
        .prepare("SELECT evidence_id FROM issue_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([approval.prepared_id().as_str()], |r| r.get::<_, String>(0))
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    for evidence_id in evidence_ids {
        tx.execute(
            "INSERT INTO issue_evidence(issue_id,evidence_id,role) VALUES(?1,?2,?3)",
            rusqlite::params![issue_id, evidence_id, role_column],
        )
        .map_err(|_| storage_error(context))?;
    }

    if let IssueH2aExecuteKind::Reopen = kind {
        let rationale = outcome
            .record
            .reopen_rationales()
            .last()
            .ok_or_else(|| storage_error(context))?;
        let reopen_ordinal: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(ordinal)+1,0) FROM issue_reopen_history WHERE issue_id=?1",
                [issue_id],
                |r| r.get(0),
            )
            .map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO issue_reopen_history(issue_id,ordinal,rationale,occurred_at) VALUES(?1,?2,?3,?4)",
            rusqlite::params![issue_id, reopen_ordinal, rationale.as_str(), occurred_millis],
        )
        .map_err(|_| storage_error(context))?;
    }

    let support_id = format!("issue-h2a-support-{}", approval.prepared_id().as_str());
    let support_ordinal: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(ordinal)+1,0) FROM issue_support_history WHERE issue_id=?1",
            [issue_id],
            |r| r.get(0),
        )
        .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO issue_support_history(issue_id,ordinal,support_id) VALUES(?1,?2,?3)",
        rusqlite::params![issue_id, support_ordinal, support_id],
    )
    .map_err(|_| storage_error(context))?;

    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'issue',?4,?5,'allowed','approved','succeeded','complete')",
        rusqlite::params![audit.id().as_str(), occurred_millis, audit.code().as_str(), issue_id, context.correlation_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    let effect_code = audit
        .actual_effects()
        .first()
        .ok_or_else(|| storage_error(context))?
        .as_str();
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete','issue',?3)",
        rusqlite::params![audit.id().as_str(), effect_code, issue_id],
    )
    .map_err(|_| storage_error(context))?;

    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![receipt_id.as_str(), context.idempotency_id.as_str(), occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;

    tx.execute(
        &format!(
            "INSERT INTO issue_h2a_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,issue_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'{}',?2,?3,'{}',?4,?5,?6)",
            kind.operation_str(),
            kind.result_kind(),
        ),
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            issue_id,
            approval.prepared_id().as_str(),
            receipt_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO issue_h2a_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        &format!(
            "INSERT INTO {}(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
            kind.command_table()
        ),
        rusqlite::params![
            context.idempotency_id.as_str(),
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn decode_v16_replay_audits(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<Vec<AuditEvent>, DomainError> {
    let rows = tx
        .prepare("SELECT audit.id,audit.occurred_at,audit.event_code,audit.target_id,audit.correlation_id FROM issue_h2a_execute_replay_audits replay JOIN audit_events audit ON audit.id=replay.audit_event_id WHERE replay.idempotency_id=?1 ORDER BY replay.ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([context.idempotency_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    rows.into_iter()
        .map(|(id, at, code, target_id, correlation_id)| {
            let code = AuditEventCode::parse(&code).map_err(|_| storage_error(context))?;
            let effect_code =
                AuditEffectCode::parse(code.as_str()).map_err(|_| storage_error(context))?;
            Ok(AuditEvent::new(
                AuditEventId::parse(id).map_err(|_| storage_error(context))?,
                UtcTimestamp::from_unix_millis(at),
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::Issue(
                        IssueId::parse(target_id).map_err(|_| storage_error(context))?,
                    ),
                ),
                pmc_domain::identity::CorrelationId::parse(correlation_id)
                    .map_err(|_| storage_error(context))?,
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

fn replay_issue_execute_outcome(
    tx: &Transaction<'_>,
    kind: IssueH2aExecuteKind,
    context: &IssueOperationContext,
) -> Result<IssueMutationOutcome, DomainError> {
    let (issue_id, receipt_id): (String, String) = tx
        .query_row(
            "SELECT issue_id,approval_receipt_id FROM issue_h2a_execute_replay_operations WHERE idempotency_id=?1 AND operation=?2",
            rusqlite::params![context.idempotency_id.as_str(), kind.operation_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let issue_id = IssueId::parse(issue_id).map_err(|_| storage_error(context))?;
    let record = decode_issue_h2a_record(tx, &issue_id, context)?;
    let audit_events = decode_v16_replay_audits(tx, context)?;
    Ok(IssueMutationOutcome {
        record,
        audit_events,
        approval_receipt_id: Some(
            ApprovalReceiptId::parse(receipt_id).map_err(|_| storage_error(context))?,
        ),
    })
}

fn next_issue_h2a_lower_classification_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM issue_h2a_lower_classification_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_issue_h2a_lower_classification_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM issue_h2a_lower_classification_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Persists the `prepared_intents` row for one Issue classification-lowering
/// preview. Unlike `persist_prepared_intent` (shared by resolve/close/
/// reopen), this operation needs no `support_witnesses` row, so `support_id`
/// is left NULL rather than routed through `support_id_for`.
fn persist_lower_issue_classification_prepared(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &IssueOperationContext,
) -> Result<(), DomainError> {
    let expires_at = prepared.preview().expires_at().unix_millis();
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,expires_at,created_at) VALUES (?1,?2,'lower_issue_classification',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',NULL,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(prepared.preview().contract_version()),
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            expires_at,
            expires_at - pmc_domain::work_management::WORK_MANAGEMENT_H2A_TTL_MILLIS,
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// See `decode_outstanding_prepared_intent` -- same "rebuild and verify the
/// exact stored digest" technique, but simpler: no support to reconstruct,
/// and the command's own scalars come from this operation's own fresh-root
/// table rather than the shared `prepared_work_management_payloads`.
fn decode_lower_issue_classification_prepared(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    record: &IssueRecord,
    context: &IssueOperationContext,
) -> Result<WorkManagementPreparedIntent, DomainError> {
    let intent = tx
        .query_row(
            "SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND intent_kind='lower_issue_classification' AND consumed_at IS NULL",
            [prepared_id.as_str()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .ok_or_else(|| transition_conflict(context))?;
    if intent.4.is_some() {
        return Err(storage_error(context));
    }
    let command = tx
        .query_row(
            "SELECT command.proposed_classification,command.rationale FROM issue_h2a_lower_classification_prepare_replay_operations replay JOIN issue_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.result_reference=?1",
            [prepared_id.as_str()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let proposed_classification =
        DataClassification::from_persisted(&command.0).map_err(|_| storage_error(context))?;
    let rationale =
        WorkManagementRationale::parse(command.1).map_err(|_| storage_error(context))?;
    let operation = WorkManagementOperation::LowerIssueClassification {
        issue_id: record.id().clone(),
        issue_version: record.version(),
        current_classification: record.classification(),
        proposed_classification,
        rationale,
    };
    let rebuilt = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        record.classification(),
        None,
        UtcTimestamp::from_unix_millis(intent.3),
    )
    .map_err(|_| storage_error(context))?;
    if rebuilt.payload_digest().as_str() != intent.0
        || rebuilt.classification().as_persisted() != intent.1
        || rebuilt.preview().expires_at().unix_millis() != intent.2
    {
        return Err(storage_error(context));
    }
    Ok(rebuilt)
}

/// See `decode_v16_replay_audits` -- identical shape, pointed at this
/// operation's own execute-side replay-audit table.
fn decode_lower_issue_classification_replay_audits(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<Vec<AuditEvent>, DomainError> {
    let rows = tx
        .prepare("SELECT audit.id,audit.occurred_at,audit.event_code,audit.target_id,audit.correlation_id FROM issue_h2a_lower_classification_execute_replay_audits replay JOIN audit_events audit ON audit.id=replay.audit_event_id WHERE replay.idempotency_id=?1 ORDER BY replay.ordinal")
        .map_err(|_| storage_error(context))?
        .query_map([context.idempotency_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| storage_error(context))?;
    rows.into_iter()
        .map(|(id, at, code, target_id, correlation_id)| {
            let code = AuditEventCode::parse(&code).map_err(|_| storage_error(context))?;
            let effect_code =
                AuditEffectCode::parse(code.as_str()).map_err(|_| storage_error(context))?;
            Ok(AuditEvent::new(
                AuditEventId::parse(id).map_err(|_| storage_error(context))?,
                UtcTimestamp::from_unix_millis(at),
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::Issue(
                        IssueId::parse(target_id).map_err(|_| storage_error(context))?,
                    ),
                ),
                pmc_domain::identity::CorrelationId::parse(correlation_id)
                    .map_err(|_| storage_error(context))?,
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

fn replay_lowered_issue_outcome(
    tx: &Transaction<'_>,
    context: &IssueOperationContext,
) -> Result<IssueMutationOutcome, DomainError> {
    let (issue_id, receipt_id): (String, String) = tx
        .query_row(
            "SELECT issue_id,approval_receipt_id FROM issue_h2a_lower_classification_execute_replay_operations WHERE idempotency_id=?1 AND operation='execute_lower_classification'",
            [context.idempotency_id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let issue_id = IssueId::parse(issue_id).map_err(|_| storage_error(context))?;
    let record = decode_issue_h2a_record(tx, &issue_id, context)?;
    let audit_events = decode_lower_issue_classification_replay_audits(tx, context)?;
    Ok(IssueMutationOutcome {
        record,
        audit_events,
        approval_receipt_id: Some(
            ApprovalReceiptId::parse(receipt_id).map_err(|_| storage_error(context))?,
        ),
    })
}

/// Persists one successful Issue classification-lowering execution. Mirrors
/// `persist_issue_execute_bundle`'s shape, but touches only `aggregate_
/// registry` (version + classification) -- `issues.state`/resolution/
/// evidence/support-history are all untouched by this operation.
fn persist_lowered_issue_bundle(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    outcome: &IssueMutationOutcome,
    ordinal: i64,
    context: &IssueOperationContext,
) -> Result<(), DomainError> {
    let issue_id = outcome.record.id().as_str();
    let audit = outcome
        .audit_events
        .first()
        .ok_or_else(|| storage_error(context))?;
    let occurred_millis = audit.occurred_at().unix_millis();
    let receipt_id = outcome
        .approval_receipt_id
        .as_ref()
        .ok_or_else(|| storage_error(context))?;

    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='issue'",
        rusqlite::params![
            i64::try_from(outcome.record.version().get()).map_err(|_| storage_error(context))?,
            outcome.record.classification().as_persisted(),
            occurred_millis,
            issue_id
        ],
    )
    .map_err(|_| storage_error(context))?;

    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'issue',?4,?5,'allowed','approved','succeeded','complete')",
        rusqlite::params![audit.id().as_str(), occurred_millis, audit.code().as_str(), issue_id, context.correlation_id.as_str()],
    ).map_err(|_| storage_error(context))?;
    let effect_code = audit
        .actual_effects()
        .first()
        .ok_or_else(|| storage_error(context))?
        .as_str();
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete','issue',?3)",
        rusqlite::params![audit.id().as_str(), effect_code, issue_id],
    )
    .map_err(|_| storage_error(context))?;

    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![receipt_id.as_str(), context.idempotency_id.as_str(), occurred_millis, approval.prepared_id().as_str()],
    )
    .map_err(|_| storage_error(context))?;

    tx.execute(
        "INSERT INTO issue_h2a_lower_classification_execute_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,issue_id,prepared_intent_id,approval_receipt_id) VALUES(?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, issue_id, approval.prepared_id().as_str(), receipt_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO issue_h2a_lower_classification_execute_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO issue_h2a_lower_classification_command_executes(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES(?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), approval.prepared_id().as_str(), approval.acknowledged_payload_digest().as_str()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}
