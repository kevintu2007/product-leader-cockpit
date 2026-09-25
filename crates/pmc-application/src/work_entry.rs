//! Record entry for work (slice 6E; DG3 record-entry amendment §2–§4 and
//! §6): an Action Request draft and its submit, a Decision Request draft
//! and its submit, and an Issue. A Risk's create is `record_entry::enter_risk`
//! (6A) and its response `risk_lifecycle::update_risk_response`.
//!
//! Same shape as the other families: `enter_*` reserves then creates
//! through the Ledger's reservation-checked writer; `submit_*` is the H1
//! lifecycle step at the version the row read (Draft → Open), which the
//! domain guards. Provenance is not a field of these records.

#![allow(clippy::result_large_err)]

use pmc_domain::actions::{
    ActionMutationOutcome, ActionOperationContext, ActionRequestRecord, SubmitActionRequest,
};
use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::decisions::{
    DecisionMutationOutcome, DecisionOperationContext, DecisionRequestRecord, SubmitDecisionRequest,
};
use pmc_domain::identity::{ActionRequestId, AggregateVersion, DecisionRequestId, IssueId};
use pmc_domain::issues::{IssueMutationOutcome, IssueOperationContext};
use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    ActionRequestDraftFields, DecisionRequestDraftFields, IssueFields, SqliteProductLedger,
    CREATE_ACTION_REQUEST_DRAFT, CREATE_DECISION_REQUEST_DRAFT, CREATE_ISSUE,
};

use crate::desktop_runtime::OpaqueIdSource;
use crate::record_entry::{reserve_record_id, RecordEntryError};

// The three creates take the Ledger's field structs (ActionRequestDraftFields,
// DecisionRequestDraftFields, IssueFields): what the sheet sends, never an id.

fn audit_id(
    ids: &mut OpaqueIdSource,
) -> Result<pmc_domain::identity::AuditEventId, RecordEntryError> {
    AuditEventIdSource::next_audit_event_id(ids).map_err(RecordEntryError::Id)
}

pub fn enter_action_request_draft(
    ledger: &mut SqliteProductLedger,
    entry: ActionRequestDraftFields,
    context: ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRequestRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<ActionRequestId>(
        ledger,
        &context.idempotency_id,
        CREATE_ACTION_REQUEST_DRAFT,
        now,
        || ids.next_action_request_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger
        .create_action_request_draft_from_reservation(&reserved, entry, context, audit, now)?)
}

/// Draft → Open, at the version the row read. The domain refuses anything
/// but a Draft, and a stale version.
pub fn submit_action_request(
    ledger: &mut SqliteProductLedger,
    id: ActionRequestId,
    expected_version: AggregateVersion,
    context: ActionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<ActionMutationOutcome<ActionRequestRecord>, RecordEntryError> {
    let audit = audit_id(ids)?;
    Ok(ledger.submit_action_request(
        SubmitActionRequest {
            request_id: id,
            expected_version,
            context,
        },
        audit,
        now,
    )?)
}

pub fn enter_decision_request_draft(
    ledger: &mut SqliteProductLedger,
    entry: DecisionRequestDraftFields,
    context: DecisionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, RecordEntryError> {
    let reserved = reserve_record_id::<DecisionRequestId>(
        ledger,
        &context.idempotency_id,
        CREATE_DECISION_REQUEST_DRAFT,
        now,
        || ids.next_decision_request_id(),
    )?;
    let audit = audit_id(ids)?;
    Ok(ledger
        .create_decision_request_draft_from_reservation(&reserved, entry, context, audit, now)?)
}

pub fn submit_decision_request(
    ledger: &mut SqliteProductLedger,
    id: DecisionRequestId,
    expected_version: AggregateVersion,
    context: DecisionOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<DecisionMutationOutcome<DecisionRequestRecord>, RecordEntryError> {
    let audit = audit_id(ids)?;
    Ok(ledger.submit_decision_request(
        SubmitDecisionRequest {
            request_id: id,
            expected_version,
            context,
        },
        audit,
        now,
    )?)
}

pub fn enter_issue(
    ledger: &mut SqliteProductLedger,
    entry: IssueFields,
    context: IssueOperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
) -> Result<IssueMutationOutcome, RecordEntryError> {
    let reserved =
        reserve_record_id::<IssueId>(ledger, &context.idempotency_id, CREATE_ISSUE, now, || {
            ids.next_issue_id()
        })?;
    let audit = audit_id(ids)?;
    Ok(ledger.create_issue_from_reservation(&reserved, entry, context, audit, now)?)
}
