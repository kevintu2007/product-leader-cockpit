//! The desktop's Risk H2a facade: prepare the exact preview for
//! recording an occurrence or closing a Risk, approve it, or refuse it
//! durably.
//!
//! Same contract as [`crate::decision_requests`]: the Ledger is the
//! authority, the host supplies ids and time, the actor is the Head of
//! Products, and the H2a `Confirmed` is the explicit Approve. Two things are
//! specific here:
//!
//! - Recording an occurrence *creates* an Issue, so the host mints that
//!   Issue's id before the preview is built -- the preview names it and the
//!   payload digest binds it. A second attempt at the same client request
//!   must therefore return the first attempt's preview rather than mint a
//!   new identity: see [`recover_prepared`].
//! - The preview is built by the domain service, rehydrated from the Risk
//!   namespace *and* every Issue at its current state, because the identity
//!   authority must be able to see an Issue that already carries the id an
//!   occurrence would create.

#![allow(clippy::result_large_err)]

use pmc_domain::audit::AuditActor;
use pmc_domain::error::DomainError;
use pmc_domain::identity::{AggregateVersion, IdempotencyId, PreparedIntentId, RiskId};
use pmc_domain::issues::{
    DenyIssueEvidenceAuthority, IssueH2aRuntimeSnapshot, IssueServiceIdSource,
    RecordedIssueClassification,
};
use pmc_domain::risks::{
    AllowRiskEvidence, ApproveAndExecuteCloseRisk, ApproveAndExecuteRecordRiskOccurrence,
    OccurredRiskOutcome, PrepareCloseRisk, PrepareRecordRiskOccurrence, RecordedRiskClassification,
    RejectRiskPreparedIntent, RiskH2aRuntimeSnapshot, RiskMutationOutcome, RiskOperationContext,
    RiskRecord, RiskServiceIdSource, UpdateRiskResponse,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ApprovalConfirmation, PreparedIntentError, RejectedPreparedIntentOutcome,
    WorkManagementApproval, WorkManagementPayloadDigest, WorkManagementPreparedIntent,
    WorkManagementRationale,
};
use pmc_domain::work_management_runtime::WorkManagementRuntimeComposition;
use pmc_domain::DomainValueError;
use pmc_ledger::sqlite::{
    IssuePersistenceLoadError, LedgerTransactionError, RiskPersistenceLoadError,
    SqliteProductLedger,
};

use crate::desktop_runtime::{FixedClock, OpaqueIdSource};
use crate::work_management_authority::{HeadOfProductsApproval, SingleUserExecutionPolicy};

#[derive(Debug)]
pub enum RiskFlowError {
    /// The Risk namespace could not be loaded for rehydration.
    Load(RiskPersistenceLoadError),
    /// Every Issue at its current state could not be loaded. Recording an
    /// occurrence needs that picture to refuse a duplicate identity.
    LoadIssues(IssuePersistenceLoadError),
    /// The domain refused (stale version, illegal transition, policy, ...).
    Domain(DomainError),
    /// The Ledger refused or failed the write.
    Ledger(LedgerTransactionError<DomainError>),
    /// The approval could not be constructed (a malformed digest).
    Approval(PreparedIntentError),
    /// The host's id source produced an invalid identifier: a host bug.
    Id(DomainValueError),
    /// This client request already prepared a preview, and that preview has
    /// since been approved or refused. Re-sending the same request cannot
    /// return it, and must not silently prepare a second one.
    PreviewAlreadyConsumed,
}

impl From<LedgerTransactionError<DomainError>> for RiskFlowError {
    fn from(error: LedgerTransactionError<DomainError>) -> Self {
        Self::Ledger(error)
    }
}

/// Never invoked: an occurrence stages its Issue through the shared identity
/// authority inside the Risk service, never by calling the Issue service.
/// Mirrors the Ledger's own `UnusedIssueIds`.
#[derive(Clone, Copy, Default)]
struct UnusedIssueIds;

impl IssueServiceIdSource for UnusedIssueIds {
    fn next_prepared_intent_id(&mut self) -> Result<PreparedIntentId, DomainValueError> {
        PreparedIntentId::parse(String::new())
    }
    fn next_approval_receipt_id(
        &mut self,
    ) -> Result<pmc_domain::identity::ApprovalReceiptId, DomainValueError> {
        pmc_domain::identity::ApprovalReceiptId::parse(String::new())
    }
    fn next_audit_event_id(
        &mut self,
    ) -> Result<pmc_domain::identity::AuditEventId, DomainValueError> {
        pmc_domain::identity::AuditEventId::parse(String::new())
    }
}

/// The preview this client request already produced, if any.
///
/// The host mints the prepared-intent id -- and, for an occurrence, the id of
/// the Issue it will create -- before it can build a preview, and the
/// Ledger's PREPARE replay compares those ids against the ones it stored. A
/// preview cannot be rebuilt either: its expiry is part of the payload
/// digest, so a second attempt at a later instant is a different preview.
/// So a retry returns what the first attempt stored, and a preview that has
/// since been consumed is reported as such rather than quietly re-prepared.
fn recover_prepared(
    ledger: &SqliteProductLedger,
    idempotency: &IdempotencyId,
) -> Result<Option<WorkManagementPreparedIntent>, RiskFlowError> {
    let Some(prepared_id) = ledger
        .risk_prepared_intent_for_client_request(idempotency)
        .map_err(RiskFlowError::Load)?
    else {
        return Ok(None);
    };
    let snapshot = ledger
        .load_risk_h2a_runtime_snapshot()
        .map_err(RiskFlowError::Load)?;
    snapshot
        .prepared()
        .iter()
        .find(|intent| intent.id() == &prepared_id)
        .cloned()
        .map(Some)
        .ok_or(RiskFlowError::PreviewAlreadyConsumed)
}

/// The two snapshots a Risk preview is built from: the Risk namespace, and
/// every Issue at its current state -- the identity authority must be able to
/// see an Issue that already carries the id an occurrence would create.
fn snapshots(
    ledger: &SqliteProductLedger,
) -> Result<(RiskH2aRuntimeSnapshot, IssueH2aRuntimeSnapshot), RiskFlowError> {
    let risks = ledger
        .load_risk_h2a_runtime_snapshot()
        .map_err(RiskFlowError::Load)?;
    let issues = ledger
        .load_issue_h2a_runtime_snapshot()
        .map_err(RiskFlowError::LoadIssues)?;
    Ok((risks, issues))
}

/// H2a step 1 for recording an occurrence. The host mints the resulting
/// Issue's id; the person supplies only the Risk they read and its version.
pub fn prepare_record_risk_occurrence(
    ledger: &mut SqliteProductLedger,
    risk_id: RiskId,
    expected_version: AggregateVersion,
    context: RiskOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, RiskFlowError> {
    if let Some(prepared) = recover_prepared(ledger, &context.idempotency_id)? {
        return Ok(prepared);
    }
    let issue_id = ids.next_issue_id().map_err(RiskFlowError::Id)?;
    let command = PrepareRecordRiskOccurrence {
        risk_id,
        expected_version,
        issue_id,
        context,
    };
    let (risks, issues) = snapshots(ledger)?;
    let mut composition = WorkManagementRuntimeComposition::rehydrate_with_risk_h2a_and_issue_h2a(
        FixedClock(now),
        ids,
        HeadOfProductsApproval,
        SingleUserExecutionPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
        FixedClock(now),
        UnusedIssueIds,
        HeadOfProductsApproval,
        SingleUserExecutionPolicy,
        DenyIssueEvidenceAuthority,
        RecordedIssueClassification,
        risks,
        issues,
    )
    .map_err(|_| RiskFlowError::Load(RiskPersistenceLoadError::InvalidRiskSnapshot))?;
    let prepared = composition
        .prepare_record_risk_occurrence(command.clone())
        .map_err(RiskFlowError::Domain)?;
    ledger
        .prepare_record_risk_occurrence(command, prepared)
        .map_err(RiskFlowError::Ledger)
}

/// H2a step 1 for closing a Risk, with the person's own rationale.
pub fn prepare_close_risk(
    ledger: &mut SqliteProductLedger,
    risk_id: RiskId,
    expected_version: AggregateVersion,
    rationale: WorkManagementRationale,
    context: RiskOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, RiskFlowError> {
    if let Some(prepared) = recover_prepared(ledger, &context.idempotency_id)? {
        return Ok(prepared);
    }
    let command = PrepareCloseRisk {
        risk_id,
        expected_version,
        rationale,
        context,
    };
    let (risks, issues) = snapshots(ledger)?;
    let mut composition = WorkManagementRuntimeComposition::rehydrate_with_risk_h2a_and_issue_h2a(
        FixedClock(now),
        ids,
        HeadOfProductsApproval,
        SingleUserExecutionPolicy,
        AllowRiskEvidence,
        RecordedRiskClassification,
        FixedClock(now),
        UnusedIssueIds,
        HeadOfProductsApproval,
        SingleUserExecutionPolicy,
        DenyIssueEvidenceAuthority,
        RecordedIssueClassification,
        risks,
        issues,
    )
    .map_err(|_| RiskFlowError::Load(RiskPersistenceLoadError::InvalidRiskSnapshot))?;
    let prepared = composition
        .prepare_close_risk(command.clone())
        .map_err(RiskFlowError::Domain)?;
    ledger
        .prepare_close_risk(command, prepared)
        .map_err(RiskFlowError::Ledger)
}

fn approval(
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: &RiskOperationContext,
) -> Result<WorkManagementApproval, RiskFlowError> {
    WorkManagementApproval::new(
        prepared_id,
        AuditActor::HeadOfProducts,
        acknowledged_payload_digest,
        context.idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .map_err(RiskFlowError::Approval)
}

/// H2a step 2 for an occurrence: three audits (the Risk transition, the
/// Issue it creates, the link between them) and one receipt.
pub fn approve_and_execute_record_risk_occurrence(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: RiskOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<OccurredRiskOutcome, RiskFlowError> {
    let approval = approval(prepared_id, acknowledged_payload_digest, &context)?;
    let audit_event_ids = [
        RiskServiceIdSource::next_audit_event_id(ids).map_err(RiskFlowError::Id)?,
        RiskServiceIdSource::next_audit_event_id(ids).map_err(RiskFlowError::Id)?,
        RiskServiceIdSource::next_audit_event_id(ids).map_err(RiskFlowError::Id)?,
    ];
    let receipt_id =
        RiskServiceIdSource::next_approval_receipt_id(ids).map_err(RiskFlowError::Id)?;
    ledger
        .approve_and_execute_record_risk_occurrence(
            ApproveAndExecuteRecordRiskOccurrence { approval, context },
            audit_event_ids,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .map_err(RiskFlowError::Ledger)
}

/// H2a step 2 for a close: one audit and one receipt.
pub fn approve_and_execute_close_risk(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: RiskOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RiskMutationOutcome<RiskRecord>, RiskFlowError> {
    let approval = approval(prepared_id, acknowledged_payload_digest, &context)?;
    let audit_event_id =
        RiskServiceIdSource::next_audit_event_id(ids).map_err(RiskFlowError::Id)?;
    let receipt_id =
        RiskServiceIdSource::next_approval_receipt_id(ids).map_err(RiskFlowError::Id)?;
    ledger
        .approve_and_execute_close_risk(
            ApproveAndExecuteCloseRisk { approval, context },
            audit_event_id,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .map_err(RiskFlowError::Ledger)
}

/// H1 `UpdateRiskResponse` (v47): the person's response to an Open Risk,
/// one Ledger transaction, replayed by its own typed rows. The Ledger writer
/// applies the domain's rules itself (parity is tested), so no in-memory
/// rehydration is needed.
pub fn update_risk_response(
    ledger: &mut SqliteProductLedger,
    command: UpdateRiskResponse,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RiskMutationOutcome<RiskRecord>, RiskFlowError> {
    let audit = RiskServiceIdSource::next_audit_event_id(ids).map_err(RiskFlowError::Id)?;
    ledger
        .update_risk_response(command, audit, now)
        .map_err(RiskFlowError::Ledger)
}

/// H2a rejection of either Risk preview: durable, audited, no effect (v46).
pub fn reject_risk_prepared_intent(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    context: RiskOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RejectedPreparedIntentOutcome, RiskFlowError> {
    let audit = RiskServiceIdSource::next_audit_event_id(ids).map_err(RiskFlowError::Id)?;
    ledger
        .reject_risk_prepared_intent(
            RejectRiskPreparedIntent {
                prepared_id,
                actor: AuditActor::HeadOfProducts,
                context,
            },
            audit,
            now,
            HeadOfProductsApproval,
        )
        .map_err(RiskFlowError::Ledger)
}
