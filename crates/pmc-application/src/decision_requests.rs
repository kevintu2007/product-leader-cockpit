//! The desktop's Decision Request facade: withdraw,
//! and the Resolve H2a loop (prepare the exact preview, approve, reject).
//!
//! Same contract as [`crate::action_requests`]: the Ledger is the authority,
//! the host supplies ids and time, the actor is the Head of Products, the
//! H2a `Confirmed` is the explicit Approve. Two things are specific here:
//!
//! - The SQLite Resolve PREPARE takes a caller-built canonical prepared
//!   value, so this facade loads the Decision persistence snapshot,
//!   rehydrates the domain service, prepares, and persists.
//! - The domain resolves Evidence through a port at prepare and again at
//!   execute. This facade reads each named reference from the Ledger first
//!   (id, classification, verification and, since v45, aggregate version)
//!   and hands the service a fixed authority over exactly those; a reference
//!   the Ledger does not hold is `NotFound`, which the domain refuses.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;

use pmc_domain::audit::AuditActor;
use pmc_domain::decisions::{
    ApproveAndExecuteResolveDecisionRequest, DecisionEvidenceAuthorityError,
    DecisionEvidenceAuthorityPort, DecisionMutationOutcome, DecisionOperationContext,
    DecisionRequestRecord, DecisionServiceIdSource, InMemoryDecisionService,
    PrepareResolveDecisionRequest, RejectDecisionPreparedIntent, ResolvedDecisionOutcome,
    WithdrawDecisionRequest,
};
use pmc_domain::error::DomainError;
use pmc_domain::evidence::EvidenceReferenceRecord;
use pmc_domain::identity::{CorrelationId, EvidenceReferenceId, PreparedIntentId};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ApprovalConfirmation, EvidenceReferenceMetadata, EvidenceRole, PreparedIntentError,
    RejectedPreparedIntentOutcome, WorkManagementApproval, WorkManagementPayloadDigest,
    WorkManagementPreparedIntent,
};
use pmc_domain::DomainValueError;
use pmc_ledger::sqlite::{
    DecisionPersistenceLoadError, LedgerTransactionError, SqliteProductLedger,
};

use crate::desktop_runtime::{FixedClock, OpaqueIdSource};
use crate::work_management_authority::{HeadOfProductsApproval, SingleUserExecutionPolicy};

#[derive(Debug)]
pub enum DecisionFlowError {
    /// The Ledger's Decision persistence could not be loaded for rehydration.
    Load(DecisionPersistenceLoadError),
    /// The domain refused (stale version, illegal transition, support gate,
    /// ...). Carries the safe envelope.
    Domain(DomainError),
    /// The Ledger refused or failed the write.
    Ledger(LedgerTransactionError<DomainError>),
    /// The approval could not be constructed (a malformed digest).
    Approval(PreparedIntentError),
    /// The host's id source produced an invalid identifier: a host bug.
    Id(DomainValueError),
}

impl From<LedgerTransactionError<DomainError>> for DecisionFlowError {
    fn from(error: LedgerTransactionError<DomainError>) -> Self {
        Self::Ledger(error)
    }
}

/// A fixed authority over the Evidence references the Ledger held when the
/// facade read them. Resolving a reference the caller never named is
/// `NotFound`: the preview binds exactly the references the person chose.
#[derive(Clone, Debug, Default)]
pub struct LedgerEvidenceAuthority {
    evidence: HashMap<EvidenceReferenceId, EvidenceReferenceMetadata>,
}

impl LedgerEvidenceAuthority {
    /// Read every named reference from the Ledger now. A reference the
    /// Ledger does not hold is simply absent, so the domain refuses it.
    pub fn read(
        ledger: &SqliteProductLedger,
        evidence_ids: &[EvidenceReferenceId],
        correlation_id: &CorrelationId,
    ) -> Result<Self, DecisionFlowError> {
        let mut evidence = HashMap::new();
        for id in evidence_ids {
            if let Some(record) = ledger
                .get_evidence_reference(id, correlation_id.clone())
                .map_err(DecisionFlowError::Domain)?
            {
                evidence.insert(id.clone(), metadata(&record));
            }
        }
        Ok(Self { evidence })
    }
}

impl DecisionEvidenceAuthorityPort for LedgerEvidenceAuthority {
    fn resolve(
        &self,
        id: &EvidenceReferenceId,
    ) -> Result<EvidenceReferenceMetadata, DecisionEvidenceAuthorityError> {
        self.evidence
            .get(id)
            .cloned()
            .ok_or(DecisionEvidenceAuthorityError::NotFound)
    }
}

fn metadata(record: &EvidenceReferenceRecord) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        record.id.clone(),
        record.version,
        record.classification,
        EvidenceRole::DecisionResolution,
        record.verification.clone(),
    )
}

/// H1 retreat route: withdraw an Open Decision Request with the person's
/// own rationale.
pub fn withdraw_decision_request(
    ledger: &mut SqliteProductLedger,
    command: WithdrawDecisionRequest,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, DecisionFlowError> {
    let audit = DecisionServiceIdSource::next_audit_event_id(ids).map_err(DecisionFlowError::Id)?;
    ledger
        .withdraw_decision_request(command, audit, now)
        .map_err(DecisionFlowError::Ledger)
}

/// H2a step 1: the exact preview for resolving an Open Decision Request.
/// The host mints the Decision id and the resulting Action Request ids
/// through `ids`; the person's statement, rationale, impact, Evidence
/// choices, Judgments and resulting requests come in `command`.
pub fn prepare_resolve_decision_request(
    ledger: &mut SqliteProductLedger,
    command: PrepareResolveDecisionRequest,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<WorkManagementPreparedIntent, DecisionFlowError> {
    let evidence = LedgerEvidenceAuthority::read(
        ledger,
        &command.evidence_ids,
        &command.context.correlation_id,
    )?;
    let snapshot = ledger
        .load_decision_persistence_snapshot()
        .map_err(DecisionFlowError::Load)?;
    let mut service = InMemoryDecisionService::rehydrate(
        FixedClock(now),
        ids,
        HeadOfProductsApproval,
        SingleUserExecutionPolicy,
        evidence,
        snapshot,
    );
    let prepared = service
        .prepare_resolve_decision_request(command.clone())
        .map_err(DecisionFlowError::Domain)?;
    ledger
        .prepare_resolve_decision_request(command, prepared)
        .map_err(DecisionFlowError::Ledger)
}

/// H2a step 2: the person pressed Approve on the exact preview. Mints the
/// three Decision audit ids, one audit id per resulting Action Request the
/// preview declares, and the receipt id; the Ledger executes atomically.
pub fn approve_and_execute_resolve_decision_request(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    acknowledged_payload_digest: WorkManagementPayloadDigest,
    context: DecisionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ResolvedDecisionOutcome, DecisionFlowError> {
    let approval = WorkManagementApproval::new(
        prepared_id.clone(),
        AuditActor::HeadOfProducts,
        acknowledged_payload_digest,
        context.idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .map_err(DecisionFlowError::Approval)?;
    // The preview says how many resulting Action Requests it creates; each
    // needs its own audit id. Read it from the persisted snapshot.
    let snapshot = ledger
        .load_decision_persistence_snapshot()
        .map_err(DecisionFlowError::Load)?;
    let (resulting_count, evidence_ids): (usize, Vec<EvidenceReferenceId>) = snapshot
        .prepared()
        .iter()
        .find(|intent| intent.id() == &prepared_id)
        .map(|intent| {
            let count = match intent.operation() {
                pmc_domain::work_management::WorkManagementOperation::ResolveDecisionRequest {
                    resulting_action_requests,
                    ..
                } => resulting_action_requests.len(),
                _ => 0,
            };
            let evidence = intent
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
            (count, evidence)
        })
        .unwrap_or_default();
    let evidence = LedgerEvidenceAuthority::read(ledger, &evidence_ids, &context.correlation_id)?;
    let audit_event_ids = [
        DecisionServiceIdSource::next_audit_event_id(ids).map_err(DecisionFlowError::Id)?,
        DecisionServiceIdSource::next_audit_event_id(ids).map_err(DecisionFlowError::Id)?,
        DecisionServiceIdSource::next_audit_event_id(ids).map_err(DecisionFlowError::Id)?,
    ];
    let mut action_audit_event_ids = Vec::with_capacity(resulting_count);
    for _ in 0..resulting_count {
        action_audit_event_ids.push(
            DecisionServiceIdSource::next_audit_event_id(ids).map_err(DecisionFlowError::Id)?,
        );
    }
    let receipt_id =
        DecisionServiceIdSource::next_approval_receipt_id(ids).map_err(DecisionFlowError::Id)?;
    ledger
        .approve_and_execute_resolve_decision_request(
            ApproveAndExecuteResolveDecisionRequest { approval, context },
            audit_event_ids,
            action_audit_event_ids,
            receipt_id,
            now,
            HeadOfProductsApproval,
            SingleUserExecutionPolicy,
            evidence,
        )
        .map_err(DecisionFlowError::Ledger)
}

/// H2a rejection of a Resolve preview: durable, audited, no effect (v45).
pub fn reject_decision_prepared_intent(
    ledger: &mut SqliteProductLedger,
    prepared_id: PreparedIntentId,
    context: DecisionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<RejectedPreparedIntentOutcome, DecisionFlowError> {
    let audit = DecisionServiceIdSource::next_audit_event_id(ids).map_err(DecisionFlowError::Id)?;
    ledger
        .reject_decision_prepared_intent(
            RejectDecisionPreparedIntent {
                prepared_id,
                actor: AuditActor::HeadOfProducts,
                context,
            },
            audit,
            now,
            HeadOfProductsApproval,
        )
        .map_err(DecisionFlowError::Ledger)
}
