//! The desktop's Issue H2a facade: prepare the exact preview for
//! resolving, closing or reopening an Issue, approve it, or refuse it
//! durably.
//!
//! Same contract as [`crate::decision_requests`]. Two things are specific:
//!
//! - All three operations are Evidence-gated, and each wants Evidence in its
//!   own role: resolution, closure verification, failed verification. The
//!   webview names Evidence ids only; the role belongs to the operation and
//!   is assigned here, where it cannot be steered from outside.
//! - The preview is built by the domain service, rehydrated from every Issue
//!   at its current state. An Issue being closed or reopened is *Resolved*,
//!   so the create-state snapshot cannot answer for it at all.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use pmc_domain::audit::AuditActor;
use pmc_domain::error::{DomainError, ErrorCode, MessageKey};
use pmc_domain::evidence::EvidenceReferenceRecord;
use pmc_domain::identity::{
    AggregateVersion, CorrelationId, EvidenceReferenceId, IssueId, PreparedIntentId,
};
use pmc_domain::issues::{
    ApproveAndExecuteIssueTransition, IssueEvidenceAuthorityError, IssueEvidenceAuthorityPort,
    IssueMutationOutcome, IssueOperationContext, IssueServiceIdSource, PrepareCloseIssue,
    PrepareReopenIssue, PrepareResolveIssue, RecordedIssueClassification,
    RejectIssuePreparedIntent,
};
use pmc_domain::risks::{AllowRiskEvidence, RecordedRiskClassification, RiskServiceIdSource};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ApprovalConfirmation, EvidenceReferenceMetadata, EvidenceRole, HumanJudgment,
    IssueResolutionType, PreparedIntentError, RejectedPreparedIntentOutcome,
    WorkManagementApproval, WorkManagementOperation, WorkManagementPayloadDigest,
    WorkManagementPreparedIntent, WorkManagementRationale,
};
use pmc_domain::work_management_runtime::WorkManagementRuntimeComposition;
use pmc_domain::DomainValueError;
use pmc_ledger::sqlite::{IssuePersistenceLoadError, LedgerTransactionError, SqliteProductLedger};

use crate::desktop_runtime::{FixedClock, OpaqueIdSource};
use crate::work_management_authority::{HeadOfProductsApproval, SingleUserExecutionPolicy};

#[derive(Debug)]
pub enum IssueFlowError {
    /// Every Issue at its current state could not be loaded.
    Load(IssuePersistenceLoadError),
    /// The domain refused (stale version, illegal transition, support gate,
    /// policy, ...).
    Domain(DomainError),
    /// The Ledger refused or failed the write.
    Ledger(LedgerTransactionError<DomainError>),
    /// The approval could not be constructed (a malformed digest).
    Approval(PreparedIntentError),
    /// The host's id source produced an invalid identifier: a host bug.
    Id(DomainValueError),
    /// This client request already prepared a preview, and that preview has
    /// since been approved or refused.
    PreviewAlreadyConsumed,
}

impl From<LedgerTransactionError<DomainError>> for IssueFlowError {
    fn from(error: LedgerTransactionError<DomainError>) -> Self {
        Self::Ledger(error)
    }
}

/// Never invoked: an Issue transition never reaches into the Risk service.
/// Mirrors the Ledger's own `UnusedRiskIds`.
#[derive(Clone, Copy, Default)]
struct UnusedRiskIds;

impl RiskServiceIdSource for UnusedRiskIds {
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

/// A fixed authority over exactly the Evidence references the person named,
/// as the Ledger held them when this facade read them.
///
/// An Evidence record carries no role of its own: a role describes what a
/// reference is being used *for*, and the operation decides that. So the role
/// is stamped here, from the operation, and the Ledger's own persist step
/// checks the stored preview carries the role that operation requires. A
/// reference the Ledger does not hold is simply absent, and the domain
/// refuses the preview.
#[derive(Clone, Debug, Default)]
pub struct LedgerIssueEvidenceAuthority {
    evidence: HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>,
}

impl LedgerIssueEvidenceAuthority {
    pub fn read(
        ledger: &SqliteProductLedger,
        evidence_ids: &[EvidenceReferenceId],
        role: EvidenceRole,
        correlation_id: &CorrelationId,
    ) -> Result<Self, IssueFlowError> {
        let mut evidence = HashMap::new();
        for id in evidence_ids {
            if let Some(record) = ledger
                .get_evidence_reference(id, correlation_id.clone())
                .map_err(IssueFlowError::Domain)?
            {
                evidence.insert(id.clone(), metadata(&record, role));
            }
        }
        Ok(Self { evidence })
    }
}

impl IssueEvidenceAuthorityPort for LedgerIssueEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, IssueEvidenceAuthorityError> {
        self.evidence
            .get(id)
            .cloned()
            .ok_or(IssueEvidenceAuthorityError::NotFound)
    }
}

fn metadata(record: &EvidenceReferenceRecord, role: EvidenceRole) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        record.id.clone(),
        record.version,
        record.classification,
        role,
        record.verification.clone(),
    )
}

/// What a prepare request asked for, compared with the preview an earlier
/// request under the same client request id produced.
struct PrepareRequest<'a> {
    operation: WorkManagementOperation,
    evidence_ids: &'a [EvidenceReferenceId],
    judgment: Option<&'a HumanJudgment>,
    context: &'a IssueOperationContext,
}

/// The preview this client request already produced, if any. See
/// `crate::risk_lifecycle::recover_prepared` for why a retry must not mint.
///
/// A retry returns that preview only when it asks for exactly the same
/// thing: the same operation, the same Evidence in the same order, and the
/// same Judgment (rationale and classification). Anything else under the
/// same client request id is an idempotency conflict, never the first
/// preview returned as if it answered the new request.
fn recover_prepared(
    ledger: &SqliteProductLedger,
    request: &PrepareRequest<'_>,
) -> Result<Option<WorkManagementPreparedIntent>, IssueFlowError> {
    let Some(prepared_id) = ledger
        .issue_prepared_intent_for_client_request(&request.context.idempotency_id)
        .map_err(IssueFlowError::Load)?
    else {
        return Ok(None);
    };
    let snapshot = ledger
        .load_issue_h2a_runtime_snapshot()
        .map_err(IssueFlowError::Load)?;
    let prepared = snapshot
        .prepared()
        .iter()
        .find(|intent| intent.id() == &prepared_id)
        .cloned()
        .ok_or(IssueFlowError::PreviewAlreadyConsumed)?;
    if !same_request(&prepared, request) {
        return Err(IssueFlowError::Domain(DomainError::new(
            ErrorCode::DomainIdempotencyConflict,
            MessageKey::parse("issue.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
            request.context.correlation_id.clone(),
            false,
        )));
    }
    Ok(Some(prepared))
}

fn same_request(prepared: &WorkManagementPreparedIntent, request: &PrepareRequest<'_>) -> bool {
    let Some(support) = prepared.preview().support() else {
        return false;
    };
    prepared.operation() == &request.operation
        && support
            .evidence()
            .iter()
            .map(EvidenceReferenceMetadata::id)
            .eq(request.evidence_ids.iter())
        && support.judgments().iter().eq(request.judgment)
}

macro_rules! prepare_issue_operation {
    ($ledger:expr, $ids:expr, $now:expr, $evidence:expr, $command:expr, $method:ident) => {{
        let snapshot = $ledger
            .load_issue_h2a_runtime_snapshot()
            .map_err(IssueFlowError::Load)?;
        let mut composition = WorkManagementRuntimeComposition::rehydrate_with_issue_h2a(
            FixedClock($now),
            UnusedRiskIds,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            AllowRiskEvidence,
            RecordedRiskClassification,
            FixedClock($now),
            $ids,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            $evidence,
            RecordedIssueClassification,
            snapshot,
        )
        .map_err(|_| IssueFlowError::Load(IssuePersistenceLoadError::InvalidIssueSnapshot))?;
        composition
            .$method($command)
            .map_err(IssueFlowError::Domain)?
    }};
}

/// H2a step 1 for resolving an Open Issue. The optional Judgment lets
/// partly verified Evidence (observed-unpinned, degraded) move forward as
/// verification-pending; it cannot carry unverified or mismatched Evidence.
#[allow(clippy::too_many_arguments)]
pub fn prepare_resolve_issue(
    ledger: &mut SqliteProductLedger,
    issue_id: IssueId,
    expected_version: AggregateVersion,
    resolution_type: IssueResolutionType,
    rationale: WorkManagementRationale,
    evidence_ids: Vec<EvidenceReferenceId>,
    judgment: Option<HumanJudgment>,
    context: IssueOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, IssueFlowError> {
    let request = PrepareRequest {
        operation: WorkManagementOperation::ResolveIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
            resolution_type,
            rationale: rationale.clone(),
        },
        evidence_ids: &evidence_ids,
        judgment: judgment.as_ref(),
        context: &context,
    };
    if let Some(prepared) = recover_prepared(ledger, &request)? {
        return Ok(prepared);
    }
    let evidence = LedgerIssueEvidenceAuthority::read(
        ledger,
        &evidence_ids,
        EvidenceRole::IssueResolution,
        &context.correlation_id,
    )?;
    let command = PrepareResolveIssue {
        issue_id,
        expected_version,
        resolution_type,
        rationale,
        evidence_ids,
        judgment,
        context,
    };
    let prepared = prepare_issue_operation!(
        ledger,
        ids,
        now,
        evidence,
        command.clone(),
        prepare_resolve_issue
    );
    ledger
        .prepare_resolve_issue(command, prepared)
        .map_err(IssueFlowError::Ledger)
}

/// H2a step 1 for closing a Resolved Issue: Evidence that the resolution was
/// verified. The operation itself carries no words; the only words are an
/// optional Judgment for partly verified Evidence, held on the support.
#[allow(clippy::too_many_arguments)]
pub fn prepare_close_issue(
    ledger: &mut SqliteProductLedger,
    issue_id: IssueId,
    expected_version: AggregateVersion,
    evidence_ids: Vec<EvidenceReferenceId>,
    judgment: Option<HumanJudgment>,
    context: IssueOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, IssueFlowError> {
    let request = PrepareRequest {
        operation: WorkManagementOperation::CloseIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
        },
        evidence_ids: &evidence_ids,
        judgment: judgment.as_ref(),
        context: &context,
    };
    if let Some(prepared) = recover_prepared(ledger, &request)? {
        return Ok(prepared);
    }
    let evidence = LedgerIssueEvidenceAuthority::read(
        ledger,
        &evidence_ids,
        EvidenceRole::IssueClosureVerification,
        &context.correlation_id,
    )?;
    let command = PrepareCloseIssue {
        issue_id,
        expected_version,
        evidence_ids,
        judgment,
        context,
    };
    let prepared = prepare_issue_operation!(
        ledger,
        ids,
        now,
        evidence,
        command.clone(),
        prepare_close_issue
    );
    ledger
        .prepare_close_issue(command, prepared)
        .map_err(IssueFlowError::Ledger)
}

/// H2a step 1 for reopening a Resolved Issue: Evidence that the verification
/// failed, plus the person's own rationale. The optional Judgment is a
/// separate thing: why relying on partly verified Evidence is acceptable.
#[allow(clippy::too_many_arguments)]
pub fn prepare_reopen_issue(
    ledger: &mut SqliteProductLedger,
    issue_id: IssueId,
    expected_version: AggregateVersion,
    rationale: WorkManagementRationale,
    evidence_ids: Vec<EvidenceReferenceId>,
    judgment: Option<HumanJudgment>,
    context: IssueOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, IssueFlowError> {
    let request = PrepareRequest {
        operation: WorkManagementOperation::ReopenIssue {
            issue_id: issue_id.clone(),
            issue_version: expected_version,
            rationale: rationale.clone(),
        },
        evidence_ids: &evidence_ids,
        judgment: judgment.as_ref(),
        context: &context,
    };
    if let Some(prepared) = recover_prepared(ledger, &request)? {
        return Ok(prepared);
    }
    let evidence = LedgerIssueEvidenceAuthority::read(
        ledger,
        &evidence_ids,
        EvidenceRole::IssueFailedVerification,
        &context.correlation_id,
    )?;
    let command = PrepareReopenIssue {
        issue_id,
        expected_version,
        rationale,
        evidence_ids,
        judgment,
        context,
    };
    let prepared = prepare_issue_operation!(
        ledger,
        ids,
        now,
        evidence,
        command.clone(),
        prepare_reopen_issue
    );
    ledger
        .prepare_reopen_issue(command, prepared)
        .map_err(IssueFlowError::Ledger)
}

/// H2a step 2 for all three transitions: one audit, one receipt.
///
/// Which transition this is, and which Evidence it binds, come from the
/// preview the host already stored -- never from the caller. The webview
/// sends only the prepared intent id and the digest it displayed, so it
/// cannot name a different operation or a different set of references than
/// the person approved.
pub fn approve_and_execute_issue_transition(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: IssueOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<IssueMutationOutcome, IssueFlowError> {
    let snapshot = ledger
        .load_issue_h2a_runtime_snapshot()
        .map_err(IssueFlowError::Load)?;
    let intent = snapshot
        .prepared()
        .iter()
        .find(|intent| intent.id() == &prepared_id)
        .ok_or(IssueFlowError::PreviewAlreadyConsumed)?;
    let (transition, role) = match intent.operation() {
        WorkManagementOperation::ResolveIssue { .. } => {
            (IssueTransition::Resolve, EvidenceRole::IssueResolution)
        }
        WorkManagementOperation::CloseIssue { .. } => (
            IssueTransition::Close,
            EvidenceRole::IssueClosureVerification,
        ),
        WorkManagementOperation::ReopenIssue { .. } => (
            IssueTransition::Reopen,
            EvidenceRole::IssueFailedVerification,
        ),
        _ => return Err(IssueFlowError::PreviewAlreadyConsumed),
    };
    let evidence_ids: Vec<EvidenceReferenceId> = intent
        .preview()
        .support()
        .map(|support| {
            support
                .evidence()
                .iter()
                .map(|evidence| evidence.id().clone())
                .collect()
        })
        .unwrap_or_default();
    let evidence =
        LedgerIssueEvidenceAuthority::read(ledger, &evidence_ids, role, &context.correlation_id)?;
    let approval = WorkManagementApproval::new(
        prepared_id,
        AuditActor::HeadOfProducts,
        acknowledged_payload_digest,
        context.idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .map_err(IssueFlowError::Approval)?;
    let audit_event_id =
        IssueServiceIdSource::next_audit_event_id(ids).map_err(IssueFlowError::Id)?;
    let receipt_id =
        IssueServiceIdSource::next_approval_receipt_id(ids).map_err(IssueFlowError::Id)?;
    let command = ApproveAndExecuteIssueTransition { approval, context };
    match transition {
        IssueTransition::Resolve => ledger.approve_and_execute_resolve_issue(
            command,
            audit_event_id,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            evidence,
            RecordedIssueClassification,
        ),
        IssueTransition::Close => ledger.approve_and_execute_close_issue(
            command,
            audit_event_id,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            evidence,
            RecordedIssueClassification,
        ),
        IssueTransition::Reopen => ledger.approve_and_execute_reopen_issue(
            command,
            audit_event_id,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            evidence,
            RecordedIssueClassification,
        ),
    }
    .map_err(IssueFlowError::Ledger)
}

/// Which Issue transition an approval is executing, as read from the stored
/// preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IssueTransition {
    Resolve,
    Close,
    Reopen,
}

/// H2a rejection of any Issue preview: durable, audited, no effect (v46).
pub fn reject_issue_prepared_intent(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    context: IssueOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RejectedPreparedIntentOutcome, IssueFlowError> {
    let audit = IssueServiceIdSource::next_audit_event_id(ids).map_err(IssueFlowError::Id)?;
    ledger
        .reject_issue_prepared_intent(
            RejectIssuePreparedIntent {
                prepared_id,
                actor: AuditActor::HeadOfProducts,
                context,
            },
            audit,
            now,
            HeadOfProductsApproval,
        )
        .map_err(IssueFlowError::Ledger)
}
