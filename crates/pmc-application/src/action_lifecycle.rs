//! The desktop's Action lifecycle facade: link
//! completion Evidence, and the Complete / Cancel / Reopen H2a loops.
//!
//! Sibling of [`crate::action_requests`] with the same contract: the Ledger
//! is the authority, the host supplies ids and time through
//! [`OpaqueIdSource`] and a fixed `now`, the actor is the Head of Products,
//! and the H2a `Confirmed` is the person's explicit Approve.
//!
//! The three transition PREPAREs are different from Accept's: the SQLite
//! methods rehydrate the domain service themselves, with the Ledger's own
//! persisted Evidence authority (which since v45 carries each reference's
//! source version), so this facade only mints the Prepared Intent id. The
//! rejection of any of the three is
//! [`crate::action_requests::reject_action_prepared_intent`].

#![allow(clippy::result_large_err)]

use pmc_domain::actions::{
    ActionMutationOutcome, ActionOperationContext, ActionRecord, ActionServiceIdSource,
    ApproveAndExecuteCancelAction, ApproveAndExecuteCompleteAction, ApproveAndExecuteReopenAction,
    LinkActionCompletionEvidence, PrepareCancelAction, PrepareCompleteAction, PrepareReopenAction,
};
use pmc_domain::audit::AuditActor;
use pmc_domain::composition_source::{
    EvidenceReferenceReadRecord, LedgerSnapshotForCompositionPort,
};
use pmc_domain::identity::{ActionId, PreparedIntentId};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ApprovalConfirmation, WorkManagementApproval, WorkManagementPayloadDigest,
    WorkManagementPreparedIntent,
};
use pmc_ledger::sqlite::SqliteProductLedger;

use crate::action_requests::ActionRequestFlowError;
use crate::desktop_runtime::OpaqueIdSource;
use crate::work_management_authority::{HeadOfProductsApproval, SingleUserExecutionPolicy};

/// What S03 needs before it can link Evidence to an Action or prepare its
/// completion: the Action as the Ledger holds it now, and every Evidence
/// reference the Ledger holds, as read records (id, role, verification,
/// pinned, classification, version; never a Vault path). Read from the
/// Action persistence snapshot and the composition snapshot; the two are
/// reported with the composition's Ledger revision, and the caller decides
/// whether a mismatch with the Action's own revision matters.
#[derive(Clone, Debug)]
pub struct ActionCompletionContext {
    pub action: ActionRecord,
    pub ledger_revision: u64,
    pub evidence_references: Vec<EvidenceReferenceReadRecord>,
}

#[derive(Debug)]
pub enum ActionCompletionContextError {
    /// The Ledger's Action persistence could not be loaded.
    Load(pmc_ledger::sqlite::ActionPersistenceLoadError),
    /// The composition snapshot could not be read.
    Composition(pmc_domain::composition_source::CompositionSnapshotReadError),
    /// No Action with this id.
    NotFound,
}

pub fn action_completion_context(
    ledger: &SqliteProductLedger,
    action_id: &ActionId,
    now: UtcTimestamp,
) -> Result<ActionCompletionContext, ActionCompletionContextError> {
    let snapshot = ledger
        .load_action_persistence_snapshot()
        .map_err(ActionCompletionContextError::Load)?;
    let action = snapshot
        .actions()
        .iter()
        .find(|action| action.id() == action_id)
        .cloned()
        .ok_or(ActionCompletionContextError::NotFound)?;
    let composition = ledger
        .read_composition_snapshot(now)
        .map_err(ActionCompletionContextError::Composition)?;
    Ok(ActionCompletionContext {
        action,
        ledger_revision: composition.ledger_revision,
        evidence_references: composition.evidence_references,
    })
}

/// H1: link an Evidence reference as completion evidence of an InProgress
/// Action. The Ledger resolves the reference (role, classification,
/// verification) itself; the caller names only the id.
pub fn link_action_completion_evidence(
    ledger: &mut SqliteProductLedger,
    command: LinkActionCompletionEvidence,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRecord>, ActionRequestFlowError> {
    let audit =
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?;
    ledger
        .link_action_completion_evidence(command, audit, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 1: the exact preview for completing an InProgress Action. The
/// Ledger evaluates the linked Evidence (and the optional Judgment) against
/// the support gate; a denial is a durable H3 terminal, not an exception.
pub fn prepare_complete_action(
    ledger: &mut SqliteProductLedger,
    command: PrepareCompleteAction,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, ActionRequestFlowError> {
    let prepared_id = prepared_intent_id(ids)?;
    ledger
        .prepare_complete_action(command, prepared_id, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 1: the exact preview for cancelling an Open or InProgress Action.
pub fn prepare_cancel_action(
    ledger: &mut SqliteProductLedger,
    command: PrepareCancelAction,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, ActionRequestFlowError> {
    let prepared_id = prepared_intent_id(ids)?;
    ledger
        .prepare_cancel_action(command, prepared_id, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 1: the exact preview for reopening a Completed or Cancelled
/// Action, in the mode the person chose.
pub fn prepare_reopen_action(
    ledger: &mut SqliteProductLedger,
    command: PrepareReopenAction,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, ActionRequestFlowError> {
    let prepared_id = prepared_intent_id(ids)?;
    ledger
        .prepare_reopen_action(command, prepared_id, now)
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 2 for Complete.
pub fn approve_and_execute_complete_action(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRecord>, ActionRequestFlowError> {
    let approval = approval(prepared_id, acknowledged_payload_digest, &context)?;
    let (audit, receipt) = execution_ids(ids)?;
    ledger
        .approve_and_execute_complete_action(
            ApproveAndExecuteCompleteAction { approval, context },
            audit,
            receipt,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
        )
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 2 for Cancel.
pub fn approve_and_execute_cancel_action(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRecord>, ActionRequestFlowError> {
    let approval = approval(prepared_id, acknowledged_payload_digest, &context)?;
    let (audit, receipt) = execution_ids(ids)?;
    ledger
        .approve_and_execute_cancel_action(
            ApproveAndExecuteCancelAction { approval, context },
            audit,
            receipt,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
        )
        .map_err(ActionRequestFlowError::Ledger)
}

/// H2a step 2 for Reopen.
pub fn approve_and_execute_reopen_action(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRecord>, ActionRequestFlowError> {
    let approval = approval(prepared_id, acknowledged_payload_digest, &context)?;
    let (audit, receipt) = execution_ids(ids)?;
    ledger
        .approve_and_execute_reopen_action(
            ApproveAndExecuteReopenAction { approval, context },
            audit,
            receipt,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
        )
        .map_err(ActionRequestFlowError::Ledger)
}

fn prepared_intent_id(
    ids: &mut OpaqueIdSource,
) -> Result<PreparedIntentId, ActionRequestFlowError> {
    ActionServiceIdSource::next_prepared_intent_id(ids).map_err(ActionRequestFlowError::Id)
}

fn execution_ids(
    ids: &mut OpaqueIdSource,
) -> Result<
    (
        pmc_domain::identity::AuditEventId,
        pmc_domain::identity::ApprovalReceiptId,
    ),
    ActionRequestFlowError,
> {
    let audit =
        ActionServiceIdSource::next_audit_event_id(ids).map_err(ActionRequestFlowError::Id)?;
    let receipt =
        ActionServiceIdSource::next_approval_receipt_id(ids).map_err(ActionRequestFlowError::Id)?;
    Ok((audit, receipt))
}

fn approval(
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: &ActionOperationContext,
) -> Result<WorkManagementApproval, ActionRequestFlowError> {
    WorkManagementApproval::new(
        prepared_id,
        AuditActor::HeadOfProducts,
        acknowledged_payload_digest,
        context.idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .map_err(ActionRequestFlowError::Approval)
}
