//! Action SQLite persistence.
//!
//! Supports the full H1 Action Request lifecycle (create/submit/decline/
//! withdraw), H2a Accept and Lower Data Classification, and
//! Cancel (the first of Cancel/Complete/Reopen to gain SQLite persistence).
//! It exposes a typed read/write boundary without exposing SQL, rows, or a
//! generic persistence API. Complete and Reopen -- and any other Action
//! shape not covered above -- remain fail-closed until their own lossless
//! decoders are implemented.

use pmc_domain::{
    actions::{
        AcceptedActionOutcome, ActionDetails, ActionEvidenceAuthorityError,
        ActionEvidenceAuthorityPort, ActionExecutionPolicy, ActionExecutionPolicyPort,
        ActionMutationOutcome, ActionOperationContext, ActionPersistenceCommand,
        ActionPersistenceDecodeInput, ActionPersistenceH3DenialCause, ActionPersistenceResult,
        ActionPersistenceSnapshot, ActionPersistenceTerminalCause, ActionPersistenceTransitionKind,
        ActionRecord, ActionReplayCapsule, ActionRequestRecord, ActionServiceIdSource, ActionTitle,
        ActionTransitionRecord, ApproveAndExecuteAcceptActionRequest,
        ApproveAndExecuteCancelAction, ApproveAndExecuteCompleteAction,
        ApproveAndExecuteLowerActionClassification, ApproveAndExecuteReopenAction,
        CreateActionRequestDraft, DeclineActionRequest, DenyActionEvidenceAuthority,
        InMemoryActionService, LinkActionCompletionEvidence, PrepareAcceptActionRequest,
        PrepareCancelAction, PrepareCompleteAction, PrepareLowerActionClassification,
        PrepareReopenAction, PreparedDisposition, RejectActionPreparedIntent, StartAction,
        SubmitActionRequest, WithdrawActionRequest,
    },
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
        AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey, MessageParam, SafeErrorExtension, SafeParamValue},
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, DecisionId, EvidenceReferenceId, IdempotencyId, PreparedIntentId,
        StakeholderId,
    },
    time::{Clock, UtcTimestamp},
    work_management::{
        prepared_intent_rejection_audit, ActionReopenMode, ActionRequestState, ActionState,
        ApprovalAuthorizationPort, DecisionResultingActionRequest, EvidenceClassificationBinding,
        EvidenceOrJudgment, EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification,
        IntegrityDigest, RejectedPreparedIntentOutcome, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
        ACTION_PREPARED_REJECTED_AUDIT_CODE, WORK_MANAGEMENT_H2A_TTL_MILLIS,
    },
};
use rusqlite::{
    Error as SqliteError, ErrorCode as SqliteErrorCode, OptionalExtension, Transaction,
};
use sha2::{Digest, Sha256};

use super::{LedgerOpenError, LedgerTransactionError, SqliteProductLedger, CURRENT_SCHEMA_VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionPersistenceLoadError {
    UnsupportedSchema { found: u32 },
    StorageUnavailable,
    ActionNamespaceNotEmpty,
    InvalidActionSnapshot,
}

impl From<LedgerOpenError> for ActionPersistenceLoadError {
    fn from(error: LedgerOpenError) -> Self {
        match error {
            LedgerOpenError::FutureSchema { found }
            | LedgerOpenError::UnsupportedSchema { found } => Self::UnsupportedSchema { found },
            LedgerOpenError::StorageUnavailable
            | LedgerOpenError::Busy
            | LedgerOpenError::CorruptDatabase
            | LedgerOpenError::InvalidMetadata
            | LedgerOpenError::PolicyViolation
            | LedgerOpenError::UnclaimedDatabase
            | LedgerOpenError::WrongApplication => Self::StorageUnavailable,
        }
    }
}

#[derive(Clone, Copy)]
struct PersistedAcceptClock(UtcTimestamp);

impl Clock for PersistedAcceptClock {
    fn now(&self) -> UtcTimestamp {
        self.0
    }
}

struct PersistedAcceptIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_ids: std::array::IntoIter<AuditEventId, 3>,
}

/// v45 rejection mints exactly one audit event and nothing else.
struct PersistedRejectIds {
    audit_id: Option<AuditEventId>,
}

impl ActionServiceIdSource for PersistedRejectIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_id.take().ok_or_else(missing_persisted_id_error)
    }
}

/// Rejection never consults the execution policy (nothing is executed), so
/// the rehydrated service is given a policy that cannot deny -- and cannot
/// be mistaken for the desktop's real one, which lives in `pmc-application`.
struct PersistedRejectPolicy;

impl ActionExecutionPolicyPort for PersistedRejectPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Allowed
    }
}

/// H2a "Lower Data Classification" for Action -- mirrors
/// `PersistedAcceptIds`, but this operation mints exactly one audit event
/// rather than three (see `approve_and_execute_lower_action_classification`'s
/// doc comment on `actions.rs`).
struct PersistedLowerActionClassificationIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_id: Option<AuditEventId>,
}

impl ActionServiceIdSource for PersistedLowerActionClassificationIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.receipt_id
            .take()
            .ok_or_else(missing_persisted_id_error)
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_id.take().ok_or_else(missing_persisted_id_error)
    }
}

/// Deterministically derives the `AuditEventId` a PREPARE-time H3 denial's
/// own audit event mints. `PersistedActionTransitionPrepareIds`
/// has no caller-supplied audit id to hand out for this -- unlike Accept/
/// Lower's caller-supplied-canonical PREPARE, or EXECUTE's own
/// `PersistedActionTransitionExecuteIds`, a denial isn't an outcome any
/// caller anticipated the way a successful PREPARE's own `prepared_intent_id`
/// is, so nothing upstream can supply one. Derived from `prepared_intent_id`
/// (already guaranteed unique per PREPARE attempt) via the same digest-
/// derivation technique `decisions.rs`'s `action_context` helper already
/// established in this codebase, truncated to fit `AuditEventId`'s 64-char
/// max (unlike `IdempotencyId`'s larger limit that helper relies on).
fn derive_prepare_h3_denial_audit_id(prepared_intent_id: &PreparedIntentId) -> AuditEventId {
    let mut hasher = Sha256::new();
    hasher.update(prepared_intent_id.as_str().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    AuditEventId::parse(format!("h3d-{}", &digest[..48])).unwrap_or_else(|_| {
        unreachable!("a short fixed prefix plus 48 lowercase hex chars always fits AuditEventId")
    })
}

/// Cancel/Complete/Reopen's PREPARE step -- unlike Accept/Lower
/// Classification's caller-supplied-canonical pattern, this rehydrates the
/// real domain service and calls its public `prepare_*_action` method
/// directly, because `authoritative_action_snapshot` (evidence resolution +
/// classification combination) is a genuine domain-logic dependency that
/// must not be reimplemented here. That means the SERVICE mints its own
/// `PreparedIntentId` via `IdSource`, unlike Accept/Lower's prepare which
/// never touches an `IdSource` at all.
struct PersistedActionTransitionPrepareIds {
    prepared_intent_id: Option<PreparedIntentId>,
    /// An H3 denial's own `record_h3_denial` (domain layer)
    /// needs a fresh `AuditEventId` too -- see `derive_prepare_h3_denial_audit_id`.
    /// Always populated at construction (deterministic, no caller input
    /// needed), so `next_audit_event_id` below can hand it out exactly once.
    h3_denial_audit_id: Option<AuditEventId>,
}

impl ActionServiceIdSource for PersistedActionTransitionPrepareIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        self.prepared_intent_id
            .take()
            .ok_or_else(missing_persisted_id_error)
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.h3_denial_audit_id
            .take()
            .ok_or_else(missing_persisted_id_error)
    }
}

/// Cancel/Complete/Reopen's EXECUTE step (`execute_h2`) mints exactly one
/// receipt and one audit event -- same shape as
/// `PersistedLowerActionClassificationIds`, kept as its own type since it is
/// shared across all three transition kinds rather than owned by one.
struct PersistedActionTransitionExecuteIds {
    receipt_id: Option<ApprovalReceiptId>,
    audit_id: Option<AuditEventId>,
}

impl ActionServiceIdSource for PersistedActionTransitionExecuteIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.receipt_id
            .take()
            .ok_or_else(missing_persisted_id_error)
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_id.take().ok_or_else(missing_persisted_id_error)
    }
}

/// Stand-in `ApprovalAuthorizationPort`/`ActionExecutionPolicyPort` for
/// PREPARE-only rehydration. `prepare_cancel_action`/`prepare_complete_action`/
/// `prepare_reopen_action` never call `authorize`/`current_policy` (only
/// EXECUTE's `self.validate` does) -- these exist purely to satisfy
/// `InMemoryActionService`'s generic bounds and must never actually be
/// reached; both deny/deny defensively in case that assumption ever breaks.
#[derive(Clone, Copy)]
struct UnreachableActionExecutionPorts;

impl ApprovalAuthorizationPort for UnreachableActionExecutionPorts {
    fn authorize(&self, _: AuditActor) -> bool {
        false
    }
}

impl ActionExecutionPolicyPort for UnreachableActionExecutionPorts {
    fn current_policy(&self, _: &WorkManagementOperation) -> ActionExecutionPolicy {
        ActionExecutionPolicy::Denied
    }
}

/// Real, SQLite-backed `ActionEvidenceAuthorityPort` -- a pre-resolved cache,
/// mirroring `PersistedDecisionEvidenceAuthority` in `decision_repository.rs`
/// exactly. `ActionEvidenceAuthorityPort::resolve` takes no transaction
/// handle, so it cannot run a live query; callers build this once (via
/// `persisted_action_evidence_authority` below) from the target Action's
/// already-linked completion evidence before rehydrating
/// `InMemoryActionService`, then hand the cache to `rehydrate`.
///
/// 2026-09 regression fix: `prepare_cancel_action`/`prepare_reopen_action`
/// and their EXECUTE counterparts used to rehydrate with
/// `DenyActionEvidenceAuthority`, which was safe only because
/// `action_completion_evidence` durably held zero rows before
/// `LinkActionCompletionEvidence`'s own SQLite writer shipped -- both
/// `prepare_cancel_action_cause`/`prepare_reopen_action_cause` and the
/// shared `execute_h2` unconditionally call `authoritative_action_snapshot`,
/// which resolves every linked evidence id regardless of Cancel/Reopen's own
/// business logic, so any Action with linked completion evidence made those
/// four call sites hard-fail the moment that changed.
struct PersistedActionEvidenceAuthority {
    evidence: Vec<EvidenceReferenceMetadata>,
}

impl ActionEvidenceAuthorityPort for PersistedActionEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, ActionEvidenceAuthorityError> {
        self.evidence
            .iter()
            .find(|item| item.id() == id)
            .cloned()
            .ok_or(ActionEvidenceAuthorityError::NotFound)
    }
}

/// Builds `PersistedActionEvidenceAuthority` for exactly the target Action's
/// own currently-linked completion evidence (scoped to the target Action,
/// not every Action in the snapshot, and read with one fixed join by
/// `action_id` rather than a dynamic `IN` list). Role is
/// always `ActionCompletion`, derived from the `action_completion_evidence`
/// association itself (Direction C) -- the legacy, nullable
/// `evidence_references.role` column is never inspected. Verification is
/// read faithfully from the real persisted shape (not a placeholder):
/// `EvidenceReferenceMetadata` is documented as authoritative, a future
/// Complete PREPARE/EXECUTE will need real verification data through this
/// same adapter, and loading the true shape surfaces a malformed Evidence
/// row as a persistence error instead of silently legitimizing it.
fn persisted_action_evidence_authority(
    tx: &Transaction<'_>,
    action_id: &ActionId,
    context: &ActionOperationContext,
) -> Result<PersistedActionEvidenceAuthority, DomainError> {
    let rows = tx
        .prepare("SELECT ev.id,reg.classification,ev.verification,ev.integrity_digest,ev.last_verified_at,reg.version FROM action_completion_evidence ace JOIN evidence_references ev ON ev.id=ace.evidence_id JOIN aggregate_registry reg ON reg.id=ev.id AND reg.aggregate_type='evidence_reference' WHERE ace.action_id=?1")
        .map_err(|_| persistence_error(context))?
        .query_map([action_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|_| persistence_error(context))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| persistence_error(context))?;
    let evidence = rows
        .into_iter()
        .map(
            |(id, classification, verification, integrity_digest, last_verified_at, version)| {
                let id = EvidenceReferenceId::parse(id).map_err(|_| persistence_error(context))?;
                let source_version = AggregateVersion::new(
                    u64::try_from(version).map_err(|_| persistence_error(context))?,
                )
                .map_err(|_| persistence_error(context))?;
                let classification = DataClassification::from_persisted(&classification)
                    .map_err(|_| persistence_error(context))?;
                let verification = match (verification.as_str(), last_verified_at, integrity_digest)
                {
                    ("verified", Some(at), Some(digest)) => EvidenceVerification::Verified {
                        verified_at: UtcTimestamp::from_unix_millis(at),
                        integrity_digest: IntegrityDigest::parse(digest)
                            .map_err(|_| persistence_error(context))?,
                    },
                    ("observed_unpinned", Some(at), Some(digest)) => {
                        EvidenceVerification::ObservedUnpinned {
                            observed_at: UtcTimestamp::from_unix_millis(at),
                            integrity_digest: IntegrityDigest::parse(digest)
                                .map_err(|_| persistence_error(context))?,
                        }
                    }
                    ("degraded_last_verified", Some(at), Some(digest)) => {
                        EvidenceVerification::DegradedLastVerified {
                            last_verified_at: UtcTimestamp::from_unix_millis(at),
                            integrity_digest: IntegrityDigest::parse(digest)
                                .map_err(|_| persistence_error(context))?,
                        }
                    }
                    ("unverified", None, None) => EvidenceVerification::Unverified,
                    ("integrity_mismatch", None, None) => EvidenceVerification::IntegrityMismatch,
                    _ => return Err(persistence_error(context)),
                };
                Ok(EvidenceReferenceMetadata::new(
                    id,
                    source_version,
                    classification,
                    EvidenceRole::ActionCompletion,
                    verification,
                ))
            },
        )
        .collect::<Result<Vec<_>, DomainError>>()?;
    Ok(PersistedActionEvidenceAuthority { evidence })
}

enum PersistedAcceptExecution {
    Accepted(Box<AcceptedActionOutcome>),
    Terminal(DomainError),
}

/// Cancel/Complete/Reopen's EXECUTE outcome wrapper -- same "commit the
/// persisted terminal capsule from inside the closure, then translate back
/// to `Err` for the caller" shape as `PersistedAcceptExecution`, just typed
/// around `ActionMutationOutcome<ActionRecord>` instead of
/// `AcceptedActionOutcome`.
enum PersistedActionTransitionExecution {
    Executed(Box<ActionMutationOutcome<ActionRecord>>),
    Terminal(DomainError),
}

/// Cancel/Reopen's PREPARE outcome wrapper -- same "commit the persisted
/// capsule from inside the closure, then translate back to `Err` for the
/// caller" shape as `PersistedActionTransitionExecution`. Without this, a
/// PREPARE that hits an H3 denial would `return Err(error)` directly from
/// inside `with_immediate_transaction`'s closure, and `with_immediate_transaction`
/// only commits on `Ok` -- silently rolling back the H3-denied Terminal
/// capsule this same call just persisted (found by independent review).
enum PersistedActionTransitionPrepare {
    Prepared(Box<WorkManagementPreparedIntent>),
    Denied(Box<DomainError>),
}

enum DecodedExecuteAccept {
    Accepted(Box<AcceptedActionOutcome>, Box<ActionReplayCapsule>),
    Terminal(Box<ActionReplayCapsule>, AuditEvent, PreparedDisposition),
}

fn missing_persisted_id_error() -> pmc_domain::DomainValueError {
    match ActionId::parse("") {
        Err(error) => error,
        Ok(_) => unreachable!("empty identifiers must be rejected by the domain"),
    }
}

impl ActionServiceIdSource for PersistedAcceptIds {
    fn next_action_id(&mut self) -> Result<ActionId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_prepared_intent_id(
        &mut self,
    ) -> Result<PreparedIntentId, pmc_domain::DomainValueError> {
        Err(missing_persisted_id_error())
    }

    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<ApprovalReceiptId, pmc_domain::DomainValueError> {
        self.receipt_id
            .take()
            .ok_or_else(missing_persisted_id_error)
    }

    fn next_audit_event_id(&mut self) -> Result<AuditEventId, pmc_domain::DomainValueError> {
        self.audit_ids.next().ok_or_else(missing_persisted_id_error)
    }
}

impl SqliteProductLedger {
    /// Load the Action namespace through a typed, validated domain seam.
    ///
    /// Decode the empty namespace or one exact normalized create-request draft.
    pub fn load_action_persistence_snapshot(
        &self,
    ) -> Result<ActionPersistenceSnapshot, ActionPersistenceLoadError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ActionPersistenceLoadError::UnsupportedSchema {
                found: self.schema_version,
            });
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| ActionPersistenceLoadError::StorageUnavailable)?;
        let request_count = count(&transaction, "SELECT count(*) FROM action_requests")?;
        let action_count = count(&transaction, "SELECT count(*) FROM actions")?;
        if request_count == 0
            && action_count == 0
            && action_namespace_is_empty(&transaction)
                .map_err(|_| ActionPersistenceLoadError::StorageUnavailable)?
        {
            let snapshot = empty_snapshot()?;
            transaction
                .commit()
                .map_err(|_| ActionPersistenceLoadError::StorageUnavailable)?;
            return Ok(snapshot);
        }
        let snapshot = decode_action_namespace(&transaction)?;
        transaction
            .commit()
            .map_err(|_| ActionPersistenceLoadError::StorageUnavailable)?;
        Ok(snapshot)
    }

    /// Durably create one canonical Action Request draft.
    ///
    /// The command is already expressed in the domain vocabulary and the
    /// caller supplies the injected audit identity/clock values used by the
    /// application composition root.  The write is deliberately limited to
    /// the H1 create shape; all rows are committed by the shared immediate
    /// transaction kernel before the outcome is returned.
    pub fn create_action_request_draft(
        &mut self,
        command: CreateActionRequestDraft,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let request = ActionRequestRecord::from_persisted_created_draft(
            command.id.clone(),
            command.title.clone(),
            command.details.clone(),
            command.intended_owner.clone(),
            command.response_due_at,
            command.intended_action_due_at,
            command.classification,
        );

        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "create_request" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT request_id,title,details,intended_owner_id,response_due_at,intended_action_due_at,classification FROM action_command_create_requests WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                            row.get::<_, Option<i64>>(5)?,
                            row.get::<_, String>(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_command_matches(&existing, &command) {
                    let snapshot = decode_action_namespace(tx)
                        .map_err(|_| persistence_error(&context))?;
                    return snapshot_create_outcome(snapshot, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let request_exists = tx
                .query_row(
                    "SELECT 1 FROM action_requests WHERE id=?1",
                    [request.id().as_str()],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
                .is_some();
            if request_exists {
                return Err(already_exists(&context));
            }

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;

            let code = AuditEventCode::parse("action_request.created")
                .map_err(|_| persistence_error(&context))?;
            let disposition = AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![AuditEffectCode::parse("action_request.created")
                    .map_err(|_| persistence_error(&context))?],
            )
            .map_err(|_| persistence_error(&context))?;
            let audit = AuditEvent::new(
                audit_event_id.clone(),
                occurred_at,
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::ActionRequest(request.id().clone()),
                ),
                context.correlation_id.clone(),
                disposition,
            );

            tx.execute(
                "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'action_request',1,?2,?3,?3)",
                rusqlite::params![
                    request.id().as_str(),
                    request.classification().as_persisted(),
                    occurred_at.unix_millis(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_requests (id,title,details,intended_owner_id,response_due_at,intended_action_due_at,state,terminal_rationale,linked_action_id,source_decision_id,superseded_premise) VALUES (?1,?2,?3,?4,?5,?6,'draft',NULL,NULL,NULL,0)",
                rusqlite::params![
                    request.id().as_str(),
                    request.title().as_str(),
                    request.details().as_str(),
                    request.intended_owner().map(StakeholderId::as_str),
                    request.response_due_at().map(|value| value.unix_millis()),
                    request
                        .intended_action_due_at()
                        .map(|value| value.unix_millis()),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'create_request',?2,?3,'request',?4,'not_applicable')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    request.id().as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_create_requests (idempotency_id,request_id,title,details,intended_owner_id,response_due_at,intended_action_due_at,classification) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    request.id().as_str(),
                    request.title().as_str(),
                    request.details().as_str(),
                    request.intended_owner().map(StakeholderId::as_str),
                    request.response_due_at().map(|value| value.unix_millis()),
                    request
                        .intended_action_due_at()
                        .map(|value| value.unix_millis()),
                    request.classification().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,?3,?4,?5,'action_request',?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(),
                    audit.module().as_persisted(),
                    audit.code().as_str(),
                    request.id().as_str(),
                    audit.correlation_id().as_str(),
                    audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(),
                    audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,?3,'action_request',?4)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.actual_effects()[0].as_str(),
                    audit.effect_scope().as_persisted(),
                    request.id().as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    audit.id().as_str(),
                    context.correlation_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            // Validate the complete post-write Action namespace while the
            // same IMMEDIATE transaction is still open.  Any malformed
            // pre-existing row or newly written value therefore aborts the
            // whole bundle before revision advancement/commit.
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;

            Ok(ActionMutationOutcome {
                record: request,
                audit_events: vec![audit],
                approval_receipt_id: None,
            })
        })
    }

    /// Durably submit one canonical Draft Action Request into Open.
    ///
    /// This is intentionally the only H1 transition exposed by this adapter.
    /// The complete Action namespace is decoded before and after the mutation,
    /// so malformed neighbouring rows fail closed without partial writes.
    pub fn submit_action_request(
        &mut self,
        command: SubmitActionRequest,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            // Idempotency is global across the Action namespace.  A create
            // key may never be reinterpreted as a submit key (or vice versa).
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "transition_request" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT request_id,expected_version,target_state,rationale FROM action_command_transition_requests WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    )),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.request_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == "open"
                    && existing.3.is_none()
                {
                    return snapshot_transition_outcome(before, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let Some(previous) = before
                .requests()
                .iter()
                .find(|request| request.id() == &command.request_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if previous.state() != ActionRequestState::Draft {
                return Err(request_transition_conflict(&context, &previous));
            }
            if previous.version() != command.expected_version {
                return Err(request_transition_conflict(&context, &previous));
            }

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            let code = AuditEventCode::parse("action_request.submitted")
                .map_err(|_| persistence_error(&context))?;
            let effect = AuditEffectCode::parse("action_request.submitted")
                .map_err(|_| persistence_error(&context))?;
            let disposition = AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![effect],
            )
            .map_err(|_| persistence_error(&context))?;
            let audit = AuditEvent::new(
                audit_event_id,
                occurred_at,
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::ActionRequest(command.request_id.clone()),
                ),
                context.correlation_id.clone(),
                disposition,
            );
            let next = ActionRequestRecord::from_persisted_submitted_open(
                previous.id().clone(),
                previous.title().clone(),
                previous.details().clone(),
                previous.intended_owner().cloned(),
                previous.response_due_at(),
                previous.intended_action_due_at(),
                previous.classification(),
            );
            tx.execute(
                "UPDATE action_requests SET state='open' WHERE id=?1",
                [command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=2,updated_at=?1 WHERE id=?2 AND aggregate_type='action_request'",
                rusqlite::params![occurred_at.unix_millis(), command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'transition_request',?2,?3,'request',?4,'not_applicable')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    command.request_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_transition_requests (idempotency_id,request_id,expected_version,target_state,rationale) VALUES (?1,?2,?3,'open',NULL)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.request_id.as_str(),
                    i64::try_from(command.expected_version.get()).unwrap_or(-1),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,?3,?4,?5,'action_request',?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    audit.id().as_str(), audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(), audit.module().as_persisted(),
                    audit.code().as_str(), command.request_id.as_str(),
                    audit.correlation_id().as_str(), audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(), audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,?3,'action_request',?4)",
                rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), audit.effect_scope().as_persisted(), command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction.advance_revision(expected_revision).map_err(|_| persistence_error(&context))?;
            Ok(ActionMutationOutcome { record: next, audit_events: vec![audit], approval_receipt_id: None })
        })
    }

    /// Durably retain one exact H2a PrepareAcceptActionRequest preview.
    ///
    /// The prepared intent is produced by the canonical domain service (and
    /// therefore already contains the generated Action identity).  This
    /// adapter only persists and rehydrates that typed value; it never
    /// accepts the request, consumes a receipt, or creates an Action row.
    pub fn prepare_accept_action_request(
        &mut self,
        command: PrepareAcceptActionRequest,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "prepare_accept" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT request_id,expected_version FROM action_command_prepare_accepts WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.request_id.as_str()
                    && existing.1
                        == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                {
                    let replayed = before
                        .replay()
                        .iter()
                        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                        .and_then(|capsule| match capsule.result() {
                            ActionPersistenceResult::Prepared(intent) => Some(intent.clone()),
                            _ => None,
                        })
                        .ok_or_else(|| persistence_error(&context))?;
                    if prepared != replayed {
                        return Err(idempotency_conflict(&context));
                    }
                    return Ok(replayed);
                }
                return Err(idempotency_conflict(&context));
            }

            let request = before
                .requests()
                .iter()
                .find(|request| request.id() == &command.request_id)
                .ok_or_else(|| not_found(&context))?;
            if request.state() != ActionRequestState::Open {
                return Err(request_transition_conflict(&context, request));
            }
            if request.version() != command.expected_version {
                return Err(request_transition_conflict(&context, request));
            }

            let canonical = canonical_prepare_accept(request, &command, &prepared)
                .map_err(|_| persistence_error(&context))?;
            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            let preview = canonical.preview();
            let operation = match canonical.operation() {
                WorkManagementOperation::AcceptActionRequest {
                    request_id,
                    request_version,
                    action_id,
                    action_classification,
                    action_subject,
                    commitment_details,
                    intended_owner,
                    intended_due_at,
                } => (
                    request_id,
                    request_version,
                    action_id,
                    action_classification,
                    action_subject,
                    commitment_details,
                    intended_owner,
                    intended_due_at,
                ),
                _ => return Err(persistence_error(&context)),
            };
            let created_at = preview
                .expires_at()
                .unix_millis()
                .checked_sub(WORK_MANAGEMENT_H2A_TTL_MILLIS)
                .ok_or_else(|| persistence_error(&context))?;
            let target = preview
                .targets()
                .first()
                .ok_or_else(|| persistence_error(&context))?;

            tx.execute(
                "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'accept_action_request',?3,?4,?5,?6,?7,NULL,NULL,?8,?9)",
                rusqlite::params![
                    canonical.id().as_str(),
                    i64::from(preview.contract_version()),
                    canonical.payload_digest().as_str(),
                    canonical.classification().as_persisted(),
                    "allowed",
                    "not_cancellable_after_submit",
                    "head_of_products",
                    preview.expires_at().unix_millis(),
                    created_at,
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            let (target_id, target_version) = match target {
                pmc_domain::work_management::WorkManagementTarget::ActionRequest(id, version) => {
                    (id.as_str(), version.get())
                }
                _ => return Err(persistence_error(&context)),
            };
            tx.execute(
                "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'action_request',?2,?3)",
                rusqlite::params![canonical.id().as_str(), target_id, i64::try_from(target_version).unwrap_or(-1)],
            )
            .map_err(|_| persistence_error(&context))?;
            for (ordinal, effect) in preview.effects().iter().enumerate() {
                let (code, target_type, target_id, second_type, second_id) = match effect {
                    pmc_domain::work_management::WorkManagementEffect::AcceptActionRequest(id) => (
                        "action_request.accepted", Some("action_request"), Some(id.as_str()), None, None,
                    ),
                    pmc_domain::work_management::WorkManagementEffect::CreateAction(id) => (
                        "action.created_from_request", Some("action"), Some(id.as_str()), None, None,
                    ),
                    pmc_domain::work_management::WorkManagementEffect::LinkActionRequestToAction(request_id, action_id) => (
                        "action_request.action_linked", Some("action_request"), Some(request_id.as_str()), Some("action"), Some(action_id.as_str()),
                    ),
                    _ => return Err(persistence_error(&context)),
                };
                tx.execute(
                    "INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    rusqlite::params![canonical.id().as_str(), i64::try_from(ordinal).unwrap_or(-1), code, target_type, target_id, second_type, second_id],
                )
                .map_err(|_| persistence_error(&context))?;
            }
            for (ordinal, source) in preview.classification_sources().iter().enumerate() {
                let (role, source_id) = match source.role() {
                    pmc_domain::work_management::WorkManagementClassificationSourceRole::PrimaryTarget => ("primary_target", None),
                    pmc_domain::work_management::WorkManagementClassificationSourceRole::CreatedAction => ("created_action", Some(operation.2.as_str())),
                    _ => return Err(persistence_error(&context)),
                };
                tx.execute(
                    "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,?2,?3,?4,?5)",
                    rusqlite::params![canonical.id().as_str(), i64::try_from(ordinal).unwrap_or(-1), role, source_id, source.classification().as_persisted()],
                )
                .map_err(|_| persistence_error(&context))?;
            }
            tx.execute(
                "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
                rusqlite::params![
                    canonical.id().as_str(),
                    operation.0.as_str(),
                    i64::try_from(operation.1.get()).unwrap_or(-1),
                    operation.2.as_str(),
                    operation.3.as_persisted(),
                    operation.4.as_str(),
                    operation.5.as_str(),
                    operation.6.as_str(),
                    operation.7.unix_millis(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'prepare_accept',?2,?3,'prepared',?4,'not_applicable')",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), operation_ordinal, canonical.id().as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_prepare_accepts (idempotency_id,request_id,expected_version) VALUES (?1,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), command.request_id.as_str(), i64::try_from(command.expected_version.get()).unwrap_or(-1)],
            )
            .map_err(|_| persistence_error(&context))?;

            let after = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            let result = after
                .replay()
                .iter()
                .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                .and_then(|capsule| match capsule.result() {
                    ActionPersistenceResult::Prepared(intent) => Some(intent.clone()),
                    _ => None,
                })
                .ok_or_else(|| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(result)
        })
    }

    /// Execute one explicit H2a approval against a durably retained preview.
    ///
    /// The canonical domain service revalidates the exact preview and mints
    /// the single-use receipt. SQLite receives only the resulting typed
    /// outcome inside the same immediate transaction.
    /// H2a rejection (v45): the Head of Products refuses a pending Accept/
    /// Complete/Cancel/Reopen preview. One immediate transaction: the domain
    /// service rejects (consuming the intent, minting one zero-effect
    /// audit), then `prepared_intents.consumed_at`, the audit row and the
    /// `action_reject_prepared_command_results` row are written together;
    /// the trigger on that table refuses a rejection that did not consume
    /// its intent at the same instant. The rejection's ordinal is drawn
    /// from the global Action operation stream. Replaying the same
    /// idempotency id returns the original outcome; reusing it for a
    /// different intent, or reusing an id any other operation claimed, is
    /// an idempotency conflict.
    pub fn reject_action_prepared_intent<Z>(
        &mut self,
        command: RejectActionPreparedIntent,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
        authorization: Z,
    ) -> Result<RejectedPreparedIntentOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            if let Some((stored_prepared, stored_actor)) = tx
                .query_row(
                    "SELECT prepared_intent_id,actor FROM action_reject_prepared_command_results WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if stored_prepared != command.prepared_id.as_str()
                    || stored_actor != command.actor.as_persisted()
                {
                    return Err(idempotency_conflict(&context));
                }
                return before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .and_then(|capsule| match capsule.result() {
                        ActionPersistenceResult::Rejected(outcome) => Some(outcome.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| persistence_error(&context));
            }
            let claimed: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ledger_idempotency_claims WHERE idempotency_id=?1)",
                    [context.idempotency_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            if claimed != 0 {
                return Err(idempotency_conflict(&context));
            }
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                PersistedRejectIds {
                    audit_id: Some(audit_event_id.clone()),
                },
                authorization,
                PersistedRejectPolicy,
                DenyActionEvidenceAuthority,
                before,
            );
            let outcome = service.reject_action_prepared_intent(command.clone())?;
            let after = service
                .persistence_snapshot()
                .map_err(|_| persistence_error(&context))?;
            let capsule = after
                .replay()
                .iter()
                .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                .ok_or_else(|| persistence_error(&context))?;
            persist_action_prepared_intent_rejection(
                tx,
                &context,
                capsule.operation_ordinal(),
                &outcome,
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(outcome)
        })
    }

    pub fn approve_and_execute_accept_action_request<Z, P>(
        &mut self,
        command: ApproveAndExecuteAcceptActionRequest,
        audit_event_ids: [AuditEventId; 3],
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
    ) -> Result<AcceptedActionOutcome, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: ActionExecutionPolicyPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "execute_accept" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                let stored = tx
                    .query_row(
                        "SELECT prepared_id,actor,acknowledged_digest FROM action_command_execute_accepts WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                    )
                    .map_err(|_| persistence_error(&context))?;
                if stored.0 != command.approval.prepared_id().as_str()
                    || stored.1 != command.approval.actor().as_persisted()
                    || stored.2 != command.approval.acknowledged_payload_digest().as_str()
                {
                    return Err(idempotency_conflict(&context));
                }
                return match replayed.result() {
                    ActionPersistenceResult::Accepted(outcome) => {
                        Ok(PersistedAcceptExecution::Accepted(Box::new(outcome.clone())))
                    }
                    ActionPersistenceResult::Terminal { error, .. } => Err(error.clone()),
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedAcceptIds {
                receipt_id: Some(approval_receipt_id.clone()),
                audit_ids: audit_event_ids.into_iter(),
            };
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                authorization,
                policy,
                DenyActionEvidenceAuthority,
                before,
            );
            let outcome = match service.approve_and_execute_accept_action_request(command.clone()) {
                Ok(outcome) => outcome,
                Err(error) => {
                    let after = service
                        .persistence_snapshot()
                        .map_err(|_| persistence_error(&context))?;
                    // v45: a refusal the domain recorded nothing for (the
                    // preview was already consumed by a rejection) has no
                    // terminal capsule to persist; refuse and roll back.
                    if !after
                        .replay()
                        .iter()
                        .any(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    {
                        return Err(error);
                    }
                    persist_execute_accept_terminal(
                        tx,
                        &after,
                        &context,
                        command.approval.prepared_id(),
                    )
                    .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    return Ok(PersistedAcceptExecution::Terminal(error));
                }
            };
            if outcome.audit_events.len() != 3 || outcome.approval_receipt_id != approval_receipt_id {
                return Err(persistence_error(&context));
            }
            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE action_requests SET state='accepted',linked_action_id=?1 WHERE id=?2 AND state='open'",
                rusqlite::params![outcome.action.id().as_str(), outcome.request.id().as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='action_request'",
                rusqlite::params![
                    i64::try_from(outcome.request.version().get()).unwrap_or(-1),
                    occurred_at.unix_millis(),
                    outcome.request.id().as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES (?1,'action',1,?2,?3,?3)",
                rusqlite::params![outcome.action.id().as_str(), outcome.action.classification().as_persisted(), occurred_at.unix_millis()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                // Decision-triggered Action provenance gap fix (2026-09):
                // `source_decision_id`/`superseded_premise` are no longer
                // hardcoded NULL/0 -- the domain's own accepted-Action
                // reconstruction (`ActionRecord::from_persisted_accepted_request*`)
                // already propagates both from the accepted Request, so an
                // Action accepted from a Decision-created (and possibly
                // Decision-marked) Request must carry them through, not
                // silently drop them at persist time.
                "INSERT INTO actions (id,source_request_id,title,details,owner_id,due_at,state,commitment_classification,support_id,transition_reason,source_decision_id,superseded_premise) VALUES (?1,?2,?3,?4,?5,?6,'open',?7,NULL,NULL,?8,?9)",
                rusqlite::params![
                    outcome.action.id().as_str(), outcome.action.source_request_id().as_str(),
                    outcome.action.title().as_str(), outcome.action.details().as_str(), outcome.action.owner().as_str(),
                    outcome.action.due_at().unix_millis(), outcome.request.classification().as_persisted(),
                    outcome.action.source_decision_id().map(|id| id.as_str()),
                    outcome.action.has_superseded_premise(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
                rusqlite::params![occurred_at.unix_millis(), command.approval.prepared_id().as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
                rusqlite::params![approval_receipt_id.as_str(), context.idempotency_id.as_str(), occurred_at.unix_millis(), command.approval.prepared_id().as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,prepared_disposition) VALUES (?1,'execute_accept',?2,?3,'accepted',?4,?5,'not_applicable')",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), operation_ordinal, outcome.action.id().as_str(), command.approval.prepared_id().as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_execute_accepts (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,'head_of_products',?3)",
                rusqlite::params![context.idempotency_id.as_str(), command.approval.prepared_id().as_str(), command.approval.acknowledged_payload_digest().as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            for (ordinal, audit) in outcome.audit_events.iter().enumerate() {
                let (target_type, target_id) = match audit.target() {
                    AuditTarget::ActionRequest(id) => ("action_request", id.as_str()),
                    AuditTarget::Action(id) => ("action", id.as_str()),
                    _ => return Err(persistence_error(&context)),
                };
                tx.execute(
                    "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')",
                    rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, context.correlation_id.as_str()],
                )
                .map_err(|_| persistence_error(&context))?;
                tx.execute(
                    "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete',?3,?4)",
                    rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_type, target_id],
                )
                .map_err(|_| persistence_error(&context))?;
                tx.execute(
                    "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![context.idempotency_id.as_str(), i64::try_from(ordinal).unwrap_or(-1), audit.id().as_str(), context.correlation_id.as_str()],
                )
                .map_err(|_| persistence_error(&context))?;
            }
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(PersistedAcceptExecution::Accepted(Box::new(outcome)))
        })?;
        match committed {
            PersistedAcceptExecution::Accepted(outcome) => Ok(*outcome),
            PersistedAcceptExecution::Terminal(error) => {
                Err(LedgerTransactionError::Operation(error))
            }
        }
    }

    /// H2a "Lower Data Classification" for Action -- step 1.
    /// Mirrors `prepare_accept_action_request`'s two-phase shape: the
    /// caller supplies an already-canonical `prepared` preview, and this
    /// method only validates it against the command and durably commits it.
    pub fn prepare_lower_action_classification(
        &mut self,
        command: PrepareLowerActionClassification,
        prepared: WorkManagementPreparedIntent,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_h2a_lower_classification_prepare_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "prepare_lower_classification" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT action_id,expected_version,proposed_classification,rationale FROM action_h2a_lower_classification_command_prepares WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.action_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.proposed_classification.as_persisted()
                    && existing.3 == command.rationale.as_str()
                {
                    let replayed = before
                        .replay()
                        .iter()
                        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                        .and_then(|capsule| match capsule.result() {
                            ActionPersistenceResult::Prepared(intent) => Some(intent.clone()),
                            _ => None,
                        })
                        .ok_or_else(|| persistence_error(&context))?;
                    if prepared != replayed {
                        return Err(idempotency_conflict(&context));
                    }
                    return Ok(replayed);
                }
                return Err(idempotency_conflict(&context));
            }

            let action = before
                .actions()
                .iter()
                .find(|action| action.id() == &command.action_id)
                .ok_or_else(|| not_found(&context))?;
            if action.version() != command.expected_version {
                return Err(action_transition_conflict(&context, action));
            }
            let operation_matches = matches!(
                prepared.operation(),
                WorkManagementOperation::LowerActionClassification {
                    action_id,
                    action_version,
                    current_classification,
                    proposed_classification,
                    rationale,
                } if action_id == &command.action_id
                    && *action_version == command.expected_version
                    && *current_classification == action.classification()
                    && *proposed_classification == command.proposed_classification
                    && rationale.as_str() == command.rationale.as_str()
            );
            if !operation_matches || prepared.classification() != action.classification() {
                return Err(persistence_error(&context));
            }

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            persist_lower_action_classification_prepared(tx, &prepared, &context)?;
            tx.execute(
                "INSERT INTO action_h2a_lower_classification_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_lower_classification',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_h2a_lower_classification_command_prepares (idempotency_id,action_id,expected_version,proposed_classification,rationale) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.action_id.as_str(),
                    i64::try_from(command.expected_version.get())
                        .map_err(|_| persistence_error(&context))?,
                    command.proposed_classification.as_persisted(),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;

            let after = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            let result = after
                .replay()
                .iter()
                .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                .and_then(|capsule| match capsule.result() {
                    ActionPersistenceResult::Prepared(intent) => Some(intent.clone()),
                    _ => None,
                })
                .ok_or_else(|| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(result)
        })
    }

    /// H2a "Lower Data Classification" for Action -- step 2.
    /// Mirrors `approve_and_execute_accept_action_request`'s rehydrate/
    /// execute/persist shape, but calls the plain (non-durable-terminal)
    /// domain method directly: per its own doc comment, Action's Lower
    /// operation never produces a durable `ActionPersistenceResult::Terminal`
    /// capsule, so a domain failure here is an ordinary rollback via `?`.
    pub fn approve_and_execute_lower_action_classification<Z, P>(
        &mut self,
        command: ApproveAndExecuteLowerActionClassification,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
    ) -> Result<ActionMutationOutcome<ActionRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: ActionExecutionPolicyPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_h2a_lower_classification_execute_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "execute_lower_classification" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                let stored = tx
                    .query_row(
                        "SELECT prepared_id,actor,acknowledged_digest FROM action_h2a_lower_classification_command_executes WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .map_err(|_| persistence_error(&context))?;
                if stored.0 != command.approval.prepared_id().as_str()
                    || stored.1 != command.approval.actor().as_persisted()
                    || stored.2 != command.approval.acknowledged_payload_digest().as_str()
                {
                    return Err(idempotency_conflict(&context));
                }
                return match replayed.result() {
                    ActionPersistenceResult::Action(outcome) => Ok(outcome.clone()),
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedLowerActionClassificationIds {
                receipt_id: Some(approval_receipt_id.clone()),
                audit_id: Some(audit_event_id.clone()),
            };
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                authorization,
                policy,
                DenyActionEvidenceAuthority,
                before,
            );
            let outcome =
                service.approve_and_execute_lower_action_classification(command.clone())?;
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
            {
                return Err(persistence_error(&context));
            }
            persist_lowered_action_bundle(tx, &command, &outcome, occurred_at, &context)?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(outcome)
        })
    }

    /// The last of Action's Cancel/Complete/Reopen trio to gain SQLite
    /// persistence -- unblocked only once both of its prerequisites
    /// (`StartAction`, `LinkActionCompletionEvidence`) shipped, since
    /// `prepare_complete_action_cause` requires `ActionState::InProgress`
    /// and non-empty `completion_evidence`. Same full-rehydrate shape as
    /// `prepare_cancel_action` (see its doc comment for why
    /// `authoritative_action_snapshot` -- and therefore a real
    /// `persisted_action_evidence_authority`, not
    /// `DenyActionEvidenceAuthority` -- is required here too), and the same
    /// `PersistedActionTransitionPrepare` wrapper for the same reason (an H3
    /// denial's Terminal capsule must not be silently rolled back by a bare
    /// `return Err(error)` inside `with_immediate_transaction`'s closure).
    ///
    /// Complete denies via H3 far more often in practice than Cancel/Reopen:
    /// `prepare_complete_action_cause` requires non-empty completion
    /// evidence, full resolution of every linked reference, and
    /// `EvidenceOrJudgment::evaluate_evidence_required()` to succeed (all
    /// evidence Verified, or judgment-backed if degraded) before it ever
    /// reaches the ordinary classification check Cancel/Reopen also run.
    /// Decoding a PREPARE-time H3 denial is still not implemented (same gap
    /// as Cancel/Reopen's own, see `persist_prepare_cancel_h3_denied`'s doc
    /// comment) -- so today, any Complete PREPARE that denies still rolls
    /// the whole attempt back with a generic persistence error rather than a
    /// clean, replay-safe denial.
    pub fn prepare_complete_action(
        &mut self,
        command: PrepareCompleteAction,
        prepared_intent_id: PreparedIntentId,
        occurred_at: UtcTimestamp,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "prepare_complete" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                return match replayed.result() {
                    ActionPersistenceResult::Prepared(intent) => Ok(
                        PersistedActionTransitionPrepare::Prepared(Box::new(intent.clone())),
                    ),
                    ActionPersistenceResult::Terminal { error, .. } => Ok(
                        PersistedActionTransitionPrepare::Denied(Box::new(error.clone())),
                    ),
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedActionTransitionPrepareIds {
                h3_denial_audit_id: Some(derive_prepare_h3_denial_audit_id(&prepared_intent_id)),
                prepared_intent_id: Some(prepared_intent_id),
            };
            let evidence_authority =
                persisted_action_evidence_authority(tx, &command.action_id, &context)?;
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                UnreachableActionExecutionPorts,
                UnreachableActionExecutionPorts,
                evidence_authority,
                before,
            );
            let result = service.prepare_complete_action(command);
            let after = service
                .persistence_snapshot()
                .map_err(|_| persistence_error(&context))?;
            match result {
                Ok(prepared) => {
                    persist_prepare_complete_prepared(tx, &after, &context, &prepared)
                        .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    Ok(PersistedActionTransitionPrepare::Prepared(Box::new(
                        prepared,
                    )))
                }
                Err(error) => {
                    persist_prepare_complete_h3_denied(tx, &after, &context)
                        .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    Ok(PersistedActionTransitionPrepare::Denied(Box::new(error)))
                }
            }
        })?;
        match committed {
            PersistedActionTransitionPrepare::Prepared(prepared) => Ok(*prepared),
            PersistedActionTransitionPrepare::Denied(error) => {
                Err(LedgerTransactionError::Operation(*error))
            }
        }
    }

    /// Complete's EXECUTE step -- mirrors
    /// `approve_and_execute_cancel_action`'s full-rehydrate shape exactly
    /// (same shared `execute_h2`, same `PersistedActionTransitionExecution`
    /// terminal-capsule-on-failure wrapper). Unlike Cancel/Reopen, the
    /// target Action id is not on the command at all; it comes from the
    /// approval's own prepared intent's `CompleteAction { action_id, .. }`
    /// operation, mirroring `approve_and_execute_cancel_action`'s own
    /// `execute_action_id` lookup.
    pub fn approve_and_execute_complete_action<Z, P>(
        &mut self,
        command: ApproveAndExecuteCompleteAction,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
    ) -> Result<ActionMutationOutcome<ActionRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: ActionExecutionPolicyPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "execute_action" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                let stored = tx
                    .query_row(
                        "SELECT kind,prepared_id,actor,acknowledged_digest FROM action_command_execute_actions WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
                    )
                    .map_err(|_| persistence_error(&context))?;
                if stored.0 != "complete"
                    || stored.1 != command.approval.prepared_id().as_str()
                    || stored.2 != command.approval.actor().as_persisted()
                    || stored.3 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                return match replayed.result() {
                    ActionPersistenceResult::Action(outcome) => Ok(
                        PersistedActionTransitionExecution::Executed(Box::new(outcome.clone())),
                    ),
                    ActionPersistenceResult::Terminal { error, .. } => {
                        Ok(PersistedActionTransitionExecution::Terminal(error.clone()))
                    }
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedActionTransitionExecuteIds {
                receipt_id: Some(approval_receipt_id.clone()),
                audit_id: Some(audit_event_id.clone()),
            };
            let execute_action_id = before
                .prepared()
                .iter()
                .find(|intent| intent.id() == command.approval.prepared_id())
                .and_then(|intent| match intent.operation() {
                    WorkManagementOperation::CompleteAction { action_id, .. } => {
                        Some(action_id.clone())
                    }
                    _ => None,
                });
            // A preview that is no longer pending (executed, discarded or
            // rejected) names no Action to read Evidence for. The service is
            // still run, over an empty authority, so the refusal is the
            // domain's own (preview expired or changed), not a persistence
            // error dressed as one.
            let evidence_authority = match &execute_action_id {
                Some(action_id) => persisted_action_evidence_authority(tx, action_id, &context)?,
                None => PersistedActionEvidenceAuthority {
                    evidence: Vec::new(),
                },
            };
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                authorization,
                policy,
                evidence_authority,
                before,
            );
            let outcome = match service.approve_and_execute_complete_action(command.clone()) {
                Ok(outcome) => outcome,
                Err(error) => {
                    let after = service
                        .persistence_snapshot()
                        .map_err(|_| persistence_error(&context))?;
                    // v45: a refusal the domain recorded nothing for (the
                    // preview was already consumed by an execution or a
                    // rejection) has no terminal capsule to persist; refuse
                    // and roll back.
                    if !after
                        .replay()
                        .iter()
                        .any(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    {
                        return Err(error);
                    }
                    persist_execute_action_transition_terminal(
                        tx,
                        &after,
                        &context,
                        command.approval.prepared_id(),
                    )
                    .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    return Ok(PersistedActionTransitionExecution::Terminal(error));
                }
            };
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
            {
                return Err(persistence_error(&context));
            }
            persist_execute_action_transition_success(
                tx,
                &command.approval,
                "complete",
                &outcome,
                occurred_at,
                &context,
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(PersistedActionTransitionExecution::Executed(Box::new(
                outcome,
            )))
        })?;
        match committed {
            PersistedActionTransitionExecution::Executed(outcome) => Ok(*outcome),
            PersistedActionTransitionExecution::Terminal(error) => {
                Err(LedgerTransactionError::Operation(error))
            }
        }
    }

    /// The first of Action's remaining three lifecycle transitions with no
    /// SQLite persistence -- Cancel, Complete, Reopen -- to gain one.
    ///
    /// Unlike Accept/Lower Classification's PREPARE (caller supplies an
    /// already-canonical preview; this repository only cross-checks and
    /// persists it), Cancel's PREPARE fully rehydrates the real domain
    /// service and calls its public `prepare_cancel_action` directly. This
    /// is deliberate: `prepare_cancel_action_cause` depends on
    /// `authoritative_action_snapshot`, which resolves the action's
    /// completion evidence and combines its classification -- genuine
    /// domain logic this repository must not reimplement independently.
    /// (2026-09 regression, fixed) `DenyActionEvidenceAuthority` used to be
    /// passed here, reasoned safe because `action_completion_evidence`
    /// durably held zero rows before `LinkActionCompletionEvidence`'s own
    /// SQLite writer shipped. It stopped being safe the moment that changed
    /// -- any Action with linked completion evidence made
    /// `authoritative_action_snapshot` hard-fail resolving it. Now uses
    /// `persisted_action_evidence_authority`, a real pre-resolved cache
    /// (mirroring `PersistedDecisionEvidenceAuthority` in
    /// `decision_repository.rs`).
    ///
    /// The closure returns `PersistedActionTransitionPrepare` rather than
    /// `Result<WorkManagementPreparedIntent, DomainError>` directly (found by
    /// independent review): `with_immediate_transaction` only commits when
    /// its closure returns `Ok`, so an H3 denial that `return Err(error)`ed
    /// straight out of the closure would silently roll back the Terminal
    /// capsule `persist_prepare_cancel_h3_denied` had just written in the
    /// same transaction. Note this does not by itself make an H3 denial
    /// durable today: the subsequent `decode_action_namespace` re-check
    /// still fails closed on it (decoding a PREPARE-time H3 denial is not
    /// implemented -- see `persist_prepare_cancel_h3_denied`'s doc comment),
    /// so the whole attempt still rolls back, just via that same intentional
    /// safety check instead of a structural code-flow bug. This is a
    /// separate, still-open gap (tracked, not fixed by the evidence-authority
    /// repair above): an Action with an Unclassified linked evidence
    /// reference (permitted by `LinkActionCompletionEvidence`'s own writer)
    /// can still legitimately hit `ClassificationUnresolved` here and roll
    /// back with a generic persistence error instead of a clean H3 denial.
    pub fn prepare_cancel_action(
        &mut self,
        command: PrepareCancelAction,
        prepared_intent_id: PreparedIntentId,
        occurred_at: UtcTimestamp,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "prepare_cancel" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                return match replayed.result() {
                    ActionPersistenceResult::Prepared(intent) => Ok(
                        PersistedActionTransitionPrepare::Prepared(Box::new(intent.clone())),
                    ),
                    ActionPersistenceResult::Terminal { error, .. } => Ok(
                        PersistedActionTransitionPrepare::Denied(Box::new(error.clone())),
                    ),
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedActionTransitionPrepareIds {
                h3_denial_audit_id: Some(derive_prepare_h3_denial_audit_id(&prepared_intent_id)),
                prepared_intent_id: Some(prepared_intent_id),
            };
            let evidence_authority =
                persisted_action_evidence_authority(tx, &command.action_id, &context)?;
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                UnreachableActionExecutionPorts,
                UnreachableActionExecutionPorts,
                evidence_authority,
                before,
            );
            let result = service.prepare_cancel_action(command);
            let after = service
                .persistence_snapshot()
                .map_err(|_| persistence_error(&context))?;
            match result {
                Ok(prepared) => {
                    persist_prepare_cancel_prepared(tx, &after, &context, &prepared)
                        .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    Ok(PersistedActionTransitionPrepare::Prepared(Box::new(
                        prepared,
                    )))
                }
                Err(error) => {
                    persist_prepare_cancel_h3_denied(tx, &after, &context)
                        .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    Ok(PersistedActionTransitionPrepare::Denied(Box::new(error)))
                }
            }
        })?;
        match committed {
            PersistedActionTransitionPrepare::Prepared(prepared) => Ok(*prepared),
            PersistedActionTransitionPrepare::Denied(error) => {
                Err(LedgerTransactionError::Operation(*error))
            }
        }
    }

    /// Cancel's EXECUTE step -- mirrors
    /// `approve_and_execute_accept_action_request`'s full-rehydrate shape
    /// exactly, including persisting a durable Terminal capsule on failure
    /// (unlike Lower Classification's EXECUTE, which never produces one; see
    /// that method's doc comment). `execute_h2` mints exactly one receipt and
    /// one audit event, like Lower Classification's EXECUTE.
    pub fn approve_and_execute_cancel_action<Z, P>(
        &mut self,
        command: ApproveAndExecuteCancelAction,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
    ) -> Result<ActionMutationOutcome<ActionRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: ActionExecutionPolicyPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "execute_action" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                let stored = tx
                    .query_row(
                        "SELECT kind,prepared_id,actor,acknowledged_digest FROM action_command_execute_actions WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
                    )
                    .map_err(|_| persistence_error(&context))?;
                if stored.0 != "cancel"
                    || stored.1 != command.approval.prepared_id().as_str()
                    || stored.2 != command.approval.actor().as_persisted()
                    || stored.3 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                return match replayed.result() {
                    ActionPersistenceResult::Action(outcome) => Ok(
                        PersistedActionTransitionExecution::Executed(Box::new(outcome.clone())),
                    ),
                    ActionPersistenceResult::Terminal { error, .. } => {
                        Ok(PersistedActionTransitionExecution::Terminal(error.clone()))
                    }
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedActionTransitionExecuteIds {
                receipt_id: Some(approval_receipt_id.clone()),
                audit_id: Some(audit_event_id.clone()),
            };
            let execute_action_id = before
                .prepared()
                .iter()
                .find(|intent| intent.id() == command.approval.prepared_id())
                .and_then(|intent| match intent.operation() {
                    WorkManagementOperation::CancelAction { action_id, .. } => {
                        Some(action_id.clone())
                    }
                    _ => None,
                });
            // A preview that is no longer pending (executed, discarded or
            // rejected) names no Action to read Evidence for. The service is
            // still run, over an empty authority, so the refusal is the
            // domain's own (preview expired or changed), not a persistence
            // error dressed as one.
            let evidence_authority = match &execute_action_id {
                Some(action_id) => persisted_action_evidence_authority(tx, action_id, &context)?,
                None => PersistedActionEvidenceAuthority {
                    evidence: Vec::new(),
                },
            };
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                authorization,
                policy,
                evidence_authority,
                before,
            );
            let outcome = match service.approve_and_execute_cancel_action(command.clone()) {
                Ok(outcome) => outcome,
                Err(error) => {
                    let after = service
                        .persistence_snapshot()
                        .map_err(|_| persistence_error(&context))?;
                    // v45: a refusal the domain recorded nothing for (the
                    // preview was already consumed by an execution or a
                    // rejection) has no terminal capsule to persist; refuse
                    // and roll back.
                    if !after
                        .replay()
                        .iter()
                        .any(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    {
                        return Err(error);
                    }
                    persist_execute_action_transition_terminal(
                        tx,
                        &after,
                        &context,
                        command.approval.prepared_id(),
                    )
                    .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    return Ok(PersistedActionTransitionExecution::Terminal(error));
                }
            };
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
            {
                return Err(persistence_error(&context));
            }
            persist_execute_action_transition_success(
                tx,
                &command.approval,
                "cancel",
                &outcome,
                occurred_at,
                &context,
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(PersistedActionTransitionExecution::Executed(Box::new(
                outcome,
            )))
        })?;
        match committed {
            PersistedActionTransitionExecution::Executed(outcome) => Ok(*outcome),
            PersistedActionTransitionExecution::Terminal(error) => {
                Err(LedgerTransactionError::Operation(error))
            }
        }
    }

    /// Reopen -- the second of Action's remaining lifecycle transitions to
    /// gain SQLite persistence. Same shape as `prepare_cancel_action` (full
    /// rehydrate with a real `persisted_action_evidence_authority` -- see
    /// that method's own doc comment for the 2026-09 regression this fixed
    /// -- same `PersistedActionTransitionPrepare` wrapper and the same
    /// reasoning for why it's needed), with one additional wrinkle:
    /// `ActionReopenMode::RestartCancelled` is reachable today (Cancelled is
    /// a real durable state as of Cancel's own slice, and this mode is
    /// itself how `ActionState::InProgress` becomes reachable too -- see the
    /// dev journal's independent-review note correcting an earlier claim
    /// that `InProgress` needed `StartAction` first; it does not).
    /// `ActionReopenMode::ReopenCompleted` remains unreachable, since
    /// `ActionState::Completed` stays blocked until Complete's own
    /// persistence lands. Both modes are implemented uniformly here
    /// regardless -- the domain service and this repository make no
    /// distinction beyond the mode value itself -- so `ReopenCompleted` will
    /// work correctly the moment Complete unblocks, with no changes needed
    /// here.
    pub fn prepare_reopen_action(
        &mut self,
        command: PrepareReopenAction,
        prepared_intent_id: PreparedIntentId,
        occurred_at: UtcTimestamp,
    ) -> Result<WorkManagementPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "prepare_reopen" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                return match replayed.result() {
                    ActionPersistenceResult::Prepared(intent) => Ok(
                        PersistedActionTransitionPrepare::Prepared(Box::new(intent.clone())),
                    ),
                    ActionPersistenceResult::Terminal { error, .. } => Ok(
                        PersistedActionTransitionPrepare::Denied(Box::new(error.clone())),
                    ),
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedActionTransitionPrepareIds {
                h3_denial_audit_id: Some(derive_prepare_h3_denial_audit_id(&prepared_intent_id)),
                prepared_intent_id: Some(prepared_intent_id),
            };
            let evidence_authority =
                persisted_action_evidence_authority(tx, &command.action_id, &context)?;
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                UnreachableActionExecutionPorts,
                UnreachableActionExecutionPorts,
                evidence_authority,
                before,
            );
            let result = service.prepare_reopen_action(command);
            let after = service
                .persistence_snapshot()
                .map_err(|_| persistence_error(&context))?;
            match result {
                Ok(prepared) => {
                    persist_prepare_reopen_prepared(tx, &after, &context, &prepared)
                        .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    Ok(PersistedActionTransitionPrepare::Prepared(Box::new(
                        prepared,
                    )))
                }
                Err(error) => {
                    persist_prepare_reopen_h3_denied(tx, &after, &context)
                        .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    Ok(PersistedActionTransitionPrepare::Denied(Box::new(error)))
                }
            }
        })?;
        match committed {
            PersistedActionTransitionPrepare::Prepared(prepared) => Ok(*prepared),
            PersistedActionTransitionPrepare::Denied(error) => {
                Err(LedgerTransactionError::Operation(*error))
            }
        }
    }

    /// Reopen's EXECUTE step -- mirrors `approve_and_execute_cancel_action`
    /// exactly, generalized `persist_execute_action_transition_success`/
    /// `_terminal` calls already handle both kinds without modification.
    pub fn approve_and_execute_reopen_action<Z, P>(
        &mut self,
        command: ApproveAndExecuteReopenAction,
        audit_event_id: AuditEventId,
        approval_receipt_id: ApprovalReceiptId,
        occurred_at: UtcTimestamp,
        authorization: Z,
        policy: P,
    ) -> Result<ActionMutationOutcome<ActionRecord>, LedgerTransactionError<DomainError>>
    where
        Z: ApprovalAuthorizationPort,
        P: ActionExecutionPolicyPort,
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        let committed = self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "execute_action" {
                    return Err(idempotency_conflict(&context));
                }
                let replayed = before
                    .replay()
                    .iter()
                    .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    .ok_or_else(|| persistence_error(&context))?;
                let stored = tx
                    .query_row(
                        "SELECT kind,prepared_id,actor,acknowledged_digest FROM action_command_execute_actions WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
                    )
                    .map_err(|_| persistence_error(&context))?;
                if stored.0 != "reopen"
                    || stored.1 != command.approval.prepared_id().as_str()
                    || stored.2 != command.approval.actor().as_persisted()
                    || stored.3 != command.approval.acknowledged_payload_digest().as_str()
                    || command.approval.idempotency_id() != &context.idempotency_id
                {
                    return Err(idempotency_conflict(&context));
                }
                return match replayed.result() {
                    ActionPersistenceResult::Action(outcome) => Ok(
                        PersistedActionTransitionExecution::Executed(Box::new(outcome.clone())),
                    ),
                    ActionPersistenceResult::Terminal { error, .. } => {
                        Ok(PersistedActionTransitionExecution::Terminal(error.clone()))
                    }
                    _ => Err(persistence_error(&context)),
                };
            }

            let ids = PersistedActionTransitionExecuteIds {
                receipt_id: Some(approval_receipt_id.clone()),
                audit_id: Some(audit_event_id.clone()),
            };
            let execute_action_id = before
                .prepared()
                .iter()
                .find(|intent| intent.id() == command.approval.prepared_id())
                .and_then(|intent| match intent.operation() {
                    WorkManagementOperation::ReopenAction { action_id, .. } => {
                        Some(action_id.clone())
                    }
                    _ => None,
                });
            // A preview that is no longer pending (executed, discarded or
            // rejected) names no Action to read Evidence for. The service is
            // still run, over an empty authority, so the refusal is the
            // domain's own (preview expired or changed), not a persistence
            // error dressed as one.
            let evidence_authority = match &execute_action_id {
                Some(action_id) => persisted_action_evidence_authority(tx, action_id, &context)?,
                None => PersistedActionEvidenceAuthority {
                    evidence: Vec::new(),
                },
            };
            let mut service = InMemoryActionService::rehydrate(
                PersistedAcceptClock(occurred_at),
                ids,
                authorization,
                policy,
                evidence_authority,
                before,
            );
            let outcome = match service.approve_and_execute_reopen_action(command.clone()) {
                Ok(outcome) => outcome,
                Err(error) => {
                    let after = service
                        .persistence_snapshot()
                        .map_err(|_| persistence_error(&context))?;
                    // v45: a refusal the domain recorded nothing for (the
                    // preview was already consumed by an execution or a
                    // rejection) has no terminal capsule to persist; refuse
                    // and roll back.
                    if !after
                        .replay()
                        .iter()
                        .any(|capsule| capsule.idempotency_id() == &context.idempotency_id)
                    {
                        return Err(error);
                    }
                    persist_execute_action_transition_terminal(
                        tx,
                        &after,
                        &context,
                        command.approval.prepared_id(),
                    )
                    .map_err(|_| persistence_error(&context))?;
                    decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    transaction
                        .advance_revision(expected_revision)
                        .map_err(|_| persistence_error(&context))?;
                    return Ok(PersistedActionTransitionExecution::Terminal(error));
                }
            };
            if outcome.audit_events.len() != 1
                || outcome.approval_receipt_id != Some(approval_receipt_id.clone())
            {
                return Err(persistence_error(&context));
            }
            persist_execute_action_transition_success(
                tx,
                &command.approval,
                "reopen",
                &outcome,
                occurred_at,
                &context,
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(PersistedActionTransitionExecution::Executed(Box::new(
                outcome,
            )))
        })?;
        match committed {
            PersistedActionTransitionExecution::Executed(outcome) => Ok(*outcome),
            PersistedActionTransitionExecution::Terminal(error) => {
                Err(LedgerTransactionError::Operation(error))
            }
        }
    }

    /// Durably decline one canonical Open Action Request.
    ///
    /// This is the H1 terminal transition only: Open/version 2 becomes
    /// Declined/version 3 and carries a required terminal rationale.  The
    /// entire Action namespace is decoded before and after the write while an
    /// IMMEDIATE transaction is open, so malformed neighbouring state and
    /// late audit collisions fail closed without a partial bundle.
    pub fn decline_action_request(
        &mut self,
        command: DeclineActionRequest,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            // Idempotency is global across Action operations.  A key used for
            // Create/Submit (or any other operation) cannot be reinterpreted
            // as Decline, while an exact Decline replay is safe.
            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "transition_request" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT request_id,expected_version,target_state,rationale FROM action_command_transition_requests WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.request_id.as_str()
                    && existing.1
                        == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == "declined"
                    && existing.3.as_deref() == Some(command.rationale.as_str())
                {
                    let snapshot = decode_action_namespace(tx)
                        .map_err(|_| persistence_error(&context))?;
                    return snapshot_transition_outcome(snapshot, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let Some(previous) = before
                .requests()
                .iter()
                .find(|request| request.id() == &command.request_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if previous.state() != ActionRequestState::Open {
                return Err(request_transition_conflict(&context, &previous));
            }
            if previous.version() != command.expected_version {
                return Err(request_transition_conflict(&context, &previous));
            }

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            let code = AuditEventCode::parse("action_request.declined")
                .map_err(|_| persistence_error(&context))?;
            let effect = AuditEffectCode::parse("action_request.declined")
                .map_err(|_| persistence_error(&context))?;
            let disposition = AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![effect],
            )
            .map_err(|_| persistence_error(&context))?;
            let audit = AuditEvent::new(
                audit_event_id,
                occurred_at,
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::ActionRequest(command.request_id.clone()),
                ),
                context.correlation_id.clone(),
                disposition,
            );
            let next = ActionRequestRecord::from_persisted_open_to_declined(
                previous.clone(),
                command.rationale.clone(),
            )
            .ok_or_else(|| persistence_error(&context))?;
            tx.execute(
                "UPDATE action_requests SET state='declined',terminal_rationale=?1 WHERE id=?2",
                rusqlite::params![command.rationale.as_str(), command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=3,updated_at=?1 WHERE id=?2 AND aggregate_type='action_request'",
                rusqlite::params![occurred_at.unix_millis(), command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'transition_request',?2,?3,'request',?4,'not_applicable')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    command.request_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_transition_requests (idempotency_id,request_id,expected_version,target_state,rationale) VALUES (?1,?2,?3,'declined',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.request_id.as_str(),
                    i64::try_from(command.expected_version.get()).unwrap_or(-1),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,?3,?4,?5,'action_request',?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(),
                    audit.module().as_persisted(),
                    audit.code().as_str(),
                    command.request_id.as_str(),
                    audit.correlation_id().as_str(),
                    audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(),
                    audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,?3,'action_request',?4)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.actual_effects()[0].as_str(),
                    audit.effect_scope().as_persisted(),
                    command.request_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    audit.id().as_str(),
                    context.correlation_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(ActionMutationOutcome {
                record: next,
                audit_events: vec![audit],
                approval_receipt_id: None,
            })
        })
    }

    /// Durably withdraw one canonical Open Action Request.
    ///
    /// This is the independent H1 terminal transition only: Open/version 2
    /// becomes Withdrawn/version 3 and carries a required terminal rationale.
    /// The full Action namespace is decoded before and after the write while
    /// the IMMEDIATE transaction is open, so malformed neighbouring rows and
    /// late audit collisions fail closed without a partial bundle.
    pub fn withdraw_action_request(
        &mut self,
        command: WithdrawActionRequest,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRequestRecord>, LedgerTransactionError<DomainError>>
    {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "transition_request" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT request_id,expected_version,target_state,rationale FROM action_command_transition_requests WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.request_id.as_str()
                    && existing.1
                        == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == "withdrawn"
                    && existing.3.as_deref() == Some(command.rationale.as_str())
                {
                    let snapshot = decode_action_namespace(tx)
                        .map_err(|_| persistence_error(&context))?;
                    return snapshot_transition_outcome(snapshot, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let Some(previous) = before
                .requests()
                .iter()
                .find(|request| request.id() == &command.request_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if previous.state() != ActionRequestState::Open {
                return Err(request_transition_conflict(&context, &previous));
            }
            if previous.version() != command.expected_version {
                return Err(request_transition_conflict(&context, &previous));
            }

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            let code = AuditEventCode::parse("action_request.withdrawn")
                .map_err(|_| persistence_error(&context))?;
            let effect = AuditEffectCode::parse("action_request.withdrawn")
                .map_err(|_| persistence_error(&context))?;
            let disposition = AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![effect],
            )
            .map_err(|_| persistence_error(&context))?;
            let audit = AuditEvent::new(
                audit_event_id,
                occurred_at,
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::ActionRequest(command.request_id.clone()),
                ),
                context.correlation_id.clone(),
                disposition,
            );
            let next = ActionRequestRecord::from_persisted_open_to_withdrawn(
                previous.clone(),
                command.rationale.clone(),
            )
            .ok_or_else(|| persistence_error(&context))?;
            tx.execute(
                "UPDATE action_requests SET state='withdrawn',terminal_rationale=?1 WHERE id=?2",
                rusqlite::params![command.rationale.as_str(), command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=3,updated_at=?1 WHERE id=?2 AND aggregate_type='action_request'",
                rusqlite::params![occurred_at.unix_millis(), command.request_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'transition_request',?2,?3,'request',?4,'not_applicable')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    command.request_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_transition_requests (idempotency_id,request_id,expected_version,target_state,rationale) VALUES (?1,?2,?3,'withdrawn',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.request_id.as_str(),
                    i64::try_from(command.expected_version.get()).unwrap_or(-1),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,?3,?4,?5,'action_request',?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(),
                    audit.module().as_persisted(),
                    audit.code().as_str(),
                    command.request_id.as_str(),
                    audit.correlation_id().as_str(),
                    audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(),
                    audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,?3,'action_request',?4)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.actual_effects()[0].as_str(),
                    audit.effect_scope().as_persisted(),
                    command.request_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    audit.id().as_str(),
                    context.correlation_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(ActionMutationOutcome {
                record: next,
                audit_events: vec![audit],
                approval_receipt_id: None,
            })
        })
    }

    /// Persist one `StartAction` transition (Open -> InProgress) through the
    /// typed H1 seam. Mirrors `withdraw_action_request`'s exact ordinary,
    /// single-shot write shape -- `start_action_cause` mutates the domain
    /// service directly with no PREPARE/EXECUTE two-phase, so this bypasses
    /// `InMemoryActionService` entirely too, reconstructing the persisted
    /// record straight from `ActionRecord::from_persisted_started` the same
    /// way `ActionRequestRecord::from_persisted_open_to_withdrawn` already
    /// does for its own H1 sibling.
    pub fn start_action(
        &mut self,
        command: StartAction,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "start_action" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT action_id,expected_version FROM action_command_start_actions WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.action_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                {
                    let snapshot =
                        decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    return snapshot_action_outcome(snapshot, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let Some(previous) = before
                .actions()
                .iter()
                .find(|action| action.id() == &command.action_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if previous.state() != ActionState::Open
                || previous.version() != command.expected_version
            {
                return Err(action_transition_conflict(&context, &previous));
            }

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            let code = AuditEventCode::parse("action.started")
                .map_err(|_| persistence_error(&context))?;
            let effect = AuditEffectCode::parse("action.started")
                .map_err(|_| persistence_error(&context))?;
            let disposition = AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![effect],
            )
            .map_err(|_| persistence_error(&context))?;
            let audit = AuditEvent::new(
                audit_event_id,
                occurred_at,
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::Action(command.action_id.clone()),
                ),
                context.correlation_id.clone(),
                disposition,
            );
            let next = ActionRecord::from_persisted_started(
                previous.clone(),
                command.expected_version,
                occurred_at,
            )
            .ok_or_else(|| persistence_error(&context))?;

            tx.execute(
                "UPDATE actions SET state='in_progress' WHERE id=?1",
                [command.action_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='action'",
                rusqlite::params![
                    i64::try_from(next.version().get()).map_err(|_| persistence_error(&context))?,
                    occurred_at.unix_millis(),
                    command.action_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_transitions (action_id,ordinal,from_state,to_state,reason,occurred_at,support_id,approval_receipt_id) VALUES (?1,0,'open','in_progress',NULL,?2,NULL,NULL)",
                rusqlite::params![command.action_id.as_str(), occurred_at.unix_millis()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'start_action',?2,?3,'action',?4,'not_applicable')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    command.action_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_start_actions (idempotency_id,action_id,expected_version) VALUES (?1,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.action_id.as_str(),
                    i64::try_from(command.expected_version.get()).unwrap_or(-1),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,?3,?4,?5,'action',?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(),
                    audit.module().as_persisted(),
                    audit.code().as_str(),
                    command.action_id.as_str(),
                    audit.correlation_id().as_str(),
                    audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(),
                    audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,?3,'action',?4)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.actual_effects()[0].as_str(),
                    audit.effect_scope().as_persisted(),
                    command.action_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    audit.id().as_str(),
                    context.correlation_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(ActionMutationOutcome {
                record: next,
                audit_events: vec![audit],
                approval_receipt_id: None,
            })
        })
    }

    /// Persist one `LinkActionCompletionEvidence` through the typed H1 seam.
    /// Mirrors `start_action`'s ordinary, single-shot write shape exactly --
    /// bypasses `InMemoryActionService` entirely, reconstructing the
    /// persisted record straight from
    /// `ActionRecord::from_persisted_completion_evidence_linked`.
    ///
    /// Design: unlike the domain's own
    /// `link_action_completion_evidence_cause`, this adapter does NOT check
    /// any pre-existing `EvidenceRole` tag on the resolved evidence -- the
    /// generalized Evidence/Vault subsystem has no real path to
    /// ever stamp `evidence_references.role='action_completion'` outside a
    /// test fixture (see the journal entry this landed with). Instead, the
    /// typed lifecycle association this writer creates --
    /// `action_completion_evidence(action_id,evidence_id)`, already present
    /// in the immutable V1 schema -- *is* the `ActionCompletion` role
    /// assignment: any resolvable Evidence Reference is eligible to become
    /// one Action's completion evidence via this specific command. This
    /// only requires the referenced Evidence Reference to durably exist
    /// (via `aggregate_registry`); it does not require it to be Verified
    /// (Complete's own future PREPARE evaluates verification at that later
    /// point, matching the domain's existing split between linking and
    /// completing).
    pub fn link_action_completion_evidence(
        &mut self,
        command: LinkActionCompletionEvidence,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<ActionMutationOutcome<ActionRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(persistence_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let before = decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;

            if let Some(existing_operation) = tx
                .query_row(
                    "SELECT operation FROM action_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing_operation != "link_completion_evidence" {
                    return Err(idempotency_conflict(&context));
                }
            }

            if let Some(existing) = tx
                .query_row(
                    "SELECT action_id,expected_version,evidence_id,evidence_classification FROM action_command_link_completion_evidence WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| persistence_error(&context))?
            {
                if existing.0 == command.action_id.as_str()
                    && existing.1 == i64::try_from(command.expected_version.get()).unwrap_or(-1)
                    && existing.2 == command.evidence_id.as_str()
                {
                    let snapshot =
                        decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
                    return snapshot_action_outcome(snapshot, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let Some(previous) = before
                .actions()
                .iter()
                .find(|action| action.id() == &command.action_id)
                .cloned()
            else {
                return Err(not_found(&context));
            };
            if previous.state() != ActionState::InProgress
                || previous.version() != command.expected_version
            {
                return Err(action_transition_conflict(&context, &previous));
            }
            if previous
                .completion_evidence()
                .iter()
                .any(|id| id == &command.evidence_id)
            {
                return Err(completion_evidence_already_linked(&context));
            }

            let evidence_classification: Option<String> = tx
                .query_row(
                    "SELECT classification FROM aggregate_registry WHERE id=?1 AND aggregate_type='evidence_reference'",
                    [command.evidence_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| persistence_error(&context))?;
            let Some(evidence_classification) = evidence_classification else {
                return Err(evidence_not_found(&context));
            };
            let evidence_classification = DataClassification::from_persisted(&evidence_classification)
                .map_err(|_| persistence_error(&context))?;

            let operation_ordinal: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| persistence_error(&context))?;
            let code = AuditEventCode::parse("action.completion_evidence_linked")
                .map_err(|_| persistence_error(&context))?;
            let effect = AuditEffectCode::parse("action.completion_evidence_linked")
                .map_err(|_| persistence_error(&context))?;
            let disposition = AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![effect],
            )
            .map_err(|_| persistence_error(&context))?;
            let audit = AuditEvent::new(
                audit_event_id,
                occurred_at,
                AuditActor::HeadOfProducts,
                AuditAction::new(
                    AuditModule::WorkManagement,
                    code,
                    AuditTarget::Action(command.action_id.clone()),
                ),
                context.correlation_id.clone(),
                disposition,
            );
            let next = ActionRecord::from_persisted_completion_evidence_linked(
                previous.clone(),
                command.expected_version,
                command.evidence_id.clone(),
                evidence_classification,
            )
            .ok_or_else(|| persistence_error(&context))?;

            tx.execute(
                "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='action'",
                rusqlite::params![
                    i64::try_from(next.version().get()).map_err(|_| persistence_error(&context))?,
                    next.classification().as_persisted(),
                    occurred_at.unix_millis(),
                    command.action_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_completion_evidence (action_id,evidence_id) VALUES (?1,?2)",
                rusqlite::params![command.action_id.as_str(), command.evidence_id.as_str()],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'link_completion_evidence',?2,?3,'action',?4,'not_applicable')",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    operation_ordinal,
                    command.action_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_command_link_completion_evidence (idempotency_id,action_id,expected_version,evidence_id,evidence_classification) VALUES (?1,?2,?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.action_id.as_str(),
                    i64::try_from(command.expected_version.get()).unwrap_or(-1),
                    command.evidence_id.as_str(),
                    evidence_classification.as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,?3,?4,?5,'action',?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.occurred_at().unix_millis(),
                    audit.actor().as_persisted(),
                    audit.module().as_persisted(),
                    audit.code().as_str(),
                    command.action_id.as_str(),
                    audit.correlation_id().as_str(),
                    audit.policy_outcome().as_persisted(),
                    audit.approval_outcome().as_persisted(),
                    audit.execution_outcome().as_persisted(),
                    audit.effect_scope().as_persisted(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,?3,'action',?4)",
                rusqlite::params![
                    audit.id().as_str(),
                    audit.actual_effects()[0].as_str(),
                    audit.effect_scope().as_persisted(),
                    command.action_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            tx.execute(
                "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    audit.id().as_str(),
                    context.correlation_id.as_str(),
                ],
            )
            .map_err(|_| persistence_error(&context))?;
            decode_action_namespace(tx).map_err(|_| persistence_error(&context))?;
            transaction
                .advance_revision(expected_revision)
                .map_err(|_| persistence_error(&context))?;
            Ok(ActionMutationOutcome {
                record: next,
                audit_events: vec![audit],
                approval_receipt_id: None,
            })
        })
    }
}

fn snapshot_action_outcome(
    snapshot: ActionPersistenceSnapshot,
    context: &ActionOperationContext,
) -> Result<ActionMutationOutcome<ActionRecord>, DomainError> {
    match snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .map(ActionReplayCapsule::result)
    {
        Some(ActionPersistenceResult::Action(outcome)) => Ok(outcome.clone()),
        _ => Err(persistence_error(context)),
    }
}

fn snapshot_create_outcome(
    snapshot: ActionPersistenceSnapshot,
    context: &ActionOperationContext,
) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
    match snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .map(ActionReplayCapsule::result)
    {
        Some(ActionPersistenceResult::Request(outcome)) => Ok(outcome.clone()),
        _ => Err(persistence_error(context)),
    }
}

fn snapshot_transition_outcome(
    snapshot: ActionPersistenceSnapshot,
    context: &ActionOperationContext,
) -> Result<ActionMutationOutcome<ActionRequestRecord>, DomainError> {
    match snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .map(ActionReplayCapsule::result)
    {
        Some(ActionPersistenceResult::Request(outcome)) => Ok(outcome.clone()),
        _ => Err(persistence_error(context)),
    }
}

fn existing_command_matches(
    existing: &(
        String,
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
        String,
    ),
    command: &CreateActionRequestDraft,
) -> bool {
    existing.0 == command.id.as_str()
        && existing.1 == command.title.as_str()
        && existing.2 == command.details.as_str()
        && existing.3.as_deref() == command.intended_owner.as_ref().map(StakeholderId::as_str)
        && existing.4 == command.response_due_at.map(|value| value.unix_millis())
        && existing.5
            == command
                .intended_action_due_at
                .map(|value| value.unix_millis())
        && existing.6 == command.classification.as_persisted()
}

fn canonical_prepare_accept(
    request: &ActionRequestRecord,
    command: &PrepareAcceptActionRequest,
    prepared: &WorkManagementPreparedIntent,
) -> Result<WorkManagementPreparedIntent, ()> {
    if request.state() != ActionRequestState::Open
        || request.version() != command.expected_version
        || request.intended_owner().is_none()
        || request.intended_action_due_at().is_none()
    {
        return Err(());
    }
    let action_id = match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            action_classification,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
        } if request_id == request.id()
            && *request_version == command.expected_version
            && *action_classification == request.classification()
            && action_subject == request.title()
            && commitment_details == request.details()
            && request.intended_owner() == Some(intended_owner)
            && request.intended_action_due_at() == Some(*intended_due_at) =>
        {
            action_id.clone()
        }
        _ => return Err(()),
    };
    if prepared.preview().support().is_some()
        || prepared.preview().policy()
            != pmc_domain::work_management::WorkManagementPolicyDecision::Allowed
        || prepared.preview().cancellation_policy()
            != pmc_domain::work_management::WorkManagementCancellationPolicy::NotCancellableAfterSubmit
        || prepared.preview().authority()
            != pmc_domain::work_management::WorkManagementAuthority::HeadOfProducts
    {
        return Err(());
    }
    let created_at = prepared
        .preview()
        .expires_at()
        .unix_millis()
        .checked_sub(WORK_MANAGEMENT_H2A_TTL_MILLIS)
        .filter(|value| *value >= 0)
        .map(UtcTimestamp::from_unix_millis)
        .ok_or(())?;
    let canonical = WorkManagementPreparedIntent::prepare(
        prepared.id().clone(),
        WorkManagementOperation::AcceptActionRequest {
            request_id: request.id().clone(),
            request_version: command.expected_version,
            action_id,
            action_classification: request.classification(),
            action_subject: request.title().clone(),
            commitment_details: request.details().clone(),
            intended_owner: request.intended_owner().cloned().ok_or(())?,
            intended_due_at: request.intended_action_due_at().ok_or(())?,
        },
        request.classification(),
        None,
        created_at,
    )
    .map_err(|_| ())?;
    if canonical != *prepared {
        return Err(());
    }
    Ok(canonical)
}

/// Persist one already-canonical H2a Action classification-lowering
/// preview. Mirrors `persist_lower_decision_prepared`'s shape: no
/// support/evidence, and its own fresh-root command table carries every
/// scalar this operation needs, so there is no
/// `prepared_work_management_payloads` row either.
fn persist_lower_action_classification_prepared(
    tx: &Transaction<'_>,
    prepared: &WorkManagementPreparedIntent,
    context: &ActionOperationContext,
) -> Result<(), DomainError> {
    let WorkManagementOperation::LowerActionClassification {
        action_id,
        action_version,
        ..
    } = prepared.operation()
    else {
        return Err(persistence_error(context));
    };
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'lower_action_classification',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(prepared.preview().contract_version()),
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            prepared.preview().expires_at().unix_millis(),
            prepared.preview().expires_at().unix_millis() - WORK_MANAGEMENT_H2A_TTL_MILLIS,
        ],
    )
    .map_err(|_| persistence_error(context))?;
    tx.execute(
        "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'action',?2,?3)",
        rusqlite::params![
            prepared.id().as_str(),
            action_id.as_str(),
            i64::try_from(action_version.get()).map_err(|_| persistence_error(context))?,
        ],
    )
    .map_err(|_| persistence_error(context))?;
    for (ordinal, effect) in prepared.effects().iter().enumerate() {
        let pmc_domain::work_management::WorkManagementEffect::LowerActionClassification(id) =
            effect
        else {
            return Err(persistence_error(context));
        };
        tx.execute(
            "INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,?2,'action.classification_lowered','action',?3,NULL,NULL)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| persistence_error(context))?,
                id.as_str(),
            ],
        )
        .map_err(|_| persistence_error(context))?;
    }
    Ok(())
}

/// Persist one successful Action classification-lowering execution.
/// Mirrors `persist_lowered_decision_bundle`'s shape: `aggregate_registry`
/// is an UPDATE (the action already exists), and `actions` itself is never
/// touched -- only `version`/`classification` change per
/// `approve_and_execute_lower_action_classification_cause`'s mutation
/// logic.
fn persist_lowered_action_bundle(
    tx: &Transaction<'_>,
    command: &ApproveAndExecuteLowerActionClassification,
    outcome: &ActionMutationOutcome<ActionRecord>,
    occurred_at: UtcTimestamp,
    context: &ActionOperationContext,
) -> Result<(), DomainError> {
    let prepared_id = command.approval.prepared_id().as_str();
    let ordinal: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
            [],
            |row| row.get(0),
        )
        .map_err(|_| persistence_error(context))?;
    if tx
        .execute(
            "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='action'",
            rusqlite::params![
                i64::try_from(outcome.record.version().get())
                    .map_err(|_| persistence_error(context))?,
                outcome.record.classification().as_persisted(),
                occurred_at.unix_millis(),
                outcome.record.id().as_str(),
            ],
        )
        .map_err(|_| persistence_error(context))?
        != 1
    {
        return Err(persistence_error(context));
    }
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_at.unix_millis(), prepared_id],
    )
    .map_err(|_| persistence_error(context))?;
    let approval_receipt_id = outcome
        .approval_receipt_id
        .clone()
        .ok_or_else(|| persistence_error(context))?;
    tx.execute(
        "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![
            approval_receipt_id.as_str(),
            context.idempotency_id.as_str(),
            occurred_at.unix_millis(),
            prepared_id
        ],
    )
    .map_err(|_| persistence_error(context))?;
    tx.execute(
        "INSERT INTO action_h2a_lower_classification_execute_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,action_id,prepared_intent_id,approval_receipt_id) VALUES (?1,'execute_lower_classification',?2,?3,'lowered',?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            outcome.record.id().as_str(),
            prepared_id,
            approval_receipt_id.as_str()
        ],
    )
    .map_err(|_| persistence_error(context))?;
    tx.execute(
        "INSERT INTO action_h2a_lower_classification_command_executes (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,'head_of_products',?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            prepared_id,
            command.approval.acknowledged_payload_digest().as_str()
        ],
    )
    .map_err(|_| persistence_error(context))?;
    for (index, audit) in outcome.audit_events.iter().enumerate() {
        let (target_type, target_id) = match audit.target() {
            AuditTarget::Action(id) => ("action", id.as_str()),
            _ => return Err(persistence_error(context)),
        };
        tx.execute("INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')", rusqlite::params![audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type, target_id, context.correlation_id.as_str()]).map_err(|_| persistence_error(context))?;
        tx.execute("INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete',?3,?4)", rusqlite::params![audit.id().as_str(), audit.actual_effects()[0].as_str(), target_type, target_id]).map_err(|_| persistence_error(context))?;
        tx.execute("INSERT INTO action_h2a_lower_classification_execute_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,?2,?3,?4)", rusqlite::params![context.idempotency_id.as_str(), i64::try_from(index).map_err(|_| persistence_error(context))?, audit.id().as_str(), context.correlation_id.as_str()]).map_err(|_| persistence_error(context))?;
    }
    Ok(())
}

/// Maps one `WorkManagementClassificationSource`'s role to its persisted
/// `(role, source_id)` pair for `prepared_intent_classification_sources`.
/// Shared by Complete (`PrimaryTarget`+`Evidence`+`HumanJudgment`, since its
/// PREPARE attaches a real `SupportWitness`) and Cancel/Reopen
/// (`PrimaryTarget`+`Evidence` only, never `HumanJudgment` -- their own
/// `SupportWitness` is always `None`). Any other role is unreachable for
/// these three operations and fails closed rather than silently mis-tagging.
fn classification_source_persisted_role(
    role: &pmc_domain::work_management::WorkManagementClassificationSourceRole,
) -> Result<(&'static str, Option<&str>), ()> {
    match role {
        pmc_domain::work_management::WorkManagementClassificationSourceRole::PrimaryTarget => {
            Ok(("primary_target", None))
        }
        pmc_domain::work_management::WorkManagementClassificationSourceRole::Evidence(id) => {
            Ok(("evidence", Some(id.as_str())))
        }
        pmc_domain::work_management::WorkManagementClassificationSourceRole::HumanJudgment => {
            Ok(("human_judgment", None))
        }
        _ => Err(()),
    }
}

/// Persists a successful Complete PREPARE result. Unlike Cancel/Reopen,
/// Complete's own preview always carries a real `SupportWitness`
/// (`prepare_complete_action_cause` always calls
/// `EvidenceOrJudgment::evaluate_evidence_required()`) -- this writes it
/// into the shared `support_witnesses`/`support_judgments`/`support_evidence`
/// tables (the SAME row `persist_execute_action_transition_success` later
/// references via `actions.support_id`, not a second witness minted at
/// EXECUTE time) plus a point-in-time `action_h2a_support_evidence_snapshots`
/// copy of every linked evidence reference (needed because
/// `evidence_references.verification` is mutable -- see
/// `V38_ACTION_H2A_COMPLETE_EVIDENCE_SNAPSHOT_SQL`'s own doc comment).
fn persist_prepare_complete_prepared(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
    prepared: &WorkManagementPreparedIntent,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let operation_ordinal = i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?;
    let preview = prepared.preview();
    let created_at = preview
        .expires_at()
        .unix_millis()
        .checked_sub(WORK_MANAGEMENT_H2A_TTL_MILLIS)
        .ok_or(())?;
    let (action_id, action_version) = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::CompleteAction {
            action_id,
            action_version,
        } => (action_id.as_str(), action_version.get()),
        _ => return Err(()),
    };
    let action_version = i64::try_from(action_version).unwrap_or(-1);

    let support = preview.support().ok_or(())?;
    let support_id = format!("action-h2a-support-{}", prepared.id().as_str());
    let disposition = match support.disposition() {
        pmc_domain::work_management::SupportDisposition::EvidenceSatisfied => "evidence_satisfied",
        pmc_domain::work_management::SupportDisposition::JudgmentSatisfied => "judgment_satisfied",
        pmc_domain::work_management::SupportDisposition::VerificationPending => {
            "verification_pending"
        }
    };
    tx.execute(
        "INSERT INTO support_witnesses (id,requirement,disposition,classification) VALUES (?1,'evidence_required',?2,?3)",
        rusqlite::params![support_id, disposition, support.classification().as_persisted()],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'complete_action',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',?5,NULL,?6,?7)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(preview.contract_version()),
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            support_id,
            preview.expires_at().unix_millis(),
            created_at,
        ],
    )
    .map_err(|_| ())?;
    for (ordinal, judgment) in support.judgments().iter().enumerate() {
        tx.execute(
            "INSERT INTO support_judgments (support_id,ordinal,actor,disposition,rationale,classification) VALUES (?1,?2,'head_of_products','proceed_with_documented_rationale',?3,?4)",
            rusqlite::params![
                support_id,
                i64::try_from(ordinal).map_err(|_| ())?,
                judgment.rationale(),
                judgment.classification().as_persisted(),
            ],
        )
        .map_err(|_| ())?;
    }
    for (ordinal, evidence) in support.evidence().iter().enumerate() {
        if evidence.role() != EvidenceRole::ActionCompletion {
            return Err(());
        }
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
        tx.execute(
            "INSERT INTO support_evidence (support_id,evidence_id) VALUES (?1,?2)",
            rusqlite::params![support_id, evidence.id().as_str()],
        )
        .map_err(|_| ())?;
        tx.execute(
            "INSERT INTO action_h2a_support_evidence_snapshots (prepared_intent_id,ordinal,evidence_id,evidence_version,classification,role,verification,last_verified_at,integrity_digest) VALUES (?1,?2,?3,?8,?4,'action_completion',?5,?6,?7)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| ())?,
                evidence.id().as_str(),
                evidence.classification().as_persisted(),
                verification,
                last_verified_at,
                integrity_digest,
                i64::try_from(evidence.source_version().get()).map_err(|_| ())?,
            ],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'action',?2,?3)",
        rusqlite::params![prepared.id().as_str(), action_id, action_version],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,0,'action.completed','action',?2,NULL,NULL)",
        rusqlite::params![prepared.id().as_str(), action_id],
    )
    .map_err(|_| ())?;
    for (ordinal, source) in preview.classification_sources().iter().enumerate() {
        let (role, source_id) = classification_source_persisted_role(source.role())?;
        tx.execute(
            "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| ())?,
                role,
                source_id,
                source.classification().as_persisted()
            ],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at) VALUES (?1,?2,?3,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
        rusqlite::params![prepared.id().as_str(), action_id, action_version],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'prepare_complete',?2,?3,'prepared',?4,'not_applicable')",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            operation_ordinal,
            prepared.id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    let judgment = support.judgments().first();
    tx.execute(
        "INSERT INTO action_command_prepare_completes (idempotency_id,action_id,expected_version,judgment_disposition,judgment_actor,judgment_rationale,judgment_classification) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            action_id,
            action_version,
            judgment.map(|_| "proceed_with_documented_rationale"),
            judgment.map(|_| "head_of_products"),
            judgment.map(pmc_domain::work_management::HumanJudgment::rationale),
            judgment.map(|item| item.classification().as_persisted()),
        ],
    )
    .map_err(|_| ())?;
    Ok(())
}

/// Persists a PREPARE-time H3 denial for Complete (the same "correct row,
/// undecoded today" shape as `persist_prepare_cancel_h3_denied` -- see its
/// doc comment; Complete's own PREPARE denies far more often in practice,
/// since `prepare_complete_action_cause` requires non-empty, fully-resolved,
/// Verified-or-judged completion evidence up front).
fn persist_prepare_complete_h3_denied(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let ActionPersistenceResult::Terminal {
        command:
            ActionPersistenceCommand::PrepareComplete {
                action_id,
                expected_version,
                judgment,
            },
        cause,
        error,
        prepared_disposition,
        audit,
    } = capsule.result()
    else {
        return Err(());
    };
    if *prepared_disposition != PreparedDisposition::NotApplicable {
        return Err(());
    }
    let ActionPersistenceTerminalCause::H3Denied(h3_cause) = cause else {
        return Err(());
    };
    let target_id = match audit.target() {
        AuditTarget::Action(id) => id.as_str(),
        _ => return Err(()),
    };
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,terminal_h3_cause,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref) VALUES (?1,'prepare_complete',?2,?3,'terminal','not_applicable','h3_denied',?4,?5,?6,?7,?8,?9)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?,
            persisted_h3_denial_cause(*h3_cause),
            error.code().as_str(),
            error.message_key().as_str(),
            error.correlation_id().as_str(),
            i64::from(error.retryable()),
            error.private_detail_ref().map(|value| value.as_str()),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_prepare_completes (idempotency_id,action_id,expected_version,judgment_disposition,judgment_actor,judgment_rationale,judgment_classification) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            action_id.as_str(),
            i64::try_from(expected_version.get()).unwrap_or(-1),
            judgment
                .as_ref()
                .map(|_| "proceed_with_documented_rationale"),
            judgment.as_ref().map(|_| "head_of_products"),
            judgment
                .as_ref()
                .map(pmc_domain::work_management::HumanJudgment::rationale),
            judgment
                .as_ref()
                .map(|item| item.classification().as_persisted()),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,'action',?4,?5,?6,?7,?8,'none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_id,
            context.correlation_id.as_str(),
            audit.policy_outcome().as_persisted(),
            audit.approval_outcome().as_persisted(),
            audit.execution_outcome().as_persisted(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| ())?;
    // Unlike this function's Cancel/Reopen siblings
    // (`persist_prepare_cancel_h3_denied`/`persist_prepare_reopen_h3_denied`),
    // this call was missing -- `error` (built via `domain_error_with_snapshot`)
    // carries a `SafeErrorExtension::CurrentVersion` for essentially every
    // real H3 denial (the target Action always exists by the time PREPARE
    // can deny it), and that extension was being silently dropped instead of
    // durably persisted. Harmless while decode never reconstructed the
    // capsule at all (the whole transaction always rolled back regardless),
    // but required now that decode makes this capsule durable.
    persist_replay_error_details(tx, context, error)?;
    Ok(())
}

/// Persists a successful Cancel PREPARE result into the shared
/// `prepared_intents` substrate plus `action_replay_operations` and
/// `action_command_prepare_cancels`. `prepared` is the domain service's OWN
/// return value from `service.prepare_cancel_action(command)` -- not
/// independently recomputed/cross-checked here, unlike Accept and Lower
/// Classification's caller-supplied-canonical PREPARE (see
/// `prepare_cancel_action`'s doc comment for why).
fn persist_prepare_cancel_prepared(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
    prepared: &WorkManagementPreparedIntent,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let operation_ordinal = i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?;
    let preview = prepared.preview();
    let created_at = preview
        .expires_at()
        .unix_millis()
        .checked_sub(WORK_MANAGEMENT_H2A_TTL_MILLIS)
        .ok_or(())?;
    let (action_id, action_version, reason, bindings) = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::CancelAction {
            action_id,
            action_version,
            reason,
            evidence_classifications,
        } => (
            action_id.as_str(),
            action_version.get(),
            reason.as_str(),
            evidence_classifications,
        ),
        _ => return Err(()),
    };
    let action_version = i64::try_from(action_version).unwrap_or(-1);

    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'cancel_action',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(preview.contract_version()),
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            preview.expires_at().unix_millis(),
            created_at,
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'action',?2,?3)",
        rusqlite::params![prepared.id().as_str(), action_id, action_version],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,0,'action.cancelled','action',?2,NULL,NULL)",
        rusqlite::params![prepared.id().as_str(), action_id],
    )
    .map_err(|_| ())?;
    let sources = preview.classification_sources();
    // 2026-09: sources is no longer always `[PrimaryTarget]` -- Cancel's own
    // preview appends one `Evidence(evidence_id)` source per linked
    // completion-evidence binding (see `WorkManagementOperation::CancelAction`/
    // `ReopenAction`'s `classification_sources` impl), reachable now that
    // `LinkActionCompletionEvidence` has real SQLite persistence. Persist
    // every source, not just an assumed-lone primary target.
    for (ordinal, source) in sources.iter().enumerate() {
        let (role, source_id) = match source.role() {
            pmc_domain::work_management::WorkManagementClassificationSourceRole::PrimaryTarget => {
                ("primary_target", None)
            }
            pmc_domain::work_management::WorkManagementClassificationSourceRole::Evidence(id) => {
                ("evidence", Some(id.as_str()))
            }
            _ => return Err(()),
        };
        tx.execute(
            "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| ())?,
                role,
                source_id,
                source.classification().as_persisted()
            ],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at) VALUES (?1,?2,?3,NULL,NULL,NULL,?4,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
        rusqlite::params![prepared.id().as_str(), action_id, action_version, reason],
    )
    .map_err(|_| ())?;
    for (ordinal, binding) in bindings.iter().enumerate() {
        tx.execute(
            "INSERT INTO prepared_evidence_classifications (prepared_intent_id,ordinal,evidence_id,classification) VALUES (?1,?2,?3,?4)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| ())?,
                binding.evidence_id().as_str(),
                binding.classification().as_persisted(),
            ],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'prepare_cancel',?2,?3,'prepared',?4,'not_applicable')",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            operation_ordinal,
            prepared.id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_prepare_cancels (idempotency_id,action_id,expected_version,reason) VALUES (?1,?2,?3,?4)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            action_id,
            action_version,
            reason,
        ],
    )
    .map_err(|_| ())?;
    Ok(())
}

/// Persists a PREPARE-time H3 denial (`ClassificationUnresolved` is the only
/// cause `prepare_cancel_action_cause` can produce today -- see
/// `is_h3_denial`) as a durable Terminal capsule. Reachable in practice
/// (2026-09 update: an Unclassified commitment, OR now also an Unclassified
/// linked completion-evidence reference -- `LinkActionCompletionEvidence`'s
/// own writer does not require Verified/classified evidence at link time,
/// see `prepare_cancel_action`'s doc comment), not merely theoretical.
/// `action_replay_terminal_shape_insert` requires
/// `prepared_disposition='not_applicable'` and `prepared_intent_id IS NULL`
/// for this operation/result_kind pair -- no `WorkManagementPreparedIntent`
/// was ever minted for a denied prepare, so there is nothing beyond the
/// replay row and its audit trail to persist. The row this writes is
/// correct and would commit (the caller's `PersistedActionTransitionPrepare`
/// wrapper ensures that), but decoding it back is not implemented (see
/// `decode_cancel_action_activity`'s own doc comment), so the caller's
/// post-persist `decode_action_namespace` re-check still fails closed and
/// the whole attempt still rolls back today -- deliberately, not a bug.
fn persist_prepare_cancel_h3_denied(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let ActionPersistenceResult::Terminal {
        command:
            ActionPersistenceCommand::PrepareCancel {
                action_id,
                expected_version,
                reason,
            },
        cause,
        error,
        prepared_disposition,
        audit,
    } = capsule.result()
    else {
        return Err(());
    };
    if *prepared_disposition != PreparedDisposition::NotApplicable {
        return Err(());
    }
    let ActionPersistenceTerminalCause::H3Denied(h3_cause) = cause else {
        return Err(());
    };
    let target_id = match audit.target() {
        AuditTarget::Action(id) => id.as_str(),
        _ => return Err(()),
    };
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,terminal_h3_cause,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref) VALUES (?1,'prepare_cancel',?2,?3,'terminal','not_applicable','h3_denied',?4,?5,?6,?7,?8,?9)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?,
            persisted_h3_denial_cause(*h3_cause),
            error.code().as_str(),
            error.message_key().as_str(),
            error.correlation_id().as_str(),
            i64::from(error.retryable()),
            error.private_detail_ref().map(|value| value.as_str()),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_prepare_cancels (idempotency_id,action_id,expected_version,reason) VALUES (?1,?2,?3,?4)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            action_id.as_str(),
            i64::try_from(expected_version.get()).unwrap_or(-1),
            reason.as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,'action',?4,?5,?6,?7,?8,'none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_id,
            context.correlation_id.as_str(),
            audit.policy_outcome().as_persisted(),
            audit.approval_outcome().as_persisted(),
            audit.execution_outcome().as_persisted(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| ())?;
    persist_replay_error_details(tx, context, error)?;
    Ok(())
}

/// `ActionReopenMode`'s persisted string -- matches
/// `action_command_prepare_reopens.mode`'s CHECK constraint exactly.
const fn reopen_mode_persisted(mode: ActionReopenMode) -> &'static str {
    match mode {
        ActionReopenMode::ReopenCompleted => "reopen_completed",
        ActionReopenMode::RestartCancelled => "restart_cancelled",
    }
}

/// Reopen's sibling of `persist_prepare_cancel_prepared` -- same shared-
/// substrate shape, with one extra column (`reopen_mode`) `CancelAction`
/// doesn't carry.
fn persist_prepare_reopen_prepared(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
    prepared: &WorkManagementPreparedIntent,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let operation_ordinal = i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?;
    let preview = prepared.preview();
    let created_at = preview
        .expires_at()
        .unix_millis()
        .checked_sub(WORK_MANAGEMENT_H2A_TTL_MILLIS)
        .ok_or(())?;
    let (action_id, action_version, mode, reason, bindings) = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::ReopenAction {
            action_id,
            action_version,
            mode,
            reason,
            evidence_classifications,
        } => (
            action_id.as_str(),
            action_version.get(),
            *mode,
            reason.as_str(),
            evidence_classifications,
        ),
        _ => return Err(()),
    };
    let action_version = i64::try_from(action_version).unwrap_or(-1);
    let mode_str = reopen_mode_persisted(mode);

    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at) VALUES (?1,?2,'reopen_action',?3,?4,'allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(preview.contract_version()),
            prepared.payload_digest().as_str(),
            prepared.classification().as_persisted(),
            preview.expires_at().unix_millis(),
            created_at,
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'action',?2,?3)",
        rusqlite::params![prepared.id().as_str(), action_id, action_version],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO prepared_intent_effects (prepared_intent_id,ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id) VALUES (?1,0,'action.reopened','action',?2,NULL,NULL)",
        rusqlite::params![prepared.id().as_str(), action_id],
    )
    .map_err(|_| ())?;
    let sources = preview.classification_sources();
    // 2026-09: sources is no longer always `[PrimaryTarget]` -- Reopen's own
    // preview appends one `Evidence(evidence_id)` source per linked
    // completion-evidence binding (see `WorkManagementOperation::CancelAction`/
    // `ReopenAction`'s `classification_sources` impl), reachable now that
    // `LinkActionCompletionEvidence` has real SQLite persistence. Persist
    // every source, not just an assumed-lone primary target.
    for (ordinal, source) in sources.iter().enumerate() {
        let (role, source_id) = match source.role() {
            pmc_domain::work_management::WorkManagementClassificationSourceRole::PrimaryTarget => {
                ("primary_target", None)
            }
            pmc_domain::work_management::WorkManagementClassificationSourceRole::Evidence(id) => {
                ("evidence", Some(id.as_str()))
            }
            _ => return Err(()),
        };
        tx.execute(
            "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| ())?,
                role,
                source_id,
                source.classification().as_persisted()
            ],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO prepared_work_management_payloads (prepared_intent_id,primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at) VALUES (?1,?2,?3,NULL,NULL,NULL,?4,NULL,NULL,NULL,NULL,NULL,NULL,NULL,?5,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL)",
        rusqlite::params![
            prepared.id().as_str(),
            action_id,
            action_version,
            reason,
            mode_str
        ],
    )
    .map_err(|_| ())?;
    for (ordinal, binding) in bindings.iter().enumerate() {
        tx.execute(
            "INSERT INTO prepared_evidence_classifications (prepared_intent_id,ordinal,evidence_id,classification) VALUES (?1,?2,?3,?4)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| ())?,
                binding.evidence_id().as_str(),
                binding.classification().as_persisted(),
            ],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_disposition) VALUES (?1,'prepare_reopen',?2,?3,'prepared',?4,'not_applicable')",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            operation_ordinal,
            prepared.id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_prepare_reopens (idempotency_id,action_id,expected_version,mode,reason) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            action_id,
            action_version,
            mode_str,
            reason,
        ],
    )
    .map_err(|_| ())?;
    Ok(())
}

/// Reopen's sibling of `persist_prepare_cancel_h3_denied` -- see that
/// function's doc comment; the same "reachable in practice" and "rolls back
/// via the decode re-check, not a bug" reasoning applies here
/// (`ClassificationUnresolved` is `prepare_reopen_action_cause`'s only
/// H3-denial cause too).
fn persist_prepare_reopen_h3_denied(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let ActionPersistenceResult::Terminal {
        command:
            ActionPersistenceCommand::PrepareReopen {
                action_id,
                expected_version,
                mode,
                reason,
            },
        cause,
        error,
        prepared_disposition,
        audit,
    } = capsule.result()
    else {
        return Err(());
    };
    if *prepared_disposition != PreparedDisposition::NotApplicable {
        return Err(());
    }
    let ActionPersistenceTerminalCause::H3Denied(h3_cause) = cause else {
        return Err(());
    };
    let target_id = match audit.target() {
        AuditTarget::Action(id) => id.as_str(),
        _ => return Err(()),
    };
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_disposition,terminal_cause,terminal_h3_cause,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref) VALUES (?1,'prepare_reopen',?2,?3,'terminal','not_applicable','h3_denied',?4,?5,?6,?7,?8,?9)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?,
            persisted_h3_denial_cause(*h3_cause),
            error.code().as_str(),
            error.message_key().as_str(),
            error.correlation_id().as_str(),
            i64::from(error.retryable()),
            error.private_detail_ref().map(|value| value.as_str()),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_prepare_reopens (idempotency_id,action_id,expected_version,mode,reason) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            action_id.as_str(),
            i64::try_from(expected_version.get()).unwrap_or(-1),
            reopen_mode_persisted(*mode),
            reason.as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,'action',?4,?5,?6,?7,?8,'none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_id,
            context.correlation_id.as_str(),
            audit.policy_outcome().as_persisted(),
            audit.approval_outcome().as_persisted(),
            audit.execution_outcome().as_persisted(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| ())?;
    persist_replay_error_details(tx, context, error)?;
    Ok(())
}

/// Persists a successful Cancel EXECUTE result: updates the durable `actions`
/// row and `aggregate_registry` in place, consumes the prepared intent, mints
/// its approval receipt, and records the replay/audit trail. Mirrors
/// `approve_and_execute_accept_action_request`'s success block, generalized
/// for `execute_action`'s shared `kind` column.
fn persist_execute_action_transition_success(
    tx: &Transaction<'_>,
    approval: &WorkManagementApproval,
    kind: &str,
    outcome: &ActionMutationOutcome<ActionRecord>,
    occurred_at: UtcTimestamp,
    context: &ActionOperationContext,
) -> Result<(), ()> {
    let record = &outcome.record;
    let receipt_id = outcome.approval_receipt_id.as_ref().ok_or(())?;
    let audit = outcome.audit_events.first().ok_or(())?;
    let state = match record.state() {
        ActionState::Open => "open",
        ActionState::InProgress => "in_progress",
        ActionState::Completed => "completed",
        ActionState::Cancelled => "cancelled",
    };
    let operation_ordinal: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM (SELECT operation_ordinal FROM action_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_prepare_replay_operations UNION ALL SELECT operation_ordinal FROM action_h2a_lower_classification_execute_replay_operations UNION ALL SELECT operation_ordinal FROM action_decision_replay_operations UNION ALL SELECT operation_ordinal FROM action_reject_prepared_command_results)",
            [],
            |row| row.get(0),
        )
        .map_err(|_| ())?;
    // `support_id` stays NULL for Cancel/Reopen (never attach a `SupportWitness`
    // -- `record.support()` is `None`). Complete is the one transition that
    // does: its PREPARE already persisted the witness under this exact id
    // (`persist_prepare_complete_prepared`'s `support_id`), so EXECUTE only
    // needs to reference it, not re-derive or re-persist it.
    let support_id = record
        .support()
        .map(|_| format!("action-h2a-support-{}", approval.prepared_id().as_str()));
    tx.execute(
        "UPDATE actions SET state=?1,transition_reason=?2,support_id=?3 WHERE id=?4",
        rusqlite::params![
            state,
            record.transition_reason().map(|reason| reason.as_str()),
            support_id,
            record.id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "UPDATE aggregate_registry SET version=?1,classification=?2,updated_at=?3 WHERE id=?4 AND aggregate_type='action'",
        rusqlite::params![
            i64::try_from(record.version().get()).unwrap_or(-1),
            record.classification().as_persisted(),
            occurred_at.unix_millis(),
            record.id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        rusqlite::params![occurred_at.unix_millis(), approval.prepared_id().as_str()],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) SELECT ?1,id,'head_of_products',payload_digest,?2,?3,expires_at,?3 FROM prepared_intents WHERE id=?4",
        rusqlite::params![
            receipt_id.as_str(),
            context.idempotency_id.as_str(),
            occurred_at.unix_millis(),
            approval.prepared_id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,prepared_disposition) VALUES (?1,'execute_action',?2,?3,'action',?4,?5,'not_applicable')",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            operation_ordinal,
            record.id().as_str(),
            approval.prepared_id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_execute_actions (idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,?3,'head_of_products',?4)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            kind,
            approval.prepared_id().as_str(),
            approval.acknowledged_payload_digest().as_str(),
        ],
    )
    .map_err(|_| ())?;
    let (target_type, target_id) = match audit.target() {
        AuditTarget::Action(id) => ("action", id.as_str()),
        _ => return Err(()),
    };
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','approved','succeeded','complete')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_type,
            target_id,
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO audit_effects (audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES (?1,0,?2,'complete',?3,?4)",
        rusqlite::params![
            audit.id().as_str(),
            audit.actual_effects()[0].as_str(),
            target_type,
            target_id,
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            audit.id().as_str(),
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| ())?;
    Ok(())
}

/// Persists an EXECUTE-time terminal failure for Cancel/Complete/Reopen
/// (`execute_action`'s shared shape). Mirrors `persist_execute_accept_terminal`
/// exactly, generalized for the `kind` column `action_command_execute_actions`
/// carries that `action_command_execute_accepts` does not.
fn persist_execute_action_transition_terminal(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
    prepared_id: &PreparedIntentId,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let ActionPersistenceResult::Terminal {
        command:
            ActionPersistenceCommand::ExecuteAction {
                kind,
                prepared_id: terminal_prepared_id,
                actor,
                acknowledged_digest,
            },
        cause,
        error,
        prepared_disposition,
        audit,
    } = capsule.result()
    else {
        return Err(());
    };
    if terminal_prepared_id != prepared_id || *actor != AuditActor::HeadOfProducts {
        return Err(());
    }
    let kind_str = match kind {
        ActionPersistenceTransitionKind::Complete => "complete",
        ActionPersistenceTransitionKind::Cancel => "cancel",
        ActionPersistenceTransitionKind::Reopen => "reopen",
    };
    let (terminal_cause, attempted_digest) = persisted_terminal_cause(cause)?;
    let (target_type, target_id) = match audit.target() {
        AuditTarget::ActionRequest(id) => ("action_request", id.as_str()),
        AuditTarget::Action(id) => ("action", id.as_str()),
        _ => return Err(()),
    };
    let disposition = match prepared_disposition {
        PreparedDisposition::Retained => "retained",
        PreparedDisposition::ConsumedAndDiscarded => "consumed_and_discarded",
        PreparedDisposition::NotApplicable => return Err(()),
    };
    if *prepared_disposition == PreparedDisposition::ConsumedAndDiscarded {
        tx.execute(
            "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
            rusqlite::params![audit.occurred_at().unix_millis(), prepared_id.as_str()],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_intent_id,prepared_disposition,terminal_cause,attempted_digest,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref) VALUES (?1,'execute_action',?2,?3,'terminal',?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            context.idempotency_id.as_str(), context.correlation_id.as_str(),
            i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?, prepared_id.as_str(),
            disposition, terminal_cause, attempted_digest, error.code().as_str(), error.message_key().as_str(),
            error.correlation_id().as_str(), i64::from(error.retryable()),
            error.private_detail_ref().map(|value| value.as_str()),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_execute_actions (idempotency_id,kind,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,?3,'head_of_products',?4)",
        rusqlite::params![context.idempotency_id.as_str(), kind_str, prepared_id.as_str(), acknowledged_digest.as_str()],
    )
    .map_err(|_| ())?;
    if *prepared_disposition == PreparedDisposition::ConsumedAndDiscarded {
        tx.execute(
            "INSERT INTO action_discarded_prepared_intents (prepared_intent_id,idempotency_id) VALUES (?1,?2)",
            rusqlite::params![prepared_id.as_str(), context.idempotency_id.as_str()],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,?7,?8,?9,'none')",
        rusqlite::params![
            audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type,
            target_id, context.correlation_id.as_str(), audit.policy_outcome().as_persisted(),
            audit.approval_outcome().as_persisted(), audit.execution_outcome().as_persisted(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| ())?;
    persist_replay_error_details(tx, context, error)?;
    Ok(())
}

/// Shared tail for every terminal-persist function: writes
/// `action_replay_error_params`/`action_replay_error_extensions` from a
/// `DomainError`'s params/extensions. Factored out of
/// `persist_execute_accept_terminal` so the new Cancel/Complete/Reopen
/// terminal-persist functions do not duplicate it a third and fourth time.
fn persist_replay_error_details(
    tx: &Transaction<'_>,
    context: &ActionOperationContext,
    error: &DomainError,
) -> Result<(), ()> {
    for (ordinal, param) in error.params().iter().enumerate() {
        let (kind, text, unsigned, boolean) = match param.value() {
            SafeParamValue::Identifier(value) => ("identifier", Some(value.as_str()), None, None),
            SafeParamValue::FieldKey(value) => ("field_key", Some(value.as_str()), None, None),
            SafeParamValue::Unsigned(value) => (
                "unsigned",
                None,
                Some(i64::try_from(*value).map_err(|_| ())?),
                None,
            ),
            SafeParamValue::Boolean(value) => ("boolean", None, None, Some(i64::from(*value))),
        };
        tx.execute(
            "INSERT INTO action_replay_error_params (idempotency_id,ordinal,param_key,param_kind,param_text,param_unsigned,param_boolean) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![context.idempotency_id.as_str(), i64::try_from(ordinal).map_err(|_| ())?, param.key(), kind, text, unsigned, boolean],
        ).map_err(|_| ())?;
    }
    for (ordinal, extension) in error.extensions().iter().enumerate() {
        match extension {
            SafeErrorExtension::CurrentVersion(version) => tx.execute(
                "INSERT INTO action_replay_error_extensions (idempotency_id,ordinal,extension_kind,current_version) VALUES (?1,?2,'current_version',?3)",
                rusqlite::params![context.idempotency_id.as_str(), i64::try_from(ordinal).map_err(|_| ())?, i64::try_from(version.get()).map_err(|_| ())?],
            ),
            SafeErrorExtension::FieldErrors(_) => return Err(()),
        }.map_err(|_| ())?;
    }
    Ok(())
}

fn persist_execute_accept_terminal(
    tx: &Transaction<'_>,
    snapshot: &ActionPersistenceSnapshot,
    context: &ActionOperationContext,
    prepared_id: &PreparedIntentId,
) -> Result<(), ()> {
    let capsule = snapshot
        .replay()
        .iter()
        .find(|capsule| capsule.idempotency_id() == &context.idempotency_id)
        .ok_or(())?;
    let ActionPersistenceResult::Terminal {
        command:
            ActionPersistenceCommand::ExecuteAccept {
                prepared_id: terminal_prepared_id,
                actor,
                acknowledged_digest,
            },
        cause,
        error,
        prepared_disposition,
        audit,
    } = capsule.result()
    else {
        return Err(());
    };
    if terminal_prepared_id != prepared_id || *actor != AuditActor::HeadOfProducts {
        return Err(());
    }
    let (terminal_cause, attempted_digest) = persisted_terminal_cause(cause)?;
    let (target_type, target_id) = match audit.target() {
        AuditTarget::ActionRequest(id) => ("action_request", id.as_str()),
        AuditTarget::Action(id) => ("action", id.as_str()),
        _ => return Err(()),
    };
    let disposition = match prepared_disposition {
        PreparedDisposition::Retained => "retained",
        PreparedDisposition::ConsumedAndDiscarded => "consumed_and_discarded",
        PreparedDisposition::NotApplicable => return Err(()),
    };
    if *prepared_disposition == PreparedDisposition::ConsumedAndDiscarded {
        tx.execute(
            "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
            rusqlite::params![audit.occurred_at().unix_millis(), prepared_id.as_str()],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO action_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,prepared_intent_id,prepared_disposition,terminal_cause,attempted_digest,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref) VALUES (?1,'execute_accept',?2,?3,'terminal',?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            context.idempotency_id.as_str(), context.correlation_id.as_str(),
            i64::try_from(capsule.operation_ordinal()).map_err(|_| ())?, prepared_id.as_str(),
            disposition, terminal_cause, attempted_digest, error.code().as_str(), error.message_key().as_str(),
            error.correlation_id().as_str(), i64::from(error.retryable()),
            error.private_detail_ref().map(|value| value.as_str()),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_command_execute_accepts (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,'head_of_products',?3)",
        rusqlite::params![context.idempotency_id.as_str(), prepared_id.as_str(), acknowledged_digest.as_str()],
    )
    .map_err(|_| ())?;
    if *prepared_disposition == PreparedDisposition::ConsumedAndDiscarded {
        tx.execute(
            "INSERT INTO action_discarded_prepared_intents (prepared_intent_id,idempotency_id) VALUES (?1,?2)",
            rusqlite::params![prepared_id.as_str(), context.idempotency_id.as_str()],
        )
        .map_err(|_| ())?;
    }
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,?7,?8,?9,'none')",
        rusqlite::params![
            audit.id().as_str(), audit.occurred_at().unix_millis(), audit.code().as_str(), target_type,
            target_id, context.correlation_id.as_str(), audit.policy_outcome().as_persisted(),
            audit.approval_outcome().as_persisted(), audit.execution_outcome().as_persisted(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| ())?;
    for (ordinal, param) in error.params().iter().enumerate() {
        let (kind, text, unsigned, boolean) = match param.value() {
            SafeParamValue::Identifier(value) => ("identifier", Some(value.as_str()), None, None),
            SafeParamValue::FieldKey(value) => ("field_key", Some(value.as_str()), None, None),
            SafeParamValue::Unsigned(value) => (
                "unsigned",
                None,
                Some(i64::try_from(*value).map_err(|_| ())?),
                None,
            ),
            SafeParamValue::Boolean(value) => ("boolean", None, None, Some(i64::from(*value))),
        };
        tx.execute(
            "INSERT INTO action_replay_error_params (idempotency_id,ordinal,param_key,param_kind,param_text,param_unsigned,param_boolean) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![context.idempotency_id.as_str(), i64::try_from(ordinal).map_err(|_| ())?, param.key(), kind, text, unsigned, boolean],
        ).map_err(|_| ())?;
    }
    for (ordinal, extension) in error.extensions().iter().enumerate() {
        match extension {
            SafeErrorExtension::CurrentVersion(version) => tx.execute(
                "INSERT INTO action_replay_error_extensions (idempotency_id,ordinal,extension_kind,current_version) VALUES (?1,?2,'current_version',?3)",
                rusqlite::params![context.idempotency_id.as_str(), i64::try_from(ordinal).map_err(|_| ())?, i64::try_from(version.get()).map_err(|_| ())?],
            ),
            SafeErrorExtension::FieldErrors(_) => return Err(()),
        }.map_err(|_| ())?;
    }
    Ok(())
}

/// Maps every non-H3 terminal cause to its persisted string (and, for
/// `DigestMismatch`, the attempted digest). Widened for the Cancel/Complete/
/// Reopen execute slice to cover every `ActionPersistenceTerminalCause`
/// variant `action_replay_operations`'s own `terminal_cause` CHECK already
/// admits (see V2_ACTION_SQL) -- purely additive: every case Accept's own
/// terminal persist path already relied on keeps its exact prior mapping.
/// `H3Denied` is deliberately excluded: it is PREPARE-only (see
/// `persisted_h3_denial_cause`) and never reaches an execute terminal.
fn persisted_terminal_cause(
    cause: &ActionPersistenceTerminalCause,
) -> Result<(&'static str, Option<&str>), ()> {
    match cause {
        ActionPersistenceTerminalCause::NotFound => Ok(("not_found", None)),
        ActionPersistenceTerminalCause::AlreadyExists => Ok(("already_exists", None)),
        ActionPersistenceTerminalCause::IllegalTransition => Ok(("illegal_transition", None)),
        ActionPersistenceTerminalCause::StaleVersion => Ok(("stale_version", None)),
        ActionPersistenceTerminalCause::MissingOwner => Ok(("missing_owner", None)),
        ActionPersistenceTerminalCause::MissingDueDate => Ok(("missing_due_date", None)),
        ActionPersistenceTerminalCause::InvalidEvidence => Ok(("invalid_evidence", None)),
        ActionPersistenceTerminalCause::InvalidReopenMode => Ok(("invalid_reopen_mode", None)),
        ActionPersistenceTerminalCause::IdempotencyConflict => Ok(("idempotency_conflict", None)),
        ActionPersistenceTerminalCause::PreparedIntentNotFound => {
            Ok(("prepared_intent_not_found", None))
        }
        ActionPersistenceTerminalCause::PreparedIntentChanged => {
            Ok(("prepared_intent_changed", None))
        }
        ActionPersistenceTerminalCause::Unauthorized => Ok(("unauthorized", None)),
        ActionPersistenceTerminalCause::DigestMismatch { attempted_digest } => {
            Ok(("digest_mismatch", Some(attempted_digest.as_str())))
        }
        ActionPersistenceTerminalCause::Expired => Ok(("expired", None)),
        ActionPersistenceTerminalCause::PolicyDenied => Ok(("policy_denied", None)),
        ActionPersistenceTerminalCause::InfrastructureFailure => {
            Ok(("infrastructure_failure", None))
        }
        ActionPersistenceTerminalCause::H3Denied(_) => Err(()),
    }
}

/// Maps `ActionPersistenceH3DenialCause` to its persisted string -- only
/// ever seen wrapped in `ActionPersistenceTerminalCause::H3Denied`, which
/// only `PrepareComplete`/`PrepareCancel`/`PrepareReopen` can produce.
fn persisted_h3_denial_cause(cause: ActionPersistenceH3DenialCause) -> &'static str {
    match cause {
        ActionPersistenceH3DenialCause::MissingCompletionEvidence => "missing_completion_evidence",
        ActionPersistenceH3DenialCause::CompletionEvidenceNotFound => {
            "completion_evidence_not_found"
        }
        ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence => {
            "unverified_completion_evidence"
        }
        ActionPersistenceH3DenialCause::UnclassifiedCompletionEvidence => {
            "unclassified_completion_evidence"
        }
        ActionPersistenceH3DenialCause::EvidenceUnavailable => "evidence_unavailable",
        ActionPersistenceH3DenialCause::ClassificationUnresolved => "classification_unresolved",
    }
}

fn persistence_error(context: &ActionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("ledger.persistence_failed")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        true,
    )
}

fn idempotency_conflict(context: &ActionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("ledger.idempotency_conflict")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
}

fn already_exists(context: &ActionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("action_request.already_exists")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
}

fn not_found(context: &ActionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("action.not_found")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
}

fn evidence_not_found(context: &ActionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("evidence_reference.not_found")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
}

fn completion_evidence_already_linked(context: &ActionOperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("action.completion_evidence_already_linked")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
}

fn request_transition_conflict(
    context: &ActionOperationContext,
    request: &ActionRequestRecord,
) -> DomainError {
    let mut error = DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("action.domain_conflict")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
    .with_extension(SafeErrorExtension::CurrentVersion(request.version()));
    if let Ok(param) = MessageParam::new(
        "current_state",
        SafeParamValue::FieldKey(request.state().as_persisted().to_owned()),
    ) {
        error = error.with_param(param);
    }
    let allowed: &[&str] = match request.state() {
        ActionRequestState::Draft => &["submit_action_request"],
        ActionRequestState::Open => &[
            "prepare_accept_action_request",
            "decline_action_request",
            "withdraw_action_request",
        ],
        ActionRequestState::Accepted
        | ActionRequestState::Declined
        | ActionRequestState::Withdrawn => &[],
    };
    for (index, intent) in allowed.iter().enumerate() {
        if let Ok(param) = MessageParam::new(
            format!("allowed_next_intent_{}", index + 1),
            SafeParamValue::FieldKey((*intent).to_owned()),
        ) {
            error = error.with_param(param);
        }
    }
    error
}

/// H2a "Lower Data Classification" for Action -- a stale
/// `expected_version` at prepare time. Mirrors `request_transition_conflict`'s
/// shape but simpler: an Action's next legal H2a intent set does not depend
/// on its current state the way an Action Request's H1 transitions do.
fn action_transition_conflict(
    context: &ActionOperationContext,
    action: &ActionRecord,
) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("action.domain_conflict")
            .unwrap_or_else(|_| unreachable!("static message key")),
        context.correlation_id.clone(),
        false,
    )
    .with_extension(SafeErrorExtension::CurrentVersion(action.version()))
}

fn empty_snapshot() -> Result<ActionPersistenceSnapshot, ActionPersistenceLoadError> {
    ActionPersistenceDecodeInput::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .decode()
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)
}

/// Decision-triggered Action provenance gap fix (2026-09): reconstructs the
/// replay capsule(s) for a request created by a Decision (Resolve/
/// Supersede) -- `create_from_decision` always, plus
/// `mark_request_superseded_premise` if the request was later marked.
/// Called instead of the ordinary H1 create+transitions block in
/// `decode_draft_open_namespace`'s main per-request loop, for any request
/// `decode_request_for_id` reports a `source_decision_id` for.
///
/// `request` is the caller's already fully-decoded, cross-checked final
/// record (`decode_request_for_id`'s own decision-sourced branch); this
/// function independently re-derives the same record from the capsule
/// history alone and fails closed if the two disagree, mirroring the
/// ordinary H1 create+transitions block's own `previous != request` check.
fn decode_decision_sourced_request_activity(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
) -> Result<(), ActionPersistenceLoadError> {
    let source_decision_id = request
        .source_decision_id()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
        .clone();
    let due_at = request
        .intended_action_due_at()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (create_idem, create_correlation, create_ordinal, create_classification): (
        String,
        String,
        i64,
        String,
    ) = tx
        .query_row(
            "SELECT op.idempotency_id,op.correlation_id,op.operation_ordinal,cmd.classification FROM action_decision_replay_operations op JOIN action_command_create_from_decisions cmd USING(idempotency_id) WHERE op.operation='create_from_decision' AND op.result_reference=?1",
            [request.id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let create_idem = IdempotencyId::parse(create_idem)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let create_correlation = CorrelationId::parse(create_correlation)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let create_ordinal = u64::try_from(create_ordinal)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let create_classification = DataClassification::from_persisted(&create_classification)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let created = ActionRequestRecord::from_persisted_created_from_decision(
        request.id().clone(),
        request.title().clone(),
        request.details().clone(),
        request
            .intended_owner()
            .cloned()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
        due_at,
        create_classification,
        source_decision_id.clone(),
    );
    // `DecisionResultingActionRequest::classification` (the pre-combine
    // spec value) is not separately durable -- only the post-combine value
    // this row was actually created with is (`create_classification` /
    // `cmd.classification` above). The domain's own decode never reads this
    // sub-field back out of the command (only `id`/`subject`/`details`/
    // `intended_owner`/`due_at`, plus the command's own top-level
    // `classification`), so reusing the combined value here is harmless --
    // documented rather than silently assumed.
    let create_command = ActionPersistenceCommand::CreateRequestFromDecision {
        request: DecisionResultingActionRequest {
            id: created.id().clone(),
            subject: created.title().clone(),
            details: created.details().clone(),
            intended_owner: created
                .intended_owner()
                .cloned()
                .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
            due_at,
            classification: create_classification,
        },
        source_decision_id: source_decision_id.clone(),
        classification: create_classification,
    };
    let create_audit = decode_decision_capsule_audit(
        tx,
        "action_decision_replay_audits",
        create_idem.as_str(),
        &create_correlation,
        "action_request.created_from_decision",
        AuditTarget::ActionRequest(created.id().clone()),
    )?;
    replay.push(ActionReplayCapsule::new(
        create_idem,
        create_correlation,
        create_ordinal,
        create_command,
        ActionPersistenceResult::Request(ActionMutationOutcome {
            record: created.clone(),
            audit_events: vec![create_audit.clone()],
            approval_receipt_id: None,
        }),
        vec![create_audit.id().clone()],
    ));
    audits.push((create_ordinal, create_audit));

    if !request.has_superseded_premise() {
        if created != *request && request.state() == ActionRequestState::Open {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        return Ok(());
    }
    let (mark_idem, mark_correlation, mark_ordinal, mark_classification): (
        String,
        String,
        i64,
        String,
    ) = tx
        .query_row(
            "SELECT op.idempotency_id,op.correlation_id,op.operation_ordinal,cmd.classification FROM action_decision_replay_operations op JOIN action_command_mark_request_superseded_premises cmd USING(idempotency_id) WHERE op.operation='mark_request_superseded_premise' AND op.result_reference=?1",
            [request.id().as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mark_idem = IdempotencyId::parse(mark_idem)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mark_correlation = CorrelationId::parse(mark_correlation)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mark_ordinal = u64::try_from(mark_ordinal)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mark_classification = DataClassification::from_persisted(&mark_classification)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let marked = ActionRequestRecord::from_persisted_superseded_premise_marked(
        created,
        AggregateVersion::new(1).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        &source_decision_id,
        mark_classification,
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mark_command = ActionPersistenceCommand::MarkRequestSupersededPremise {
        request_id: marked.id().clone(),
        expected_version: AggregateVersion::new(1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        source_decision_id,
        classification: mark_classification,
    };
    let mark_audit = decode_decision_capsule_audit(
        tx,
        "action_decision_replay_audits",
        mark_idem.as_str(),
        &mark_correlation,
        "action_request.superseded_premise_marked",
        AuditTarget::ActionRequest(marked.id().clone()),
    )?;
    replay.push(ActionReplayCapsule::new(
        mark_idem,
        mark_correlation,
        mark_ordinal,
        mark_command,
        ActionPersistenceResult::Request(ActionMutationOutcome {
            record: marked.clone(),
            audit_events: vec![mark_audit.clone()],
            approval_receipt_id: None,
        }),
        vec![mark_audit.id().clone()],
    ));
    audits.push((mark_ordinal, mark_audit));
    if marked != *request && request.state() == ActionRequestState::Open {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    Ok(())
}

/// Shared audit decode for the Decision-triggered replay authority tables
/// (`action_decision_replay_audits` today) -- mirrors `decode_audit_event`'s
/// cross-checks exactly, except `approval_outcome='approved'` (these
/// mutations are always part of an approved Decision execute), not
/// `'not_required'` like the ordinary H1 create/transition audits
/// `decode_audit_event` decodes.
fn decode_decision_capsule_audit(
    tx: &Transaction<'_>,
    audit_table: &str,
    idempotency_id: &str,
    correlation: &CorrelationId,
    expected_code: &str,
    expected_target: AuditTarget,
) -> Result<AuditEvent, ActionPersistenceLoadError> {
    let event_id: String = tx
        .query_row(
            &format!(
                "SELECT audit_event_id FROM {audit_table} WHERE idempotency_id=?1 AND ordinal=0"
            ),
            [idempotency_id],
            |row| row.get(0),
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let row = tx.query_row(
        "SELECT id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope FROM audit_events WHERE id=?1",
        [event_id.as_str()], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?,row.get::<_,String>(11)?)),
    ).map_err(decode_sqlite_error)?;
    let (expected_target_type, expected_target_id) = match &expected_target {
        AuditTarget::ActionRequest(id) => ("action_request", id.as_str().to_owned()),
        AuditTarget::Action(id) => ("action", id.as_str().to_owned()),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    if row.1 < 0
        || row.2 != "head_of_products"
        || row.3 != "work_management"
        || row.4 != expected_code
        || row.5 != expected_target_type
        || row.6 != expected_target_id
        || row.7 != correlation.as_str()
        || row.8 != "allowed"
        || row.9 != "approved"
        || row.10 != "succeeded"
        || row.11 != "complete"
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let code = AuditEventCode::parse(row.4)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mut statement = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?;
    let effects = statement
        .query_map([event_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0].0 != 0
        || effects[0].1 != expected_code
        || effects[0].2 != "complete"
        || effects[0].3 != expected_target_type
        || effects[0].4 != expected_target_id
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effect_code = AuditEffectCode::parse(effects[0].1.clone())
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::Approved,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![effect_code],
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action = AuditAction::new(AuditModule::WorkManagement, code, expected_target);
    Ok(AuditEvent::new(
        event_id,
        UtcTimestamp::from_unix_millis(row.1),
        AuditActor::HeadOfProducts,
        action,
        correlation.clone(),
        disposition,
    ))
}

/// `pub(crate)`, not private: the Decision-triggered Action provenance gap
/// fix needs to rehydrate the real Action namespace from within
/// `decision_repository.rs` (Decision Resolve/Supersede's own EXECUTE must
/// mutate the REAL `InMemoryActionService`, not a throwaway fresh one, so
/// its exported capsules carry correct, globally-contiguous operation
/// ordinals) -- see `crates/pmc-ledger/src/sqlite/decision_repository.rs`'s
/// `approve_and_execute_resolve_decision_request`. A purely additive
/// visibility widening within this one ledger crate, not a change to any
/// frozen `pmc-domain` contract.
pub(crate) fn decode_action_namespace(
    tx: &Transaction<'_>,
) -> Result<ActionPersistenceSnapshot, ActionPersistenceLoadError> {
    let request_count = count(tx, "SELECT count(*) FROM action_requests")?;
    if request_count == 0 {
        if !action_namespace_is_empty(tx).map_err(classify_sqlite_error)? {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        return empty_snapshot();
    }
    let open_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE state='open'",
    )?;
    let declined_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE state='declined'",
    )?;
    let withdrawn_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE state='withdrawn'",
    )?;
    let accepted_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE state='accepted'",
    )?;
    if open_count == 0 && accepted_count == 0 && declined_count == 0 && withdrawn_count == 0 {
        return decode_create_draft(tx);
    }
    decode_draft_open_namespace(
        tx,
        request_count,
        open_count,
        accepted_count,
        declined_count,
        withdrawn_count,
    )
}

fn decode_draft_open_namespace(
    tx: &Transaction<'_>,
    request_count: i64,
    open_count: i64,
    accepted_count: i64,
    declined_count: i64,
    withdrawn_count: i64,
) -> Result<ActionPersistenceSnapshot, ActionPersistenceLoadError> {
    // Decision-triggered Action provenance gap fix (2026-09): a Decision-
    // created request (Resolve/Supersede) never goes through the ordinary
    // H1 create+transition capsule machinery at all (see
    // `decode_decision_sourced_request_activity`'s own doc comment) --
    // every count below that assumed `request_count`/`open_count`/
    // `accepted_count` map 1:1 onto `action_command_create_requests`/
    // `action_command_transition_requests`/`action_replay_operations` rows
    // must first subtract these out. Declined/withdrawn are always 0 here
    // by construction: `decode_request_for_id`'s decision-sourced branch
    // fails closed on those states, so if such a row ever existed this
    // query wouldn't even matter -- decode would already have rejected it
    // earlier.
    let decision_sourced_open_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE state='open' AND source_decision_id IS NOT NULL",
    )?;
    let decision_sourced_accepted_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE state='accepted' AND source_decision_id IS NOT NULL",
    )?;
    let decision_sourced_request_count =
        decision_sourced_open_count + decision_sourced_accepted_count;
    let decision_sourced_marked_count = count(
        tx,
        "SELECT count(*) FROM action_requests WHERE source_decision_id IS NOT NULL AND superseded_premise=1",
    )?;
    // A terminal record preserves both transitions: Draft→Open and
    // Open→Declined/Withdrawn. Open records carry only the first transition.
    let transitioned_count = (open_count - decision_sourced_open_count)
        + (accepted_count - decision_sourced_accepted_count)
        + (2 * (declined_count + withdrawn_count));
    let prepare_count = count(tx, "SELECT count(*) FROM action_command_prepare_accepts")?;
    let execute_count = count(tx, "SELECT count(*) FROM action_command_execute_accepts")?;
    let terminal_execute_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='execute_accept' AND result_kind='terminal'",
    )?;
    let retained_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='execute_accept' AND result_kind='terminal' AND prepared_disposition='retained'",
    )?;
    let discarded_terminal_count = terminal_execute_count - retained_terminal_count;
    if discarded_terminal_count < 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let accepted_execute_count = execute_count - terminal_execute_count;
    if accepted_execute_count < 0 || accepted_count != accepted_execute_count {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    // H2a "Lower Data Classification" for Action -- its own
    // fresh-root V30/V31 replay authority, decoded separately below once
    // every accepted Action is known.
    let lower_prepare_count = count(
        tx,
        "SELECT count(*) FROM action_h2a_lower_classification_command_prepares",
    )?;
    let lower_execute_count = count(
        tx,
        "SELECT count(*) FROM action_h2a_lower_classification_command_executes",
    )?;
    // Complete/Cancel/Reopen's own
    // shared-substrate replay authority (`action_replay_operations`/
    // `prepared_intents` etc., not a fresh-root table like Lower
    // Classification's) -- filtering `action_command_execute_actions` by
    // `kind` distinguishes the three at the shared `execute_action` table.
    let complete_prepare_success_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='prepare_complete' AND result_kind='prepared'",
    )?;
    let complete_prepare_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='prepare_complete' AND result_kind='terminal'",
    )?;
    let complete_prepare_count = complete_prepare_success_count + complete_prepare_terminal_count;
    let complete_execute_success_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='action' AND cmd.kind='complete'",
    )?;
    let complete_execute_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='terminal' AND cmd.kind='complete'",
    )?;
    let complete_execute_count = complete_execute_success_count + complete_execute_terminal_count;
    let complete_retained_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='terminal' AND replay.prepared_disposition='retained' AND cmd.kind='complete'",
    )?;
    let complete_discarded_terminal_count =
        complete_execute_terminal_count - complete_retained_terminal_count;
    if complete_discarded_terminal_count < 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let cancel_prepare_success_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='prepare_cancel' AND result_kind='prepared'",
    )?;
    let cancel_prepare_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='prepare_cancel' AND result_kind='terminal'",
    )?;
    let cancel_prepare_count = cancel_prepare_success_count + cancel_prepare_terminal_count;
    let cancel_execute_success_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='action' AND cmd.kind='cancel'",
    )?;
    let cancel_execute_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='terminal' AND cmd.kind='cancel'",
    )?;
    let cancel_execute_count = cancel_execute_success_count + cancel_execute_terminal_count;
    let cancel_retained_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='terminal' AND replay.prepared_disposition='retained' AND cmd.kind='cancel'",
    )?;
    let cancel_discarded_terminal_count =
        cancel_execute_terminal_count - cancel_retained_terminal_count;
    if cancel_discarded_terminal_count < 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    // Reopen's own counts, mirroring Complete/Cancel's exactly (see the
    // comment above complete_prepare_success_count).
    let reopen_prepare_success_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='prepare_reopen' AND result_kind='prepared'",
    )?;
    let reopen_prepare_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='prepare_reopen' AND result_kind='terminal'",
    )?;
    let reopen_prepare_count = reopen_prepare_success_count + reopen_prepare_terminal_count;
    let reopen_execute_success_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='action' AND cmd.kind='reopen'",
    )?;
    let reopen_execute_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='terminal' AND cmd.kind='reopen'",
    )?;
    let reopen_execute_count = reopen_execute_success_count + reopen_execute_terminal_count;
    let reopen_retained_terminal_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND replay.result_kind='terminal' AND replay.prepared_disposition='retained' AND cmd.kind='reopen'",
    )?;
    let reopen_discarded_terminal_count =
        reopen_execute_terminal_count - reopen_retained_terminal_count;
    if reopen_discarded_terminal_count < 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    // StartAction's own count, and the first real writer to `action_transitions`
    // (Cancel/Reopen reconstruct their transition-history entries from their
    // own dedicated capsule tables instead, never from this table).
    let start_action_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='start_action'",
    )?;
    // LinkActionCompletionEvidence's own count -- an Action can accumulate
    // more than one linked evidence reference, so `action_completion_evidence`'s
    // row count is this, not a 0/1 flag.
    let link_completion_evidence_count = count(
        tx,
        "SELECT count(*) FROM action_replay_operations WHERE operation='link_completion_evidence'",
    )?;
    // 2026-09: a successfully-prepared Cancel/Reopen intent persists one
    // `prepared_intent_classification_sources` row per linked
    // completion-evidence binding, on top of its one `primary_target` row --
    // see `persist_prepare_cancel_prepared`'s own comment. These counts feed
    // the `prepared_intent_classification_sources` cross-checks below.
    let cancel_evidence_binding_count = count(
        tx,
        "SELECT count(*) FROM prepared_evidence_classifications AS binding JOIN prepared_intents AS intent ON intent.id=binding.prepared_intent_id WHERE intent.intent_kind='cancel_action'",
    )?;
    let reopen_evidence_binding_count = count(
        tx,
        "SELECT count(*) FROM prepared_evidence_classifications AS binding JOIN prepared_intents AS intent ON intent.id=binding.prepared_intent_id WHERE intent.intent_kind='reopen_action'",
    )?;
    // Complete's own `prepared_intent_classification_sources` contributors:
    // unlike Cancel/Reopen (one `Evidence(id)` row per
    // `prepared_evidence_classifications` binding), Complete's preview
    // derives them from its `SupportWitness` -- one row per linked evidence
    // reference (`action_h2a_support_evidence_snapshots`) plus one per
    // judgment (`support_judgments`, joined through the prepared intent's
    // own `support_id`).
    let complete_evidence_source_count = count(
        tx,
        "SELECT count(*) FROM action_h2a_support_evidence_snapshots AS snap JOIN prepared_intents AS intent ON intent.id=snap.prepared_intent_id WHERE intent.intent_kind='complete_action'",
    )?;
    let complete_judgment_source_count = count(
        tx,
        "SELECT count(*) FROM support_judgments AS judgment JOIN prepared_intents AS intent ON intent.support_id=judgment.support_id WHERE intent.intent_kind='complete_action'",
    )?;
    // v45: rejections, split by the kind of preview they consumed. A
    // rejection is a successful command: it never joins the terminal
    // counts, never discards through `action_discarded_prepared_intents`,
    // and retains every prepared payload/target/effect/source row; only
    // `consumed_at`, one zero-effect audit and its own result row mark it.
    let reject_accept_count = count(
        tx,
        "SELECT count(*) FROM action_reject_prepared_command_results AS rejection JOIN prepared_intents AS intent ON intent.id=rejection.prepared_intent_id WHERE intent.intent_kind='accept_action_request'",
    )?;
    let reject_complete_count = count(
        tx,
        "SELECT count(*) FROM action_reject_prepared_command_results AS rejection JOIN prepared_intents AS intent ON intent.id=rejection.prepared_intent_id WHERE intent.intent_kind='complete_action'",
    )?;
    let reject_cancel_count = count(
        tx,
        "SELECT count(*) FROM action_reject_prepared_command_results AS rejection JOIN prepared_intents AS intent ON intent.id=rejection.prepared_intent_id WHERE intent.intent_kind='cancel_action'",
    )?;
    let reject_reopen_count = count(
        tx,
        "SELECT count(*) FROM action_reject_prepared_command_results AS rejection JOIN prepared_intents AS intent ON intent.id=rejection.prepared_intent_id WHERE intent.intent_kind='reopen_action'",
    )?;
    let reject_count =
        reject_accept_count + reject_complete_count + reject_cancel_count + reject_reopen_count;
    let expected = [
        ("SELECT count(*) FROM actions", accepted_execute_count),
        (
            "SELECT count(*) FROM action_reject_prepared_command_results",
            reject_count,
        ),
        ("SELECT count(*) FROM action_transitions", start_action_count),
        (
            "SELECT count(*) FROM action_completion_evidence",
            link_completion_evidence_count,
        ),
        (
            "SELECT count(*) FROM action_replay_operations",
            (request_count - decision_sourced_request_count)
                + transitioned_count
                + prepare_count
                + execute_count
                + complete_prepare_count
                + complete_execute_count
                + cancel_prepare_count
                + cancel_execute_count
                + reopen_prepare_count
                + reopen_execute_count
                + start_action_count
                + link_completion_evidence_count,
        ),
        (
            "SELECT count(*) FROM action_replay_audits",
            (request_count - decision_sourced_request_count)
                + transitioned_count
                + (3 * accepted_execute_count)
                + terminal_execute_count
                + complete_prepare_terminal_count
                + complete_execute_count
                + cancel_prepare_terminal_count
                + cancel_execute_count
                + reopen_prepare_terminal_count
                + reopen_execute_count
                + start_action_count
                + link_completion_evidence_count,
        ),
        (
            "SELECT count(*) FROM action_command_create_requests",
            request_count - decision_sourced_request_count,
        ),
        ("SELECT count(*) FROM action_command_transition_requests", transitioned_count),
        (
            "SELECT count(*) FROM action_decision_replay_operations",
            decision_sourced_request_count + decision_sourced_marked_count,
        ),
        (
            "SELECT count(*) FROM action_decision_replay_audits",
            decision_sourced_request_count + decision_sourced_marked_count,
        ),
        (
            "SELECT count(*) FROM action_command_create_from_decisions",
            decision_sourced_request_count,
        ),
        (
            "SELECT count(*) FROM action_command_mark_request_superseded_premises",
            decision_sourced_marked_count,
        ),
        // Decision Supersede (not yet implemented) will be the only source
        // of `mark_action_superseded_premise` capsules -- stays hardcoded 0
        // without its own query until that lands.
        ("SELECT count(*) FROM action_command_mark_action_superseded_premises", 0),
        ("SELECT count(*) FROM action_command_prepare_accepts", prepare_count),
        ("SELECT count(*) FROM action_command_execute_accepts", execute_count),
        ("SELECT count(*) FROM action_command_prepare_completes", complete_prepare_count),
        ("SELECT count(*) FROM action_command_prepare_cancels", cancel_prepare_count),
        ("SELECT count(*) FROM action_command_prepare_reopens", reopen_prepare_count),
        (
            "SELECT count(*) FROM action_command_execute_actions",
            complete_execute_count + cancel_execute_count + reopen_execute_count,
        ),
        ("SELECT count(*) FROM action_command_start_actions", start_action_count),
        (
            "SELECT count(*) FROM action_command_link_completion_evidence",
            link_completion_evidence_count,
        ),
        (
            "SELECT count(*) FROM action_discarded_prepared_intents",
            discarded_terminal_count
                + complete_discarded_terminal_count
                + cancel_discarded_terminal_count
                + reopen_discarded_terminal_count,
        ),
        (
            "SELECT count(*) FROM prepared_intents WHERE intent_kind IN ('complete_action', 'cancel_action', 'reopen_action')",
            complete_prepare_success_count + cancel_prepare_success_count + reopen_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intents WHERE intent_kind='accept_action_request' AND consumed_at IS NOT NULL",
            accepted_execute_count + discarded_terminal_count + reject_accept_count,
        ),
        (
            "SELECT count(*) FROM prepared_intents WHERE intent_kind='complete_action' AND consumed_at IS NOT NULL",
            complete_execute_success_count + complete_discarded_terminal_count + reject_complete_count,
        ),
        (
            "SELECT count(*) FROM prepared_intents WHERE intent_kind='cancel_action' AND consumed_at IS NOT NULL",
            cancel_execute_success_count + cancel_discarded_terminal_count + reject_cancel_count,
        ),
        (
            "SELECT count(*) FROM prepared_intents WHERE intent_kind='reopen_action' AND consumed_at IS NOT NULL",
            reopen_execute_success_count + reopen_discarded_terminal_count + reject_reopen_count,
        ),
        (
            "SELECT count(*) FROM approval_receipts WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            accepted_execute_count,
        ),
        (
            "SELECT count(*) FROM approval_receipts WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='complete_action')",
            complete_execute_success_count,
        ),
        (
            "SELECT count(*) FROM approval_receipts WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='cancel_action')",
            cancel_execute_success_count,
        ),
        (
            "SELECT count(*) FROM approval_receipts WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='reopen_action')",
            reopen_execute_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_work_management_payloads AS payload JOIN prepared_intents AS intent ON intent.id=payload.prepared_intent_id WHERE intent.intent_kind='complete_action'",
            complete_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_targets AS target JOIN prepared_intents AS intent ON intent.id=target.prepared_intent_id WHERE intent.intent_kind='complete_action'",
            complete_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_effects AS effect JOIN prepared_intents AS intent ON intent.id=effect.prepared_intent_id WHERE intent.intent_kind='complete_action'",
            complete_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_classification_sources AS source JOIN prepared_intents AS intent ON intent.id=source.prepared_intent_id WHERE intent.intent_kind='complete_action'",
            complete_prepare_success_count + complete_evidence_source_count + complete_judgment_source_count,
        ),
        (
            "SELECT count(*) FROM prepared_work_management_payloads AS payload JOIN prepared_intents AS intent ON intent.id=payload.prepared_intent_id WHERE intent.intent_kind='cancel_action'",
            cancel_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_targets AS target JOIN prepared_intents AS intent ON intent.id=target.prepared_intent_id WHERE intent.intent_kind='cancel_action'",
            cancel_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_effects AS effect JOIN prepared_intents AS intent ON intent.id=effect.prepared_intent_id WHERE intent.intent_kind='cancel_action'",
            cancel_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_classification_sources AS source JOIN prepared_intents AS intent ON intent.id=source.prepared_intent_id WHERE intent.intent_kind='cancel_action'",
            cancel_prepare_success_count + cancel_evidence_binding_count,
        ),
        (
            "SELECT count(*) FROM prepared_work_management_payloads AS payload JOIN prepared_intents AS intent ON intent.id=payload.prepared_intent_id WHERE intent.intent_kind='reopen_action'",
            reopen_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_targets AS target JOIN prepared_intents AS intent ON intent.id=target.prepared_intent_id WHERE intent.intent_kind='reopen_action'",
            reopen_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_effects AS effect JOIN prepared_intents AS intent ON intent.id=effect.prepared_intent_id WHERE intent.intent_kind='reopen_action'",
            reopen_prepare_success_count,
        ),
        (
            "SELECT count(*) FROM prepared_intent_classification_sources AS source JOIN prepared_intents AS intent ON intent.id=source.prepared_intent_id WHERE intent.intent_kind='reopen_action'",
            reopen_prepare_success_count + reopen_evidence_binding_count,
        ),
        (
            "SELECT count(*) FROM operations WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        (
            "SELECT count(*) FROM prepared_resulting_action_requests WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        (
            "SELECT count(*) FROM prepared_evidence_classifications WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        (
            "SELECT count(*) FROM prepared_incomplete_downstream WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        (
            "SELECT count(*) FROM prepared_intent_recovery_evidence WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        (
            "SELECT count(*) FROM prepared_removal_payloads WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        (
            "SELECT count(*) FROM prepared_removal_endpoints WHERE prepared_intent_id IN (SELECT id FROM prepared_intents WHERE intent_kind='accept_action_request')",
            0,
        ),
        ("SELECT count(*) FROM prepared_intents WHERE intent_kind='accept_action_request'", prepare_count),
        ("SELECT count(*) FROM prepared_work_management_payloads AS payload JOIN prepared_intents AS intent ON intent.id=payload.prepared_intent_id WHERE intent.intent_kind='accept_action_request'", prepare_count),
        ("SELECT count(*) FROM prepared_intent_targets AS target JOIN prepared_intents AS intent ON intent.id=target.prepared_intent_id WHERE intent.intent_kind='accept_action_request'", prepare_count),
        ("SELECT count(*) FROM prepared_intent_effects AS effect JOIN prepared_intents AS intent ON intent.id=effect.prepared_intent_id WHERE intent.intent_kind='accept_action_request'", prepare_count * 3),
        ("SELECT count(*) FROM prepared_intent_classification_sources AS source JOIN prepared_intents AS intent ON intent.id=source.prepared_intent_id WHERE intent.intent_kind='accept_action_request'", prepare_count * 2),
        ("SELECT count(*) FROM aggregate_registry WHERE aggregate_type='action_request'", request_count),
        ("SELECT count(*) FROM aggregate_registry WHERE aggregate_type='action'", accepted_execute_count),
        ("SELECT count(*) FROM audit_events WHERE target_type='action_request'", request_count + transitioned_count + (2 * accepted_execute_count) + terminal_execute_count + decision_sourced_marked_count + reject_accept_count),
        (
            "SELECT count(*) FROM audit_events WHERE target_type='action'",
            accepted_execute_count
                + lower_execute_count
                + complete_prepare_terminal_count
                + complete_execute_count
                + cancel_prepare_terminal_count
                + cancel_execute_count
                + reopen_prepare_terminal_count
                + reopen_execute_count
                + start_action_count
                + link_completion_evidence_count
                + reject_complete_count
                + reject_cancel_count
                + reject_reopen_count,
        ),
        ("SELECT count(*) FROM audit_effects WHERE target_type='action_request'", request_count + transitioned_count + (2 * accepted_execute_count) + decision_sourced_marked_count),
        (
            "SELECT count(*) FROM audit_effects WHERE target_type='action'",
            accepted_execute_count
                + lower_execute_count
                + complete_execute_success_count
                + cancel_execute_success_count
                + reopen_execute_success_count
                + start_action_count
                + link_completion_evidence_count,
        ),
        ("SELECT count(*) FROM operations WHERE namespace='action'", 0),
        ("SELECT count(*) FROM idempotency_outcomes WHERE namespace='action'", 0),
        (
            "SELECT count(*) FROM action_h2a_lower_classification_prepare_replay_operations",
            lower_prepare_count,
        ),
        (
            "SELECT count(*) FROM action_h2a_lower_classification_execute_replay_operations",
            lower_execute_count,
        ),
        (
            "SELECT count(*) FROM action_h2a_lower_classification_execute_replay_audits",
            lower_execute_count,
        ),
    ];
    for (sql, value) in expected {
        let actual = count(tx, sql)?;
        if actual != value {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
    }
    if prepare_count == 0 && !action_owned_shared_topology_is_empty(tx)? {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let mut ids = tx
        .prepare("SELECT id FROM action_requests ORDER BY id")
        .map_err(decode_sqlite_error)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    ids.sort();
    let mut requests = Vec::with_capacity(ids.len());
    let mut actions = Vec::with_capacity(usize::try_from(accepted_count).unwrap_or(0));
    let transitioned_count = open_count + accepted_count + (2 * (declined_count + withdrawn_count));
    let mut replay = Vec::with_capacity((request_count + transitioned_count) as usize);
    let mut audits = Vec::with_capacity((request_count + transitioned_count) as usize);
    let mut prepared_intents =
        Vec::with_capacity((prepare_count + retained_terminal_count) as usize);
    let mut discarded_prepared = Vec::new();
    // H2a "Lower Data Classification" for Action -- captures
    // the Accept-time `(request, prepared)` inputs for every accepted
    // Action, so the lowering decode loop below can rebuild each historical
    // lowering's `ActionRecord` via
    // `ActionRecord::from_persisted_accepted_request_with_lowered_classification`
    // without re-deriving the Accept-specific fields a second time.
    let mut accepted_inputs: std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    > = std::collections::HashMap::new();
    for request_id in ids {
        let request = decode_request_for_id(tx, Some(&request_id))?;
        if request.source_decision_id().is_some() {
            // Decision-triggered Action provenance gap fix (2026-09): a
            // Decision-created request has its own, entirely separate
            // replay authority and capsule shape -- see
            // `decode_decision_sourced_request_activity`'s own doc comment.
            decode_decision_sourced_request_activity(tx, &request, &mut replay, &mut audits)?;
        } else {
            let create_record = ActionRequestRecord::from_persisted_created_draft(
                request.id().clone(),
                request.title().clone(),
                request.details().clone(),
                request.intended_owner().cloned(),
                request.response_due_at(),
                request.intended_action_due_at(),
                request.classification(),
            );
            let (create_idem, create_command) =
                decode_command_for_request(tx, &request, Some(&request_id))?;
            let create_row = replay_row(tx, &create_idem, "create_request")?;
            if create_row.0 != request.id().as_str() {
                return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
            }
            let create_audit = replay_audit(tx, &create_idem, &create_row.0, &create_row.1)?;
            let create_event = decode_audit_event(
                tx,
                &request,
                &create_audit.0,
                &create_audit.1,
                "action_request.created",
            )?;
            replay.push(ActionReplayCapsule::new(
                create_idem,
                create_row.1,
                create_row.2,
                create_command,
                ActionPersistenceResult::Request(ActionMutationOutcome {
                    record: create_record.clone(),
                    audit_events: vec![create_event.clone()],
                    approval_receipt_id: None,
                }),
                vec![create_event.id().clone()],
            ));
            audits.push((create_row.2, create_event));
            if matches!(
                request.state(),
                ActionRequestState::Open
                    | ActionRequestState::Accepted
                    | ActionRequestState::Declined
                    | ActionRequestState::Withdrawn
            ) {
                let transitions = tx
                    .prepare(
                        "SELECT idempotency_id,expected_version,target_state,rationale FROM action_command_transition_requests WHERE request_id=?1 ORDER BY expected_version",
                    )
                    .map_err(decode_sqlite_error)?
                    .query_map([&request_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .map_err(decode_sqlite_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(decode_sqlite_error)?;
                let expected_transitions = if matches!(
                    request.state(),
                    ActionRequestState::Open | ActionRequestState::Accepted
                ) {
                    1
                } else {
                    2
                };
                if transitions.len() != expected_transitions {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                }
                let mut previous = create_record.clone();
                for transition in transitions {
                    let (target_state, rationale, audit_code) = match previous.state() {
                        ActionRequestState::Draft => {
                            if transition.2 != "open" || transition.3.is_some() || transition.1 != 1
                            {
                                return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                            }
                            (ActionRequestState::Open, None, "action_request.submitted")
                        }
                        ActionRequestState::Open => {
                            let Some(rationale) = transition.3.clone() else {
                                return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                            };
                            let rationale = ActionDetails::parse(rationale)
                                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
                            if !matches!(transition.2.as_str(), "declined" | "withdrawn")
                                || transition.1 != 2
                            {
                                return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                            }
                            (
                                if transition.2 == "declined" {
                                    ActionRequestState::Declined
                                } else {
                                    ActionRequestState::Withdrawn
                                },
                                Some(rationale),
                                if transition.2 == "declined" {
                                    "action_request.declined"
                                } else {
                                    "action_request.withdrawn"
                                },
                            )
                        }
                        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
                    };
                    let idem = IdempotencyId::parse(transition.0)
                        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
                    let command = ActionPersistenceCommand::TransitionRequest {
                        request_id: request.id().clone(),
                        expected_version: AggregateVersion::new(
                            u64::try_from(transition.1)
                                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                        )
                        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                        target_state,
                        rationale: rationale.clone(),
                    };
                    let row = replay_row(tx, &idem, "transition_request")?;
                    if row.0 != request.id().as_str() || row.2 == 0 {
                        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                    }
                    let link = replay_audit(tx, &idem, &row.0, &row.1)?;
                    let event = decode_audit_event(tx, &request, &link.0, &link.1, audit_code)?;
                    previous = match target_state {
                        ActionRequestState::Open => {
                            ActionRequestRecord::from_persisted_submitted_open(
                                previous.id().clone(),
                                previous.title().clone(),
                                previous.details().clone(),
                                previous.intended_owner().cloned(),
                                previous.response_due_at(),
                                previous.intended_action_due_at(),
                                previous.classification(),
                            )
                        }
                        ActionRequestState::Declined => {
                            ActionRequestRecord::from_persisted_open_to_declined(
                                previous,
                                rationale
                                    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
                            )
                            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
                        }
                        ActionRequestState::Withdrawn => {
                            ActionRequestRecord::from_persisted_open_to_withdrawn(
                                previous,
                                rationale
                                    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
                            )
                            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
                        }
                        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
                    };
                    replay.push(ActionReplayCapsule::new(
                        idem,
                        row.1,
                        row.2,
                        command,
                        ActionPersistenceResult::Request(ActionMutationOutcome {
                            record: previous.clone(),
                            audit_events: vec![event.clone()],
                            approval_receipt_id: None,
                        }),
                        vec![event.id().clone()],
                    ));
                    audits.push((row.2, event));
                }
                if request.state() != ActionRequestState::Accepted && previous != request {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                }
            }
            let _ = create_record;
        };
        let prepared_rows = tx
            .prepare(
                "SELECT idempotency_id FROM action_command_prepare_accepts WHERE request_id=?1 ORDER BY idempotency_id",
            )
            .map_err(decode_sqlite_error)?
            .query_map([&request_id], |row| row.get::<_, String>(0))
            .map_err(decode_sqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(decode_sqlite_error)?;
        for idempotency in prepared_rows {
            let idempotency = IdempotencyId::parse(idempotency)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
            let (intent, capsule) = decode_prepared_accept(tx, &request, &idempotency)?;
            replay.push(capsule);
            let decoded_executes = decode_execute_accept(tx, &request, &intent)?;
            let has_execute = !decoded_executes.is_empty();
            let mut retained = false;
            for decoded_execute in decoded_executes {
                match decoded_execute {
                    DecodedExecuteAccept::Accepted(outcome, execute_capsule) => {
                        if outcome.request != request {
                            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                        }
                        actions.push(outcome.action.clone());
                        accepted_inputs.insert(
                            outcome.action.id().clone(),
                            (request.clone(), intent.clone()),
                        );
                        audits.extend(
                            outcome
                                .audit_events
                                .iter()
                                .map(|audit| (execute_capsule.operation_ordinal(), audit.clone())),
                        );
                        replay.push(*execute_capsule);
                    }
                    DecodedExecuteAccept::Terminal(execute_capsule, audit, disposition) => {
                        if disposition == PreparedDisposition::Retained {
                            retained = true;
                        } else {
                            discarded_prepared.push(intent.clone());
                        }
                        audits.push((execute_capsule.operation_ordinal(), audit));
                        replay.push(*execute_capsule);
                    }
                }
            }
            if let Some((rejection, audit)) = decode_prepared_intent_rejection(tx, &intent)? {
                audits.push((rejection.operation_ordinal(), audit));
                replay.push(rejection);
                discarded_prepared.push(intent);
            } else if !has_execute || retained {
                prepared_intents.push(intent);
            }
        }
        requests.push(request);
    }
    // One globally-ordinal-ordered fold across every
    // post-accept Action activity kind (Start/Link/Lower Classification/
    // Complete/Cancel/Reopen), replacing what used to be six separate
    // fixed-order passes -- see the module-level comment above
    // `ActionActivityEvent` for the full rationale.
    fold_action_activity_events(
        tx,
        &accepted_inputs,
        &mut actions,
        &mut replay,
        &mut prepared_intents,
        &mut discarded_prepared,
        &mut audits,
    )?;
    verify_final_action_rows(tx, &actions)?;
    replay.sort_by_key(ActionReplayCapsule::operation_ordinal);
    if replay
        .iter()
        .enumerate()
        .any(|(index, capsule)| capsule.operation_ordinal() != index as u64)
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let mut ordered_audits = audits;
    ordered_audits.sort_by_key(|(ordinal, _)| *ordinal);
    let audit_values = ordered_audits.into_iter().map(|(_, audit)| audit).collect();
    ActionPersistenceDecodeInput::new(
        requests,
        actions,
        prepared_intents,
        discarded_prepared,
        replay,
        audit_values,
    )
    .decode()
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)
}

/// Cross-checks every accepted action's raw durable `actions.state`/
/// `transition_reason` and `aggregate_registry.version`/`classification`
/// against the fully-folded `actions` this decode has arrived at, once,
/// after every activity decoder (Lower Classification, Cancel, Reopen) has
/// run. This used to be enforced piecemeal inside each activity decoder
/// (checking the raw row immediately after decoding that operation's own
/// capsule), but that only holds when the operation being decoded is
/// guaranteed to be an action's LAST one -- which stopped being true the
/// moment a later operation could revisit an action a prior one already
/// touched (concretely: Reopen decodes after Cancel, so Cancel's own
/// immediate raw-row check would see a `cancelled` row that a later Reopen
/// has since moved to `in_progress`, and incorrectly reject it). Checking
/// once here, after the full chain, sidesteps that ordering dependency
/// entirely and remains correct regardless of how many transition kinds an
/// action has been through.
fn verify_final_action_rows(
    tx: &Transaction<'_>,
    actions: &[ActionRecord],
) -> Result<(), ActionPersistenceLoadError> {
    for record in actions {
        let raw_row = tx
            .query_row(
                "SELECT actions.state,actions.transition_reason,reg.version,reg.classification,actions.support_id FROM actions JOIN aggregate_registry reg ON reg.id=actions.id AND reg.aggregate_type='action' WHERE actions.id=?1",
                [record.id().as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .map_err(decode_sqlite_error)?;
        let expected_state = match record.state() {
            ActionState::Open => "open",
            ActionState::InProgress => "in_progress",
            ActionState::Completed => "completed",
            ActionState::Cancelled => "cancelled",
        };
        if raw_row.0 != expected_state
            || raw_row.1.as_deref() != record.transition_reason().map(ActionDetails::as_str)
            || raw_row.2
                != i64::try_from(record.version().get())
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?
            || raw_row.3 != record.classification().as_persisted()
            // Only Complete ever sets a non-NULL `support_id` (Cancel/Reopen
            // always leave it NULL -- see `persist_execute_action_transition_success`'s
            // own comment); a full string-equality check would need the
            // originating PreparedIntentId this function doesn't have, but
            // presence/absence must still agree with `record.support()`.
            || raw_row.4.is_some() != record.support().is_some()
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
    }
    Ok(())
}

/// Reconstructs a `PrepareComplete` command from `action_command_prepare_completes`
/// alone -- the read-side counterpart of `persist_prepare_complete_h3_denied`'s
/// own INSERT into that table. Used only by `decode_complete_prepare_h3_denials`
/// Unlike `decode_prepared_complete`, there is no
/// `prepared_intents`/`support_witnesses` row to also read here -- a denied
/// PREPARE never mints a witness, only the judgment (if any) that was
/// evaluated before the H3 check failed. Judgment reconstruction logic
/// mirrors `decode_prepared_complete`'s own opening block exactly (same
/// columns, same cross-checks) -- kept as a separate, self-contained copy
/// rather than a shared helper, matching this file's established practice of
/// not retrofitting already-tested decode functions for DRY's sake alone.
fn decode_prepare_complete_h3_denial_command(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
) -> Result<ActionPersistenceCommand, ActionPersistenceLoadError> {
    let command_row = tx
        .query_row(
            "SELECT action_id,expected_version,judgment_disposition,judgment_actor,judgment_rationale,judgment_classification FROM action_command_prepare_completes WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let action_id = ActionId::parse(command_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let expected_version = AggregateVersion::new(
        u64::try_from(command_row.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if command_row
        .2
        .as_deref()
        .is_some_and(|value| value != "proceed_with_documented_rationale")
        || command_row
            .3
            .as_deref()
            .is_some_and(|value| value != "head_of_products")
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let judgment = match (&command_row.2, &command_row.4, &command_row.5) {
        (None, None, None) => None,
        (Some(_), Some(rationale), Some(classification)) => Some(
            pmc_domain::work_management::HumanJudgment::new(
                pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                rationale.clone(),
                DataClassification::from_persisted(classification)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    Ok(ActionPersistenceCommand::PrepareComplete {
        action_id,
        expected_version,
        judgment,
    })
}

/// Reconstructs a `PrepareCancel` command from `action_command_prepare_cancels`
/// alone -- the read-side counterpart of `persist_prepare_cancel_h3_denied`'s
/// own INSERT into that table. Used only by `decode_cancel_prepare_h3_denials`
/// Unlike `decode_prepared_cancel`, there is no
/// `prepared_intents` row to also read here -- a denied PREPARE never mints
/// one.
fn decode_prepare_cancel_h3_denial_command(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
) -> Result<ActionPersistenceCommand, ActionPersistenceLoadError> {
    let command_row = tx
        .query_row(
            "SELECT action_id,expected_version,reason FROM action_command_prepare_cancels WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let action_id = ActionId::parse(command_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let expected_version = AggregateVersion::new(
        u64::try_from(command_row.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let reason = ActionDetails::parse(command_row.2)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok(ActionPersistenceCommand::PrepareCancel {
        action_id,
        expected_version,
        reason,
    })
}

/// Reopen's own sibling of `decode_prepare_cancel_h3_denial_command` -- same
/// shape, reading the extra `mode` column `action_command_prepare_reopens`
/// carries that `action_command_prepare_cancels` does not (mirrors
/// `decode_prepared_reopen`'s own analogous relationship to
/// `decode_prepared_cancel`).
fn decode_prepare_reopen_h3_denial_command(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
) -> Result<ActionPersistenceCommand, ActionPersistenceLoadError> {
    let command_row = tx
        .query_row(
            "SELECT action_id,expected_version,mode,reason FROM action_command_prepare_reopens WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let action_id = ActionId::parse(command_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let expected_version = AggregateVersion::new(
        u64::try_from(command_row.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mode = match command_row.2.as_str() {
        "reopen_completed" => ActionReopenMode::ReopenCompleted,
        "restart_cancelled" => ActionReopenMode::RestartCancelled,
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let reason = ActionDetails::parse(command_row.3)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok(ActionPersistenceCommand::PrepareReopen {
        action_id,
        expected_version,
        mode,
        reason,
    })
}

// ============================================================================
// Unified, globally-ordinal-ordered Action activity fold.
//
// Replaces the fixed six-pass pipeline above (`decode_start_action_activity`
// -> `decode_link_completion_evidence_activity` ->
// `decode_lower_action_classification_activity` ->
// `decode_complete_action_activity` -> `decode_cancel_action_activity` ->
// `decode_reopen_action_activity`, each of which fully completed its own
// pass over every Action before the next began). That fixed order silently
// assumed every Action's own state-mutating events happened in exactly that
// operation-kind order -- true for the tests this repository shipped, but
// not for the ledger's own actual guarantee (only the global
// `operation_ordinal` is authoritative). Confirmed-reachable failures with
// only currently-shipped operations: `Start -> Lower Classification -> Link`
// (documented on the old `decode_link_completion_evidence_activity`),
// `Lower -> Start`, and `Cancel -> Reopen -> Cancel -> Reopen` -- each
// silently read the wrong "previous" `ActionRecord` for a later step and
// failed closed with `InvalidActionSnapshot`, even though the ledger's own
// data was completely legitimate.
//
// `gather_action_activity_events` below is a purely mechanical extraction:
// the exact same thirteen `SELECT`s the six old functions used (now
// collapsed to four gather call sites via `gather_transition_events`, since
// Complete/Cancel/Reopen's own PREPARE-success/H3-denial/EXECUTE rows are
// byte-for-byte identical in shape across all three, differing only in
// which `operation`/`cmd.kind` string filters them), just returning tagged
// rows instead of processing them inline. `fold_action_activity_events`
// sorts the combined result by the real global `operation_ordinal` and
// applies them one at a time via `apply_*` functions below, each of which
// is otherwise UNCHANGED from the corresponding old function's own per-row
// body (same queries, same cross-checks, same `from_persisted_*`
// reconstruction constructors) -- the only structural change is that
// `actions`/the shared `prepared_by_id` registry now reflect the exact
// state as of THIS event's own ordinal, not whatever the old fixed pipeline
// had already produced by the time that operation kind's own pass ran.
// ============================================================================

/// One raw activity event, tagged with the real `operation_ordinal` SQLite
/// already assigned it. See the module-level comment above for the overall
/// design.
enum ActionActivityEvent {
    Start {
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        action_id: String,
        expected_version: i64,
    },
    Link {
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        action_id: String,
        expected_version: i64,
        evidence_id: String,
        evidence_classification: String,
    },
    PrepareLower {
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        action_id: String,
        expected_version: i64,
        proposed_classification: String,
        rationale: String,
        result_reference: String,
    },
    ExecuteLower {
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        action_id: String,
        prepared_intent_id: String,
        approval_receipt_id: String,
    },
    PrepareTransition {
        kind: ActionPersistenceTransitionKind,
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        result_reference: String,
    },
    PrepareTransitionH3 {
        kind: ActionPersistenceTransitionKind,
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
    },
    /// v45: a rejection of a Complete/Cancel/Reopen preview. Everything
    /// else about it is decoded from its own row against the intent that
    /// is pending at this ordinal (`decode_prepared_intent_rejection`).
    RejectPrepared {
        ordinal: i64,
        prepared_intent_id: String,
    },
    ExecuteTransition {
        kind: ActionPersistenceTransitionKind,
        idempotency_id: String,
        correlation_id: String,
        ordinal: i64,
        result_kind: String,
        prepared_id_text: String,
        disposition_text: String,
        terminal_cause: Option<String>,
        attempted_digest: Option<String>,
        error_code: Option<String>,
        error_message_key: Option<String>,
        error_correlation_id: Option<String>,
        error_retryable: Option<i64>,
        private_detail_ref: Option<String>,
        actor: String,
        acknowledged_digest: String,
    },
}

impl ActionActivityEvent {
    const fn ordinal(&self) -> i64 {
        match self {
            Self::Start { ordinal, .. }
            | Self::Link { ordinal, .. }
            | Self::PrepareLower { ordinal, .. }
            | Self::ExecuteLower { ordinal, .. }
            | Self::PrepareTransition { ordinal, .. }
            | Self::PrepareTransitionH3 { ordinal, .. }
            | Self::RejectPrepared { ordinal, .. }
            | Self::ExecuteTransition { ordinal, .. } => *ordinal,
        }
    }
}

/// Gathers every Action activity event across all thirteen underlying
/// sources, in no particular order -- `fold_action_activity_events` sorts
/// them by `operation_ordinal` afterward. Purely mechanical: identical
/// `SELECT`s to the ones the old per-operation decoders used.
fn gather_action_activity_events(
    tx: &Transaction<'_>,
) -> Result<Vec<ActionActivityEvent>, ActionPersistenceLoadError> {
    let mut events = Vec::new();

    let start_rows = tx
        .prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,command.action_id,command.expected_version FROM action_replay_operations replay JOIN action_command_start_actions command USING(idempotency_id) WHERE replay.operation='start_action' AND replay.result_kind='action'")
        .map_err(decode_sqlite_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (idempotency_id, correlation_id, ordinal, action_id, expected_version) in start_rows {
        events.push(ActionActivityEvent::Start {
            idempotency_id,
            correlation_id,
            ordinal,
            action_id,
            expected_version,
        });
    }

    let link_rows = tx
        .prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,command.action_id,command.expected_version,command.evidence_id,command.evidence_classification FROM action_replay_operations replay JOIN action_command_link_completion_evidence command USING(idempotency_id) WHERE replay.operation='link_completion_evidence' AND replay.result_kind='action'")
        .map_err(decode_sqlite_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (
        idempotency_id,
        correlation_id,
        ordinal,
        action_id,
        expected_version,
        evidence_id,
        evidence_classification,
    ) in link_rows
    {
        events.push(ActionActivityEvent::Link {
            idempotency_id,
            correlation_id,
            ordinal,
            action_id,
            expected_version,
            evidence_id,
            evidence_classification,
        });
    }

    let lower_prepares = tx
        .prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.result_reference,command.action_id,command.expected_version,command.proposed_classification,command.rationale FROM action_h2a_lower_classification_prepare_replay_operations replay JOIN action_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.operation='prepare_lower_classification' AND replay.result_kind='prepared'")
        .map_err(decode_sqlite_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (
        idempotency_id,
        correlation_id,
        ordinal,
        result_reference,
        action_id,
        expected_version,
        proposed_classification,
        rationale,
    ) in lower_prepares
    {
        events.push(ActionActivityEvent::PrepareLower {
            idempotency_id,
            correlation_id,
            ordinal,
            action_id,
            expected_version,
            proposed_classification,
            rationale,
            result_reference,
        });
    }

    let lower_executes = tx
        .prepare("SELECT idempotency_id,correlation_id,operation_ordinal,action_id,prepared_intent_id,approval_receipt_id FROM action_h2a_lower_classification_execute_replay_operations WHERE operation='execute_lower_classification' AND result_kind='lowered'")
        .map_err(decode_sqlite_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (
        idempotency_id,
        correlation_id,
        ordinal,
        action_id,
        prepared_intent_id,
        approval_receipt_id,
    ) in lower_executes
    {
        events.push(ActionActivityEvent::ExecuteLower {
            idempotency_id,
            correlation_id,
            ordinal,
            action_id,
            prepared_intent_id,
            approval_receipt_id,
        });
    }

    gather_transition_events(
        tx,
        ActionPersistenceTransitionKind::Complete,
        "prepare_complete",
        "complete",
        &mut events,
    )?;
    gather_transition_events(
        tx,
        ActionPersistenceTransitionKind::Cancel,
        "prepare_cancel",
        "cancel",
        &mut events,
    )?;
    gather_transition_events(
        tx,
        ActionPersistenceTransitionKind::Reopen,
        "prepare_reopen",
        "reopen",
        &mut events,
    )?;

    // v45: rejections of Complete/Cancel/Reopen previews join the same
    // globally-ordered fold (Accept rejections are decoded with their
    // request, next to Accept's own execute rows).
    let rejection_rows = tx
        .prepare("SELECT rejection.operation_ordinal,rejection.prepared_intent_id FROM action_reject_prepared_command_results AS rejection JOIN prepared_intents AS intent ON intent.id=rejection.prepared_intent_id WHERE intent.intent_kind IN ('complete_action','cancel_action','reopen_action') ORDER BY rejection.operation_ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (ordinal, prepared_intent_id) in rejection_rows {
        events.push(ActionActivityEvent::RejectPrepared {
            ordinal,
            prepared_intent_id,
        });
    }
    Ok(events)
}

/// Shared gather body for Complete/Cancel/Reopen's PREPARE-success,
/// PREPARE-H3-denial, and EXECUTE rows -- the three share byte-for-byte
/// identical column shapes across all three operations (only the
/// `operation`/`cmd.kind` filter values differ), so this one parameterized
/// function replaces what were nine near-identical query blocks. Purely
/// mechanical -- no parsing, validation, or business logic happens here,
/// exactly as before; `prepare_operation`/`execute_kind` are always one of
/// three fixed literals supplied by `gather_action_activity_events` above,
/// never external input.
fn gather_transition_events(
    tx: &Transaction<'_>,
    kind: ActionPersistenceTransitionKind,
    prepare_operation: &str,
    execute_kind: &str,
    events: &mut Vec<ActionActivityEvent>,
) -> Result<(), ActionPersistenceLoadError> {
    let prepared_rows = tx
        .prepare("SELECT idempotency_id,correlation_id,operation_ordinal,result_reference FROM action_replay_operations WHERE operation=?1 AND result_kind='prepared'")
        .map_err(decode_sqlite_error)?
        .query_map([prepare_operation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (idempotency_id, correlation_id, ordinal, result_reference) in prepared_rows {
        events.push(ActionActivityEvent::PrepareTransition {
            kind,
            idempotency_id,
            correlation_id,
            ordinal,
            result_reference,
        });
    }

    let h3_rows = tx
        .prepare("SELECT idempotency_id,correlation_id,operation_ordinal FROM action_replay_operations WHERE operation=?1 AND result_kind='terminal'")
        .map_err(decode_sqlite_error)?
        .query_map([prepare_operation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (idempotency_id, correlation_id, ordinal) in h3_rows {
        events.push(ActionActivityEvent::PrepareTransitionH3 {
            kind,
            idempotency_id,
            correlation_id,
            ordinal,
        });
    }

    let execute_rows = tx
        .prepare("SELECT replay.idempotency_id,replay.correlation_id,replay.operation_ordinal,replay.result_kind,replay.prepared_intent_id,replay.prepared_disposition,replay.terminal_cause,replay.attempted_digest,replay.error_code,replay.error_message_key,replay.error_correlation_id,replay.error_retryable,replay.private_detail_ref,cmd.actor,cmd.acknowledged_digest FROM action_replay_operations replay JOIN action_command_execute_actions cmd ON cmd.idempotency_id=replay.idempotency_id WHERE replay.operation='execute_action' AND cmd.kind=?1")
        .map_err(decode_sqlite_error)?
        .query_map([execute_kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, String>(13)?,
                row.get::<_, String>(14)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    for (
        idempotency_id,
        correlation_id,
        ordinal,
        result_kind,
        prepared_id_text,
        disposition_text,
        terminal_cause,
        attempted_digest,
        error_code,
        error_message_key,
        error_correlation_id,
        error_retryable,
        private_detail_ref,
        actor,
        acknowledged_digest,
    ) in execute_rows
    {
        events.push(ActionActivityEvent::ExecuteTransition {
            kind,
            idempotency_id,
            correlation_id,
            ordinal,
            result_kind,
            prepared_id_text,
            disposition_text,
            terminal_cause,
            attempted_digest,
            error_code,
            error_message_key,
            error_correlation_id,
            error_retryable,
            private_detail_ref,
            actor,
            acknowledged_digest,
        });
    }
    Ok(())
}

/// Folds every gathered Action activity event in true global-ordinal order
/// -- so activity replays in the order it happened, not a fixed per-kind
/// order. Sorts `gather_action_activity_events`'s
/// output once, then applies each event via the `apply_*` functions below,
/// each of which is otherwise UNCHANGED from the pre-redesign per-operation
/// decoder's own per-row body. `prepared_by_id` is now ONE shared registry
/// across Lower/Complete/Cancel/Reopen (previously four separate maps, one
/// per operation kind, each built and drained within its own fixed pass) --
/// safe because every `PreparedIntentId` is globally unique by construction,
/// never reused across operation kinds.
fn fold_action_activity_events(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    prepared_intents: &mut Vec<WorkManagementPreparedIntent>,
    discarded_prepared: &mut Vec<WorkManagementPreparedIntent>,
    audits: &mut Vec<(u64, AuditEvent)>,
) -> Result<(), ActionPersistenceLoadError> {
    let mut events = gather_action_activity_events(tx)?;
    events.sort_by_key(ActionActivityEvent::ordinal);

    let mut prepared_by_id: std::collections::HashMap<
        PreparedIntentId,
        WorkManagementPreparedIntent,
    > = std::collections::HashMap::new();

    for event in events {
        match event {
            ActionActivityEvent::Start {
                idempotency_id,
                correlation_id,
                ordinal,
                action_id,
                expected_version,
            } => {
                apply_start_action_event(
                    tx,
                    accepted_inputs,
                    actions,
                    replay,
                    audits,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                    action_id,
                    expected_version,
                )?;
            }
            ActionActivityEvent::Link {
                idempotency_id,
                correlation_id,
                ordinal,
                action_id,
                expected_version,
                evidence_id,
                evidence_classification,
            } => {
                apply_link_completion_evidence_event(
                    tx,
                    accepted_inputs,
                    actions,
                    replay,
                    audits,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                    action_id,
                    expected_version,
                    evidence_id,
                    evidence_classification,
                )?;
            }
            ActionActivityEvent::PrepareLower {
                idempotency_id,
                correlation_id,
                ordinal,
                action_id,
                expected_version,
                proposed_classification,
                rationale,
                result_reference,
            } => {
                apply_prepare_lower_event(
                    tx,
                    actions,
                    replay,
                    prepared_intents,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                    action_id,
                    expected_version,
                    proposed_classification,
                    rationale,
                    result_reference,
                )?;
            }
            ActionActivityEvent::ExecuteLower {
                idempotency_id,
                correlation_id,
                ordinal,
                action_id,
                prepared_intent_id,
                approval_receipt_id,
            } => {
                apply_execute_lower_event(
                    tx,
                    actions,
                    replay,
                    prepared_intents,
                    audits,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                    action_id,
                    prepared_intent_id,
                    approval_receipt_id,
                )?;
            }
            ActionActivityEvent::PrepareTransition {
                kind,
                idempotency_id,
                correlation_id,
                ordinal,
                result_reference,
            } => {
                apply_prepare_transition_event(
                    tx,
                    accepted_inputs,
                    replay,
                    &mut prepared_by_id,
                    kind,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                    result_reference,
                )?;
            }
            ActionActivityEvent::PrepareTransitionH3 {
                kind,
                idempotency_id,
                correlation_id,
                ordinal,
            } => {
                apply_prepare_transition_h3_event(
                    tx,
                    accepted_inputs,
                    replay,
                    audits,
                    kind,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                )?;
            }
            ActionActivityEvent::RejectPrepared {
                ordinal,
                prepared_intent_id,
            } => {
                let prepared_id = PreparedIntentId::parse(prepared_intent_id)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
                let Some(intent) = prepared_by_id.remove(&prepared_id) else {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                };
                let Some((rejection, audit)) = decode_prepared_intent_rejection(tx, &intent)?
                else {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                };
                if i64::try_from(rejection.operation_ordinal())
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?
                    != ordinal
                {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                }
                audits.push((rejection.operation_ordinal(), audit));
                replay.push(rejection);
                discarded_prepared.push(intent);
            }
            ActionActivityEvent::ExecuteTransition {
                kind,
                idempotency_id,
                correlation_id,
                ordinal,
                result_kind,
                prepared_id_text,
                disposition_text,
                terminal_cause,
                attempted_digest,
                error_code,
                error_message_key,
                error_correlation_id,
                error_retryable,
                private_detail_ref,
                actor,
                acknowledged_digest,
            } => {
                apply_execute_transition_event(
                    tx,
                    accepted_inputs,
                    actions,
                    replay,
                    &mut prepared_by_id,
                    discarded_prepared,
                    audits,
                    kind,
                    idempotency_id,
                    correlation_id,
                    ordinal,
                    result_kind,
                    prepared_id_text,
                    disposition_text,
                    terminal_cause,
                    attempted_digest,
                    error_code,
                    error_message_key,
                    error_correlation_id,
                    error_retryable,
                    private_detail_ref,
                    actor,
                    acknowledged_digest,
                )?;
            }
        }
    }

    prepared_intents.extend(prepared_by_id.into_values());
    Ok(())
}

/// Applies one `StartAction` event -- unchanged from the old
/// `decode_start_action_activity`'s own per-row body, except `previous` is
/// read live from `actions` at this call (always correct now, since the
/// fold calls this at the event's own true ordinal position) instead of
/// that function's own now-removed fixed-pass ordering assumption.
#[allow(clippy::too_many_arguments)]
fn apply_start_action_event(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
    action_id: String,
    expected_version: i64,
) -> Result<(), ActionPersistenceLoadError> {
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_id = ActionId::parse(action_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let expected_version = AggregateVersion::new(
        u64::try_from(expected_version)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if !accepted_inputs.contains_key(&action_id) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let index = actions
        .iter()
        .position(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation_id {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([audit_row.0.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0]
            != (
                0,
                "action.started".to_owned(),
                "complete".to_owned(),
                "action".to_owned(),
                action_id.as_str().to_owned(),
            )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(audit_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let event_code = AuditEventCode::parse("action.started")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let effect = AuditEffectCode::parse("action.started")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(correlation_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let occurred_at = UtcTimestamp::from_unix_millis(audit_row.1);
    let audit = AuditEvent::new(
        event_id,
        occurred_at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            event_code,
            AuditTarget::Action(action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![effect],
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    );
    let previous = actions[index].clone();
    let next = ActionRecord::from_persisted_started(previous, expected_version, occurred_at)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let outcome = ActionMutationOutcome {
        record: next.clone(),
        audit_events: vec![audit.clone()],
        approval_receipt_id: None,
    };
    replay.push(ActionReplayCapsule::new(
        IdempotencyId::parse(idempotency_id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        correlation,
        ordinal,
        ActionPersistenceCommand::StartAction {
            action_id,
            expected_version,
        },
        ActionPersistenceResult::Action(outcome),
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    actions[index] = next;
    Ok(())
}

/// Applies one `LinkActionCompletionEvidence` event -- unchanged from the
/// old `decode_link_completion_evidence_activity`'s own per-row body except
/// `previous` is now always read live (see `apply_start_action_event`'s
/// doc comment).
#[allow(clippy::too_many_arguments)]
fn apply_link_completion_evidence_event(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
    action_id: String,
    expected_version: i64,
    evidence_id: String,
    evidence_classification: String,
) -> Result<(), ActionPersistenceLoadError> {
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_id = ActionId::parse(action_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let expected_version = AggregateVersion::new(
        u64::try_from(expected_version)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let evidence_id = EvidenceReferenceId::parse(evidence_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let evidence_classification = DataClassification::from_persisted(&evidence_classification)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if !accepted_inputs.contains_key(&action_id) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let index = actions
        .iter()
        .position(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation_id {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([audit_row.0.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0]
            != (
                0,
                "action.completion_evidence_linked".to_owned(),
                "complete".to_owned(),
                "action".to_owned(),
                action_id.as_str().to_owned(),
            )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(audit_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let event_code = AuditEventCode::parse("action.completion_evidence_linked")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let effect = AuditEffectCode::parse("action.completion_evidence_linked")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(correlation_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let occurred_at = UtcTimestamp::from_unix_millis(audit_row.1);
    let audit = AuditEvent::new(
        event_id,
        occurred_at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            event_code,
            AuditTarget::Action(action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![effect],
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    );
    let previous = actions[index].clone();
    let next = ActionRecord::from_persisted_completion_evidence_linked(
        previous,
        expected_version,
        evidence_id,
        evidence_classification,
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let outcome = ActionMutationOutcome {
        record: next.clone(),
        audit_events: vec![audit.clone()],
        approval_receipt_id: None,
    };
    replay.push(ActionReplayCapsule::new(
        IdempotencyId::parse(idempotency_id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        correlation,
        ordinal,
        ActionPersistenceCommand::LinkCompletionEvidence {
            action_id,
            expected_version,
            evidence_id: next
                .completion_evidence()
                .last()
                .cloned()
                .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
            evidence_classification,
        },
        ActionPersistenceResult::Action(outcome),
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    actions[index] = next;
    Ok(())
}

/// Applies one Lower Classification PREPARE event. Unlike the old
/// `decode_lower_action_classification_activity`, `tracked_version` is read
/// live from `actions` (the fold's own single source of truth) instead of a
/// separately-maintained `current_version` map seeded once from
/// `accepted_inputs` -- the same class of fix as every other `apply_*`
/// function here.
#[allow(clippy::too_many_arguments)]
fn apply_prepare_lower_event(
    tx: &Transaction<'_>,
    actions: &[ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    prepared_intents: &mut Vec<WorkManagementPreparedIntent>,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
    action_id: String,
    expected_version: i64,
    proposed_classification: String,
    rationale: String,
    result_reference: String,
) -> Result<(), ActionPersistenceLoadError> {
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_id = ActionId::parse(action_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let expected_version = AggregateVersion::new(
        u64::try_from(expected_version)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let tracked = actions
        .iter()
        .find(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if tracked.version() != expected_version {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let tracked_classification = tracked.classification();
    let proposed_classification = DataClassification::from_persisted(&proposed_classification)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let rationale = WorkManagementRationale::parse(rationale)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let prepared_id = PreparedIntentId::parse(result_reference)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let intent = decode_lower_action_classification_prepared(
        tx,
        &prepared_id,
        &action_id,
        expected_version,
        tracked_classification,
        proposed_classification,
        &rationale,
    )?;
    let command = ActionPersistenceCommand::PrepareLowerClassification {
        action_id,
        expected_version,
        proposed_classification,
        rationale,
    };
    replay.push(ActionReplayCapsule::new(
        IdempotencyId::parse(idempotency_id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        CorrelationId::parse(correlation_id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ordinal,
        command,
        ActionPersistenceResult::Prepared(intent.clone()),
        Vec::new(),
    ));
    prepared_intents.push(intent);
    Ok(())
}

/// Applies one Lower Classification EXECUTE event. Unlike the old
/// `decode_lower_action_classification_activity`, this reconstructs the
/// next `ActionRecord` by threading the LIVE `actions[index]` through the
/// new `ActionRecord::from_persisted_classification_lowered` constructor
/// (added for this redesign) instead of always re-deriving a fresh
/// accept-time record via `from_persisted_accepted_request_with_lowered_classification`
/// -- the old function's own final "current_version/current_classification
/// tracked maps, then one closing overwrite loop" mechanism is gone
/// entirely, since `actions[index]` is now always correct at the moment
/// each event applies.
#[allow(clippy::too_many_arguments)]
fn apply_execute_lower_event(
    tx: &Transaction<'_>,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    prepared_intents: &mut Vec<WorkManagementPreparedIntent>,
    audits: &mut Vec<(u64, AuditEvent)>,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
    action_id: String,
    prepared_intent_id: String,
    approval_receipt_id: String,
) -> Result<(), ActionPersistenceLoadError> {
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_id = ActionId::parse(action_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let prepared_id = PreparedIntentId::parse(prepared_intent_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let receipt_id = ApprovalReceiptId::parse(approval_receipt_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (actor, acknowledged_digest) = tx
        .query_row(
            "SELECT actor,acknowledged_digest FROM action_h2a_lower_classification_command_executes WHERE idempotency_id=?1",
            [idempotency_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(decode_sqlite_error)?;
    if actor != "head_of_products" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let proposed_classification: String = tx
        .query_row(
            "SELECT command.proposed_classification FROM action_h2a_lower_classification_prepare_replay_operations replay JOIN action_h2a_lower_classification_command_prepares command USING(idempotency_id) WHERE replay.result_reference=?1",
            [prepared_id.as_str()],
            |row| row.get(0),
        )
        .map_err(decode_sqlite_error)?;
    let proposed_classification = DataClassification::from_persisted(&proposed_classification)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let index = actions
        .iter()
        .position(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let previous = actions[index].clone();
    let expected_version = previous.version();
    let record = ActionRecord::from_persisted_classification_lowered(
        previous,
        expected_version,
        proposed_classification,
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN action_h2a_lower_classification_execute_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation_id {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([&audit_row.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0]
            != (
                0,
                "action.classification_lowered".to_owned(),
                "complete".to_owned(),
                "action".to_owned(),
                action_id.as_str().to_owned(),
            )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(audit_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let event_code = AuditEventCode::parse("action.classification_lowered")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let effect = AuditEffectCode::parse("action.classification_lowered")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(correlation_id.clone())
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit = AuditEvent::new(
        event_id,
        UtcTimestamp::from_unix_millis(audit_row.1),
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            event_code,
            AuditTarget::Action(action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Approved,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![effect],
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    );
    let outcome = ActionMutationOutcome {
        record: record.clone(),
        audit_events: vec![audit.clone()],
        approval_receipt_id: Some(receipt_id),
    };
    let command = ActionPersistenceCommand::ExecuteLowerClassification {
        prepared_id: prepared_id.clone(),
        actor: AuditActor::HeadOfProducts,
        acknowledged_digest:
            pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted(
                acknowledged_digest,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    };
    replay.push(ActionReplayCapsule::new(
        IdempotencyId::parse(idempotency_id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        correlation,
        ordinal,
        command,
        ActionPersistenceResult::Action(outcome),
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    prepared_intents.retain(|intent| intent.id() != &prepared_id);
    actions[index] = record;
    Ok(())
}

/// Applies one Complete/Cancel/Reopen PREPARE-success event -- shared
/// across all three kinds (only the per-row column shape is identical; the
/// command-decode step itself still dispatches to the kind-specific
/// `decode_prepared_{complete,cancel,reopen}`, exactly as the three old
/// separate functions did).
#[allow(clippy::too_many_arguments)]
fn apply_prepare_transition_event(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    replay: &mut Vec<ActionReplayCapsule>,
    prepared_by_id: &mut std::collections::HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    kind: ActionPersistenceTransitionKind,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
    result_reference: String,
) -> Result<(), ActionPersistenceLoadError> {
    let idempotency = IdempotencyId::parse(idempotency_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let prepared_id = PreparedIntentId::parse(result_reference)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (prepared, command) = match kind {
        ActionPersistenceTransitionKind::Complete => {
            decode_prepared_complete(tx, &prepared_id, &idempotency)?
        }
        ActionPersistenceTransitionKind::Cancel => {
            decode_prepared_cancel(tx, &prepared_id, &idempotency)?
        }
        ActionPersistenceTransitionKind::Reopen => {
            decode_prepared_reopen(tx, &prepared_id, &idempotency)?
        }
    };
    let action_id = match &command {
        ActionPersistenceCommand::PrepareComplete { action_id, .. }
        | ActionPersistenceCommand::PrepareCancel { action_id, .. }
        | ActionPersistenceCommand::PrepareReopen { action_id, .. } => action_id,
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    if !accepted_inputs.contains_key(action_id) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    replay.push(ActionReplayCapsule::new(
        idempotency,
        CorrelationId::parse(correlation_id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ordinal,
        command,
        ActionPersistenceResult::Prepared(prepared.clone()),
        Vec::new(),
    ));
    prepared_by_id.insert(prepared.id().clone(), prepared);
    Ok(())
}

/// Applies one Complete/Cancel/Reopen PREPARE H3-denial event -- shared
/// across all three kinds, same relationship to the (already-shared)
/// `decode_prepare_h3_denial_audit`/`decode_h3_denial_cause`
/// helpers as `apply_prepare_transition_event` has to `decode_prepared_*`.
#[allow(clippy::too_many_arguments)]
fn apply_prepare_transition_h3_event(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
    kind: ActionPersistenceTransitionKind,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
) -> Result<(), ActionPersistenceLoadError> {
    let idempotency = IdempotencyId::parse(idempotency_id.clone())
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(correlation_id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let command = match kind {
        ActionPersistenceTransitionKind::Complete => {
            decode_prepare_complete_h3_denial_command(tx, &idempotency)?
        }
        ActionPersistenceTransitionKind::Cancel => {
            decode_prepare_cancel_h3_denial_command(tx, &idempotency)?
        }
        ActionPersistenceTransitionKind::Reopen => {
            decode_prepare_reopen_h3_denial_command(tx, &idempotency)?
        }
    };
    let action_id = match &command {
        ActionPersistenceCommand::PrepareComplete { action_id, .. }
        | ActionPersistenceCommand::PrepareCancel { action_id, .. }
        | ActionPersistenceCommand::PrepareReopen { action_id, .. } => action_id.clone(),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    if !accepted_inputs.contains_key(&action_id) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let row = tx
        .query_row(
            "SELECT terminal_h3_cause,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref FROM action_replay_operations WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let cause = ActionPersistenceTerminalCause::H3Denied(decode_h3_denial_cause(
        row.0
            .as_deref()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )?);
    let error = decode_terminal_error(tx, &idempotency, row.1, row.2, row.3, row.4, row.5)?;
    let audit = decode_prepare_h3_denial_audit(tx, &idempotency, &correlation, &action_id)?;
    replay.push(ActionReplayCapsule::new(
        idempotency,
        correlation,
        ordinal,
        command.clone(),
        ActionPersistenceResult::Terminal {
            command,
            cause,
            error,
            prepared_disposition: PreparedDisposition::NotApplicable,
            audit: audit.clone(),
        },
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    Ok(())
}

/// Applies one Complete/Cancel/Reopen EXECUTE event -- the shared preamble
/// (actor/digest checks, `ActionPersistenceCommand::ExecuteAction` build)
/// and the entire terminal (`result_kind == "terminal"`) branch are
/// byte-for-byte identical across all three kinds in the old code (the only
/// per-kind difference in the terminal branch was which `WorkManagementOperation`
/// variant to match for `target_action_id`, folded into one small match
/// here), so both stay in this one shared function. The success
/// (`result_kind == "action"`) branch has real per-kind differences
/// (target state, classification formula, event code, Reopen's extra mode
/// check) and stays as three separate `apply_execute_{complete,cancel,reopen}_success`
/// functions, matching this file's established practice of not collapsing
/// genuinely different business logic for DRY's sake alone.
#[allow(clippy::too_many_arguments)]
fn apply_execute_transition_event(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    prepared_by_id: &mut std::collections::HashMap<PreparedIntentId, WorkManagementPreparedIntent>,
    discarded_prepared: &mut Vec<WorkManagementPreparedIntent>,
    audits: &mut Vec<(u64, AuditEvent)>,
    kind: ActionPersistenceTransitionKind,
    idempotency_id: String,
    correlation_id: String,
    ordinal: i64,
    result_kind: String,
    prepared_id_text: String,
    disposition_text: String,
    terminal_cause: Option<String>,
    attempted_digest: Option<String>,
    error_code: Option<String>,
    error_message_key: Option<String>,
    error_correlation_id: Option<String>,
    error_retryable: Option<i64>,
    private_detail_ref: Option<String>,
    actor: String,
    acknowledged_digest: String,
) -> Result<(), ActionPersistenceLoadError> {
    if actor != "head_of_products" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let prepared_id = PreparedIntentId::parse(prepared_id_text)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let idempotency = IdempotencyId::parse(idempotency_id.clone())
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(correlation_id.clone())
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let ordinal =
        u64::try_from(ordinal).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let digest = pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted(
        acknowledged_digest.clone(),
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let command = ActionPersistenceCommand::ExecuteAction {
        kind,
        prepared_id: prepared_id.clone(),
        actor: AuditActor::HeadOfProducts,
        acknowledged_digest: digest,
    };

    if result_kind == "terminal" {
        let disposition = match disposition_text.as_str() {
            "retained" => PreparedDisposition::Retained,
            "consumed_and_discarded" => PreparedDisposition::ConsumedAndDiscarded,
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        };
        let prepared = prepared_by_id
            .get(&prepared_id)
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
            .clone();
        let digest_matches = acknowledged_digest == prepared.payload_digest().as_str();
        let cause = decode_terminal_cause(terminal_cause.as_deref(), attempted_digest.as_deref())?;
        if matches!(cause, ActionPersistenceTerminalCause::DigestMismatch { .. }) != !digest_matches
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let error = decode_terminal_error(
            tx,
            &idempotency,
            error_code,
            error_message_key,
            error_correlation_id,
            error_retryable,
            private_detail_ref,
        )?;
        let audit_row = tx
            .query_row(
                "SELECT audit.id,audit.occurred_at,audit.correlation_id,audit.event_code,audit.target_type,audit.target_id,audit.policy_outcome,audit.approval_outcome,audit.execution_outcome FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
                [idempotency_id.as_str()],
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
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .map_err(decode_sqlite_error)?;
        if audit_row.2 != correlation_id || audit_row.4 != "action" {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let target_action_id = match prepared.operation() {
            pmc_domain::work_management::WorkManagementOperation::CompleteAction {
                action_id,
                ..
            }
            | pmc_domain::work_management::WorkManagementOperation::CancelAction {
                action_id,
                ..
            }
            | pmc_domain::work_management::WorkManagementOperation::ReopenAction {
                action_id,
                ..
            } => action_id.clone(),
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        };
        if audit_row.5 != target_action_id.as_str() {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let effects = tx
            .query_row(
                "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1",
                [&audit_row.0],
                |row| row.get::<_, i64>(0),
            )
            .map_err(decode_sqlite_error)?;
        if effects != 0 {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let audit = AuditEvent::new(
            AuditEventId::parse(audit_row.0)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            UtcTimestamp::from_unix_millis(audit_row.1),
            AuditActor::HeadOfProducts,
            AuditAction::new(
                AuditModule::WorkManagement,
                AuditEventCode::parse(audit_row.3)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                AuditTarget::Action(target_action_id),
            ),
            correlation.clone(),
            AuditDisposition::new(
                AuditPolicyOutcome::from_persisted(&audit_row.6)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                AuditApprovalOutcome::from_persisted(&audit_row.7)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                AuditExecutionOutcome::from_persisted(&audit_row.8)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                AuditEffectScope::None,
                Vec::new(),
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        );
        let discarded_row = tx
            .query_row(
                "SELECT 1 FROM action_discarded_prepared_intents WHERE prepared_intent_id=?1 AND idempotency_id=?2",
                rusqlite::params![prepared_id.as_str(), idempotency_id.as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(decode_sqlite_error)?
            .is_some();
        let consumed_at = tx
            .query_row(
                "SELECT consumed_at FROM prepared_intents WHERE id=?1",
                [prepared_id.as_str()],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(decode_sqlite_error)?;
        match disposition {
            PreparedDisposition::ConsumedAndDiscarded => {
                if !discarded_row || consumed_at != Some(audit.occurred_at().unix_millis()) {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                }
                let removed = prepared_by_id
                    .remove(&prepared_id)
                    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
                discarded_prepared.push(removed);
            }
            PreparedDisposition::Retained => {
                if discarded_row || consumed_at.is_some() {
                    return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
                }
            }
            PreparedDisposition::NotApplicable => {
                return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
            }
        }
        replay.push(ActionReplayCapsule::new(
            idempotency,
            correlation,
            ordinal,
            command.clone(),
            ActionPersistenceResult::Terminal {
                command,
                cause,
                error,
                prepared_disposition: disposition,
                audit: audit.clone(),
            },
            vec![audit.id().clone()],
        ));
        audits.push((ordinal, audit));
        return Ok(());
    }

    if result_kind != "action" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let prepared = prepared_by_id
        .remove(&prepared_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if acknowledged_digest != prepared.payload_digest().as_str() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    match kind {
        ActionPersistenceTransitionKind::Complete => apply_execute_complete_success(
            tx,
            accepted_inputs,
            actions,
            replay,
            audits,
            idempotency,
            correlation,
            ordinal,
            command,
            prepared,
            idempotency_id,
            acknowledged_digest,
        ),
        ActionPersistenceTransitionKind::Cancel => apply_execute_cancel_success(
            tx,
            accepted_inputs,
            actions,
            replay,
            audits,
            idempotency,
            correlation,
            ordinal,
            command,
            prepared,
            idempotency_id,
            acknowledged_digest,
        ),
        ActionPersistenceTransitionKind::Reopen => apply_execute_reopen_success(
            tx,
            accepted_inputs,
            actions,
            replay,
            audits,
            idempotency,
            correlation,
            ordinal,
            command,
            prepared,
            idempotency_id,
            acknowledged_digest,
        ),
    }
}

/// Applies one successful Complete EXECUTE -- unchanged from the old
/// `decode_complete_action_activity`'s own per-row body except `previous`
/// is now read live from `actions` (see `apply_start_action_event`'s doc
/// comment) instead of `previous_by_action` (a snapshot captured once at
/// that old function's own entry).
#[allow(clippy::too_many_arguments)]
fn apply_execute_complete_success(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
    idempotency: IdempotencyId,
    correlation: CorrelationId,
    ordinal: u64,
    command: ActionPersistenceCommand,
    prepared: WorkManagementPreparedIntent,
    idempotency_id: String,
    acknowledged_digest: String,
) -> Result<(), ActionPersistenceLoadError> {
    let (action_id, action_version) = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::CompleteAction {
            action_id,
            action_version,
        } => (action_id.clone(), *action_version),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let index = actions
        .iter()
        .position(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let previous = actions[index].clone();
    if previous.version() != action_version || previous.state() != ActionState::InProgress {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt = tx
        .query_row(
            "SELECT id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at FROM approval_receipts WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let prepared_consumed_at = tx
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(decode_sqlite_error)?;
    if receipt.1 != "head_of_products"
        || receipt.2 != acknowledged_digest
        || receipt.3 != idempotency_id
        || receipt.4 < 0
        || receipt.5 != prepared.preview().expires_at().unix_millis()
        || receipt.6 != Some(receipt.4)
        || prepared_consumed_at != Some(receipt.4)
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt_id = ApprovalReceiptId::parse(receipt.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation.as_str() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([&audit_row.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0]
            != (
                0,
                "action.completed".to_owned(),
                "complete".to_owned(),
                "action".to_owned(),
                action_id.as_str().to_owned(),
            )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(audit_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let event_code = AuditEventCode::parse("action.completed")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let effect = AuditEffectCode::parse("action.completed")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let occurred_at = UtcTimestamp::from_unix_millis(audit_row.1);
    let audit = AuditEvent::new(
        event_id,
        occurred_at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            event_code,
            AuditTarget::Action(action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Approved,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![effect],
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    );
    let next_version = previous
        .version()
        .next()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (accept_request, accept_prepared) = accepted_inputs
        .get(&action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let support = prepared
        .preview()
        .support()
        .cloned()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let new_classification = accept_prepared
        .classification()
        .combine(support.classification());
    let mut transition_history = previous.transition_history().to_vec();
    transition_history.push(ActionTransitionRecord::from_persisted(
        previous.state(),
        ActionState::Completed,
        None,
        occurred_at,
        Some(support.clone()),
        Some(receipt_id.clone()),
    ));
    let updated = ActionRecord::from_persisted_accepted_request_with_transition(
        accept_request,
        accept_prepared,
        ActionState::Completed,
        new_classification,
        next_version,
        None,
        transition_history,
        Some(support),
        previous.completion_evidence().to_vec(),
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let outcome = ActionMutationOutcome {
        record: updated.clone(),
        audit_events: vec![audit.clone()],
        approval_receipt_id: Some(receipt_id),
    };
    replay.push(ActionReplayCapsule::new(
        idempotency,
        correlation,
        ordinal,
        command,
        ActionPersistenceResult::Action(outcome),
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    actions[index] = updated;
    Ok(())
}

/// Applies one successful Cancel EXECUTE -- same relationship to the old
/// `decode_cancel_action_activity` as `apply_execute_complete_success` has
/// to `decode_complete_action_activity`.
#[allow(clippy::too_many_arguments)]
fn apply_execute_cancel_success(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
    idempotency: IdempotencyId,
    correlation: CorrelationId,
    ordinal: u64,
    command: ActionPersistenceCommand,
    prepared: WorkManagementPreparedIntent,
    idempotency_id: String,
    acknowledged_digest: String,
) -> Result<(), ActionPersistenceLoadError> {
    let (action_id, action_version, reason) = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::CancelAction {
            action_id,
            action_version,
            reason,
            ..
        } => (action_id.clone(), *action_version, reason.clone()),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let index = actions
        .iter()
        .position(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let previous = actions[index].clone();
    if previous.version() != action_version
        || !matches!(
            previous.state(),
            ActionState::Open | ActionState::InProgress
        )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt = tx
        .query_row(
            "SELECT id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at FROM approval_receipts WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let prepared_consumed_at = tx
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(decode_sqlite_error)?;
    if receipt.1 != "head_of_products"
        || receipt.2 != acknowledged_digest
        || receipt.3 != idempotency_id
        || receipt.4 < 0
        || receipt.5 != prepared.preview().expires_at().unix_millis()
        || receipt.6 != Some(receipt.4)
        || prepared_consumed_at != Some(receipt.4)
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt_id = ApprovalReceiptId::parse(receipt.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation.as_str() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([&audit_row.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0]
            != (
                0,
                "action.cancelled".to_owned(),
                "complete".to_owned(),
                "action".to_owned(),
                action_id.as_str().to_owned(),
            )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(audit_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let event_code = AuditEventCode::parse("action.cancelled")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let effect = AuditEffectCode::parse("action.cancelled")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let occurred_at = UtcTimestamp::from_unix_millis(audit_row.1);
    let audit = AuditEvent::new(
        event_id,
        occurred_at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            event_code,
            AuditTarget::Action(action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Approved,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![effect],
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    );
    let next_version = previous
        .version()
        .next()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (accept_request, accept_prepared) = accepted_inputs
        .get(&action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    // `execute_h2` recomputes the non-Complete classification from
    // `authoritative_action_snapshot` (`commitment_classification` combined
    // with completion-evidence) rather than carrying `previous.classification()`
    // forward, so a Cancel executed against a previously-lowered action
    // reverts to the accept-time classification -- see
    // [[project-lower-classification-persistence-snapshot-unsupported-state-bug]]
    // for why that specific combination cannot actually occur today
    // regardless (a separate, deeper bug, deliberately out of scope for this
    // redesign). `accept_prepared.classification()` is that accept-time
    // value, combined with any linked completion-evidence bindings the
    // EXECUTE's own `prepared.operation()` already carries (trustworthy
    // without a fresh live re-resolution, since EXECUTE fails closed on
    // `PreparedIntentChanged` if a fresh resolution ever disagreed).
    let evidence_classifications = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::CancelAction {
            evidence_classifications,
            ..
        } => evidence_classifications.clone(),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let new_classification = evidence_classifications
        .iter()
        .fold(accept_prepared.classification(), |acc, binding| {
            acc.combine(binding.classification())
        });
    let mut transition_history = previous.transition_history().to_vec();
    transition_history.push(ActionTransitionRecord::from_persisted(
        previous.state(),
        ActionState::Cancelled,
        Some(reason.clone()),
        occurred_at,
        None,
        Some(receipt_id.clone()),
    ));
    let updated = ActionRecord::from_persisted_accepted_request_with_transition(
        accept_request,
        accept_prepared,
        ActionState::Cancelled,
        new_classification,
        next_version,
        Some(reason),
        transition_history,
        None,
        previous.completion_evidence().to_vec(),
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let outcome = ActionMutationOutcome {
        record: updated.clone(),
        audit_events: vec![audit.clone()],
        approval_receipt_id: Some(receipt_id),
    };
    replay.push(ActionReplayCapsule::new(
        idempotency,
        correlation,
        ordinal,
        command,
        ActionPersistenceResult::Action(outcome),
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    actions[index] = updated;
    Ok(())
}

/// Applies one successful Reopen EXECUTE -- same relationship to the old
/// `decode_reopen_action_activity` as `apply_execute_complete_success` has
/// to `decode_complete_action_activity`. `previous.state()`/`mode` legality
/// no longer assumes at most one Cancel/Reopen cycle -- reading `actions`
/// live means `Cancel -> Reopen -> Cancel -> Reopen` now folds correctly.
#[allow(clippy::too_many_arguments)]
fn apply_execute_reopen_success(
    tx: &Transaction<'_>,
    accepted_inputs: &std::collections::HashMap<
        ActionId,
        (ActionRequestRecord, WorkManagementPreparedIntent),
    >,
    actions: &mut [ActionRecord],
    replay: &mut Vec<ActionReplayCapsule>,
    audits: &mut Vec<(u64, AuditEvent)>,
    idempotency: IdempotencyId,
    correlation: CorrelationId,
    ordinal: u64,
    command: ActionPersistenceCommand,
    prepared: WorkManagementPreparedIntent,
    idempotency_id: String,
    acknowledged_digest: String,
) -> Result<(), ActionPersistenceLoadError> {
    let (action_id, action_version, mode, reason) = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::ReopenAction {
            action_id,
            action_version,
            mode,
            reason,
            ..
        } => (action_id.clone(), *action_version, *mode, reason.clone()),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let index = actions
        .iter()
        .position(|record| record.id() == &action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let previous = actions[index].clone();
    if previous.version() != action_version
        || !matches!(
            (previous.state(), mode),
            (ActionState::Completed, ActionReopenMode::ReopenCompleted)
                | (ActionState::Cancelled, ActionReopenMode::RestartCancelled)
        )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt = tx
        .query_row(
            "SELECT id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at FROM approval_receipts WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let prepared_consumed_at = tx
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(decode_sqlite_error)?;
    if receipt.1 != "head_of_products"
        || receipt.2 != acknowledged_digest
        || receipt.3 != idempotency_id
        || receipt.4 < 0
        || receipt.5 != prepared.preview().expires_at().unix_millis()
        || receipt.6 != Some(receipt.4)
        || prepared_consumed_at != Some(receipt.4)
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt_id = ApprovalReceiptId::parse(receipt.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation.as_str() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([&audit_row.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0]
            != (
                0,
                "action.reopened".to_owned(),
                "complete".to_owned(),
                "action".to_owned(),
                action_id.as_str().to_owned(),
            )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(audit_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let event_code = AuditEventCode::parse("action.reopened")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let effect = AuditEffectCode::parse("action.reopened")
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let occurred_at = UtcTimestamp::from_unix_millis(audit_row.1);
    let audit = AuditEvent::new(
        event_id,
        occurred_at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            event_code,
            AuditTarget::Action(action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Approved,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![effect],
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    );
    let next_version = previous
        .version()
        .next()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (accept_request, accept_prepared) = accepted_inputs
        .get(&action_id)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    // Same reasoning as `apply_execute_cancel_success`.
    let evidence_classifications = match prepared.operation() {
        pmc_domain::work_management::WorkManagementOperation::ReopenAction {
            evidence_classifications,
            ..
        } => evidence_classifications.clone(),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let new_classification = evidence_classifications
        .iter()
        .fold(accept_prepared.classification(), |acc, binding| {
            acc.combine(binding.classification())
        });
    let mut transition_history = previous.transition_history().to_vec();
    transition_history.push(ActionTransitionRecord::from_persisted(
        previous.state(),
        ActionState::InProgress,
        Some(reason.clone()),
        occurred_at,
        None,
        Some(receipt_id.clone()),
    ));
    let updated = ActionRecord::from_persisted_accepted_request_with_transition(
        accept_request,
        accept_prepared,
        ActionState::InProgress,
        new_classification,
        next_version,
        Some(reason),
        transition_history,
        None,
        previous.completion_evidence().to_vec(),
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let outcome = ActionMutationOutcome {
        record: updated.clone(),
        audit_events: vec![audit.clone()],
        approval_receipt_id: Some(receipt_id),
    };
    replay.push(ActionReplayCapsule::new(
        idempotency,
        correlation,
        ordinal,
        command,
        ActionPersistenceResult::Action(outcome),
        vec![audit.id().clone()],
    ));
    audits.push((ordinal, audit));
    actions[index] = updated;
    Ok(())
}

/// Complete's sibling of `decode_prepared_cancel` -- same "rebuild and
/// verify the exact stored digest" technique, but Complete's own preview
/// always carries a real `SupportWitness` (unlike Cancel/Reopen's `None`),
/// reconstructed here from `support_judgments`/`action_h2a_support_evidence_snapshots`
/// via the same public `EvidenceOrJudgment::evaluate_evidence_required()`
/// path `prepare_complete_action_cause` itself calls -- mirroring how
/// `decode_prepared_resolve` reconstructs Decision Resolve's own witness in
/// `decision_repository.rs` (`evaluate_evidence_or_judgment()` there;
/// Complete requires evidence specifically, never judgment alone).
fn decode_prepared_complete(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    idempotency: &IdempotencyId,
) -> Result<(WorkManagementPreparedIntent, ActionPersistenceCommand), ActionPersistenceLoadError> {
    let command_row = tx
        .query_row(
            "SELECT action_id,expected_version,judgment_disposition,judgment_actor,judgment_rationale,judgment_classification FROM action_command_prepare_completes WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let action_id = ActionId::parse(command_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_version = AggregateVersion::new(
        u64::try_from(command_row.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if command_row
        .2
        .as_deref()
        .is_some_and(|value| value != "proceed_with_documented_rationale")
        || command_row
            .3
            .as_deref()
            .is_some_and(|value| value != "head_of_products")
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let command_judgment = match (&command_row.2, &command_row.4, &command_row.5) {
        (None, None, None) => None,
        (Some(_), Some(rationale), Some(classification)) => Some(
            pmc_domain::work_management::HumanJudgment::new(
                pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                rationale.clone(),
                DataClassification::from_persisted(classification)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };

    let payload = tx
        .query_row(
            "SELECT contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at FROM prepared_intents WHERE id=?1",
            [prepared_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if payload.0 != 1
        || payload.1 != "complete_action"
        || payload.4 != "allowed"
        || payload.5 != "not_cancellable_after_submit"
        || payload.6 != "head_of_products"
        || payload.8.is_some()
        || payload.9 < payload.10
        || payload.10 < 0
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let support_id = payload
        .7
        .clone()
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let classification = DataClassification::from_persisted(&payload.3)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let created_at = UtcTimestamp::from_unix_millis(payload.10);

    let judgment_rows = tx
        .prepare("SELECT ordinal,rationale,classification FROM support_judgments WHERE support_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([support_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if judgment_rows.len() > 1 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let mut judgments = Vec::with_capacity(judgment_rows.len());
    for (index, (ordinal, rationale, judgment_classification)) in
        judgment_rows.into_iter().enumerate()
    {
        if ordinal != index as i64 {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        judgments.push(
            pmc_domain::work_management::HumanJudgment::new(
                pmc_domain::work_management::HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                rationale,
                DataClassification::from_persisted(&judgment_classification)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        );
    }
    // `action_command_prepare_completes`'s own judgment_* columns are a
    // second, redundant copy of the SAME judgment (needed to reconstruct
    // the typed `PrepareComplete` command below without a support_id
    // round trip) -- cross-check they agree with `support_judgments`
    // rather than silently trusting either alone.
    if command_judgment != judgments.first().cloned() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let evidence_rows = tx
        .prepare("SELECT ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest,evidence_version FROM action_h2a_support_evidence_snapshots WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    let mut evidence = Vec::with_capacity(evidence_rows.len());
    for (
        index,
        (
            ordinal,
            evidence_id,
            evidence_classification,
            role,
            verification,
            last_verified_at,
            integrity_digest,
            evidence_version,
        ),
    ) in evidence_rows.into_iter().enumerate()
    {
        let source_version = AggregateVersion::new(
            u64::try_from(evidence_version)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        if ordinal != index as i64 || role != "action_completion" {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let verification = match (verification.as_str(), last_verified_at, integrity_digest) {
            ("verified", Some(at), Some(digest)) => EvidenceVerification::Verified {
                verified_at: UtcTimestamp::from_unix_millis(at),
                integrity_digest: IntegrityDigest::parse(digest)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            },
            ("observed_unpinned", Some(at), Some(digest)) => {
                EvidenceVerification::ObservedUnpinned {
                    observed_at: UtcTimestamp::from_unix_millis(at),
                    integrity_digest: IntegrityDigest::parse(digest)
                        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                }
            }
            ("degraded_last_verified", Some(at), Some(digest)) => {
                EvidenceVerification::DegradedLastVerified {
                    last_verified_at: UtcTimestamp::from_unix_millis(at),
                    integrity_digest: IntegrityDigest::parse(digest)
                        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                }
            }
            ("unverified", None, None) => EvidenceVerification::Unverified,
            ("integrity_mismatch", None, None) => EvidenceVerification::IntegrityMismatch,
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        };
        evidence.push(EvidenceReferenceMetadata::new(
            EvidenceReferenceId::parse(evidence_id)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            source_version,
            DataClassification::from_persisted(&evidence_classification)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            EvidenceRole::ActionCompletion,
            verification,
        ));
    }
    let support = EvidenceOrJudgment::new(evidence, judgments)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?
        .evaluate_evidence_required()
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;

    let operation = pmc_domain::work_management::WorkManagementOperation::CompleteAction {
        action_id: action_id.clone(),
        action_version,
    };
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        classification,
        Some(support),
        created_at,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if prepared.payload_digest().as_str() != payload.2
        || prepared.classification().as_persisted() != payload.3
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let targets = tx
        .prepare("SELECT ordinal,target_type,target_id,expected_version FROM prepared_intent_targets WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if targets.len() != 1
        || targets[0].0 != 0
        || targets[0].1 != "action"
        || targets[0].2 != action_id.as_str()
        || targets[0].3 != command_row.1
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id FROM prepared_intent_effects WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0].0 != 0
        || effects[0].1 != "action.completed"
        || effects[0].2.as_deref() != Some("action")
        || effects[0].3.as_deref() != Some(action_id.as_str())
        || effects[0].4.is_some()
        || effects[0].5.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let sources = tx
        .prepare("SELECT ordinal,role,source_id,classification FROM prepared_intent_classification_sources WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    let expected_sources = prepared.preview().classification_sources();
    if sources.len() != expected_sources.len()
        || sources.iter().zip(expected_sources.iter()).enumerate().any(
            |(index, (row, expected))| {
                let Ok((expected_role, expected_source_id)) =
                    classification_source_persisted_role(expected.role())
                else {
                    return true;
                };
                row.0 != index as i64
                    || row.1 != expected_role
                    || row.2.as_deref() != expected_source_id
                    || row.3 != expected.classification().as_persisted()
            },
        )
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let payload_row = tx
        .query_row(
            "SELECT primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at FROM prepared_work_management_payloads WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if payload_row.0 != action_id.as_str()
        || payload_row.1 != command_row.1
        || payload_row.2.is_some()
        || payload_row.3.is_some()
        || payload_row.4.is_some()
        || payload_row.5.is_some()
        || payload_row.6.is_some()
        || payload_row.7.is_some()
        || payload_row.8.is_some()
        || payload_row.9.is_some()
        || payload_row.10.is_some()
        || payload_row.11.is_some()
        || payload_row.12.is_some()
        || payload_row.13.is_some()
        || payload_row.14.is_some()
        || payload_row.15.is_some()
        || payload_row.16.is_some()
        || payload_row.17.is_some()
        || payload_row.18.is_some()
        || payload_row.19.is_some()
        || payload_row.20.is_some()
        || payload_row.21.is_some()
        || payload_row.22.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let audit_count = tx
        .query_row(
            "SELECT count(*) FROM action_replay_audits WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(decode_sqlite_error)?;
    if audit_count != 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let command = ActionPersistenceCommand::PrepareComplete {
        action_id,
        expected_version: action_version,
        judgment: command_judgment,
    };
    Ok((prepared, command))
}

/// See `decode_prepared_accept` -- same "rebuild and verify the exact
/// stored digest" technique, but reconstructing a `cancel_action` intent
/// from the shared substrate (`prepared_intents`/`prepared_work_management_payloads`/
/// `prepared_evidence_classifications`) instead of Accept's own tables.
/// Returns the intent together with the typed `PrepareCancel` command so the
/// caller only has to destructure it once.
fn decode_prepared_cancel(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    idempotency: &IdempotencyId,
) -> Result<(WorkManagementPreparedIntent, ActionPersistenceCommand), ActionPersistenceLoadError> {
    let command_row = tx
        .query_row(
            "SELECT action_id,expected_version,reason FROM action_command_prepare_cancels WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let action_id = ActionId::parse(command_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_version = AggregateVersion::new(
        u64::try_from(command_row.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let reason = ActionDetails::parse(command_row.2)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;

    let payload = tx
        .query_row(
            "SELECT contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at FROM prepared_intents WHERE id=?1",
            [prepared_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if payload.0 != 1
        || payload.1 != "cancel_action"
        || payload.4 != "allowed"
        || payload.5 != "not_cancellable_after_submit"
        || payload.6 != "head_of_products"
        || payload.7.is_some()
        || payload.8.is_some()
        || payload.9 < payload.10
        || payload.10 < 0
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let classification = DataClassification::from_persisted(&payload.3)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let created_at = UtcTimestamp::from_unix_millis(payload.10);

    let binding_rows = tx
        .prepare("SELECT ordinal,evidence_id,classification FROM prepared_evidence_classifications WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    let mut evidence_classifications = Vec::with_capacity(binding_rows.len());
    for (index, (ordinal, evidence_id, evidence_classification)) in
        binding_rows.into_iter().enumerate()
    {
        if ordinal != index as i64 {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        evidence_classifications.push(EvidenceClassificationBinding::new(
            EvidenceReferenceId::parse(evidence_id)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            DataClassification::from_persisted(&evidence_classification)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ));
    }

    let operation = pmc_domain::work_management::WorkManagementOperation::CancelAction {
        action_id: action_id.clone(),
        action_version,
        reason: reason.clone(),
        evidence_classifications,
    };
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        classification,
        None,
        created_at,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if prepared.payload_digest().as_str() != payload.2
        || prepared.classification().as_persisted() != payload.3
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let targets = tx
        .prepare("SELECT ordinal,target_type,target_id,expected_version FROM prepared_intent_targets WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if targets.len() != 1
        || targets[0].0 != 0
        || targets[0].1 != "action"
        || targets[0].2 != action_id.as_str()
        || targets[0].3 != command_row.1
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id FROM prepared_intent_effects WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0].0 != 0
        || effects[0].1 != "action.cancelled"
        || effects[0].2.as_deref() != Some("action")
        || effects[0].3.as_deref() != Some(action_id.as_str())
        || effects[0].4.is_some()
        || effects[0].5.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let sources = tx
        .prepare("SELECT ordinal,role,source_id,classification FROM prepared_intent_classification_sources WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    // 2026-09: `sources` is no longer always a lone `primary_target` row --
    // see `persist_prepare_cancel_prepared`'s own comment. `prepared` (built
    // above from the already-parsed `evidence_classifications` bindings)
    // deterministically derives the full expected source list, so validate
    // against that instead of a hardcoded single-row shape.
    let expected_sources = prepared.preview().classification_sources();
    if sources.len() != expected_sources.len()
        || sources
            .iter()
            .zip(expected_sources.iter())
            .enumerate()
            .any(|(index, (row, expected))| {
                let (expected_role, expected_source_id): (&str, Option<&str>) =
                    match expected.role() {
                        pmc_domain::work_management::WorkManagementClassificationSourceRole::PrimaryTarget => {
                            ("primary_target", None)
                        }
                        pmc_domain::work_management::WorkManagementClassificationSourceRole::Evidence(id) => {
                            ("evidence", Some(id.as_str()))
                        }
                        _ => return true,
                    };
                row.0 != index as i64
                    || row.1 != expected_role
                    || row.2.as_deref() != expected_source_id
                    || row.3 != expected.classification().as_persisted()
            })
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let payload_row = tx
        .query_row(
            "SELECT primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at FROM prepared_work_management_payloads WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if payload_row.0 != action_id.as_str()
        || payload_row.1 != command_row.1
        || payload_row.2.is_some()
        || payload_row.3.is_some()
        || payload_row.4.is_some()
        || payload_row.5.as_deref() != Some(reason.as_str())
        || payload_row.6.is_some()
        || payload_row.7.is_some()
        || payload_row.8.is_some()
        || payload_row.9.is_some()
        || payload_row.10.is_some()
        || payload_row.11.is_some()
        || payload_row.12.is_some()
        || payload_row.13.is_some()
        || payload_row.14.is_some()
        || payload_row.15.is_some()
        || payload_row.16.is_some()
        || payload_row.17.is_some()
        || payload_row.18.is_some()
        || payload_row.19.is_some()
        || payload_row.20.is_some()
        || payload_row.21.is_some()
        || payload_row.22.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let audit_count = tx
        .query_row(
            "SELECT count(*) FROM action_replay_audits WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(decode_sqlite_error)?;
    if audit_count != 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let command = ActionPersistenceCommand::PrepareCancel {
        action_id,
        expected_version: action_version,
        reason,
    };
    Ok((prepared, command))
}

/// Reopen's sibling of `decode_prepared_cancel` -- same shared-substrate
/// shape, reading the extra `mode` column `action_command_prepare_reopens`
/// carries that `action_command_prepare_cancels` does not.
fn decode_prepared_reopen(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    idempotency: &IdempotencyId,
) -> Result<(WorkManagementPreparedIntent, ActionPersistenceCommand), ActionPersistenceLoadError> {
    let command_row = tx
        .query_row(
            "SELECT action_id,expected_version,mode,reason FROM action_command_prepare_reopens WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    let action_id = ActionId::parse(command_row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_version = AggregateVersion::new(
        u64::try_from(command_row.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mode = match command_row.2.as_str() {
        "reopen_completed" => ActionReopenMode::ReopenCompleted,
        "restart_cancelled" => ActionReopenMode::RestartCancelled,
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let reason = ActionDetails::parse(command_row.3)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;

    let payload = tx
        .query_row(
            "SELECT contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at FROM prepared_intents WHERE id=?1",
            [prepared_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if payload.0 != 1
        || payload.1 != "reopen_action"
        || payload.4 != "allowed"
        || payload.5 != "not_cancellable_after_submit"
        || payload.6 != "head_of_products"
        || payload.7.is_some()
        || payload.8.is_some()
        || payload.9 < payload.10
        || payload.10 < 0
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let classification = DataClassification::from_persisted(&payload.3)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let created_at = UtcTimestamp::from_unix_millis(payload.10);

    let binding_rows = tx
        .prepare("SELECT ordinal,evidence_id,classification FROM prepared_evidence_classifications WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared_id.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    let mut evidence_classifications = Vec::with_capacity(binding_rows.len());
    for (index, (ordinal, evidence_id, evidence_classification)) in
        binding_rows.into_iter().enumerate()
    {
        if ordinal != index as i64 {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        evidence_classifications.push(EvidenceClassificationBinding::new(
            EvidenceReferenceId::parse(evidence_id)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            DataClassification::from_persisted(&evidence_classification)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ));
    }

    let operation = pmc_domain::work_management::WorkManagementOperation::ReopenAction {
        action_id: action_id.clone(),
        action_version,
        mode,
        reason: reason.clone(),
        evidence_classifications,
    };
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        classification,
        None,
        created_at,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if prepared.payload_digest().as_str() != payload.2
        || prepared.classification().as_persisted() != payload.3
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let targets = tx
        .prepare("SELECT ordinal,target_type,target_id,expected_version FROM prepared_intent_targets WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if targets.len() != 1
        || targets[0].0 != 0
        || targets[0].1 != "action"
        || targets[0].2 != action_id.as_str()
        || targets[0].3 != command_row.1
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id FROM prepared_intent_effects WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0].0 != 0
        || effects[0].1 != "action.reopened"
        || effects[0].2.as_deref() != Some("action")
        || effects[0].3.as_deref() != Some(action_id.as_str())
        || effects[0].4.is_some()
        || effects[0].5.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let sources = tx
        .prepare("SELECT ordinal,role,source_id,classification FROM prepared_intent_classification_sources WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    // 2026-09: `sources` is no longer always a lone `primary_target` row --
    // see `persist_prepare_cancel_prepared`'s own comment (Reopen mirrors
    // it exactly). `prepared` (built above from the already-parsed
    // `evidence_classifications` bindings) deterministically derives the
    // full expected source list, so validate against that instead of a
    // hardcoded single-row shape.
    let expected_sources = prepared.preview().classification_sources();
    if sources.len() != expected_sources.len()
        || sources
            .iter()
            .zip(expected_sources.iter())
            .enumerate()
            .any(|(index, (row, expected))| {
                let (expected_role, expected_source_id): (&str, Option<&str>) =
                    match expected.role() {
                        pmc_domain::work_management::WorkManagementClassificationSourceRole::PrimaryTarget => {
                            ("primary_target", None)
                        }
                        pmc_domain::work_management::WorkManagementClassificationSourceRole::Evidence(id) => {
                            ("evidence", Some(id.as_str()))
                        }
                        _ => return true,
                    };
                row.0 != index as i64
                    || row.1 != expected_role
                    || row.2.as_deref() != expected_source_id
                    || row.3 != expected.classification().as_persisted()
            })
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let payload_row = tx
        .query_row(
            "SELECT primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at FROM prepared_work_management_payloads WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if payload_row.0 != action_id.as_str()
        || payload_row.1 != command_row.1
        || payload_row.2.is_some()
        || payload_row.3.is_some()
        || payload_row.4.is_some()
        || payload_row.5.as_deref() != Some(reason.as_str())
        || payload_row.6.is_some()
        || payload_row.7.is_some()
        || payload_row.8.is_some()
        || payload_row.9.is_some()
        || payload_row.10.is_some()
        || payload_row.11.is_some()
        || payload_row.12.is_some()
        || payload_row.13.as_deref() != Some(command_row.2.as_str())
        || payload_row.14.is_some()
        || payload_row.15.is_some()
        || payload_row.16.is_some()
        || payload_row.17.is_some()
        || payload_row.18.is_some()
        || payload_row.19.is_some()
        || payload_row.20.is_some()
        || payload_row.21.is_some()
        || payload_row.22.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let audit_count = tx
        .query_row(
            "SELECT count(*) FROM action_replay_audits WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(decode_sqlite_error)?;
    if audit_count != 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let command = ActionPersistenceCommand::PrepareReopen {
        action_id,
        expected_version: action_version,
        mode,
        reason,
    };
    Ok((prepared, command))
}

/// See `decode_prepared_accept` -- same "rebuild and verify the exact
/// stored digest" technique, but for `LowerActionClassification`'s much
/// simpler shape (no support, no separate `prepared_work_management_payloads`
/// row: `action_h2a_lower_classification_command_prepares` already carries
/// every scalar this operation needs except the point-in-time
/// `current_classification`, which the caller derives locally).
fn decode_lower_action_classification_prepared(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    action_id: &ActionId,
    action_version: AggregateVersion,
    current_classification: DataClassification,
    proposed_classification: DataClassification,
    rationale: &WorkManagementRationale,
) -> Result<WorkManagementPreparedIntent, ActionPersistenceLoadError> {
    let intent = tx
        .query_row(
            "SELECT payload_digest,classification,expires_at,created_at,support_id FROM prepared_intents WHERE id=?1 AND contract_version=1 AND intent_kind='lower_action_classification' AND policy='allowed' AND cancellation_policy='not_cancellable_after_submit' AND authority='head_of_products'",
            [prepared_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if intent.2 < intent.3 || intent.4.is_some() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let operation = WorkManagementOperation::LowerActionClassification {
        action_id: action_id.clone(),
        action_version,
        current_classification,
        proposed_classification,
        rationale: rationale.clone(),
    };
    let rebuilt = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        operation,
        current_classification,
        None,
        UtcTimestamp::from_unix_millis(intent.3),
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if rebuilt.payload_digest().as_str() != intent.0
        || rebuilt.classification().as_persisted() != intent.1
        || rebuilt.preview().expires_at().unix_millis() != intent.2
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    Ok(rebuilt)
}

fn replay_row(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
    operation: &str,
) -> Result<(String, CorrelationId, u64), ActionPersistenceLoadError> {
    let row = tx.query_row(
        "SELECT result_reference,correlation_id,operation_ordinal,result_kind,prepared_intent_id,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable,attempted_digest,private_detail_ref FROM action_replay_operations WHERE idempotency_id=?1 AND operation=?2",
        rusqlite::params![idempotency.as_str(), operation],
        |row| Ok((row.get::<_,Option<String>>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,Option<String>>(8)?,row.get::<_,Option<String>>(9)?,row.get::<_,Option<i64>>(10)?,row.get::<_,Option<String>>(11)?,row.get::<_,Option<String>>(12)?)),
    ).map_err(decode_sqlite_error)?;
    if row.3 != "request"
        || row.0.is_none()
        || row.4.is_some()
        || row.5 != "not_applicable"
        || row.6.is_some()
        || row.7.is_some()
        || row.8.is_some()
        || row.9.is_some()
        || row.10.is_some()
        || row.11.is_some()
        || row.12.is_some()
        || row.2 < 0
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let reference = row
        .0
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(row.1)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok((
        reference,
        correlation,
        u64::try_from(row.2).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    ))
}

/// The audit target a rejection of this intent records (mirrors the
/// domain's own rule: Action Request for Accept, Action for the three
/// transitions; nothing else is rejectable here).
fn rejection_target(intent: &WorkManagementPreparedIntent) -> Option<AuditTarget> {
    match intent.operation() {
        WorkManagementOperation::AcceptActionRequest { request_id, .. } => {
            Some(AuditTarget::ActionRequest(request_id.clone()))
        }
        WorkManagementOperation::CompleteAction { action_id, .. }
        | WorkManagementOperation::CancelAction { action_id, .. }
        | WorkManagementOperation::ReopenAction { action_id, .. } => {
            Some(AuditTarget::Action(action_id.clone()))
        }
        _ => None,
    }
}

/// Decodes the v45 rejection of `intent`, if one was recorded: rebuilds the
/// one legal audit through the domain's own constructor and requires the
/// stored audit row to agree with it column for column (and to carry no
/// effects), and requires the intent's `consumed_at` to be the rejection
/// instant. Returns the replay capsule and its audit.
fn decode_prepared_intent_rejection(
    tx: &Transaction<'_>,
    intent: &WorkManagementPreparedIntent,
) -> Result<Option<(ActionReplayCapsule, AuditEvent)>, ActionPersistenceLoadError> {
    let Some(row) = tx
        .query_row(
            "SELECT idempotency_id,correlation_id,operation_ordinal,rejected_at,audit_event_id,actor FROM action_reject_prepared_command_results WHERE prepared_intent_id=?1",
            [intent.id().as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(decode_sqlite_error)?
    else {
        return Ok(None);
    };
    if row.5 != "head_of_products" || row.2 < 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let target =
        rejection_target(intent).ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(row.1)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let rejected_at = UtcTimestamp::from_unix_millis(row.3);
    let audit_id = AuditEventId::parse(row.4)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let audit = prepared_intent_rejection_audit(
        audit_id.clone(),
        rejected_at,
        ACTION_PREPARED_REJECTED_AUDIT_CODE,
        target.clone(),
        correlation.clone(),
    )
    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let (target_type, target_id) = match &target {
        AuditTarget::ActionRequest(id) => ("action_request", id.as_str()),
        AuditTarget::Action(id) => ("action", id.as_str()),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let agreeing_rows: i64 = tx
        .query_row(
            "SELECT count(*) FROM audit_events WHERE id=?1 AND occurred_at=?2 AND actor='head_of_products' AND module='work_management' AND event_code=?3 AND target_type=?4 AND target_id=?5 AND correlation_id=?6 AND policy_outcome='allowed' AND approval_outcome='rejected' AND execution_outcome='not_attempted' AND effect_scope='none' AND NOT EXISTS(SELECT 1 FROM audit_effects WHERE audit_event_id=?1)",
            rusqlite::params![
                audit_id.as_str(),
                rejected_at.unix_millis(),
                ACTION_PREPARED_REJECTED_AUDIT_CODE,
                target_type,
                target_id,
                correlation.as_str(),
            ],
            |row| row.get(0),
        )
        .map_err(decode_sqlite_error)?;
    let consumed_at: Option<i64> = tx
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [intent.id().as_str()],
            |row| row.get(0),
        )
        .map_err(decode_sqlite_error)?;
    if agreeing_rows != 1 || consumed_at != Some(row.3) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let outcome = RejectedPreparedIntentOutcome::new(
        intent.id().clone(),
        rejected_at,
        rejected_at >= intent.preview().expires_at(),
        audit.clone(),
    );
    let capsule = ActionReplayCapsule::new(
        IdempotencyId::parse(row.0)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        correlation,
        u64::try_from(row.2).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ActionPersistenceCommand::RejectPrepared {
            prepared_id: intent.id().clone(),
            actor: AuditActor::HeadOfProducts,
        },
        ActionPersistenceResult::Rejected(outcome),
        vec![audit_id],
    );
    Ok(Some((capsule, audit)))
}

/// Writes the three rows a rejection is made of, in the order the binding
/// trigger needs: the intent is consumed first, then the zero-effect audit,
/// then the result row that references both.
fn persist_action_prepared_intent_rejection(
    tx: &Transaction<'_>,
    context: &ActionOperationContext,
    operation_ordinal: u64,
    outcome: &RejectedPreparedIntentOutcome,
) -> Result<(), ()> {
    let audit = outcome.audit_event();
    let (target_type, target_id) = match audit.target() {
        AuditTarget::ActionRequest(id) => ("action_request", id.as_str()),
        AuditTarget::Action(id) => ("action", id.as_str()),
        _ => return Err(()),
    };
    if tx
        .execute(
            "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
            rusqlite::params![
                outcome.rejected_at().unix_millis(),
                outcome.prepared_intent_id().as_str()
            ],
        )
        .map_err(|_| ())?
        != 1
    {
        return Err(());
    }
    tx.execute(
        "INSERT INTO audit_events (id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES (?1,?2,'head_of_products','work_management',?3,?4,?5,?6,'allowed','rejected','not_attempted','none')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_type,
            target_id,
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| ())?;
    tx.execute(
        "INSERT INTO action_reject_prepared_command_results (operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES ('reject_prepared',?1,?2,?3,?4,'head_of_products',?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            i64::try_from(operation_ordinal).map_err(|_| ())?,
            outcome.prepared_intent_id().as_str(),
            outcome.rejected_at().unix_millis(),
            audit.id().as_str(),
        ],
    )
    .map_err(|_| ())?;
    Ok(())
}

fn decode_prepared_accept(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    idempotency: &IdempotencyId,
) -> Result<(WorkManagementPreparedIntent, ActionReplayCapsule), ActionPersistenceLoadError> {
    let command = tx
        .query_row(
            "SELECT request_id,expected_version FROM action_command_prepare_accepts WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(decode_sqlite_error)?;
    if command.0 != request.id().as_str() || command.1 <= 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let expected_version = AggregateVersion::new(
        u64::try_from(command.1).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let replay = tx
        .query_row(
            "SELECT correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable,attempted_digest,private_detail_ref FROM action_replay_operations WHERE idempotency_id=?1 AND operation='prepare_accept'",
            [idempotency.as_str()],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?, row.get::<_, Option<i64>>(10)?, row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            )),
        )
        .map_err(decode_sqlite_error)?;
    if replay.1 < 0
        || replay.2 != "prepared"
        || replay.3.is_none()
        || replay.4.is_some()
        || replay.5 != "not_applicable"
        || replay.6.is_some()
        || replay.7.is_some()
        || replay.8.is_some()
        || replay.9.is_some()
        || replay.10.is_some()
        || replay.11.is_some()
        || replay.12.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let prepared_id = PreparedIntentId::parse(
        replay
            .3
            .as_deref()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let correlation = CorrelationId::parse(replay.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let payload = tx
        .query_row(
            "SELECT contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at FROM prepared_intents WHERE id=?1",
            [prepared_id.as_str()],
            |row| Ok((
                row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                row.get::<_, String>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?, row.get::<_, i64>(10)?,
            )),
        )
        .map_err(decode_sqlite_error)?;
    if payload.0 != 1
        || payload.1 != "accept_action_request"
        || payload.4 != "allowed"
        || payload.5 != "not_cancellable_after_submit"
        || payload.6 != "head_of_products"
        || payload.7.is_some()
        || payload.8.is_some()
        || payload.9 < payload.10
        || payload.10 < 0
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let classification = DataClassification::from_persisted(&payload.3)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let created_at = UtcTimestamp::from_unix_millis(payload.10);
    let payload_row = tx
        .query_row(
            "SELECT primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at,statement,rationale,impact,decision_owner_id,decided_at,reopen_mode,resolution_type,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at FROM prepared_work_management_payloads WHERE prepared_intent_id=?1",
            [prepared_id.as_str()],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?, row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?, row.get::<_, Option<String>>(9)?, row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?, row.get::<_, Option<i64>>(12)?, row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?, row.get::<_, Option<String>>(15)?, row.get::<_, Option<i64>>(16)?,
                row.get::<_, Option<String>>(17)?, row.get::<_, Option<String>>(18)?, row.get::<_, Option<String>>(19)?,
                row.get::<_, Option<String>>(20)?, row.get::<_, Option<String>>(21)?, row.get::<_, Option<String>>(22)?,
            )),
        )
        .map_err(decode_sqlite_error)?;
    if payload_row.0 != request.id().as_str()
        || payload_row.1 != command.1
        || payload_row.2.is_none()
        || payload_row.3.is_none()
        || payload_row.4.is_none()
        || payload_row.5.is_none()
        || payload_row.6.is_none()
        || payload_row.7.is_none()
        || payload_row.8.is_some()
        || payload_row.9.is_some()
        || payload_row.10.is_some()
        || payload_row.11.is_some()
        || payload_row.12.is_some()
        || payload_row.13.is_some()
        || payload_row.14.is_some()
        || payload_row.15.is_some()
        || payload_row.16.is_some()
        || payload_row.17.is_some()
        || payload_row.18.is_some()
        || payload_row.19.is_some()
        || payload_row.20.is_some()
        || payload_row.21.is_some()
        || payload_row.22.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let action_id = ActionId::parse(
        payload_row
            .2
            .as_deref()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action_classification = DataClassification::from_persisted(
        payload_row
            .3
            .as_deref()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let subject = ActionTitle::parse(
        payload_row
            .4
            .clone()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let details = ActionDetails::parse(
        payload_row
            .5
            .clone()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let owner = StakeholderId::parse(
        payload_row
            .6
            .clone()
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let due_at = payload_row
        .7
        .filter(|value| *value >= 0)
        .map(UtcTimestamp::from_unix_millis)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let operation = WorkManagementOperation::AcceptActionRequest {
        request_id: request.id().clone(),
        request_version: expected_version,
        action_id,
        action_classification,
        action_subject: subject,
        commitment_details: details,
        intended_owner: owner,
        intended_due_at: due_at,
    };
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id,
        operation,
        classification,
        None,
        created_at,
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if prepared.payload_digest().as_str() != payload.2
        || prepared.classification().as_persisted() != payload.3
        || prepared.preview().expires_at().unix_millis() != payload.9
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let targets = tx
        .prepare("SELECT ordinal,target_type,target_id,expected_version FROM prepared_intent_targets WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)))
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if targets.len() != 1
        || targets[0].0 != 0
        || targets[0].1 != "action_request"
        || targets[0].2 != request.id().as_str()
        || targets[0].3 != command.1
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .prepare("SELECT ordinal,effect_code,target_type,target_id,secondary_target_type,secondary_target_id FROM prepared_intent_effects WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?)))
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    let action_id = match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest { action_id, .. } => action_id.as_str(),
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let expected_effects = [
        (
            0,
            "action_request.accepted",
            "action_request",
            request.id().as_str(),
            None,
            None,
        ),
        (
            1,
            "action.created_from_request",
            "action",
            action_id,
            None,
            None,
        ),
        (
            2,
            "action_request.action_linked",
            "action_request",
            request.id().as_str(),
            Some("action"),
            Some(action_id),
        ),
    ];
    if effects.len() != expected_effects.len()
        || effects
            .iter()
            .zip(expected_effects)
            .any(|(actual, expected)| {
                actual.0 != expected.0
                    || actual.1 != expected.1
                    || actual.2.as_deref() != Some(expected.2)
                    || actual.3.as_deref() != Some(expected.3)
                    || actual.4.as_deref() != expected.4
                    || actual.5.as_deref() != expected.5
            })
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let sources = tx
        .prepare("SELECT ordinal,role,source_id,classification FROM prepared_intent_classification_sources WHERE prepared_intent_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([prepared.id().as_str()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?)))
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if sources.len() != 2
        || sources[0].0 != 0
        || sources[0].1 != "primary_target"
        || sources[0].2.is_some()
        || sources[0].3 != payload.3
        || sources[1].0 != 1
        || sources[1].1 != "created_action"
        || sources[1].2.as_deref() != Some(action_id)
        || sources[1].3 != payload.3
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let audit_ids = tx
        .query_row(
            "SELECT count(*) FROM action_replay_audits WHERE idempotency_id=?1",
            [idempotency.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(decode_sqlite_error)?;
    if audit_ids != 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let capsule = ActionReplayCapsule::new(
        idempotency.clone(),
        correlation,
        u64::try_from(replay.1).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ActionPersistenceCommand::PrepareAccept {
            request_id: request.id().clone(),
            expected_version,
        },
        ActionPersistenceResult::Prepared(prepared.clone()),
        Vec::new(),
    );
    Ok((prepared, capsule))
}

fn decode_accepted_action(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    prepared: &WorkManagementPreparedIntent,
) -> Result<ActionRecord, ActionPersistenceLoadError> {
    let WorkManagementOperation::AcceptActionRequest {
        action_id,
        action_classification,
        action_subject,
        commitment_details,
        intended_owner,
        intended_due_at,
        ..
    } = prepared.operation()
    else {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    };
    if request.state() != ActionRequestState::Accepted
        || request.linked_action_id() != Some(action_id)
        || request.classification() != *action_classification
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let row = tx
        .query_row(
            "SELECT source_request_id,title,details,owner_id,due_at,state,commitment_classification,support_id,transition_reason,source_decision_id,superseded_premise,reg.version,reg.classification FROM actions JOIN aggregate_registry reg ON reg.id=actions.id AND reg.aggregate_type='action' WHERE actions.id=?1",
            [action_id.as_str()],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, String>(5)?,
                row.get::<_, String>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?, row.get::<_, i64>(10)?, row.get::<_, i64>(11)?, row.get::<_, String>(12)?,
            )),
        )
        .map_err(decode_sqlite_error)?;
    // `row.5`/`row.8` (`actions.state`/`transition_reason`) are deliberately
    // NOT pinned to "open"/`NULL` here, unlike every other field: the
    // Cancel/Complete/Reopen transitions are the first thing that can
    // ever move an accepted Action's current durable state away from "open"
    // (Lower Classification never does). This function still only ever
    // reconstructs the FRESH, version-1 accept-time snapshot (see its own
    // doc comment) -- the raw current `state`/`transition_reason` columns
    // are cross-checked against replayed transition history instead, by
    // `decode_cancel_action_activity` (mirroring how `row.11`/`row.12`
    // already tolerate post-accept classification/version drift here, cross-
    // checked precisely by `decode_lower_action_classification_activity`).
    if row.0 != request.id().as_str()
        || row.1 != action_subject.as_str()
        || row.2 != commitment_details.as_str()
        || row.3 != intended_owner.as_str()
        || row.4 != intended_due_at.unix_millis()
        || !matches!(
            row.5.as_str(),
            "open" | "in_progress" | "completed" | "cancelled"
        )
        || row.6 != request.classification().as_persisted()
        // `row.7` (`support_id`) is likewise no longer pinned to `NULL` here
        // (2026-09, Complete): Complete's own EXECUTE is the first writer
        // that can ever set it. `verify_final_action_rows` is the actual
        // source of truth for this relationship (presence cross-checked
        // against the fully-folded final record's `support()`), same
        // deferral already used for `row.5`/`row.8` below.
        // `row.8` (`transition_reason`) is likewise not cross-checked
        // against `row.5` here (2026-09, StartAction): a non-open state no
        // longer implies a non-NULL reason -- `StartAction` legitimately
        // leaves it NULL (mirrors `ActionTransitionRecord{reason: None,..}`
        // in `start_action_cause`), while Cancel/Reopen always set a real
        // one. `verify_final_action_rows` (which runs once, after every
        // activity decoder) is the actual source of truth for this
        // relationship, comparing the raw row against the fully-folded
        // final record instead of guessing from state alone.
        // Decision-triggered Action provenance gap fix (2026-09): the
        // accepted Action always inherits `source_decision_id`/
        // `superseded_premise` from the accepted Request unchanged (see
        // `ActionRecord::from_persisted_accepted_request` below) -- these
        // are no longer hardcoded NULL/0, but cross-checked for consistency
        // against `request`'s own values, mirroring `row.6` just above.
        || row.9.as_deref() != request.source_decision_id().map(DecisionId::as_str)
        || row.10 != i64::from(request.has_superseded_premise())
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    // `action_transitions`/`action_completion_evidence` are no longer
    // required to be empty here (2026-09, StartAction's and
    // LinkActionCompletionEvidence's own SQLite writers): a row can
    // legitimately exist once this action has moved past its fresh
    // accept-time snapshot. The later activity decoders
    // (`decode_start_action_activity`/`decode_link_completion_evidence_activity`
    // and friends) reconstruct and cross-check that history, not this
    // function.
    // `decode_accepted_action` always reconstructs the ORIGINAL, fresh
    // (version 1) accept-time Action -- this is what the historical
    // `ExecuteAccept` replay capsule's own `outcome.action` must equal,
    // regardless of any later classification-lowering
    // activity (`row.11`/`row.12` can legitimately be past version 1 by
    // the time this decodes; an Open Action past version 1 has had its
    // classification governed-lowered one or more times, since
    // `LowerActionClassification` is the only operation that advances an
    // Open Action's version without a lifecycle transition). The final,
    // current-state `actions` list entry is corrected separately by
    // `decode_lower_action_classification_activity`, which cross-checks
    // this same row against its own replayed lowering history.
    if row.11 < 1 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    if row.11 == 1 && row.12 != prepared.classification().as_persisted() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    ActionRecord::from_persisted_accepted_request(request, prepared)
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)
}

fn decode_execute_accept(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    prepared: &WorkManagementPreparedIntent,
) -> Result<Vec<DecodedExecuteAccept>, ActionPersistenceLoadError> {
    let mut statement = tx
        .prepare("SELECT idempotency_id FROM action_command_execute_accepts WHERE prepared_id=?1 ORDER BY idempotency_id")
        .map_err(decode_sqlite_error)?;
    let idempotencies = statement
        .query_map([prepared.id().as_str()], |row| row.get::<_, String>(0))
        .map_err(decode_sqlite_error)?;
    let mut decoded = Vec::new();
    for idempotency in idempotencies {
        decoded.push(decode_execute_accept_by_idempotency(
            tx,
            request,
            prepared,
            &IdempotencyId::parse(idempotency.map_err(decode_sqlite_error)?)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        )?);
    }
    Ok(decoded)
}

fn decode_execute_accept_by_idempotency(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    prepared: &WorkManagementPreparedIntent,
    idempotency: &IdempotencyId,
) -> Result<DecodedExecuteAccept, ActionPersistenceLoadError> {
    let (actor, digest) = tx
        .query_row(
            "SELECT actor,acknowledged_digest FROM action_command_execute_accepts WHERE idempotency_id=?1 AND prepared_id=?2",
            rusqlite::params![idempotency.as_str(), prepared.id().as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(decode_sqlite_error)?;
    if actor != "head_of_products" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let digest_matches = digest == prepared.payload_digest().as_str();
    let idempotency = idempotency.clone();
    let replay = tx
        .query_row(
            "SELECT correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,prepared_disposition FROM action_replay_operations WHERE idempotency_id=?1 AND operation='execute_accept'",
            [idempotency.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, String>(5)?)),
        )
        .map_err(decode_sqlite_error)?;
    if replay.2 == "terminal" {
        return decode_execute_accept_terminal(
            tx,
            request,
            prepared,
            &idempotency,
            &actor,
            &digest,
            digest_matches,
        );
    }
    if !digest_matches {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    if replay.1 < 0
        || replay.2 != "accepted"
        || replay.3.is_none()
        || replay.3.as_deref() != request.linked_action_id().map(ActionId::as_str)
        || replay.4.as_deref() != Some(prepared.id().as_str())
        || replay.5 != "not_applicable"
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let correlation = CorrelationId::parse(replay.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let receipt = tx
        .query_row(
            "SELECT id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at FROM approval_receipts WHERE prepared_intent_id=?1",
            [prepared.id().as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?, row.get::<_, Option<i64>>(6)?)),
        )
        .map_err(decode_sqlite_error)?;
    let prepared_consumed_at = tx
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(decode_sqlite_error)?;
    if receipt.1 != "head_of_products"
        || receipt.2 != digest
        || receipt.3 != idempotency.as_str()
        || receipt.4 < 0
        || receipt.5 != prepared.preview().expires_at().unix_millis()
        || receipt.6 != Some(receipt.4)
        || prepared_consumed_at != Some(receipt.4)
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let receipt_id = ApprovalReceiptId::parse(receipt.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let action = decode_accepted_action(tx, request, prepared)?;
    let audit_rows = tx
        .prepare("SELECT ordinal,audit_event_id,correlation_id FROM action_replay_audits WHERE idempotency_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?
        .query_map([idempotency.as_str()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(decode_sqlite_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_sqlite_error)?;
    if audit_rows.len() != 3
        || audit_rows
            .iter()
            .enumerate()
            .any(|(index, row)| row.0 != index as i64 || row.2 != correlation.as_str())
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let expected = [
        (
            "action_request.accepted",
            "action_request",
            request.id().as_str(),
        ),
        (
            "action.created_from_request",
            "action",
            action.id().as_str(),
        ),
        (
            "action_request.action_linked",
            "action_request",
            request.id().as_str(),
        ),
    ];
    let mut audits = Vec::with_capacity(3);
    for (row, (code, target_type, target_id)) in audit_rows.iter().zip(expected) {
        let audit = tx
            .query_row(
                "SELECT id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope FROM audit_events WHERE id=?1",
                [&row.1],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?, r.get::<_, String>(8)?, r.get::<_, String>(9)?, r.get::<_, String>(10)?, r.get::<_, String>(11)?)),
            )
            .map_err(decode_sqlite_error)?;
        if audit.1 != receipt.4
            || audit.2 != "head_of_products"
            || audit.3 != "work_management"
            || audit.4 != code
            || audit.5 != target_type
            || audit.6 != target_id
            || audit.7 != correlation.as_str()
            || audit.8 != "allowed"
            || audit.9 != "approved"
            || audit.10 != "succeeded"
            || audit.11 != "complete"
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let effects = tx
            .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
            .map_err(decode_sqlite_error)?
            .query_map([&audit.0], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))
            .map_err(decode_sqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(decode_sqlite_error)?;
        if effects.len() != 1
            || effects[0]
                != (
                    0,
                    code.to_owned(),
                    "complete".to_owned(),
                    target_type.to_owned(),
                    target_id.to_owned(),
                )
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let event_id = AuditEventId::parse(audit.0)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let event_code = AuditEventCode::parse(audit.4)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let effect = AuditEffectCode::parse(effects[0].1.clone())
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let target = if target_type == "action" {
            AuditTarget::Action(action.id().clone())
        } else {
            AuditTarget::ActionRequest(request.id().clone())
        };
        audits.push(AuditEvent::new(
            event_id,
            UtcTimestamp::from_unix_millis(audit.1),
            AuditActor::HeadOfProducts,
            AuditAction::new(AuditModule::WorkManagement, event_code, target),
            correlation.clone(),
            AuditDisposition::new(
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec![effect],
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ));
    }
    let outcome = AcceptedActionOutcome {
        request: request.clone(),
        action: action.clone(),
        audit_events: audits.clone(),
        approval_receipt_id: receipt_id,
    };
    let capsule = ActionReplayCapsule::new(
        idempotency,
        correlation,
        u64::try_from(replay.1).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ActionPersistenceCommand::ExecuteAccept {
            prepared_id: prepared.id().clone(),
            actor: AuditActor::HeadOfProducts,
            acknowledged_digest: prepared.payload_digest().clone(),
        },
        ActionPersistenceResult::Accepted(outcome.clone()),
        audits.iter().map(|audit| audit.id().clone()).collect(),
    );
    Ok(DecodedExecuteAccept::Accepted(
        Box::new(outcome),
        Box::new(capsule),
    ))
}

fn decode_execute_accept_terminal(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    prepared: &WorkManagementPreparedIntent,
    idempotency: &IdempotencyId,
    actor: &str,
    digest: &str,
    digest_matches: bool,
) -> Result<DecodedExecuteAccept, ActionPersistenceLoadError> {
    if actor != "head_of_products" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let row = tx.query_row(
        "SELECT correlation_id,operation_ordinal,prepared_intent_id,prepared_disposition,terminal_cause,attempted_digest,error_code,error_message_key,error_correlation_id,error_retryable,private_detail_ref FROM action_replay_operations WHERE idempotency_id=?1 AND operation='execute_accept'",
        [idempotency.as_str()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?, row.get::<_, Option<i64>>(9)?, row.get::<_, Option<String>>(10)?)),
    ).map_err(decode_sqlite_error)?;
    let disposition = match row.3.as_str() {
        "retained" => PreparedDisposition::Retained,
        "consumed_and_discarded" => PreparedDisposition::ConsumedAndDiscarded,
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    if row.1 < 0 || row.2.as_deref() != Some(prepared.id().as_str()) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let correlation = CorrelationId::parse(row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let cause = decode_terminal_cause(row.4.as_deref(), row.5.as_deref())?;
    if matches!(cause, ActionPersistenceTerminalCause::DigestMismatch { .. }) != !digest_matches {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    if matches!(cause, ActionPersistenceTerminalCause::PreparedIntentChanged)
        != (disposition == PreparedDisposition::Retained)
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let error = decode_terminal_error(tx, idempotency, row.6, row.7, row.8, row.9, row.10)?;
    let discarded = tx.query_row(
        "SELECT 1 FROM action_discarded_prepared_intents WHERE prepared_intent_id=?1 AND idempotency_id=?2",
        rusqlite::params![prepared.id().as_str(), idempotency.as_str()], |_| Ok(()),
    ).optional().map_err(decode_sqlite_error)?.is_some();
    let consumed_at = tx
        .query_row(
            "SELECT consumed_at FROM prepared_intents WHERE id=?1",
            [prepared.id().as_str()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(decode_sqlite_error)?;
    let audit = decode_terminal_audit(tx, request, idempotency, &correlation)?;
    if disposition == PreparedDisposition::ConsumedAndDiscarded
        && (!discarded || consumed_at != Some(audit.occurred_at().unix_millis()))
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    if disposition == PreparedDisposition::Retained && (discarded || consumed_at.is_some()) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let capsule = ActionReplayCapsule::new(
        idempotency.clone(),
        correlation,
        u64::try_from(row.1).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ActionPersistenceCommand::ExecuteAccept {
            prepared_id: prepared.id().clone(),
            actor: AuditActor::HeadOfProducts,
            acknowledged_digest:
                pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted(
                    digest.to_owned(),
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        },
        ActionPersistenceResult::Terminal {
            command: ActionPersistenceCommand::ExecuteAccept {
                prepared_id: prepared.id().clone(),
                actor: AuditActor::HeadOfProducts,
                acknowledged_digest:
                    pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted(
                        digest.to_owned(),
                    )
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            },
            cause,
            error,
            prepared_disposition: disposition,
            audit: audit.clone(),
        },
        vec![audit.id().clone()],
    );
    Ok(DecodedExecuteAccept::Terminal(
        Box::new(capsule),
        audit,
        disposition,
    ))
}

fn decode_terminal_cause(
    cause: Option<&str>,
    attempted: Option<&str>,
) -> Result<ActionPersistenceTerminalCause, ActionPersistenceLoadError> {
    match cause {
        Some("unauthorized") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::Unauthorized)
        }
        Some("digest_mismatch") => Ok(ActionPersistenceTerminalCause::DigestMismatch {
            attempted_digest:
                pmc_domain::work_management::WorkManagementPayloadDigest::from_persisted(
                    attempted
                        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
                        .to_owned(),
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        }),
        Some("expired") if attempted.is_none() => Ok(ActionPersistenceTerminalCause::Expired),
        Some("policy_denied") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::PolicyDenied)
        }
        Some("infrastructure_failure") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::InfrastructureFailure)
        }
        Some("prepared_intent_changed") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::PreparedIntentChanged)
        }
        // Widened for the Cancel/Complete/Reopen EXECUTE terminal slice --
        // Accept's own execute can only ever produce the six causes above
        // (its `_cause` method has no code path to the others), so this
        // purely-additive widening cannot change how any existing Accept
        // ledger decodes.
        Some("not_found") if attempted.is_none() => Ok(ActionPersistenceTerminalCause::NotFound),
        Some("already_exists") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::AlreadyExists)
        }
        Some("illegal_transition") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::IllegalTransition)
        }
        Some("stale_version") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::StaleVersion)
        }
        Some("missing_owner") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::MissingOwner)
        }
        Some("missing_due_date") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::MissingDueDate)
        }
        Some("invalid_evidence") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::InvalidEvidence)
        }
        Some("invalid_reopen_mode") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::InvalidReopenMode)
        }
        Some("idempotency_conflict") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::IdempotencyConflict)
        }
        Some("prepared_intent_not_found") if attempted.is_none() => {
            Ok(ActionPersistenceTerminalCause::PreparedIntentNotFound)
        }
        _ => Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    }
}

/// Reverses `persisted_h3_denial_cause` -- the read side of Cancel/Reopen/
/// Complete's PREPARE-time H3 denial decode. Unlike
/// `decode_terminal_cause` (shared with EXECUTE's own terminal decode, which
/// never produces an H3-denied cause), this is only ever called from the new
/// `decode_*_prepare_h3_denials` functions, so it is kept separate rather
/// than folded into that shared function.
fn decode_h3_denial_cause(
    value: &str,
) -> Result<ActionPersistenceH3DenialCause, ActionPersistenceLoadError> {
    match value {
        "missing_completion_evidence" => {
            Ok(ActionPersistenceH3DenialCause::MissingCompletionEvidence)
        }
        "completion_evidence_not_found" => {
            Ok(ActionPersistenceH3DenialCause::CompletionEvidenceNotFound)
        }
        "unverified_completion_evidence" => {
            Ok(ActionPersistenceH3DenialCause::UnverifiedCompletionEvidence)
        }
        "unclassified_completion_evidence" => {
            Ok(ActionPersistenceH3DenialCause::UnclassifiedCompletionEvidence)
        }
        "evidence_unavailable" => Ok(ActionPersistenceH3DenialCause::EvidenceUnavailable),
        "classification_unresolved" => Ok(ActionPersistenceH3DenialCause::ClassificationUnresolved),
        _ => Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    }
}

/// Decodes the durable audit event for a PREPARE-time H3 denial Terminal
/// capsule -- Cancel/Reopen/Complete all share this exact shape (one audit
/// row, `effect_scope='none'`, target the denied Action), so this is shared
/// across all three operations' new H3-denial decode paths. Mirrors the
/// EXECUTE terminal decode's own audit-reconstruction
/// technique (see `decode_cancel_action_activity`'s "terminal" branch).
fn decode_prepare_h3_denial_audit(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
    correlation: &CorrelationId,
    target_action_id: &ActionId,
) -> Result<AuditEvent, ActionPersistenceLoadError> {
    let audit_row = tx
        .query_row(
            "SELECT audit.id,audit.occurred_at,audit.correlation_id,audit.event_code,audit.target_type,audit.target_id,audit.policy_outcome,audit.approval_outcome,audit.execution_outcome FROM audit_events audit JOIN action_replay_audits link ON link.audit_event_id=audit.id WHERE link.idempotency_id=?1 AND link.ordinal=0",
            [idempotency.as_str()],
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
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .map_err(decode_sqlite_error)?;
    if audit_row.2 != correlation.as_str() || audit_row.4 != "action" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    if audit_row.5 != target_action_id.as_str() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .query_row(
            "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1",
            [&audit_row.0],
            |row| row.get::<_, i64>(0),
        )
        .map_err(decode_sqlite_error)?;
    if effects != 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    Ok(AuditEvent::new(
        AuditEventId::parse(audit_row.0)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        UtcTimestamp::from_unix_millis(audit_row.1),
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(audit_row.3)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            AuditTarget::Action(target_action_id.clone()),
        ),
        correlation.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::from_persisted(&audit_row.6)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            AuditApprovalOutcome::from_persisted(&audit_row.7)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            AuditExecutionOutcome::from_persisted(&audit_row.8)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            AuditEffectScope::None,
            Vec::new(),
        )
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
    ))
}

fn decode_terminal_error(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
    code: Option<String>,
    key: Option<String>,
    correlation: Option<String>,
    retryable: Option<i64>,
    private_detail: Option<String>,
) -> Result<DomainError, ActionPersistenceLoadError> {
    let code = match code.as_deref() {
        Some("VALIDATION_INVALID_FIELD") => ErrorCode::ValidationInvalidField,
        Some("DOMAIN_CONFLICT") => ErrorCode::DomainConflict,
        Some("DOMAIN_NOT_FOUND") => ErrorCode::DomainNotFound,
        Some("SECURITY_POLICY_DENIED") => ErrorCode::SecurityPolicyDenied,
        Some("AI_POLICY_DENIED") => ErrorCode::AiPolicyDenied,
        Some("SECURITY_PREVIEW_EXPIRED_OR_CHANGED") => ErrorCode::SecurityPreviewExpiredOrChanged,
        Some("DOMAIN_IDEMPOTENCY_CONFLICT") => ErrorCode::DomainIdempotencyConflict,
        Some("PLATFORM_INTERNAL") => ErrorCode::PlatformInternal,
        _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
    };
    let mut error = DomainError::new(
        code,
        MessageKey::parse(key.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        CorrelationId::parse(correlation.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        match retryable {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        },
    );
    let params = tx.prepare("SELECT ordinal,param_key,param_kind,param_text,param_unsigned,param_boolean FROM action_replay_error_params WHERE idempotency_id=?1 ORDER BY ordinal").map_err(decode_sqlite_error)?.query_map([idempotency.as_str()], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<i64>>(4)?,row.get::<_,Option<i64>>(5)?))).map_err(decode_sqlite_error)?.collect::<Result<Vec<_>,_>>().map_err(decode_sqlite_error)?;
    for (index, param) in params.into_iter().enumerate() {
        if param.0 != index as i64 {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let value = match param.2.as_str() {
            "identifier" => SafeParamValue::Identifier(
                param
                    .3
                    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
            ),
            "field_key" => SafeParamValue::FieldKey(
                param
                    .3
                    .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
            ),
            "unsigned" => SafeParamValue::Unsigned(
                u64::try_from(
                    param
                        .4
                        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            ),
            "boolean" => SafeParamValue::Boolean(match param.5 {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
            }),
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        };
        error = error.with_param(
            MessageParam::new(param.1, value)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        );
    }
    let extensions = tx.prepare("SELECT ordinal,extension_kind,current_version,field_key,reason_key FROM action_replay_error_extensions WHERE idempotency_id=?1 ORDER BY ordinal").map_err(decode_sqlite_error)?.query_map([idempotency.as_str()], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<i64>>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<String>>(4)?))).map_err(decode_sqlite_error)?.collect::<Result<Vec<_>,_>>().map_err(decode_sqlite_error)?;
    for (index, extension) in extensions.into_iter().enumerate() {
        if extension.0 != index as i64
            || extension.1 != "current_version"
            || extension.3.is_some()
            || extension.4.is_some()
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        error = error.with_extension(SafeErrorExtension::CurrentVersion(
            AggregateVersion::new(
                u64::try_from(
                    extension
                        .2
                        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        ));
    }
    if private_detail.is_some() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    Ok(error)
}

fn decode_terminal_audit(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    idempotency: &IdempotencyId,
    correlation: &CorrelationId,
) -> Result<AuditEvent, ActionPersistenceLoadError> {
    let replay = tx.query_row("SELECT ordinal,audit_event_id,correlation_id FROM action_replay_audits WHERE idempotency_id=?1", [idempotency.as_str()], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).map_err(decode_sqlite_error)?;
    if replay.0 != 0 || replay.2 != correlation.as_str() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let row = tx.query_row("SELECT occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope FROM audit_events WHERE id=?1", [&replay.1], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?))).map_err(decode_sqlite_error)?;
    if row.0 < 0
        || row.1 != "head_of_products"
        || row.2 != "work_management"
        || row.4 != "action_request"
        || row.5 != request.id().as_str()
        || row.6 != correlation.as_str()
        || row.10 != "none"
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effects = tx
        .query_row(
            "SELECT count(*) FROM audit_effects WHERE audit_event_id=?1",
            [&replay.1],
            |row| row.get::<_, i64>(0),
        )
        .map_err(decode_sqlite_error)?;
    if effects != 0 {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::from_persisted(&row.7)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        AuditApprovalOutcome::from_persisted(&row.8)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        AuditExecutionOutcome::from_persisted(&row.9)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        AuditEffectScope::None,
        vec![],
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok(AuditEvent::new(
        AuditEventId::parse(replay.1)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        UtcTimestamp::from_unix_millis(row.0),
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(row.3)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            AuditTarget::ActionRequest(request.id().clone()),
        ),
        correlation.clone(),
        disposition,
    ))
}

fn replay_audit(
    tx: &Transaction<'_>,
    idempotency: &IdempotencyId,
    reference: &str,
    correlation: &CorrelationId,
) -> Result<(String, CorrelationId), ActionPersistenceLoadError> {
    let row = tx.query_row(
        "SELECT ordinal,audit_event_id,correlation_id FROM action_replay_audits WHERE idempotency_id=?1",
        [idempotency.as_str()], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)),
    ).map_err(decode_sqlite_error)?;
    if row.0 != 0 || row.2 != correlation.as_str() || reference.is_empty() {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_correlation = CorrelationId::parse(row.2)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok((row.1, event_correlation))
}

fn decode_audit_event(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    event_id: &str,
    correlation: &CorrelationId,
    expected_code: &str,
) -> Result<AuditEvent, ActionPersistenceLoadError> {
    let row = tx.query_row(
        "SELECT id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope FROM audit_events WHERE id=?1",
        [event_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?,row.get::<_,String>(11)?)),
    ).map_err(decode_sqlite_error)?;
    if row.1 < 0
        || row.2 != "head_of_products"
        || row.3 != "work_management"
        || row.4 != expected_code
        || row.5 != "action_request"
        || row.6 != request.id().as_str()
        || row.7 != correlation.as_str()
        || row.8 != "allowed"
        || row.9 != "not_required"
        || row.10 != "succeeded"
        || row.11 != "complete"
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id = AuditEventId::parse(row.0)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let code = AuditEventCode::parse(row.4)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mut statement = tx
        .prepare("SELECT ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?;
    let effects: Vec<_> = statement
        .query_map([event_id.as_str()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(decode_sqlite_error)?
        .collect::<Result<_, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0].0 != 0
        || effects[0].1 != expected_code
        || effects[0].2 != "complete"
        || effects[0].3.as_deref() != Some("action_request")
        || effects[0].4.as_deref() != Some(request.id().as_str())
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let disposition = AuditDisposition::new(
        AuditPolicyOutcome::Allowed,
        AuditApprovalOutcome::NotRequired,
        AuditExecutionOutcome::Succeeded,
        AuditEffectScope::Complete,
        vec![AuditEffectCode::parse(effects[0].1.clone())
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?],
    )
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok(AuditEvent::new(
        event_id,
        UtcTimestamp::from_unix_millis(row.1),
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            code,
            AuditTarget::ActionRequest(request.id().clone()),
        ),
        correlation.clone(),
        disposition,
    ))
}

fn count(tx: &Transaction<'_>, sql: &str) -> Result<i64, ActionPersistenceLoadError> {
    tx.query_row(sql, [], |row| row.get(0))
        .map_err(classify_sqlite_error)
}

fn classify_sqlite_error(error: SqliteError) -> ActionPersistenceLoadError {
    match error {
        SqliteError::QueryReturnedNoRows
        | SqliteError::FromSqlConversionFailure(..)
        | SqliteError::IntegralValueOutOfRange(..)
        | SqliteError::InvalidColumnType(..)
        | SqliteError::Utf8Error(..) => ActionPersistenceLoadError::InvalidActionSnapshot,
        SqliteError::SqliteFailure(details, _) => match details.code {
            SqliteErrorCode::ConstraintViolation | SqliteErrorCode::TypeMismatch => {
                ActionPersistenceLoadError::InvalidActionSnapshot
            }
            _ => ActionPersistenceLoadError::StorageUnavailable,
        },
        _ => ActionPersistenceLoadError::StorageUnavailable,
    }
}

fn decode_sqlite_error(error: SqliteError) -> ActionPersistenceLoadError {
    classify_sqlite_error(error)
}

fn decode_create_draft(
    tx: &Transaction<'_>,
) -> Result<ActionPersistenceSnapshot, ActionPersistenceLoadError> {
    let request_count = count(tx, "SELECT count(*) FROM action_requests")?;
    if request_count > 1 {
        return decode_multiple_create_drafts(tx, request_count);
    }
    let expected = [
        ("SELECT count(*) FROM action_requests", 1),
        ("SELECT count(*) FROM actions", 0),
        ("SELECT count(*) FROM action_transitions", 0),
        ("SELECT count(*) FROM action_completion_evidence", 0),
        ("SELECT count(*) FROM action_replay_operations", 1),
        ("SELECT count(*) FROM action_replay_audits", 1),
        ("SELECT count(*) FROM action_command_create_requests", 1),
        ("SELECT count(*) FROM action_command_transition_requests", 0),
        ("SELECT count(*) FROM action_command_prepare_accepts", 0),
        ("SELECT count(*) FROM action_command_execute_accepts", 0),
        ("SELECT count(*) FROM action_command_prepare_completes", 0),
        ("SELECT count(*) FROM action_command_prepare_cancels", 0),
        ("SELECT count(*) FROM action_command_prepare_reopens", 0),
        ("SELECT count(*) FROM action_command_execute_actions", 0),
        ("SELECT count(*) FROM action_command_start_actions", 0),
        (
            "SELECT count(*) FROM action_command_link_completion_evidence",
            0,
        ),
        ("SELECT count(*) FROM action_replay_error_params", 0),
        ("SELECT count(*) FROM action_replay_error_extensions", 0),
        ("SELECT count(*) FROM action_discarded_prepared_intents", 0),
        ("SELECT count(*) FROM prepared_intents WHERE intent_kind IN ('accept_action_request','complete_action','cancel_action','reopen_action')", 0),
        (
            "SELECT count(*) FROM aggregate_registry WHERE aggregate_type='action_request'",
            1,
        ),
        (
            "SELECT count(*) FROM aggregate_registry WHERE aggregate_type='action'",
            0,
        ),
        (
            "SELECT count(*) FROM audit_events WHERE target_type='action_request'",
            1,
        ),
        (
            "SELECT count(*) FROM audit_events WHERE target_type='action'",
            0,
        ),
        (
            "SELECT count(*) FROM audit_effects WHERE target_type='action_request'",
            1,
        ),
        (
            "SELECT count(*) FROM audit_effects WHERE target_type='action'",
            0,
        ),
        (
            "SELECT count(*) FROM operations WHERE namespace='action'",
            0,
        ),
        (
            "SELECT count(*) FROM idempotency_outcomes WHERE namespace='action'",
            0,
        ),
    ];
    for (sql, value) in expected {
        let actual = count(tx, sql)?;
        if actual != value {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
    }
    if !action_owned_shared_topology_is_empty(tx)? {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let request = decode_request(tx)?;
    let (idempotency, command) = decode_command(tx, &request)?;
    let audit = decode_audit(tx, request.id())?;
    let (correlation, ordinal, result_kind, reference, prepared, disposition, cause, error_code, message_key, error_corr, retryable, attempted, private_ref) = tx.query_row(
        "SELECT correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable,attempted_digest,private_detail_ref FROM action_replay_operations WHERE idempotency_id=?1 AND operation='create_request'",
        [idempotency.as_str()], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,String>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,Option<String>>(8)?,row.get::<_,Option<String>>(9)?,row.get::<_,Option<i64>>(10)?,row.get::<_,Option<String>>(11)?,row.get::<_,Option<String>>(12)?))).map_err(decode_sqlite_error)?;
    if ordinal != 0
        || result_kind != "request"
        || reference.as_deref() != Some(request.id().as_str())
        || prepared.is_some()
        || disposition != "not_applicable"
        || cause.is_some()
        || error_code.is_some()
        || message_key.is_some()
        || error_corr.is_some()
        || retryable.is_some()
        || attempted.is_some()
        || private_ref.is_some()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let correlation = CorrelationId::parse(correlation)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let replay_audit = tx.query_row("SELECT ordinal,audit_event_id,correlation_id FROM action_replay_audits WHERE idempotency_id=?1", [idempotency.as_str()], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).map_err(decode_sqlite_error)?;
    if replay_audit.0 != 0
        || replay_audit.1 != audit.id().as_str()
        || replay_audit.2 != correlation.as_str()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let capsule = ActionReplayCapsule::new(
        idempotency,
        correlation,
        0,
        command,
        ActionPersistenceResult::Request(ActionMutationOutcome {
            record: request.clone(),
            audit_events: vec![audit.clone()],
            approval_receipt_id: None,
        }),
        vec![audit.id().clone()],
    );
    ActionPersistenceDecodeInput::new(
        vec![request],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        vec![capsule],
        vec![audit],
    )
    .decode()
    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)
}

fn decode_multiple_create_drafts(
    tx: &Transaction<'_>,
    request_count: i64,
) -> Result<ActionPersistenceSnapshot, ActionPersistenceLoadError> {
    let expected = [
        ("SELECT count(*) FROM actions", 0),
        ("SELECT count(*) FROM action_transitions", 0),
        ("SELECT count(*) FROM action_completion_evidence", 0),
        ("SELECT count(*) FROM action_replay_operations", request_count),
        ("SELECT count(*) FROM action_replay_audits", request_count),
        ("SELECT count(*) FROM action_command_create_requests", request_count),
        ("SELECT count(*) FROM action_command_transition_requests", 0),
        ("SELECT count(*) FROM action_command_prepare_accepts", 0),
        ("SELECT count(*) FROM action_command_execute_accepts", 0),
        ("SELECT count(*) FROM action_command_prepare_completes", 0),
        ("SELECT count(*) FROM action_command_prepare_cancels", 0),
        ("SELECT count(*) FROM action_command_prepare_reopens", 0),
        ("SELECT count(*) FROM action_command_execute_actions", 0),
        ("SELECT count(*) FROM action_command_start_actions", 0),
        ("SELECT count(*) FROM action_command_link_completion_evidence", 0),
        ("SELECT count(*) FROM action_replay_error_params", 0),
        ("SELECT count(*) FROM action_replay_error_extensions", 0),
        ("SELECT count(*) FROM action_discarded_prepared_intents", 0),
        ("SELECT count(*) FROM prepared_intents WHERE intent_kind IN ('accept_action_request','complete_action','cancel_action','reopen_action')", 0),
        ("SELECT count(*) FROM aggregate_registry WHERE aggregate_type='action_request'", request_count),
        ("SELECT count(*) FROM aggregate_registry WHERE aggregate_type='action'", 0),
        ("SELECT count(*) FROM audit_events WHERE target_type='action_request'", request_count),
        ("SELECT count(*) FROM audit_events WHERE target_type='action'", 0),
        ("SELECT count(*) FROM audit_effects WHERE target_type='action_request'", request_count),
        ("SELECT count(*) FROM audit_effects WHERE target_type='action'", 0),
        ("SELECT count(*) FROM operations WHERE namespace='action'", 0),
        ("SELECT count(*) FROM idempotency_outcomes WHERE namespace='action'", 0),
        ("SELECT count(*) FROM action_h2a_lower_classification_command_prepares", 0),
        ("SELECT count(*) FROM action_h2a_lower_classification_command_executes", 0),
    ];
    for (sql, value) in expected {
        if count(tx, sql)? != value {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
    }
    if !action_owned_shared_topology_is_empty(tx)? {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }

    let mut statement = tx
        .prepare("SELECT id FROM action_requests ORDER BY id")
        .map_err(decode_sqlite_error)?;
    let request_ids: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .map_err(decode_sqlite_error)?
        .collect::<Result<_, _>>()
        .map_err(decode_sqlite_error)?;
    if i64::try_from(request_ids.len()).ok() != Some(request_count) {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    drop(statement);

    let mut requests = Vec::with_capacity(request_ids.len());
    let mut replay = Vec::with_capacity(request_ids.len());
    let mut audits = Vec::with_capacity(request_ids.len());
    for request_id in request_ids {
        let request = decode_request_for_id(tx, Some(&request_id))?;
        let (idempotency, command) = decode_command_for_request(tx, &request, Some(&request_id))?;
        let audit = decode_audit_for_request(tx, request.id(), Some(&request_id))?;
        let row = tx
            .query_row(
                "SELECT correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,prepared_disposition,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable,attempted_digest,private_detail_ref FROM action_replay_operations WHERE idempotency_id=?1 AND operation='create_request'",
                [idempotency.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                },
            )
            .map_err(decode_sqlite_error)?;
        if row.2 != "request"
            || row.3.as_deref() != Some(request.id().as_str())
            || row.4.is_some()
            || row.5 != "not_applicable"
            || row.6.is_some()
            || row.7.is_some()
            || row.8.is_some()
            || row.9.is_some()
            || row.10.is_some()
            || row.11.is_some()
            || row.12.is_some()
            || row.1 < 0
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let correlation = CorrelationId::parse(row.0)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let replay_audit = tx
            .query_row(
                "SELECT ordinal,audit_event_id,correlation_id FROM action_replay_audits WHERE idempotency_id=?1",
                [idempotency.as_str()],
                |value| {
                    Ok((
                        value.get::<_, i64>(0)?,
                        value.get::<_, String>(1)?,
                        value.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(decode_sqlite_error)?;
        if replay_audit.0 != 0
            || replay_audit.1 != audit.id().as_str()
            || replay_audit.2 != correlation.as_str()
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        replay.push(ActionReplayCapsule::new(
            idempotency,
            correlation,
            u64::try_from(row.1).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
            command,
            ActionPersistenceResult::Request(ActionMutationOutcome {
                record: request.clone(),
                audit_events: vec![audit.clone()],
                approval_receipt_id: None,
            }),
            vec![audit.id().clone()],
        ));
        requests.push(request);
        audits.push(audit);
    }
    replay.sort_by_key(ActionReplayCapsule::operation_ordinal);
    audits.sort_by_key(|audit| {
        replay
            .iter()
            .position(|capsule| capsule.audit_event_ids().contains(audit.id()))
            .unwrap_or(usize::MAX)
    });
    ActionPersistenceDecodeInput::new(requests, Vec::new(), Vec::new(), Vec::new(), replay, audits)
        .decode()
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)
}

fn decode_request(tx: &Transaction<'_>) -> Result<ActionRequestRecord, ActionPersistenceLoadError> {
    decode_request_for_id(tx, None)
}

fn decode_request_for_id(
    tx: &Transaction<'_>,
    request_id: Option<&str>,
) -> Result<ActionRequestRecord, ActionPersistenceLoadError> {
    let row = tx.query_row("SELECT r.id,r.title,r.details,r.intended_owner_id,r.response_due_at,r.intended_action_due_at,r.state,r.terminal_rationale,r.linked_action_id,r.source_decision_id,r.superseded_premise,reg.version,reg.classification,reg.created_at,reg.updated_at FROM action_requests r JOIN aggregate_registry reg ON reg.id=r.id AND reg.aggregate_type='action_request' WHERE (?1 IS NULL OR r.id=?1)", [request_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<i64>>(4)?,row.get::<_,Option<i64>>(5)?,row.get::<_,String>(6)?,row.get::<_,Option<String>>(7)?,row.get::<_,Option<String>>(8)?,row.get::<_,Option<String>>(9)?,row.get::<_,i64>(10)?,row.get::<_,i64>(11)?,row.get::<_,String>(12)?,row.get::<_,i64>(13)?,row.get::<_,i64>(14)?))).map_err(decode_sqlite_error)?;
    let (
        id,
        title,
        details,
        owner,
        response_due,
        action_due,
        state,
        terminal,
        linked,
        source,
        superseded,
        version,
        classification,
        created,
        updated,
    ) = row;
    // Decision-triggered Action provenance gap fix (2026-09): a request
    // created by a Decision (Resolve/Supersede) never goes through this
    // function's ordinary H1 create/submit shape at all -- it starts Open/
    // version 1, may be marked superseded (version 2) by a later Decision,
    // and its replay authority lives in `action_decision_replay_operations`
    // (see `decode_decision_sourced_request_activity`), not
    // `action_command_create_requests`/`action_command_transition_requests`.
    // Declining/withdrawing a Decision-created request via the ordinary H1
    // path is out of scope here (documented gap, fails closed below rather
    // than silently misdecoding) -- narrower than the general case, same as
    // this codebase's other documented decode gaps (e.g. Lower-after-Cancel).
    if source.is_some() {
        if !matches!(state.as_str(), "open" | "accepted")
            || (state == "accepted") != linked.is_some()
            || terminal.is_some()
            || created < 0
            || updated < created
            || !matches!(superseded, 0 | 1)
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let id = ActionRequestId::parse(id)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let title = ActionTitle::parse(title)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let details = ActionDetails::parse(details)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let owner = owner
            .map(StakeholderId::parse)
            .transpose()
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let action_due =
            parse_time(action_due)?.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?;
        if response_due.is_some() {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        let final_classification = DataClassification::from_persisted(&classification)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let source_decision_id =
            DecisionId::parse(source.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        // Open/accepted-without-mark is version 1/2; marked-then-accepted is
        // version 2/3 -- `superseded` pins which shape this row is, since
        // the mark is otherwise invisible from `action_requests` alone.
        let expected_version: i64 = match (state.as_str(), superseded) {
            ("open", 0) => 1,
            ("open", 1) => 2,
            ("accepted", 0) => 2,
            ("accepted", 1) => 3,
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        };
        if version != expected_version {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        // `combine` is not identity over `Unclassified` (either side present
        // collapses the whole combination to `Unclassified`), so the exact
        // pre-mark classification the domain actually used at create time
        // must be read back from `action_command_create_from_decisions`,
        // not guessed -- the row's own `classification` column only holds
        // the final, already-combined value.
        let create_classification: String = tx
            .query_row(
                "SELECT classification FROM action_command_create_from_decisions WHERE request_id=?1",
                [id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let create_classification = DataClassification::from_persisted(&create_classification)
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
        let created = ActionRequestRecord::from_persisted_created_from_decision(
            id,
            title,
            details,
            owner,
            action_due,
            create_classification,
            source_decision_id.clone(),
        );
        let pre_accept = if superseded == 1 {
            let mark_classification: String = tx
                .query_row(
                    "SELECT classification FROM action_command_mark_request_superseded_premises WHERE request_id=?1",
                    [created.id().as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
            let mark_classification = DataClassification::from_persisted(&mark_classification)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
            ActionRequestRecord::from_persisted_superseded_premise_marked(
                created,
                AggregateVersion::new(1)
                    .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                &source_decision_id,
                mark_classification,
            )
            .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
        } else {
            created
        };
        let final_record = match state.as_str() {
            "open" => pre_accept,
            "accepted" => {
                let linked_id = ActionId::parse(
                    linked.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
                let open_version = pre_accept.version();
                ActionRequestRecord::from_persisted_open_to_accepted_from_decision(
                    pre_accept,
                    open_version,
                    linked_id,
                )
                .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?
            }
            _ => return Err(ActionPersistenceLoadError::InvalidActionSnapshot),
        };
        if final_record.classification() != final_classification
            || final_record.version()
                != AggregateVersion::new(
                    u64::try_from(version)
                        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
                )
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?
        {
            return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
        }
        return Ok(final_record);
    }
    if !matches!(
        state.as_str(),
        "draft" | "open" | "accepted" | "declined" | "withdrawn"
    ) || (state != "accepted" && linked.is_some())
        || superseded != 0
        || (state == "draft" && (version != 1 || terminal.is_some()))
        || (state == "open" && (version != 2 || terminal.is_some()))
        || (state == "accepted" && (version != 3 || terminal.is_some() || linked.is_none()))
        || (state == "declined" && (version != 3 || terminal.is_none()))
        || (state == "withdrawn" && (version != 3 || terminal.is_none()))
        || created < 0
        || updated < created
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let id = ActionRequestId::parse(id)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let title =
        ActionTitle::parse(title).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let details = ActionDetails::parse(details)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let owner = owner
        .map(StakeholderId::parse)
        .transpose()
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let response_due = parse_time(response_due)?;
    let action_due = parse_time(action_due)?;
    let classification = DataClassification::from_persisted(&classification)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    match state.as_str() {
        "open" => Ok(ActionRequestRecord::from_persisted_submitted_open(
            id,
            title,
            details,
            owner,
            response_due,
            action_due,
            classification,
        )),
        "declined" => ActionRequestRecord::from_persisted_open_to_declined(
            ActionRequestRecord::from_persisted_submitted_open(
                id,
                title,
                details,
                owner,
                response_due,
                action_due,
                classification,
            ),
            ActionDetails::parse(
                terminal.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        )
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot),
        "withdrawn" => ActionRequestRecord::from_persisted_open_to_withdrawn(
            ActionRequestRecord::from_persisted_submitted_open(
                id,
                title,
                details,
                owner,
                response_due,
                action_due,
                classification,
            ),
            ActionDetails::parse(
                terminal.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?,
            )
            .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        )
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot),
        "accepted" => ActionRequestRecord::from_persisted_open_to_accepted(
            ActionRequestRecord::from_persisted_submitted_open(
                id,
                title,
                details,
                owner,
                response_due,
                action_due,
                classification,
            ),
            ActionId::parse(linked.ok_or(ActionPersistenceLoadError::InvalidActionSnapshot)?)
                .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?,
        )
        .ok_or(ActionPersistenceLoadError::InvalidActionSnapshot),
        _ => Ok(ActionRequestRecord::from_persisted_created_draft(
            id,
            title,
            details,
            owner,
            response_due,
            action_due,
            classification,
        )),
    }
}

fn parse_time(value: Option<i64>) -> Result<Option<UtcTimestamp>, ActionPersistenceLoadError> {
    value
        .map(|v| {
            if v < 0 {
                Err(ActionPersistenceLoadError::InvalidActionSnapshot)
            } else {
                Ok(UtcTimestamp::from_unix_millis(v))
            }
        })
        .transpose()
}

fn decode_command(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
) -> Result<(IdempotencyId, ActionPersistenceCommand), ActionPersistenceLoadError> {
    decode_command_for_request(tx, request, None)
}

fn decode_command_for_request(
    tx: &Transaction<'_>,
    request: &ActionRequestRecord,
    request_id: Option<&str>,
) -> Result<(IdempotencyId, ActionPersistenceCommand), ActionPersistenceLoadError> {
    let row=tx.query_row("SELECT idempotency_id,request_id,title,details,intended_owner_id,response_due_at,intended_action_due_at,classification FROM action_command_create_requests WHERE (?1 IS NULL OR request_id=?1)", [request_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,Option<i64>>(5)?,row.get::<_,Option<i64>>(6)?,row.get::<_,String>(7)?))).map_err(decode_sqlite_error)?;
    let (idem, rid, title, details, owner, response_due, action_due, class) = row;
    let idem = IdempotencyId::parse(idem)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let rid = ActionRequestId::parse(rid)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let title =
        ActionTitle::parse(title).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let details = ActionDetails::parse(details)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let owner = owner
        .map(StakeholderId::parse)
        .transpose()
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let response_due = parse_time(response_due)?;
    let action_due = parse_time(action_due)?;
    let class = DataClassification::from_persisted(&class)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    if rid != *request.id()
        || title != *request.title()
        || details != *request.details()
        || owner.as_ref() != request.intended_owner()
        || response_due != request.response_due_at()
        || action_due != request.intended_action_due_at()
        || class != request.classification()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    Ok((
        idem,
        ActionPersistenceCommand::CreateRequest {
            id: rid,
            title,
            details,
            intended_owner: owner,
            response_due_at: response_due,
            intended_action_due_at: action_due,
            classification: class,
        },
    ))
}

fn decode_audit(
    tx: &Transaction<'_>,
    request: &ActionRequestId,
) -> Result<AuditEvent, ActionPersistenceLoadError> {
    decode_audit_for_request(tx, request, None)
}

fn decode_audit_for_request(
    tx: &Transaction<'_>,
    request: &ActionRequestId,
    request_id: Option<&str>,
) -> Result<AuditEvent, ActionPersistenceLoadError> {
    let row=tx.query_row("SELECT id,occurred_at,actor,module,event_code,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope FROM audit_events WHERE target_type='action_request' AND (?1 IS NULL OR target_id=?1)", [request_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?))).map_err(decode_sqlite_error)?;
    let (id, when, actor, module, code, target, corr, policy, approval, execution, scope) = row;
    if when < 0 || target != request.as_str() || code != "action_request.created" {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let event_id =
        AuditEventId::parse(id).map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let corr = CorrelationId::parse(corr)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let target = ActionRequestId::parse(target)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let actor = AuditActor::from_persisted(&actor)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let module = AuditModule::from_persisted(&module)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let code = AuditEventCode::parse(code)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let policy = AuditPolicyOutcome::from_persisted(&policy)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let approval = AuditApprovalOutcome::from_persisted(&approval)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let execution = AuditExecutionOutcome::from_persisted(&execution)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let scope = AuditEffectScope::from_persisted(&scope)
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let mut statement = tx
        .prepare("SELECT audit_event_id,ordinal,effect_code,scope,target_type,target_id FROM audit_effects WHERE audit_event_id=?1 ORDER BY ordinal")
        .map_err(decode_sqlite_error)?;
    let mut effects = statement
        .query_map([event_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(decode_sqlite_error)?;
    let effects: Vec<_> = effects
        .by_ref()
        .collect::<Result<_, _>>()
        .map_err(decode_sqlite_error)?;
    if effects.len() != 1
        || effects[0].0 != event_id.as_str()
        || effects[0].1 != 0
        || effects[0].2 != code.as_str()
        || effects[0].3 != scope.as_persisted()
        || effects[0].4 != "action_request"
        || effects[0].5 != request.as_str()
    {
        return Err(ActionPersistenceLoadError::InvalidActionSnapshot);
    }
    let effect = AuditEffectCode::parse(effects[0].2.clone())
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    let disposition = AuditDisposition::new(policy, approval, execution, scope, vec![effect])
        .map_err(|_| ActionPersistenceLoadError::InvalidActionSnapshot)?;
    Ok(AuditEvent::new(
        event_id,
        UtcTimestamp::from_unix_millis(when),
        actor,
        AuditAction::new(module, code, AuditTarget::ActionRequest(target)),
        corr,
        disposition,
    ))
}

fn action_owned_shared_topology_is_empty(
    connection: &Transaction<'_>,
) -> Result<bool, ActionPersistenceLoadError> {
    const ACTION_SHARED_TOPOLOGY_ROW: &str = r#"
        SELECT 1 FROM prepared_intent_targets
            WHERE target_type IN ('action_request', 'action')
        -- Decision-triggered Action provenance gap fix, widened again
        -- for downstream-marking test coverage:
        -- `persist_resolve_prepared`/`persist_supersede_prepared`
        -- legitimately write an Action-shaped `prepared_intent_effects` row
        -- for a resulting Action Request that does not durably exist yet,
        -- or for Supersede's own `incomplete_downstream` flagging -- the
        -- four known effect codes excluded below
        -- (`action_request.created_from_decision` from
        -- `WorkManagementEffect::CreateResultingActionRequest`,
        -- `decision.action_request_linked` from
        -- `WorkManagementEffect::LinkDecisionToActionRequest`,
        -- `action_request.superseded_premise_flagged`/
        -- `action.superseded_premise_flagged` from
        -- `WorkManagementEffect::FlagSupersededPremiseActionRequest`/
        -- `FlagSupersededPremiseAction` -- unreachable until this session's
        -- own downstream-marking test was the first to ever populate
        -- `incomplete_downstream` in a real PREPARE), always tied to a
        -- `resolve_decision_request`- or `supersede_decision`-kind intent.
        -- Any OTHER Action-shaped effect row, on any intent kind, remains
        -- flagged -- this must not become a blanket exemption for the whole
        -- intent.
        UNION ALL SELECT 1 FROM prepared_intent_effects AS effect
            JOIN prepared_intents AS intent ON intent.id = effect.prepared_intent_id
            WHERE (effect.target_type IN ('action_request', 'action')
                OR effect.secondary_target_type IN ('action_request', 'action'))
              AND NOT (
                intent.intent_kind IN ('resolve_decision_request', 'supersede_decision')
                AND effect.effect_code IN ('action_request.created_from_decision', 'decision.action_request_linked', 'action_request.superseded_premise_flagged', 'action.superseded_premise_flagged')
              )
        UNION ALL SELECT 1 FROM prepared_intent_classification_sources
            WHERE role IN ('created_action', 'downstream_action_request', 'downstream_action', 'resulting_action_request')
        UNION ALL SELECT 1 FROM prepared_work_management_payloads AS payload
            JOIN prepared_intents AS intent ON intent.id = payload.prepared_intent_id
            WHERE intent.intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        -- Decision-triggered Action provenance gap fix (2026-09):
        -- `prepared_resulting_action_requests` and `prepared_incomplete_downstream`
        -- are deliberately NOT unioned here. Unlike every other row above,
        -- neither carries a WHERE clause scoping it to an Action-owned
        -- `prepared_intents` kind -- both are Decision Resolve/Supersede's
        -- own PREPARE-time staging tables (resulting Action Requests that do
        -- not exist yet, and Supersede's own `incomplete_downstream` list),
        -- populated by `persist_resolve_prepared`/`persist_supersede_prepared`
        -- well before any Action-side row is ever written. Unioning either
        -- unscoped meant this "Action namespace is empty" check went false
        -- the instant a Decision PREPARE staged one, even though zero Action
        -- Requests/Actions durably exist -- a false positive nothing
        -- exercised until Decision Resolve/Supersede started actually
        -- calling into this Action decode path.
        UNION ALL SELECT 1 FROM approval_receipts AS receipt
            JOIN prepared_intents AS intent ON intent.id = receipt.prepared_intent_id
            WHERE intent.intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        UNION ALL SELECT 1 FROM operations AS operation
            JOIN prepared_intents AS intent ON intent.id = operation.prepared_intent_id
            WHERE intent.intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        UNION ALL SELECT 1 FROM operations WHERE namespace = 'action'
        UNION ALL SELECT 1 FROM operation_effects
            WHERE target_type IN ('action_request', 'action')
        LIMIT 1
    "#;
    query_has_row(connection, ACTION_SHARED_TOPOLOGY_ROW)
        .map(|has_row| !has_row)
        .map_err(classify_sqlite_error)
}

fn action_namespace_is_empty(connection: &Transaction<'_>) -> Result<bool, rusqlite::Error> {
    // One statement gives every probe one SQLite read snapshot.  Do not split
    // these probes into autocommit queries: another writer could otherwise
    // commit Action state between probes and fabricate an empty snapshot.
    const ACTION_NAMESPACE_ROW: &str = r#"
        SELECT 1 FROM action_requests
        UNION ALL SELECT 1 FROM actions
        UNION ALL SELECT 1 FROM action_transitions
        UNION ALL SELECT 1 FROM action_completion_evidence
        UNION ALL SELECT 1 FROM action_replay_operations
        UNION ALL SELECT 1 FROM action_replay_audits
        UNION ALL SELECT 1 FROM action_discarded_prepared_intents
        UNION ALL SELECT 1 FROM action_reject_prepared_command_results
        UNION ALL SELECT 1 FROM action_replay_error_params
        UNION ALL SELECT 1 FROM action_replay_error_extensions
        UNION ALL SELECT 1 FROM action_command_create_requests
        UNION ALL SELECT 1 FROM action_command_transition_requests
        UNION ALL SELECT 1 FROM action_command_prepare_accepts
        UNION ALL SELECT 1 FROM action_command_execute_accepts
        UNION ALL SELECT 1 FROM action_command_prepare_completes
        UNION ALL SELECT 1 FROM action_command_prepare_cancels
        UNION ALL SELECT 1 FROM action_command_prepare_reopens
        UNION ALL SELECT 1 FROM action_command_execute_actions
        UNION ALL SELECT 1 FROM action_command_start_actions
        UNION ALL SELECT 1 FROM action_command_link_completion_evidence
        UNION ALL SELECT 1 FROM idempotency_outcomes
            WHERE namespace = 'action'
        UNION ALL SELECT 1 FROM aggregate_registry
            WHERE aggregate_type IN ('action_request', 'action')
        UNION ALL SELECT 1 FROM audit_events
            WHERE target_type IN ('action_request', 'action')
        UNION ALL SELECT 1 FROM audit_effects
            WHERE target_type IN ('action_request', 'action')
        UNION ALL SELECT 1 FROM prepared_intents
            WHERE intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        UNION ALL SELECT 1 FROM prepared_intent_targets
            WHERE target_type IN ('action_request', 'action')
        -- Decision-triggered Action provenance gap fix, widened again
        -- for downstream-marking test coverage -- see `action_owned_shared_topology_is_empty`'s own,
        -- more detailed comment above the identical clause for the full
        -- reasoning (`action_request.superseded_premise_flagged`/
        -- `action.superseded_premise_flagged` from
        -- `WorkManagementEffect::FlagSupersededPremiseActionRequest`/
        -- `FlagSupersededPremiseAction` needed the same exemption
        -- `action_request.created_from_decision`/`decision.action_request_linked`
        -- already had).
        -- `prepared_intent_targets`/`prepared_intent_classification_sources`
        -- need no such exemption: `persist_resolve_prepared` never writes an
        -- Action-shaped row to either (its own `prepared_intent_targets`
        -- entry is always `target_type='decision_request'`, and it never
        -- touches `prepared_intent_classification_sources` at all), so any
        -- Action-shaped row there is genuinely orphaned regardless of the
        -- owning intent's kind.
        UNION ALL SELECT 1 FROM prepared_intent_effects AS effect
            JOIN prepared_intents AS intent ON intent.id = effect.prepared_intent_id
            WHERE (effect.target_type IN ('action_request', 'action')
                OR effect.secondary_target_type IN ('action_request', 'action'))
              AND NOT (
                intent.intent_kind IN ('resolve_decision_request', 'supersede_decision')
                AND effect.effect_code IN ('action_request.created_from_decision', 'decision.action_request_linked', 'action_request.superseded_premise_flagged', 'action.superseded_premise_flagged')
              )
        UNION ALL SELECT 1 FROM prepared_intent_classification_sources
            WHERE role IN ('created_action', 'downstream_action_request', 'downstream_action', 'resulting_action_request')
        UNION ALL SELECT 1 FROM prepared_work_management_payloads AS payload
            JOIN prepared_intents AS intent ON intent.id = payload.prepared_intent_id
            WHERE intent.intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        -- Decision-triggered Action provenance gap fix (2026-09):
        -- `prepared_resulting_action_requests` and `prepared_incomplete_downstream`
        -- are deliberately NOT unioned here. Unlike every other row above,
        -- neither carries a WHERE clause scoping it to an Action-owned
        -- `prepared_intents` kind -- both are Decision Resolve/Supersede's
        -- own PREPARE-time staging tables (resulting Action Requests that do
        -- not exist yet, and Supersede's own `incomplete_downstream` list),
        -- populated by `persist_resolve_prepared`/`persist_supersede_prepared`
        -- well before any Action-side row is ever written. Unioning either
        -- unscoped meant this "Action namespace is empty" check went false
        -- the instant a Decision PREPARE staged one, even though zero Action
        -- Requests/Actions durably exist -- a false positive nothing
        -- exercised until Decision Resolve/Supersede started actually
        -- calling into this Action decode path.
        UNION ALL SELECT 1 FROM approval_receipts AS receipt
            JOIN prepared_intents AS intent ON intent.id = receipt.prepared_intent_id
            WHERE intent.intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        UNION ALL SELECT 1 FROM operations AS operation
            JOIN prepared_intents AS intent ON intent.id = operation.prepared_intent_id
            WHERE intent.intent_kind IN ('accept_action_request', 'complete_action', 'cancel_action', 'reopen_action')
        UNION ALL SELECT 1 FROM operations
            WHERE namespace = 'action'
        UNION ALL SELECT 1 FROM operation_effects
            WHERE target_type IN ('action_request', 'action')
        LIMIT 1
    "#;

    query_has_row(connection, ACTION_NAMESPACE_ROW).map(|has_row| !has_row)
}

fn query_has_row(connection: &Transaction<'_>, query: &str) -> Result<bool, rusqlite::Error> {
    Ok(connection
        .query_row(query, [], |_| Ok(()))
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests {
    use rusqlite::{ffi, Error as SqliteError};

    use super::{classify_sqlite_error, ActionPersistenceLoadError};

    #[test]
    fn sqlite_busy_is_reported_as_storage_unavailable() {
        let error = SqliteError::SqliteFailure(ffi::Error::new(ffi::SQLITE_BUSY), None);

        assert_eq!(
            classify_sqlite_error(error),
            ActionPersistenceLoadError::StorageUnavailable
        );
    }
}
