//! The application facade for Action Requests and Actions: what a desktop
//! command calls so that no domain logic is re-derived in the host.
//!
//! The SQLite Ledger's `prepare_accept_action_request` takes an already
//! canonical `WorkManagementPreparedIntent` (it persists; it does not
//! prepare). Preparing is the domain's job: rehydrate `InMemoryActionService`
//! from the Ledger's persistence snapshot, let it produce the canonical
//! intent (targets, expected versions, classification fold, digest, expiry),
//! then hand that intent to the Ledger to persist. Hand-building the intent
//! in the host would duplicate that logic and drift from it.
//!
//! Everything time- and identity-shaped comes in from the host: one `now`
//! per command (the Ledger never originates a timestamp) and an id source
//! for the Action, Prepared Intent, receipt and audit ids the webview must
//! never supply.
//!
//! Approval is constructed here with the fixed `HeadOfProducts` actor and
//! `Confirmed`: the confirmation is the person's explicit Approve, which is
//! all H2a requires (no typed phrase -- that is H2b).

#![allow(clippy::result_large_err)]

use pmc_domain::actions::ActionServiceIdSource;
use pmc_domain::actions::{
    AcceptedActionOutcome, ActionMutationOutcome, ActionRecord, ActionRequestRecord,
    ApproveAndExecuteAcceptActionRequest, DeclineActionRequest, InMemoryActionService,
    PrepareAcceptActionRequest, RejectActionPreparedIntent, StartAction, WithdrawActionRequest,
};
use pmc_domain::audit::AuditActor;
use pmc_domain::error::DomainError;
use pmc_domain::identity::PreparedIntentId;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ApprovalConfirmation, PreparedIntentError, RejectedPreparedIntentOutcome,
    WorkManagementApproval, WorkManagementPayloadDigest, WorkManagementPreparedIntent,
};
use pmc_domain::DomainValueError;
use pmc_ledger::sqlite::{ActionPersistenceLoadError, LedgerTransactionError, SqliteProductLedger};

use crate::desktop_runtime::{FixedClock, OpaqueIdSource};
use crate::work_management_authority::{
    HeadOfProductsApproval, NoEvidenceAuthority, SingleUserExecutionPolicy,
};

/// Fail-closed errors of this facade. Every variant means nothing was
/// written, except `Ledger`, whose own variants say what the Ledger did.
#[derive(Debug)]
pub enum ActionRequestFlowError {
    /// The Ledger's Action persistence could not be loaded for rehydration.
    Load(ActionPersistenceLoadError),
    /// The domain refused to prepare (stale version, illegal transition,
    /// classification, ...). Carries the safe envelope.
    Domain(DomainError),
    /// The Ledger refused or failed the write.
    Ledger(LedgerTransactionError<DomainError>),
    /// The approval could not be constructed -- with the fixed actor and
    /// confirmation this only happens for a malformed digest.
    Approval(PreparedIntentError),
    /// The host's id source produced an invalid identifier: a host bug.
    Id(DomainValueError),
}

impl From<LedgerTransactionError<DomainError>> for ActionRequestFlowError {
    fn from(error: LedgerTransactionError<DomainError>) -> Self {
        Self::Ledger(error)
    }
}

/// H2a step 1: prepare the exact preview for accepting an Action Request.
///
/// Returns the persisted canonical intent: its `payload_digest()` is what
/// the person acknowledges on Approve, and its `expires_at()` is when the
/// preview stops being valid.
pub fn prepare_accept_action_request(
    ledger: &mut SqliteProductLedger,
    command: PrepareAcceptActionRequest,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, ActionRequestFlowError> {
    let snapshot = ledger
        .load_action_persistence_snapshot()
        .map_err(ActionRequestFlowError::Load)?;
    let mut service = InMemoryActionService::rehydrate(
        FixedClock(now),
        ids,
        HeadOfProductsApproval,
        SingleUserExecutionPolicy,
        NoEvidenceAuthority,
        snapshot,
    );
    let prepared = service
        .prepare_accept_action_request(command.clone())
        .map_err(ActionRequestFlowError::Domain)?;
    ledger
        .prepare_accept_action_request(command, prepared)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 2: the person pressed Approve on the exact preview.
///
/// `acknowledged_payload_digest` must be the digest the preview showed;
/// the Ledger re-validates it, the expiry, the target versions and the
/// policy inside one transaction and consumes the Prepared Intent exactly
/// once. A repeat with the same idempotency id replays the original
/// outcome; a different payload under the same id is a conflict.
pub fn approve_and_execute_accept_action_request(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: pmc_domain::actions::ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<AcceptedActionOutcome, ActionRequestFlowError> {
    let approval = WorkManagementApproval::new(
        prepared_id,
        AuditActor::HeadOfProducts,
        acknowledged_payload_digest,
        context.idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .map_err(ActionRequestFlowError::Approval)?;
    let audit_event_ids = [
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?,
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?,
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?,
    ];
    let receipt_id = ids
        .next_approval_receipt_id()
        .map_err(ActionRequestFlowError::Id)?;
    ledger
        .approve_and_execute_accept_action_request(
            ApproveAndExecuteAcceptActionRequest { approval, context },
            audit_event_ids,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
        )
        .map_err(ActionRequestFlowError::Ledger)
}

/// H1-User: decline an Open Action Request with a rationale.
pub fn decline_action_request(
    ledger: &mut SqliteProductLedger,
    command: DeclineActionRequest,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionRequestFlowError> {
    let audit =
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?;
    ledger
        .decline_action_request(command, audit, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H1-User: withdraw an Open Action Request with a rationale.
pub fn withdraw_action_request(
    ledger: &mut SqliteProductLedger,
    command: WithdrawActionRequest,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRequestRecord>, ActionRequestFlowError> {
    let audit =
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?;
    ledger
        .withdraw_action_request(command, audit, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H1-User: start an accepted Action.
pub fn start_action(
    ledger: &mut SqliteProductLedger,
    command: StartAction,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRecord>, ActionRequestFlowError> {
    let audit =
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?;
    ledger
        .start_action(command, audit, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a rejection: the person pressed Reject on the exact preview. Durable
/// and audited in the Ledger (v45); consumes the intent with no effect and
/// mints no receipt. Replaying the same context returns the same outcome.
pub fn reject_action_prepared_intent(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    context: pmc_domain::actions::ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RejectedPreparedIntentOutcome, ActionRequestFlowError> {
    let audit =
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?;
    ledger
        .reject_action_prepared_intent(
            RejectActionPreparedIntent {
                prepared_id,
                actor: AuditActor::HeadOfProducts,
                context,
            },
            audit,
            now,
            HeadOfProductsApproval,
        )
        .map_err(ActionRequestFlowError::Ledger)
}
