//! S03's write path: the six IPC commands behind the
//! Work Queue's Action Request actions and the O03 H2a Focused Review.
//!
//! What crosses the webview boundary is deliberately narrow. The webview
//! supplies identity of the record it read (id + expected version), the
//! person's own words (a rationale), and one opaque `clientRequestId` per
//! submitted command (the idempotency id, reused verbatim on a retry). The
//! host mints everything else: correlation ids, timestamps, audit / receipt
//! / prepared-intent / action ids. The actor, the policy, the receipt and
//! the confirmation are never webview fields -- the H2a `Confirmed` is the
//! explicit Approve click, fixed here.
//!
//! Every command holds the Ledger mutex only for its own transaction; the
//! preview sits with the person (up to the H2a TTL) with nothing locked.

// The safe error envelope is every command's Err type by contract (DG3);
// its size is the contract's, not an accident.
#![allow(clippy::result_large_err)]

use pmc_application::action_lifecycle::{
    action_completion_context, approve_and_execute_cancel_action as approve_cancel,
    approve_and_execute_complete_action as approve_complete,
    approve_and_execute_reopen_action as approve_reopen,
    link_action_completion_evidence as link_evidence, prepare_cancel_action as prepare_cancel,
    prepare_complete_action as prepare_complete, prepare_reopen_action as prepare_reopen,
    ActionCompletionContextError,
};
use pmc_application::action_requests::{
    approve_and_execute_accept_action_request as approve_accept, decline_action_request as decline,
    prepare_accept_action_request as prepare_accept,
    reject_action_prepared_intent as reject_prepared, start_action as start,
    withdraw_action_request as withdraw, ActionRequestFlowError,
};
use pmc_application::decision_requests::{
    approve_and_execute_resolve_decision_request as approve_resolve,
    prepare_resolve_decision_request as prepare_resolve,
    reject_decision_prepared_intent as reject_decision,
    withdraw_decision_request as withdraw_decision, DecisionFlowError,
};
use pmc_application::evidence_writes::{
    link_to_product, pin_fingerprint, reobserve, EvidenceWriteError, ReobserveResult,
    VaultUnavailable,
};
use pmc_application::issue_lifecycle::{
    approve_and_execute_issue_transition as approve_issue,
    prepare_close_issue as prepare_issue_close, prepare_reopen_issue as prepare_issue_reopen,
    prepare_resolve_issue as prepare_issue_resolve, reject_issue_prepared_intent as reject_issue,
    IssueFlowError,
};
use pmc_application::risk_lifecycle::{
    approve_and_execute_close_risk as approve_risk_close,
    approve_and_execute_record_risk_occurrence as approve_risk_occurrence,
    prepare_close_risk as prepare_risk_close,
    prepare_record_risk_occurrence as prepare_risk_occurrence,
    reject_risk_prepared_intent as reject_risk, RiskFlowError,
};
use pmc_domain::actions::{
    ActionDetails, ActionMutationOutcome, ActionOperationContext, ActionRecord,
    ActionRequestRecord, ActionTitle, DeclineActionRequest, LinkActionCompletionEvidence,
    PrepareAcceptActionRequest, PrepareCancelAction, PrepareCompleteAction, PrepareReopenAction,
    StartAction, WithdrawActionRequest,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::EvidenceReferenceReadRecord;
use pmc_domain::decisions::{
    DecisionOperationContext, DecisionRecord, DecisionRequestRecord, DecisionText,
    DecisionWithdrawalRationale, PrepareResolveDecisionRequest, WithdrawDecisionRequest,
};
use pmc_domain::evidence::EvidenceReferenceRecord;
use pmc_domain::identity::ProductId;
use pmc_domain::identity::{
    ActionId, ActionRequestId, AggregateVersion, CorrelationId, DecisionRequestId,
    EvidenceReferenceId, IdempotencyId, IssueId, PreparedIntentId, RiskId, StakeholderId,
};
use pmc_domain::issues::{IssueOperationContext, IssueRecord};
use pmc_domain::risks::{RiskOperationContext, RiskRecord};
use pmc_domain::work_management::{
    ActionReopenMode, DecisionResultingActionRequest, EvidenceRole, EvidenceVerification,
    HumanJudgment, HumanJudgmentDisposition, IssueResolutionType, RejectedPreparedIntentOutcome,
    SupportDisposition, SupportWitness, WorkManagementAuthority, WorkManagementCancellationPolicy,
    WorkManagementClassificationSourceRole, WorkManagementEffect, WorkManagementH2aIntentKind,
    WorkManagementOperation, WorkManagementPayloadDigest, WorkManagementPolicyDecision,
    WorkManagementPreparedIntent, WorkManagementRationale, WorkManagementTarget,
    WORK_MANAGEMENT_H2A_TTL_MILLIS,
};
use pmc_ledger::sqlite::ActionPersistenceLoadError;
use pmc_ledger::sqlite::DecisionPersistenceLoadError;
use pmc_ledger::sqlite::IssuePersistenceLoadError;
use pmc_ledger::sqlite::RiskPersistenceLoadError;
use serde::Serialize;
use tauri::State;

use crate::backup_gate::BackupGate;
use crate::display_settings::SettingsState;
use crate::ledger_state::{LedgerState, VaultState};
use crate::runtime::{host_correlation, HostRuntime};
use crate::safe_error::{SafeErrorDto, SafeErrorExtensionDto};

// ---------------------------------------------------------------------------
// DTOs -- the transport edge of the typed prepared contract and the
// authoritative outcomes. camelCase on the wire, nothing private.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTargetDto {
    pub kind: &'static str,
    pub id: String,
    pub expected_version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedEffectDto {
    pub kind: &'static str,
    pub ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationSourceDto {
    pub role: &'static str,
    /// The id the role points at, where it has one (downstream / resulting
    /// records, Evidence); `null` for the structural roles.
    pub id: Option<String>,
    pub classification: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceVerificationDto {
    pub kind: &'static str,
    pub at_millis: Option<i64>,
    pub integrity_digest: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportEvidenceDto {
    pub id: String,
    /// The Evidence reference's aggregate version bound at prepare time
    /// (v45): part of the digest the person acknowledges.
    pub source_version: u64,
    pub classification: &'static str,
    pub role: &'static str,
    pub verification: EvidenceVerificationDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportJudgmentDto {
    pub disposition: &'static str,
    pub rationale: String,
    pub classification: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportWitnessDto {
    pub disposition: &'static str,
    pub classification: &'static str,
    pub evidence: Vec<SupportEvidenceDto>,
    pub judgments: Vec<SupportJudgmentDto>,
}

/// One Evidence reference's classification as the Cancel/Reopen preview
/// bound it (the sources that fold into the operation's classification).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceClassificationDto {
    pub evidence_id: String,
    pub classification: &'static str,
}

/// The exact operation payload. A discriminated union so the webview can
/// render each kind's own fields: the four Action H2a kinds S03 wires.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreparedOperationDto {
    #[serde(rename_all = "camelCase")]
    AcceptActionRequest {
        request_id: String,
        request_version: u64,
        action_id: String,
        action_classification: &'static str,
        action_subject: String,
        commitment_details: String,
        intended_owner: String,
        intended_due_at_millis: i64,
    },
    #[serde(rename_all = "camelCase")]
    CompleteAction {
        action_id: String,
        action_version: u64,
    },
    #[serde(rename_all = "camelCase")]
    CancelAction {
        action_id: String,
        action_version: u64,
        reason: String,
        evidence_classifications: Vec<EvidenceClassificationDto>,
    },
    #[serde(rename_all = "camelCase")]
    ReopenAction {
        action_id: String,
        action_version: u64,
        mode: &'static str,
        reason: String,
        evidence_classifications: Vec<EvidenceClassificationDto>,
    },
    #[serde(rename_all = "camelCase")]
    ResolveDecisionRequest {
        request_id: String,
        request_version: u64,
        decision_id: String,
        decision_classification: &'static str,
        statement: String,
        rationale: String,
        impact: String,
        decision_owner: String,
        decided_at_millis: i64,
        resulting_action_requests: Vec<ResultingActionRequestDto>,
    },
    #[serde(rename_all = "camelCase")]
    RecordRiskOccurrence {
        risk_id: String,
        risk_version: u64,
        /// The Issue this occurrence will create. Host-minted, and part of
        /// the digest the person acknowledges.
        issue_id: String,
        issue_classification: &'static str,
    },
    #[serde(rename_all = "camelCase")]
    CloseRisk {
        risk_id: String,
        risk_version: u64,
        rationale: String,
    },
    #[serde(rename_all = "camelCase")]
    ResolveIssue {
        issue_id: String,
        issue_version: u64,
        resolution_type: &'static str,
        rationale: String,
    },
    #[serde(rename_all = "camelCase")]
    CloseIssue {
        issue_id: String,
        issue_version: u64,
    },
    #[serde(rename_all = "camelCase")]
    ReopenIssue {
        issue_id: String,
        issue_version: u64,
        rationale: String,
    },
}

/// A Risk as the Ledger holds it after a transition.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskSummaryDto {
    pub id: String,
    pub title: String,
    pub details: String,
    pub classification: &'static str,
    pub state: &'static str,
    pub version: u64,
}

/// An Issue as the Ledger holds it after a transition.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSummaryDto {
    pub id: String,
    pub title: String,
    pub details: String,
    pub classification: &'static str,
    pub state: &'static str,
    pub version: u64,
    pub source_risk_id: Option<String>,
    pub resolution_type: Option<&'static str>,
}

/// Recording an occurrence transitions the Risk, creates the Issue the
/// preview named, and links them: three audits and one receipt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OccurredRiskDto {
    pub risk: RiskSummaryDto,
    pub issue: IssueSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub approval_receipt_id: String,
    pub correlation_id: String,
}

/// Closing a Risk: one audit, one receipt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskOutcomeDto {
    pub risk: RiskSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub approval_receipt_id: Option<String>,
    pub correlation_id: String,
}

/// Resolving, closing or reopening an Issue: one audit, one receipt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueOutcomeDto {
    pub issue: IssueSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub approval_receipt_id: Option<String>,
    pub correlation_id: String,
}

/// One resulting Action Request as the Resolve preview declares it: the
/// host-minted id and the person's own fields.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultingActionRequestDto {
    pub id: String,
    pub subject: String,
    pub details: String,
    pub intended_owner: String,
    pub due_at_millis: i64,
    pub classification: &'static str,
}

/// What the webview supplies for one resulting Action Request. The id is
/// minted by the host.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultingActionRequestInput {
    pub subject: String,
    pub details: String,
    pub intended_owner: String,
    pub due_at_millis: i64,
    pub classification: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequestSummaryDto {
    pub id: String,
    pub subject: String,
    pub details: String,
    pub intended_owner: Option<String>,
    pub classification: &'static str,
    pub state: &'static str,
    pub version: u64,
    pub withdrawal_rationale: Option<String>,
    pub linked_decision_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSummaryDto {
    pub id: String,
    pub source_request_id: Option<String>,
    pub statement: String,
    pub rationale: String,
    pub impact: String,
    pub owner: String,
    pub decided_at_millis: i64,
    pub classification: &'static str,
    pub state: &'static str,
    pub version: u64,
    pub resulting_action_request_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequestOutcomeDto {
    pub request: DecisionRequestSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDecisionDto {
    pub request: DecisionRequestSummaryDto,
    pub decision: DecisionSummaryDto,
    pub resulting_action_request_ids: Vec<String>,
    pub audit_event_ids: Vec<String>,
    pub approval_receipt_id: String,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReferencesDto {
    pub ledger_revision: u64,
    pub evidence_references: Vec<EvidenceReferenceSummaryDto>,
    pub correlation_id: String,
}

/// One Evidence reference as the Ledger holds it now: what S03 needs to
/// pick completion Evidence for an Action. Never a Vault path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReferenceSummaryDto {
    pub id: String,
    pub role: Option<&'static str>,
    pub verification: EvidenceVerificationDto,
    pub pinned: bool,
    pub classification: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionCompletionContextDto {
    pub action: ActionSummaryDto,
    pub linked_evidence_ids: Vec<String>,
    pub ledger_revision: u64,
    pub evidence_references: Vec<EvidenceReferenceSummaryDto>,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedIntentDto {
    pub prepared_intent_id: String,
    pub contract_version: u16,
    pub intent_type: &'static str,
    pub operation: PreparedOperationDto,
    pub targets: Vec<PreparedTargetDto>,
    pub declared_effects: Vec<PreparedEffectDto>,
    /// The canonical payload digest the person acknowledges on Approve.
    /// Full, never truncated: the webview shows it whole and sends it back
    /// whole.
    pub payload_digest: String,
    pub resolved_classification: &'static str,
    pub classification_sources: Vec<ClassificationSourceDto>,
    pub support: Option<SupportWitnessDto>,
    pub policy_result: &'static str,
    pub authority: &'static str,
    /// Derived from `expiresAtMillis` and the fixed H2a TTL, which is how
    /// the canonical constructor derives expiry from preparation.
    pub prepared_at_millis: i64,
    pub expires_at_millis: i64,
    pub cancellation_policy: &'static str,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequestSummaryDto {
    pub id: String,
    pub title: String,
    pub details: String,
    pub intended_owner: Option<String>,
    pub response_due_at_millis: Option<i64>,
    pub intended_action_due_at_millis: Option<i64>,
    pub classification: &'static str,
    pub state: &'static str,
    pub version: u64,
    pub terminal_rationale: Option<String>,
    pub linked_action_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionSummaryDto {
    pub id: String,
    pub source_request_id: String,
    pub title: String,
    pub details: String,
    pub owner: String,
    pub due_at_millis: i64,
    pub classification: &'static str,
    pub state: &'static str,
    pub version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedActionDto {
    pub request: ActionRequestSummaryDto,
    pub action: ActionSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub approval_receipt_id: String,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequestOutcomeDto {
    pub request: ActionRequestSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionOutcomeDto {
    pub action: ActionSummaryDto,
    pub audit_event_ids: Vec<String>,
    pub correlation_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedPreparedIntentDto {
    pub prepared_intent_id: String,
    pub disposition: &'static str,
    pub rejected_at_millis: i64,
    /// The preview had already expired when it was rejected. Expiry limits
    /// approval, not refusal; this only tells the person which it was.
    pub expired_at_rejection: bool,
    pub correlation_id: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// H2a step 1 for S03: prepare the exact preview for accepting an Open
/// Action Request. Persists the canonical Prepared Intent and returns the
/// whole typed contract for O03 to render; nothing is executed.
#[tauri::command]
pub fn prepare_accept_action_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    request_id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = PrepareAcceptActionRequest {
        request_id: parse_argument(ActionRequestId::parse(request_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_accept(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 2 for S03: the person pressed Approve on the exact preview.
/// The acknowledged digest is the one O03 showed whole; the host fixes the
/// actor and the confirmation. Atomic and single-use in the Ledger.
#[tauri::command]
pub fn approve_and_execute_accept_action_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<AcceptedActionDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_accept(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(AcceptedActionDto {
        request: request_summary(&outcome.request),
        action: action_summary(&outcome.action),
        audit_event_ids: outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str().to_owned())
            .collect(),
        approval_receipt_id: outcome.approval_receipt_id.as_str().to_owned(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a rejection for S03: the person pressed Reject (or Escape) on the
/// preview. Durable, audited, no effect (v45); closing the dialog locally
/// is not a rejection.
#[tauri::command]
pub fn reject_prepared_accept_action_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    client_request_id: String,
) -> Result<RejectedPreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = reject_prepared(&mut ledger, prepared_id, context, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(RejectedPreparedIntentDto {
        prepared_intent_id: outcome.prepared_intent_id().as_str().to_owned(),
        disposition: "rejected",
        rejected_at_millis: outcome.rejected_at().unix_millis(),
        expired_at_rejection: outcome.expired_at_rejection(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H1 retreat route: decline an Open Action Request with the person's own
/// rationale.
#[tauri::command]
pub fn decline_action_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    request_id: String,
    expected_version: u64,
    rationale: String,
    client_request_id: String,
) -> Result<ActionRequestOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = DeclineActionRequest {
        request_id: parse_argument(ActionRequestId::parse(request_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        rationale: parse_argument(ActionDetails::parse(rationale), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = decline(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(request_outcome(outcome, &correlation))
}

/// H1 retreat route: withdraw an Open Action Request with the person's own
/// rationale.
#[tauri::command]
pub fn withdraw_action_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    request_id: String,
    expected_version: u64,
    rationale: String,
    client_request_id: String,
) -> Result<ActionRequestOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = WithdrawActionRequest {
        request_id: parse_argument(ActionRequestId::parse(request_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        rationale: parse_argument(ActionDetails::parse(rationale), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = withdraw(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(request_outcome(outcome, &correlation))
}

/// H1: start an Open Action (Open -> InProgress).
#[tauri::command]
pub fn start_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    action_id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<ActionOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = StartAction {
        action_id: parse_argument(ActionId::parse(action_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = start(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(ActionOutcomeDto {
        action: action_summary(&outcome.record),
        audit_event_ids: audit_ids(&outcome),
        correlation_id: correlation.as_str().to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Action lifecycle
// ---------------------------------------------------------------------------

/// H0: what S03 needs before linking Evidence to an Action or preparing
/// its completion. Read-only; carries no Vault path.
#[tauri::command]
pub fn get_action_completion_context(
    ledger: State<'_, LedgerState>,
    runtime: State<'_, HostRuntime>,
    action_id: String,
) -> Result<ActionCompletionContextDto, SafeErrorDto> {
    let correlation = host_correlation();
    let action_id = parse_argument(ActionId::parse(action_id), &correlation)?;
    let now = runtime.now();
    let ledger = ledger
        .read()
        .map_err(|closed| closed.to_safe_error(&correlation))?;
    let context =
        action_completion_context(&ledger, &action_id, now).map_err(|error| match error {
            ActionCompletionContextError::NotFound => SafeErrorDto::host(
                "DOMAIN_NOT_FOUND",
                "desktop.action_not_found",
                &correlation,
                false,
            ),
            ActionCompletionContextError::Load(load) => {
                flow_error(ActionRequestFlowError::Load(load), &correlation)
            }
            ActionCompletionContextError::Composition(_) => SafeErrorDto::host(
                "LEDGER_SNAPSHOT_UNAVAILABLE",
                "desktop.snapshot_unavailable",
                &correlation,
                true,
            ),
        })?;
    Ok(ActionCompletionContextDto {
        action: action_summary(&context.action),
        linked_evidence_ids: context
            .action
            .completion_evidence()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
        ledger_revision: context.ledger_revision,
        evidence_references: context
            .evidence_references
            .iter()
            .map(evidence_reference_summary)
            .collect(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H1: link an Evidence reference as completion evidence of an InProgress
/// Action.
#[tauri::command]
pub fn link_action_completion_evidence(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    action_id: String,
    expected_version: u64,
    evidence_id: String,
    client_request_id: String,
) -> Result<ActionOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = LinkActionCompletionEvidence {
        action_id: parse_argument(ActionId::parse(action_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        evidence_id: parse_argument(EvidenceReferenceId::parse(evidence_id), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = link_evidence(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(ActionOutcomeDto {
        action: action_summary(&outcome.record),
        audit_event_ids: audit_ids(&outcome),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a step 1: the exact preview for completing an InProgress Action. The
/// optional Judgment is the person's own rationale; its disposition is fixed
/// by the host (proceed with documented rationale), never a webview field.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn prepare_complete_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    action_id: String,
    expected_version: u64,
    judgment_rationale: Option<String>,
    judgment_classification: Option<String>,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let judgment = match (judgment_rationale, judgment_classification) {
        (None, None) => None,
        (Some(rationale), Some(classification)) => {
            let classification = parse_argument(
                DataClassification::from_persisted(&classification),
                &correlation,
            )?;
            Some(parse_argument(
                HumanJudgment::new(
                    HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                    rationale,
                    classification,
                ),
                &correlation,
            )?)
        }
        _ => {
            return Err(SafeErrorDto::host(
                "VALIDATION_INVALID_FIELD",
                "desktop.invalid_argument",
                &correlation,
                false,
            ))
        }
    };
    let command = PrepareCompleteAction {
        action_id: parse_argument(ActionId::parse(action_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        judgment,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_complete(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 1: the exact preview for cancelling an Open or InProgress Action.
#[tauri::command]
pub fn prepare_cancel_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    action_id: String,
    expected_version: u64,
    reason: String,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = PrepareCancelAction {
        action_id: parse_argument(ActionId::parse(action_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        reason: parse_argument(ActionDetails::parse(reason), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_cancel(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 1: the exact preview for reopening a Completed or Cancelled
/// Action, in the mode the person chose.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn prepare_reopen_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    action_id: String,
    expected_version: u64,
    mode: String,
    reason: String,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let mode = match mode.as_str() {
        "reopen_completed" => ActionReopenMode::ReopenCompleted,
        "restart_cancelled" => ActionReopenMode::RestartCancelled,
        _ => {
            return Err(SafeErrorDto::host(
                "VALIDATION_INVALID_FIELD",
                "desktop.invalid_argument",
                &correlation,
                false,
            ))
        }
    };
    let command = PrepareReopenAction {
        action_id: parse_argument(ActionId::parse(action_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        mode,
        reason: parse_argument(ActionDetails::parse(reason), &correlation)?,
        context: context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_reopen(&mut ledger, command, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 2 for Complete.
#[tauri::command]
pub fn approve_and_execute_complete_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<ActionOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_complete(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(ActionOutcomeDto {
        action: action_summary(&outcome.record),
        audit_event_ids: audit_ids(&outcome),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a step 2 for Cancel.
#[tauri::command]
pub fn approve_and_execute_cancel_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<ActionOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_cancel(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(ActionOutcomeDto {
        action: action_summary(&outcome.record),
        audit_event_ids: audit_ids(&outcome),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a step 2 for Reopen.
#[tauri::command]
pub fn approve_and_execute_reopen_action(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<ActionOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_reopen(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(ActionOutcomeDto {
        action: action_summary(&outcome.record),
        audit_event_ids: audit_ids(&outcome),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a rejection of a Complete / Cancel / Reopen preview: the same durable,
/// audited, no-effect refusal as for an Accept preview.
#[tauri::command]
pub fn reject_prepared_action_intent(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    client_request_id: String,
) -> Result<RejectedPreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let context = context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = reject_prepared(&mut ledger, prepared_id, context, &mut ids, now)
        .map_err(|error| flow_error(error, &correlation))?;
    Ok(RejectedPreparedIntentDto {
        prepared_intent_id: outcome.prepared_intent_id().as_str().to_owned(),
        disposition: "rejected",
        rejected_at_millis: outcome.rejected_at().unix_millis(),
        expired_at_rejection: outcome.expired_at_rejection(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

fn evidence_reference_summary(record: &EvidenceReferenceReadRecord) -> EvidenceReferenceSummaryDto {
    EvidenceReferenceSummaryDto {
        id: record.id.as_str().to_owned(),
        role: record.role.as_ref().map(evidence_role),
        verification: verification_dto(&record.verification),
        pinned: record.pinned,
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

fn evidence_role(role: &EvidenceRole) -> &'static str {
    match role {
        EvidenceRole::ActionCompletion => "action_completion",
        EvidenceRole::DecisionResolution => "decision_resolution",
        EvidenceRole::IssueResolution => "issue_resolution",
        EvidenceRole::IssueClosureVerification => "issue_closure_verification",
        EvidenceRole::IssueFailedVerification => "issue_failed_verification",
    }
}

// ---------------------------------------------------------------------------
// Decision Requests
// ---------------------------------------------------------------------------

/// H0: every Evidence reference the Ledger holds, as read records; what S03
/// needs to pick Evidence for a Decision resolution. Never a Vault path.
#[tauri::command]
pub fn get_evidence_references(
    ledger: State<'_, LedgerState>,
    runtime: State<'_, HostRuntime>,
) -> Result<EvidenceReferencesDto, SafeErrorDto> {
    use pmc_domain::composition_source::LedgerSnapshotForCompositionPort;
    let correlation = host_correlation();
    let now = runtime.now();
    let ledger = ledger
        .read()
        .map_err(|closed| closed.to_safe_error(&correlation))?;
    let composition = ledger.read_composition_snapshot(now).map_err(|_| {
        SafeErrorDto::host(
            "LEDGER_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            &correlation,
            true,
        )
    })?;
    Ok(EvidenceReferencesDto {
        ledger_revision: composition.ledger_revision,
        evidence_references: composition
            .evidence_references
            .iter()
            .map(evidence_reference_summary)
            .collect(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H1 retreat route: withdraw an Open Decision Request with the person's
/// own rationale.
#[tauri::command]
pub fn withdraw_decision_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    request_id: String,
    expected_version: u64,
    rationale: String,
    client_request_id: String,
) -> Result<DecisionRequestOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let command = WithdrawDecisionRequest {
        request_id: parse_argument(DecisionRequestId::parse(request_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        rationale: parse_argument(DecisionWithdrawalRationale::parse(rationale), &correlation)?,
        context: decision_context(client_request_id, &correlation)?,
    };
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = withdraw_decision(&mut ledger, command, &mut ids, now)
        .map_err(|error| decision_flow_error(error, &correlation))?;
    Ok(DecisionRequestOutcomeDto {
        request: decision_request_summary(&outcome.record),
        audit_event_ids: outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str().to_owned())
            .collect(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a step 1: the exact preview for resolving an Open Decision Request.
/// The host mints the Decision id, every resulting Action Request id and
/// the Prepared Intent id; the optional Judgment's disposition is fixed by
/// the host. Evidence is read from the Ledger by the ids the person chose.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn prepare_resolve_decision_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    request_id: String,
    expected_version: u64,
    statement: String,
    rationale: String,
    impact: String,
    evidence_ids: Vec<String>,
    judgment_rationale: Option<String>,
    judgment_classification: Option<String>,
    resulting_action_requests: Vec<ResultingActionRequestInput>,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let judgments = match (judgment_rationale, judgment_classification) {
        (None, None) => Vec::new(),
        (Some(rationale), Some(classification)) => {
            let classification = parse_argument(
                DataClassification::from_persisted(&classification),
                &correlation,
            )?;
            vec![parse_argument(
                HumanJudgment::new(
                    HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                    rationale,
                    classification,
                ),
                &correlation,
            )?]
        }
        _ => {
            return Err(SafeErrorDto::host(
                "VALIDATION_INVALID_FIELD",
                "desktop.invalid_argument",
                &correlation,
                false,
            ))
        }
    };
    let evidence_ids = evidence_ids
        .into_iter()
        .map(|id| parse_argument(EvidenceReferenceId::parse(id), &correlation))
        .collect::<Result<Vec<_>, _>>()?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let resulting = resulting_action_requests
        .into_iter()
        .map(|input| {
            Ok(DecisionResultingActionRequest {
                id: parse_argument(ids.next_action_request_id(), &correlation)?,
                subject: parse_argument(ActionTitle::parse(input.subject), &correlation)?,
                details: parse_argument(ActionDetails::parse(input.details), &correlation)?,
                intended_owner: parse_argument(
                    StakeholderId::parse(input.intended_owner),
                    &correlation,
                )?,
                due_at: parse_argument(due_at(input.due_at_millis), &correlation)?,
                classification: parse_argument(
                    DataClassification::from_persisted(&input.classification),
                    &correlation,
                )?,
            })
        })
        .collect::<Result<Vec<_>, SafeErrorDto>>()?;
    let command = PrepareResolveDecisionRequest {
        request_id: parse_argument(DecisionRequestId::parse(request_id), &correlation)?,
        expected_version: parse_argument(AggregateVersion::new(expected_version), &correlation)?,
        statement: parse_argument(DecisionText::parse(statement), &correlation)?,
        rationale: parse_argument(DecisionText::parse(rationale), &correlation)?,
        impact: parse_argument(DecisionText::parse(impact), &correlation)?,
        evidence_ids,
        judgments,
        resulting_action_requests: resulting,
        context: decision_context(client_request_id, &correlation)?,
    };
    let prepared = prepare_resolve(&mut ledger, command, &mut ids, now)
        .map_err(|error| decision_flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 2 for Resolve.
#[tauri::command]
pub fn approve_and_execute_resolve_decision_request(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<ResolvedDecisionDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = decision_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_resolve(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| decision_flow_error(error, &correlation))?;
    Ok(ResolvedDecisionDto {
        request: decision_request_summary(&outcome.request),
        decision: decision_summary(&outcome.decision),
        resulting_action_request_ids: outcome
            .resulting_action_request_ids
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
        audit_event_ids: outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str().to_owned())
            .collect(),
        approval_receipt_id: outcome.approval_receipt_id.as_str().to_owned(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a rejection of a Resolve preview: durable, audited, no effect (v45).
#[tauri::command]
pub fn reject_prepared_decision_intent(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    client_request_id: String,
) -> Result<RejectedPreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let context = decision_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = reject_decision(&mut ledger, prepared_id, context, &mut ids, now)
        .map_err(|error| decision_flow_error(error, &correlation))?;
    Ok(RejectedPreparedIntentDto {
        prepared_intent_id: outcome.prepared_intent_id().as_str().to_owned(),
        disposition: "rejected",
        rejected_at_millis: outcome.rejected_at().unix_millis(),
        expired_at_rejection: outcome.expired_at_rejection(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

fn decision_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<DecisionOperationContext, SafeErrorDto> {
    Ok(DecisionOperationContext {
        idempotency_id: parse_argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

// ---- S03 Risk and Issue H2a write path. Same boundary as the
// Action and Decision commands above.

fn risk_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<RiskOperationContext, SafeErrorDto> {
    Ok(RiskOperationContext {
        idempotency_id: parse_argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn issue_context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<IssueOperationContext, SafeErrorDto> {
    Ok(IssueOperationContext {
        idempotency_id: parse_argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn risk_summary(record: &RiskRecord) -> RiskSummaryDto {
    RiskSummaryDto {
        id: record.id().as_str().to_owned(),
        title: record.title().as_str().to_owned(),
        details: record.details().as_str().to_owned(),
        classification: record.classification().as_persisted(),
        state: record.state().as_persisted(),
        version: record.version().get(),
    }
}

fn issue_summary(record: &IssueRecord) -> IssueSummaryDto {
    IssueSummaryDto {
        id: record.id().as_str().to_owned(),
        title: record.title().as_str().to_owned(),
        details: record.details().as_str().to_owned(),
        classification: record.classification().as_persisted(),
        state: record.state().as_persisted(),
        version: record.version().get(),
        source_risk_id: record.source_risk_id().map(|id| id.as_str().to_owned()),
        resolution_type: record.resolution_type().map(|kind| kind.as_persisted()),
    }
}

fn risk_flow_error(error: RiskFlowError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        RiskFlowError::Domain(domain) => SafeErrorDto::from_domain(&domain),
        RiskFlowError::Ledger(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        RiskFlowError::Load(RiskPersistenceLoadError::StorageUnavailable) => SafeErrorDto::host(
            "LEDGER_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            correlation,
            true,
        ),
        RiskFlowError::Load(RiskPersistenceLoadError::UnsupportedSchema { .. }) => {
            SafeErrorDto::host(
                "LEDGER_UNSUPPORTED_SCHEMA",
                "ledger.open.unsupported_schema",
                correlation,
                false,
            )
        }
        RiskFlowError::Load(RiskPersistenceLoadError::InvalidRiskSnapshot)
        | RiskFlowError::LoadIssues(_) => SafeErrorDto::host(
            "LEDGER_SNAPSHOT_INVALID",
            "desktop.snapshot_invalid",
            correlation,
            false,
        ),
        RiskFlowError::Approval(_) => SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            correlation,
            false,
        ),
        RiskFlowError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_id_source_failed",
            correlation,
            false,
        ),
        // Not retryable: the same request id can never produce that preview
        // again. The person re-reads the row and prepares afresh.
        RiskFlowError::PreviewAlreadyConsumed => SafeErrorDto::host(
            "DOMAIN_CONFLICT",
            "desktop.preview_already_consumed",
            correlation,
            false,
        ),
    }
}

fn issue_flow_error(error: IssueFlowError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        IssueFlowError::Domain(domain) => SafeErrorDto::from_domain(&domain),
        IssueFlowError::Ledger(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        IssueFlowError::Load(IssuePersistenceLoadError::StorageUnavailable) => SafeErrorDto::host(
            "LEDGER_SNAPSHOT_UNAVAILABLE",
            "desktop.snapshot_unavailable",
            correlation,
            true,
        ),
        IssueFlowError::Load(IssuePersistenceLoadError::InvalidIssueSnapshot) => {
            SafeErrorDto::host(
                "LEDGER_SNAPSHOT_INVALID",
                "desktop.snapshot_invalid",
                correlation,
                false,
            )
        }
        IssueFlowError::Approval(_) => SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            correlation,
            false,
        ),
        IssueFlowError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_id_source_failed",
            correlation,
            false,
        ),
        IssueFlowError::PreviewAlreadyConsumed => SafeErrorDto::host(
            "DOMAIN_CONFLICT",
            "desktop.preview_already_consumed",
            correlation,
            false,
        ),
    }
}

fn parse_evidence_ids(
    ids: Vec<String>,
    correlation: &CorrelationId,
) -> Result<Vec<EvidenceReferenceId>, SafeErrorDto> {
    ids.into_iter()
        .map(|id| parse_argument(EvidenceReferenceId::parse(id), correlation))
        .collect()
}

/// H2a step 1: the exact preview for recording an occurrence of an Open
/// Risk. The Issue it will create is named by the host inside the preview.
#[tauri::command]
pub fn prepare_record_risk_occurrence(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    risk_id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let risk_id = parse_argument(RiskId::parse(risk_id), &correlation)?;
    let expected_version = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let context = risk_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_risk_occurrence(
        &mut ledger,
        risk_id,
        expected_version,
        context,
        &mut ids,
        now,
    )
    .map_err(|error| risk_flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 1: the exact preview for closing an Open Risk.
#[tauri::command]
pub fn prepare_close_risk(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    risk_id: String,
    expected_version: u64,
    rationale: String,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let risk_id = parse_argument(RiskId::parse(risk_id), &correlation)?;
    let expected_version = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let rationale = parse_argument(WorkManagementRationale::parse(rationale), &correlation)?;
    let context = risk_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_risk_close(
        &mut ledger,
        risk_id,
        expected_version,
        rationale,
        context,
        &mut ids,
        now,
    )
    .map_err(|error| risk_flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 2 for an occurrence.
#[tauri::command]
pub fn approve_and_execute_record_risk_occurrence(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<OccurredRiskDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = risk_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_risk_occurrence(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| risk_flow_error(error, &correlation))?;
    Ok(OccurredRiskDto {
        risk: risk_summary(&outcome.risk),
        issue: issue_summary(&outcome.issue),
        audit_event_ids: outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str().to_owned())
            .collect(),
        approval_receipt_id: outcome.approval_receipt_id.as_str().to_owned(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a step 2 for a close.
#[tauri::command]
pub fn approve_and_execute_close_risk(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<RiskOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = risk_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_risk_close(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| risk_flow_error(error, &correlation))?;
    Ok(RiskOutcomeDto {
        risk: risk_summary(&outcome.record),
        audit_event_ids: outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str().to_owned())
            .collect(),
        approval_receipt_id: outcome
            .approval_receipt_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a rejection of either Risk preview: durable, audited, no effect (v46).
#[tauri::command]
pub fn reject_prepared_risk_intent(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    client_request_id: String,
) -> Result<RejectedPreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let context = risk_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = reject_risk(&mut ledger, prepared_id, context, &mut ids, now)
        .map_err(|error| risk_flow_error(error, &correlation))?;
    Ok(rejected_prepared_intent_dto(&outcome, &correlation))
}

/// The optional Judgment an Issue transition carries: both halves or
/// neither. Its disposition is fixed by the host (proceed with documented
/// rationale), never a webview field -- the same rule as Action complete and
/// Decision resolve.
fn optional_issue_judgment(
    rationale: Option<String>,
    classification: Option<String>,
    correlation: &CorrelationId,
) -> Result<Option<HumanJudgment>, SafeErrorDto> {
    match (rationale, classification) {
        (None, None) => Ok(None),
        (Some(rationale), Some(classification)) => {
            let classification = parse_argument(
                DataClassification::from_persisted(&classification),
                correlation,
            )?;
            Ok(Some(parse_argument(
                HumanJudgment::new(
                    HumanJudgmentDisposition::ProceedWithDocumentedRationale,
                    rationale,
                    classification,
                ),
                correlation,
            )?))
        }
        _ => Err(SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            correlation,
            false,
        )),
    }
}

/// H2a step 1 for resolving an Open Issue. The optional Judgment lets
/// partly verified Evidence move forward; it cannot carry unverified or
/// mismatched Evidence.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn prepare_resolve_issue(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    issue_id: String,
    expected_version: u64,
    resolution_type: String,
    rationale: String,
    evidence_ids: Vec<String>,
    judgment_rationale: Option<String>,
    judgment_classification: Option<String>,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let judgment =
        optional_issue_judgment(judgment_rationale, judgment_classification, &correlation)?;
    let issue_id = parse_argument(IssueId::parse(issue_id), &correlation)?;
    let expected_version = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let resolution_type = parse_argument(
        IssueResolutionType::from_persisted(&resolution_type),
        &correlation,
    )?;
    let rationale = parse_argument(WorkManagementRationale::parse(rationale), &correlation)?;
    let evidence = parse_evidence_ids(evidence_ids, &correlation)?;
    let context = issue_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_issue_resolve(
        &mut ledger,
        issue_id,
        expected_version,
        resolution_type,
        rationale,
        evidence,
        judgment,
        context,
        &mut ids,
        now,
    )
    .map_err(|error| issue_flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 1 for closing a Resolved Issue: verification Evidence, plus an
/// optional Judgment when that Evidence is only partly verified.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn prepare_close_issue(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    issue_id: String,
    expected_version: u64,
    evidence_ids: Vec<String>,
    judgment_rationale: Option<String>,
    judgment_classification: Option<String>,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let judgment =
        optional_issue_judgment(judgment_rationale, judgment_classification, &correlation)?;
    let issue_id = parse_argument(IssueId::parse(issue_id), &correlation)?;
    let expected_version = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let evidence = parse_evidence_ids(evidence_ids, &correlation)?;
    let context = issue_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_issue_close(
        &mut ledger,
        issue_id,
        expected_version,
        evidence,
        judgment,
        context,
        &mut ids,
        now,
    )
    .map_err(|error| issue_flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 1 for reopening a Resolved Issue. The transition rationale and
/// the optional Judgment are separate: the first says why the Issue reopens,
/// the second why partly verified Evidence is enough to rely on.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn prepare_reopen_issue(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    issue_id: String,
    expected_version: u64,
    rationale: String,
    evidence_ids: Vec<String>,
    judgment_rationale: Option<String>,
    judgment_classification: Option<String>,
    client_request_id: String,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let judgment =
        optional_issue_judgment(judgment_rationale, judgment_classification, &correlation)?;
    let issue_id = parse_argument(IssueId::parse(issue_id), &correlation)?;
    let expected_version = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let rationale = parse_argument(WorkManagementRationale::parse(rationale), &correlation)?;
    let evidence = parse_evidence_ids(evidence_ids, &correlation)?;
    let context = issue_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let prepared = prepare_issue_reopen(
        &mut ledger,
        issue_id,
        expected_version,
        rationale,
        evidence,
        judgment,
        context,
        &mut ids,
        now,
    )
    .map_err(|error| issue_flow_error(error, &correlation))?;
    prepared_intent_dto(&prepared, &correlation)
}

/// H2a step 2 for all three Issue transitions. Which one this is, and which
/// Evidence it binds, come from the preview the host stored -- the webview
/// sends only the preview id and the digest it displayed.
#[tauri::command]
pub fn approve_and_execute_issue_transition(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    acknowledged_payload_digest: String,
    client_request_id: String,
) -> Result<IssueOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let digest = parse_argument(
        WorkManagementPayloadDigest::from_persisted(acknowledged_payload_digest),
        &correlation,
    )?;
    let context = issue_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = approve_issue(&mut ledger, prepared_id, digest, context, &mut ids, now)
        .map_err(|error| issue_flow_error(error, &correlation))?;
    Ok(IssueOutcomeDto {
        issue: issue_summary(&outcome.record),
        audit_event_ids: outcome
            .audit_events
            .iter()
            .map(|audit| audit.id().as_str().to_owned())
            .collect(),
        approval_receipt_id: outcome
            .approval_receipt_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H2a rejection of any Issue preview: durable, audited, no effect (v46).
#[tauri::command]
pub fn reject_prepared_issue_intent(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    prepared_intent_id: String,
    client_request_id: String,
) -> Result<RejectedPreparedIntentDto, SafeErrorDto> {
    let correlation = host_correlation();
    let prepared_id = parse_argument(PreparedIntentId::parse(prepared_intent_id), &correlation)?;
    let context = issue_context(client_request_id, &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = reject_issue(&mut ledger, prepared_id, context, &mut ids, now)
        .map_err(|error| issue_flow_error(error, &correlation))?;
    Ok(rejected_prepared_intent_dto(&outcome, &correlation))
}

fn rejected_prepared_intent_dto(
    outcome: &RejectedPreparedIntentOutcome,
    correlation: &CorrelationId,
) -> RejectedPreparedIntentDto {
    RejectedPreparedIntentDto {
        prepared_intent_id: outcome.prepared_intent_id().as_str().to_owned(),
        disposition: "rejected",
        rejected_at_millis: outcome.rejected_at().unix_millis(),
        expired_at_rejection: outcome.expired_at_rejection(),
        correlation_id: correlation.as_str().to_owned(),
    }
}

fn decision_flow_error(error: DecisionFlowError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        DecisionFlowError::Domain(domain) => SafeErrorDto::from_domain(&domain),
        DecisionFlowError::Ledger(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        DecisionFlowError::Load(DecisionPersistenceLoadError::StorageUnavailable) => {
            SafeErrorDto::host(
                "LEDGER_SNAPSHOT_UNAVAILABLE",
                "desktop.snapshot_unavailable",
                correlation,
                true,
            )
        }
        DecisionFlowError::Load(DecisionPersistenceLoadError::UnsupportedSchema { .. }) => {
            SafeErrorDto::host(
                "LEDGER_UNSUPPORTED_SCHEMA",
                "ledger.open.unsupported_schema",
                correlation,
                false,
            )
        }
        DecisionFlowError::Load(DecisionPersistenceLoadError::InvalidDecisionSnapshot) => {
            SafeErrorDto::host(
                "LEDGER_SNAPSHOT_INVALID",
                "desktop.snapshot_invalid",
                correlation,
                false,
            )
        }
        DecisionFlowError::Approval(_) => SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            correlation,
            false,
        ),
        DecisionFlowError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_id_source_failed",
            correlation,
            false,
        ),
    }
}

fn decision_request_summary(record: &DecisionRequestRecord) -> DecisionRequestSummaryDto {
    DecisionRequestSummaryDto {
        id: record.id().as_str().to_owned(),
        subject: record.subject().as_str().to_owned(),
        details: record.details().as_str().to_owned(),
        intended_owner: record
            .intended_owner()
            .map(|owner| owner.as_str().to_owned()),
        classification: record.classification().as_persisted(),
        state: record.state().as_persisted(),
        version: record.version().get(),
        withdrawal_rationale: record
            .withdrawal_rationale()
            .map(|rationale| rationale.as_str().to_owned()),
        linked_decision_id: record.linked_decision_id().map(|id| id.as_str().to_owned()),
    }
}

fn decision_summary(record: &DecisionRecord) -> DecisionSummaryDto {
    DecisionSummaryDto {
        id: record.id().as_str().to_owned(),
        source_request_id: record.source_request_id().map(|id| id.as_str().to_owned()),
        statement: record.statement().as_str().to_owned(),
        rationale: record.rationale().as_str().to_owned(),
        impact: record.impact().as_str().to_owned(),
        owner: record.owner().as_str().to_owned(),
        decided_at_millis: record.decided_at().unix_millis(),
        classification: record.classification().as_persisted(),
        state: record.state().as_persisted(),
        version: record.version().get(),
        resulting_action_request_ids: record
            .resulting_action_request_ids()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

/// A webview argument that does not parse as the domain type it must be.
/// The envelope names the failure class only; the offending value is not
/// echoed back (it may be anything the webview held).
/// The one instant the webview authors: the deadline a person sets on a
/// resulting Action Request. It is their choice, not host authority -- but
/// `UtcTimestamp` validates nothing, and the Ledger stores it under
/// `CHECK(intended_action_due_at>=0)`. Refuse it here, so a bad value is a
/// refusal the person sees before approving rather than a persistence failure
/// after.
fn due_at(millis: i64) -> Result<pmc_domain::time::UtcTimestamp, ()> {
    if millis < 0 {
        return Err(());
    }
    Ok(pmc_domain::time::UtcTimestamp::from_unix_millis(millis))
}

fn parse_argument<T, E>(
    parsed: Result<T, E>,
    correlation: &CorrelationId,
) -> Result<T, SafeErrorDto> {
    parsed.map_err(|_| {
        SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            correlation,
            false,
        )
    })
}

fn context(
    client_request_id: String,
    correlation: &CorrelationId,
) -> Result<ActionOperationContext, SafeErrorDto> {
    Ok(ActionOperationContext {
        idempotency_id: parse_argument(IdempotencyId::parse(client_request_id), correlation)?,
        correlation_id: correlation.clone(),
    })
}

fn flow_error(error: ActionRequestFlowError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        ActionRequestFlowError::Domain(domain) => SafeErrorDto::from_domain(&domain),
        ActionRequestFlowError::Ledger(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        // Only an unavailable store is worth retrying; a schema the host does
        // not support, a namespace the writer refuses, or a snapshot that
        // fails its own consistency validation will not change by retrying.
        ActionRequestFlowError::Load(ActionPersistenceLoadError::StorageUnavailable) => {
            SafeErrorDto::host(
                "LEDGER_SNAPSHOT_UNAVAILABLE",
                "desktop.snapshot_unavailable",
                correlation,
                true,
            )
        }
        ActionRequestFlowError::Load(ActionPersistenceLoadError::UnsupportedSchema { .. }) => {
            SafeErrorDto::host(
                "LEDGER_UNSUPPORTED_SCHEMA",
                "ledger.open.unsupported_schema",
                correlation,
                false,
            )
        }
        ActionRequestFlowError::Load(
            ActionPersistenceLoadError::ActionNamespaceNotEmpty
            | ActionPersistenceLoadError::InvalidActionSnapshot,
        ) => SafeErrorDto::host(
            "LEDGER_SNAPSHOT_INVALID",
            "desktop.snapshot_invalid",
            correlation,
            false,
        ),
        ActionRequestFlowError::Approval(_) => SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            correlation,
            false,
        ),
        ActionRequestFlowError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_id_source_failed",
            correlation,
            false,
        ),
    }
}

fn audit_ids<T>(outcome: &ActionMutationOutcome<T>) -> Vec<String> {
    outcome
        .audit_events
        .iter()
        .map(|audit| audit.id().as_str().to_owned())
        .collect()
}

fn request_outcome(
    outcome: ActionMutationOutcome<ActionRequestRecord>,
    correlation: &CorrelationId,
) -> ActionRequestOutcomeDto {
    ActionRequestOutcomeDto {
        request: request_summary(&outcome.record),
        audit_event_ids: audit_ids(&outcome),
        correlation_id: correlation.as_str().to_owned(),
    }
}

fn request_summary(record: &ActionRequestRecord) -> ActionRequestSummaryDto {
    ActionRequestSummaryDto {
        id: record.id().as_str().to_owned(),
        title: record.title().as_str().to_owned(),
        details: record.details().as_str().to_owned(),
        intended_owner: record
            .intended_owner()
            .map(|owner| owner.as_str().to_owned()),
        response_due_at_millis: record.response_due_at().map(|at| at.unix_millis()),
        intended_action_due_at_millis: record.intended_action_due_at().map(|at| at.unix_millis()),
        classification: record.classification().as_persisted(),
        state: record.state().as_persisted(),
        version: record.version().get(),
        terminal_rationale: record
            .terminal_rationale()
            .map(|rationale| rationale.as_str().to_owned()),
        linked_action_id: record.linked_action_id().map(|id| id.as_str().to_owned()),
    }
}

fn action_summary(record: &ActionRecord) -> ActionSummaryDto {
    ActionSummaryDto {
        id: record.id().as_str().to_owned(),
        source_request_id: record.source_request_id().as_str().to_owned(),
        title: record.title().as_str().to_owned(),
        details: record.details().as_str().to_owned(),
        owner: record.owner().as_str().to_owned(),
        due_at_millis: record.due_at().unix_millis(),
        classification: record.classification().as_persisted(),
        state: record.state().as_persisted(),
        version: record.version().get(),
    }
}

/// The whole typed prepared contract. The four Action kinds S03 wires are
/// mapped; a preview of any other kind here would be a host bug, reported
/// as one rather than rendered as something it is not.
pub(crate) fn prepared_intent_dto(
    prepared: &WorkManagementPreparedIntent,
    correlation: &CorrelationId,
) -> Result<PreparedIntentDto, SafeErrorDto> {
    let preview = prepared.preview();
    let intent_type = match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest { .. } => {
            WorkManagementH2aIntentKind::AcceptActionRequest
        }
        WorkManagementOperation::CompleteAction { .. } => {
            WorkManagementH2aIntentKind::CompleteAction
        }
        WorkManagementOperation::CancelAction { .. } => WorkManagementH2aIntentKind::CancelAction,
        WorkManagementOperation::ReopenAction { .. } => WorkManagementH2aIntentKind::ReopenAction,
        WorkManagementOperation::ResolveDecisionRequest { .. } => {
            WorkManagementH2aIntentKind::ResolveDecisionRequest
        }
        WorkManagementOperation::RecordRiskOccurrence { .. } => {
            WorkManagementH2aIntentKind::RecordRiskOccurrence
        }
        WorkManagementOperation::CloseRisk { .. } => WorkManagementH2aIntentKind::CloseRisk,
        WorkManagementOperation::ResolveIssue { .. } => WorkManagementH2aIntentKind::ResolveIssue,
        WorkManagementOperation::CloseIssue { .. } => WorkManagementH2aIntentKind::CloseIssue,
        WorkManagementOperation::ReopenIssue { .. } => WorkManagementH2aIntentKind::ReopenIssue,
        _ => {
            return Err(SafeErrorDto::host(
                "PLATFORM_INTERNAL",
                "desktop.unsupported_preview",
                correlation,
                false,
            ))
        }
    }
    .as_persisted();
    let operation = match prepared.operation() {
        WorkManagementOperation::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            action_classification,
            action_subject,
            commitment_details,
            intended_owner,
            intended_due_at,
        } => PreparedOperationDto::AcceptActionRequest {
            request_id: request_id.as_str().to_owned(),
            request_version: request_version.get(),
            action_id: action_id.as_str().to_owned(),
            action_classification: action_classification.as_persisted(),
            action_subject: action_subject.as_str().to_owned(),
            commitment_details: commitment_details.as_str().to_owned(),
            intended_owner: intended_owner.as_str().to_owned(),
            intended_due_at_millis: intended_due_at.unix_millis(),
        },
        WorkManagementOperation::CompleteAction {
            action_id,
            action_version,
        } => PreparedOperationDto::CompleteAction {
            action_id: action_id.as_str().to_owned(),
            action_version: action_version.get(),
        },
        WorkManagementOperation::CancelAction {
            action_id,
            action_version,
            reason,
            evidence_classifications,
        } => PreparedOperationDto::CancelAction {
            action_id: action_id.as_str().to_owned(),
            action_version: action_version.get(),
            reason: reason.as_str().to_owned(),
            evidence_classifications: evidence_classifications
                .iter()
                .map(|binding| EvidenceClassificationDto {
                    evidence_id: binding.evidence_id().as_str().to_owned(),
                    classification: binding.classification().as_persisted(),
                })
                .collect(),
        },
        WorkManagementOperation::ReopenAction {
            action_id,
            action_version,
            mode,
            reason,
            evidence_classifications,
        } => PreparedOperationDto::ReopenAction {
            action_id: action_id.as_str().to_owned(),
            action_version: action_version.get(),
            mode: match mode {
                ActionReopenMode::ReopenCompleted => "reopen_completed",
                ActionReopenMode::RestartCancelled => "restart_cancelled",
            },
            reason: reason.as_str().to_owned(),
            evidence_classifications: evidence_classifications
                .iter()
                .map(|binding| EvidenceClassificationDto {
                    evidence_id: binding.evidence_id().as_str().to_owned(),
                    classification: binding.classification().as_persisted(),
                })
                .collect(),
        },
        WorkManagementOperation::ResolveDecisionRequest {
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
        } => PreparedOperationDto::ResolveDecisionRequest {
            request_id: request_id.as_str().to_owned(),
            request_version: request_version.get(),
            decision_id: decision_id.as_str().to_owned(),
            decision_classification: decision_classification.as_persisted(),
            statement: statement.as_str().to_owned(),
            rationale: rationale.as_str().to_owned(),
            impact: impact.as_str().to_owned(),
            decision_owner: decision_owner.as_str().to_owned(),
            decided_at_millis: decided_at.unix_millis(),
            resulting_action_requests: resulting_action_requests
                .iter()
                .map(|request| ResultingActionRequestDto {
                    id: request.id.as_str().to_owned(),
                    subject: request.subject.as_str().to_owned(),
                    details: request.details.as_str().to_owned(),
                    intended_owner: request.intended_owner.as_str().to_owned(),
                    due_at_millis: request.due_at.unix_millis(),
                    classification: request.classification.as_persisted(),
                })
                .collect(),
        },
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id,
            risk_version,
            issue_id,
            issue_classification,
        } => PreparedOperationDto::RecordRiskOccurrence {
            risk_id: risk_id.as_str().to_owned(),
            risk_version: risk_version.get(),
            issue_id: issue_id.as_str().to_owned(),
            issue_classification: issue_classification.as_persisted(),
        },
        WorkManagementOperation::CloseRisk {
            risk_id,
            risk_version,
            rationale,
        } => PreparedOperationDto::CloseRisk {
            risk_id: risk_id.as_str().to_owned(),
            risk_version: risk_version.get(),
            rationale: rationale.as_str().to_owned(),
        },
        WorkManagementOperation::ResolveIssue {
            issue_id,
            issue_version,
            resolution_type,
            rationale,
        } => PreparedOperationDto::ResolveIssue {
            issue_id: issue_id.as_str().to_owned(),
            issue_version: issue_version.get(),
            resolution_type: resolution_type.as_persisted(),
            rationale: rationale.as_str().to_owned(),
        },
        WorkManagementOperation::CloseIssue {
            issue_id,
            issue_version,
        } => PreparedOperationDto::CloseIssue {
            issue_id: issue_id.as_str().to_owned(),
            issue_version: issue_version.get(),
        },
        WorkManagementOperation::ReopenIssue {
            issue_id,
            issue_version,
            rationale,
        } => PreparedOperationDto::ReopenIssue {
            issue_id: issue_id.as_str().to_owned(),
            issue_version: issue_version.get(),
            rationale: rationale.as_str().to_owned(),
        },
        _ => {
            return Err(SafeErrorDto::host(
                "PLATFORM_INTERNAL",
                "desktop.unsupported_preview",
                correlation,
                false,
            ))
        }
    };
    let expires_at_millis = preview.expires_at().unix_millis();
    Ok(PreparedIntentDto {
        prepared_intent_id: prepared.id().as_str().to_owned(),
        contract_version: preview.contract_version(),
        intent_type,
        operation,
        targets: preview.targets().iter().map(target_dto).collect(),
        declared_effects: preview.effects().iter().map(effect_dto).collect(),
        payload_digest: prepared.payload_digest().as_str().to_owned(),
        resolved_classification: prepared.classification().as_persisted(),
        classification_sources: preview
            .classification_sources()
            .iter()
            .map(|source| {
                let (role, id) = source_role(source.role());
                ClassificationSourceDto {
                    role,
                    id,
                    classification: source.classification().as_persisted(),
                }
            })
            .collect(),
        support: preview.support().map(support_dto),
        // Read from the contract, not asserted: each enum has one variant
        // today, and a second one must reach the wire through this match.
        policy_result: match preview.policy() {
            WorkManagementPolicyDecision::Allowed => "allowed",
        },
        authority: match preview.authority() {
            WorkManagementAuthority::HeadOfProducts => "head_of_products",
        },
        prepared_at_millis: expires_at_millis - WORK_MANAGEMENT_H2A_TTL_MILLIS,
        expires_at_millis,
        cancellation_policy: match preview.cancellation_policy() {
            WorkManagementCancellationPolicy::NotCancellableAfterSubmit => {
                "not_cancellable_after_submit"
            }
        },
        correlation_id: correlation.as_str().to_owned(),
    })
}

fn target_dto(target: &WorkManagementTarget) -> PreparedTargetDto {
    let (kind, id, version) = match target {
        WorkManagementTarget::ActionRequest(id, version) => {
            ("action_request", id.as_str(), version)
        }
        WorkManagementTarget::Action(id, version) => ("action", id.as_str(), version),
        WorkManagementTarget::DecisionRequest(id, version) => {
            ("decision_request", id.as_str(), version)
        }
        WorkManagementTarget::Decision(id, version) => ("decision", id.as_str(), version),
        WorkManagementTarget::Risk(id, version) => ("risk", id.as_str(), version),
        WorkManagementTarget::Issue(id, version) => ("issue", id.as_str(), version),
        WorkManagementTarget::Portfolio(id, version) => ("portfolio", id.as_str(), version),
        WorkManagementTarget::Product(id, version) => ("product", id.as_str(), version),
        WorkManagementTarget::Roadmap(id, version) => ("roadmap", id.as_str(), version),
        WorkManagementTarget::Kpi(id, version) => ("kpi", id.as_str(), version),
        WorkManagementTarget::KpiObservation(id, version) => {
            ("kpi_observation", id.as_str(), version)
        }
        WorkManagementTarget::Initiative(id, version) => ("initiative", id.as_str(), version),
        WorkManagementTarget::Project(id, version) => ("project", id.as_str(), version),
        WorkManagementTarget::Milestone(id, version) => ("milestone", id.as_str(), version),
    };
    PreparedTargetDto {
        kind,
        id: id.to_owned(),
        expected_version: version.get(),
    }
}

fn effect_dto(effect: &WorkManagementEffect) -> PreparedEffectDto {
    let (kind, ids): (&'static str, Vec<&str>) = match effect {
        WorkManagementEffect::AcceptActionRequest(id) => {
            ("accept_action_request", vec![id.as_str()])
        }
        WorkManagementEffect::CreateAction(id) => ("create_action", vec![id.as_str()]),
        WorkManagementEffect::LinkActionRequestToAction(request, action) => (
            "link_action_request_to_action",
            vec![request.as_str(), action.as_str()],
        ),
        WorkManagementEffect::ResolveDecisionRequest(id) => {
            ("resolve_decision_request", vec![id.as_str()])
        }
        WorkManagementEffect::CreateDecision(id) => ("create_decision", vec![id.as_str()]),
        WorkManagementEffect::LinkDecisionRequestToDecision(request, decision) => (
            "link_decision_request_to_decision",
            vec![request.as_str(), decision.as_str()],
        ),
        WorkManagementEffect::CreateResultingActionRequest(id) => {
            ("create_resulting_action_request", vec![id.as_str()])
        }
        WorkManagementEffect::LinkDecisionToActionRequest(decision, request) => (
            "link_decision_to_action_request",
            vec![decision.as_str(), request.as_str()],
        ),
        WorkManagementEffect::CompleteAction(id) => ("complete_action", vec![id.as_str()]),
        WorkManagementEffect::CancelAction(id) => ("cancel_action", vec![id.as_str()]),
        WorkManagementEffect::ReopenAction(id) => ("reopen_action", vec![id.as_str()]),
        WorkManagementEffect::SupersedeDecision(id) => ("supersede_decision", vec![id.as_str()]),
        WorkManagementEffect::LinkReplacementDecision(old, new) => (
            "link_replacement_decision",
            vec![old.as_str(), new.as_str()],
        ),
        WorkManagementEffect::FlagSupersededPremiseActionRequest(id) => {
            ("flag_superseded_premise_action_request", vec![id.as_str()])
        }
        WorkManagementEffect::FlagSupersededPremiseAction(id) => {
            ("flag_superseded_premise_action", vec![id.as_str()])
        }
        WorkManagementEffect::RecordRiskOccurrence(id) => {
            ("record_risk_occurrence", vec![id.as_str()])
        }
        WorkManagementEffect::CreateIssue(id) => ("create_issue", vec![id.as_str()]),
        WorkManagementEffect::LinkRiskToIssue(risk, issue) => {
            ("link_risk_to_issue", vec![risk.as_str(), issue.as_str()])
        }
        WorkManagementEffect::CloseRisk(id) => ("close_risk", vec![id.as_str()]),
        WorkManagementEffect::ResolveIssue(id) => ("resolve_issue", vec![id.as_str()]),
        WorkManagementEffect::CloseIssue(id) => ("close_issue", vec![id.as_str()]),
        WorkManagementEffect::ReopenIssue(id) => ("reopen_issue", vec![id.as_str()]),
        WorkManagementEffect::LowerPortfolioClassification(id) => {
            ("lower_portfolio_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerProductClassification(id) => {
            ("lower_product_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerRoadmapClassification(id) => {
            ("lower_roadmap_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerKpiClassification(id) => {
            ("lower_kpi_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerKpiObservationClassification(id) => {
            ("lower_kpi_observation_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerActionClassification(id) => {
            ("lower_action_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerDecisionClassification(id) => {
            ("lower_decision_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerRiskClassification(id) => {
            ("lower_risk_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerIssueClassification(id) => {
            ("lower_issue_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerInitiativeClassification(id) => {
            ("lower_initiative_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerProjectClassification(id) => {
            ("lower_project_classification", vec![id.as_str()])
        }
        WorkManagementEffect::LowerMilestoneClassification(id) => {
            ("lower_milestone_classification", vec![id.as_str()])
        }
    };
    PreparedEffectDto {
        kind,
        ids: ids.into_iter().map(str::to_owned).collect(),
    }
}

fn source_role(role: &WorkManagementClassificationSourceRole) -> (&'static str, Option<String>) {
    match role {
        WorkManagementClassificationSourceRole::PrimaryTarget => ("primary_target", None),
        WorkManagementClassificationSourceRole::CreatedAction => ("created_action", None),
        WorkManagementClassificationSourceRole::CreatedDecision => ("created_decision", None),
        WorkManagementClassificationSourceRole::ReplacementDecision => {
            ("replacement_decision", None)
        }
        WorkManagementClassificationSourceRole::CreatedIssue => ("created_issue", None),
        WorkManagementClassificationSourceRole::DownstreamActionRequest(id) => {
            ("downstream_action_request", Some(id.as_str().to_owned()))
        }
        WorkManagementClassificationSourceRole::DownstreamAction(id) => {
            ("downstream_action", Some(id.as_str().to_owned()))
        }
        WorkManagementClassificationSourceRole::ResultingActionRequest(id) => {
            ("resulting_action_request", Some(id.as_str().to_owned()))
        }
        WorkManagementClassificationSourceRole::Evidence(id) => {
            ("evidence", Some(id.as_str().to_owned()))
        }
        WorkManagementClassificationSourceRole::HumanJudgment => ("human_judgment", None),
    }
}

fn support_dto(support: &SupportWitness) -> SupportWitnessDto {
    SupportWitnessDto {
        disposition: match support.disposition() {
            SupportDisposition::EvidenceSatisfied => "evidence_satisfied",
            SupportDisposition::JudgmentSatisfied => "judgment_satisfied",
            SupportDisposition::VerificationPending => "verification_pending",
        },
        classification: support.classification().as_persisted(),
        evidence: support
            .evidence()
            .iter()
            .map(|evidence| SupportEvidenceDto {
                id: evidence.id().as_str().to_owned(),
                source_version: evidence.source_version().get(),
                classification: evidence.classification().as_persisted(),
                role: evidence_role(&evidence.role()),
                verification: verification_dto(evidence.verification()),
            })
            .collect(),
        judgments: support
            .judgments()
            .iter()
            .map(|judgment| SupportJudgmentDto {
                disposition: match judgment.disposition() {
                    HumanJudgmentDisposition::ProceedWithDocumentedRationale => {
                        "proceed_with_documented_rationale"
                    }
                },
                rationale: judgment.rationale().to_owned(),
                classification: judgment.classification().as_persisted(),
            })
            .collect(),
    }
}

fn verification_dto(verification: &EvidenceVerification) -> EvidenceVerificationDto {
    let (at_millis, integrity_digest) = match verification {
        EvidenceVerification::Verified {
            verified_at,
            integrity_digest,
        } => (
            Some(verified_at.unix_millis()),
            Some(integrity_digest.as_str().to_owned()),
        ),
        EvidenceVerification::ObservedUnpinned {
            observed_at,
            integrity_digest,
        } => (
            Some(observed_at.unix_millis()),
            Some(integrity_digest.as_str().to_owned()),
        ),
        EvidenceVerification::DegradedLastVerified {
            last_verified_at,
            integrity_digest,
        } => (
            Some(last_verified_at.unix_millis()),
            Some(integrity_digest.as_str().to_owned()),
        ),
        EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch => (None, None),
    };
    EvidenceVerificationDto {
        kind: verification.kind_as_persisted(),
        at_millis,
        integrity_digest,
    }
}

// ---------------------------------------------------------------------------
// Evidence writes from the Product inspector
// ---------------------------------------------------------------------------

/// What one Evidence write left behind. `changed` is the honest half: a
/// re-observation that found the live state already matching what was stored
/// writes nothing, so there is no new version and no audit event, and this
/// DTO must not imply otherwise.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceWriteOutcomeDto {
    pub changed: bool,
    pub evidence: EvidenceReferenceSummaryDto,
    pub correlation_id: String,
}

/// H1-User:pin a fingerprint onto an Evidence reference created
/// without one, from the live bytes at the path the record already stores.
///
/// The webview supplies the identity it read and one opaque request id. It
/// supplies no path: the record's own `vault_path` is used, so no Vault path
/// crosses this boundary in either direction.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn pin_evidence_fingerprint(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    vault: State<'_, VaultState>,
    settings: State<'_, SettingsState>,
    runtime: State<'_, HostRuntime>,
    evidence_id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<EvidenceWriteOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let vault = vault.current(&settings);
    let id = parse_argument(EvidenceReferenceId::parse(evidence_id), &correlation)?;
    let expected = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let idempotency = parse_argument(IdempotencyId::parse(client_request_id), &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let outcome = pin_fingerprint(
        &mut ledger,
        &vault,
        &id,
        expected,
        idempotency,
        correlation.clone(),
        &mut *ids,
        now,
    )
    .map_err(|error| evidence_write_error(error, &correlation))?;
    Ok(EvidenceWriteOutcomeDto {
        changed: true,
        evidence: evidence_record_summary(&outcome.record),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// H1-User:re-observe one Evidence source against the live
/// filesystem, persisting only when the observed state differs from what is
/// stored.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn reobserve_evidence_verification(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    vault: State<'_, VaultState>,
    settings: State<'_, SettingsState>,
    runtime: State<'_, HostRuntime>,
    evidence_id: String,
    expected_version: u64,
    client_request_id: String,
) -> Result<EvidenceWriteOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let vault = vault.current(&settings);
    let id = parse_argument(EvidenceReferenceId::parse(evidence_id), &correlation)?;
    let expected = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let idempotency = parse_argument(IdempotencyId::parse(client_request_id), &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let result = reobserve(
        &mut ledger,
        &vault,
        &id,
        expected,
        idempotency,
        correlation.clone(),
        &mut *ids,
        now,
    )
    .map_err(|error| evidence_write_error(error, &correlation))?;
    Ok(match result {
        ReobserveResult::Unchanged(record) => EvidenceWriteOutcomeDto {
            changed: false,
            evidence: evidence_record_summary(&record),
            correlation_id: correlation.as_str().to_owned(),
        },
        ReobserveResult::Persisted(outcome) => EvidenceWriteOutcomeDto {
            changed: true,
            evidence: evidence_record_summary(&outcome.record),
            correlation_id: correlation.as_str().to_owned(),
        },
    })
}

/// What the Vault can do for this workspace right now (H0, read-only). O01
/// reads it so the filesystem actions can be shown disabled *with a reason*
/// instead of failing when clicked. Validated at the moment of asking, so it
/// is as fresh as the operations it gates -- and just as non-perpetual.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatusDto {
    /// A Vault folder is set for this workspace: Training's own, or the one
    /// the person chose for Live (item ⑦). False is "none is set", never
    /// "the folder is missing" -- a folder on an unplugged drive is
    /// configured and unavailable, and says so.
    pub configured: bool,
    pub available: bool,
    /// `notConfigured` (no Vault is set) or `invalidRoot` (one is set but is
    /// missing, not a directory, or a link). `null` when available.
    pub reason: Option<&'static str>,
    /// The folder's own name -- its last component only -- so a person can
    /// recognise it. `null` when none is set, for a drive root, and always
    /// for Training, whose Vault the person did not choose. The rest of the
    /// path never crosses this boundary (ADR 0011).
    pub folder_name: Option<String>,
    pub correlation_id: String,
}

#[tauri::command]
pub fn get_vault_status(
    vault: State<'_, VaultState>,
    settings: State<'_, SettingsState>,
) -> VaultStatusDto {
    let correlation = host_correlation();
    // One read of the settings for both facts: availability and the name
    // must describe the same folder.
    let (current, folder_name) = vault.snapshot(&settings);
    let reason = match current.root() {
        Ok(_) => None,
        Err(VaultUnavailable::NotConfigured) => Some("notConfigured"),
        Err(VaultUnavailable::InvalidRoot) => Some("invalidRoot"),
        Err(VaultUnavailable::ChangeUnresolved) => Some("changeUnresolved"),
    };
    VaultStatusDto {
        configured: reason != Some("notConfigured"),
        available: reason.is_none(),
        reason,
        folder_name,
        correlation_id: correlation.as_str().to_owned(),
    }
}

/// What a link left behind: the pair, and the classification the Ledger
/// recorded for it at link time (the combine of both sides).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceLinkOutcomeDto {
    pub evidence_id: String,
    pub product_id: String,
    pub classification_at_link: &'static str,
    pub correlation_id: String,
}

/// H1-User:link an Evidence reference to the inspected Product.
/// Ledger-only -- works while the Vault is unavailable. The webview supplies
/// the two identities it read and the Evidence version it saw, and -- from
/// "Add Evidence from a file…" (DG3 Vault-root amendment §4) -- the Product
/// version it read: a Product that changed since is refused, not linked.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn link_evidence_to_product(
    ledger: State<'_, LedgerState>,
    gate: State<'_, BackupGate>,
    runtime: State<'_, HostRuntime>,
    product_id: String,
    evidence_id: String,
    expected_version: u64,
    client_request_id: String,
    expected_product_version: Option<u64>,
) -> Result<EvidenceLinkOutcomeDto, SafeErrorDto> {
    let correlation = host_correlation();
    let product = parse_argument(ProductId::parse(product_id), &correlation)?;
    let id = parse_argument(EvidenceReferenceId::parse(evidence_id), &correlation)?;
    let expected = parse_argument(AggregateVersion::new(expected_version), &correlation)?;
    let idempotency = parse_argument(IdempotencyId::parse(client_request_id), &correlation)?;
    let now = runtime.now();
    let mut ledger = ledger
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    // Checked under the write lock the link itself takes, so no write can
    // land between this read and the link.
    if expected_product_version.is_some() {
        let current = ledger
            .read_product_entry(&product)
            .map_err(|open| SafeErrorDto::from_open(open, &correlation))?
            .map(|record| record.version);
        product_as_read(current, expected_product_version, &correlation)?;
    }
    let mut ids = runtime.ids();
    let outcome = link_to_product(
        &mut ledger,
        product.clone(),
        &id,
        expected,
        idempotency,
        correlation.clone(),
        &mut *ids,
        now,
    )
    .map_err(|error| evidence_write_error(error, &correlation))?;
    Ok(EvidenceLinkOutcomeDto {
        evidence_id: outcome.record.evidence_id.as_str().to_owned(),
        product_id: product.as_str().to_owned(),
        classification_at_link: outcome.record.classification.as_persisted(),
        correlation_id: correlation.as_str().to_owned(),
    })
}

/// The Product is still the version the view read. No version to compare
/// (an older caller), or no such Product (the link refuses an unknown
/// target itself), is not this check's refusal.
fn product_as_read(
    current: Option<AggregateVersion>,
    read: Option<u64>,
    correlation: &CorrelationId,
) -> Result<(), SafeErrorDto> {
    match (current, read) {
        (Some(current), Some(read)) if current.get() != read => Err(SafeErrorDto::host(
            "PRODUCT_CHANGED",
            "product.stale_version",
            correlation,
            false,
        )),
        _ => Ok(()),
    }
}

/// The same read-only shape `get_evidence_references` returns, built from a
/// full record instead of a read record. Still never a Vault path.
fn evidence_record_summary(record: &EvidenceReferenceRecord) -> EvidenceReferenceSummaryDto {
    EvidenceReferenceSummaryDto {
        id: record.id.as_str().to_owned(),
        role: None,
        verification: verification_dto(&record.verification),
        pinned: record.fingerprint.is_some(),
        classification: record.classification.as_persisted(),
        version: record.version.get(),
    }
}

/// Every variant is a condition a person can be told about. Retryable means
/// the same request id may be resubmitted and could then succeed: a Vault
/// that is not yet seeded, or a source file that is not readable right now,
/// can both change without the application doing anything, and neither left
/// a Ledger write behind. A Vault that is not configured, a reference that
/// is already pinned, and a containment failure will still be true on the
/// next attempt, so they are not.
fn evidence_write_error(error: EvidenceWriteError, correlation: &CorrelationId) -> SafeErrorDto {
    match error {
        EvidenceWriteError::Vault(VaultUnavailable::NotConfigured) => SafeErrorDto::host(
            "VAULT_NOT_CONFIGURED",
            "desktop.vault_not_configured",
            correlation,
            false,
        ),
        // Retryable on purpose: this is what an unseeded Training workspace
        // looks like, and running the seed makes the same request succeed
        // without restarting the application.
        EvidenceWriteError::Vault(VaultUnavailable::InvalidRoot) => SafeErrorDto::host(
            "VAULT_ROOT_UNAVAILABLE",
            "desktop.vault_root_unavailable",
            correlation,
            true,
        ),
        // Not retryable in this run: only the next start can resolve it.
        EvidenceWriteError::Vault(VaultUnavailable::ChangeUnresolved) => SafeErrorDto::host(
            "VAULT_CHANGE_UNRESOLVED",
            "desktop.vault_change_unresolved",
            correlation,
            false,
        ),
        EvidenceWriteError::NotFound => SafeErrorDto::host(
            "EVIDENCE_NOT_FOUND",
            "desktop.evidence_not_found",
            correlation,
            false,
        ),
        // The view was stale before the host even read the record. Not
        // retryable as-is: the same request would conflict again. The
        // current version rides along so the surface can re-read and offer
        // the person a fresh decision, the way every other version conflict
        // on this boundary does.
        EvidenceWriteError::VersionConflict { current } => {
            let mut error = SafeErrorDto::host(
                "DOMAIN_CONFLICT",
                "desktop.evidence_version_conflict",
                correlation,
                false,
            );
            error
                .extensions
                .push(SafeErrorExtensionDto::CurrentVersion {
                    version: current.get(),
                });
            error
        }
        EvidenceWriteError::AlreadyPinned => SafeErrorDto::host(
            "EVIDENCE_ALREADY_PINNED",
            "desktop.evidence_already_pinned",
            correlation,
            false,
        ),
        // Retryable: the file can appear or become readable, and no write
        // happened, so replaying the same request id would then succeed.
        EvidenceWriteError::SourceNotObservable(_) => SafeErrorDto::host(
            "EVIDENCE_SOURCE_NOT_OBSERVABLE",
            "desktop.evidence_source_not_observable",
            correlation,
            true,
        ),
        EvidenceWriteError::Containment => SafeErrorDto::host(
            "EVIDENCE_PATH_CONTAINMENT_FAILED",
            "desktop.evidence_path_containment_failed",
            correlation,
            false,
        ),
        EvidenceWriteError::Read(domain) => SafeErrorDto::from_domain(&domain),
        EvidenceWriteError::Write(transaction) => {
            SafeErrorDto::from_transaction(transaction, correlation)
        }
        // A host-minted id that fails its own parser is this host's bug, not
        // the caller's, and retrying mints another one of the same shape.
        EvidenceWriteError::Id(_) => SafeErrorDto::host(
            "PLATFORM_INTERNAL",
            "desktop.host_identifier_failed",
            correlation,
            false,
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use pmc_domain::actions::{ActionDetails, ActionTitle};
    use pmc_domain::classification::DataClassification;
    use pmc_domain::identity::{ActionId, ActionRequestId, IssueId, RiskId, StakeholderId};
    use pmc_domain::time::UtcTimestamp;
    use pmc_domain::work_management::EvidenceReferenceMetadata;

    use super::*;

    fn accept_preview() -> WorkManagementPreparedIntent {
        WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("prepared-1").unwrap(),
            WorkManagementOperation::AcceptActionRequest {
                request_id: ActionRequestId::parse("request-1").unwrap(),
                request_version: AggregateVersion::new(2).unwrap(),
                action_id: ActionId::parse("action-1").unwrap(),
                action_classification: DataClassification::Internal,
                action_subject: ActionTitle::parse("Synthetic subject").unwrap(),
                commitment_details: ActionDetails::parse("Synthetic commitment.").unwrap(),
                intended_owner: StakeholderId::parse("owner-1").unwrap(),
                intended_due_at: UtcTimestamp::from_unix_millis(300_000),
            },
            DataClassification::Internal,
            None,
            UtcTimestamp::from_unix_millis(1_000),
        )
        .unwrap()
    }

    /// The deadline is the one instant the webview authors, so it is the one
    /// that has to be refused rather than carried into the Ledger. A negative
    /// value would violate `CHECK(intended_action_due_at>=0)` at execute time --
    /// after the person approved -- which is a persistence failure where a
    /// refusal belongs.
    #[test]
    fn a_negative_resulting_due_date_is_refused_and_a_valid_one_is_carried() {
        assert!(due_at(-1).is_err());
        assert_eq!(due_at(0).unwrap(), UtcTimestamp::from_unix_millis(0));
        assert_eq!(
            due_at(10_000).unwrap(),
            UtcTimestamp::from_unix_millis(10_000)
        );
    }

    /// Evidence from a file links to the Product as the view read it (DG3
    /// Vault-root amendment §4 Boundary): a Product that moved on is refused
    /// before any link, and not retryable as-is.
    #[test]
    fn a_link_to_a_product_that_changed_since_it_was_read_is_refused() {
        let correlation = host_correlation();
        let v = |n| Some(AggregateVersion::new(n).unwrap());
        let refused = product_as_read(v(3), Some(2), &correlation).unwrap_err();
        assert_eq!(refused.message_key, "product.stale_version");
        assert!(!refused.retryable);
        assert!(product_as_read(v(2), Some(2), &correlation).is_ok());
        // The existing link control sends no Product version.
        assert!(product_as_read(v(3), None, &correlation).is_ok());
        // An unknown Product is the link's own refusal.
        assert!(product_as_read(None, Some(2), &correlation).is_ok());
    }

    /// A stale view is a conflict the person can act on: the code says so,
    /// the current version rides along, and it is not retryable as-is.
    #[test]
    fn a_stale_evidence_version_maps_to_a_domain_conflict_carrying_the_current_version() {
        let error = evidence_write_error(
            EvidenceWriteError::VersionConflict {
                current: AggregateVersion::new(4).unwrap(),
            },
            &host_correlation(),
        );
        assert_eq!(error.error_code, "DOMAIN_CONFLICT");
        assert_eq!(error.message_key, "desktop.evidence_version_conflict");
        assert!(!error.retryable);
        assert_eq!(
            error.extensions,
            vec![SafeErrorExtensionDto::CurrentVersion { version: 4 }]
        );
    }

    #[test]
    fn the_prepared_dto_carries_the_whole_typed_contract_and_the_full_digest() {
        let prepared = accept_preview();
        let dto = prepared_intent_dto(&prepared, &host_correlation()).unwrap();
        assert_eq!(dto.prepared_intent_id, "prepared-1");
        assert_eq!(dto.intent_type, "accept_action_request");
        assert_eq!(dto.payload_digest, prepared.payload_digest().as_str());
        assert_eq!(dto.payload_digest.len(), 64);
        assert_eq!(
            dto.expires_at_millis,
            1_000 + WORK_MANAGEMENT_H2A_TTL_MILLIS
        );
        assert_eq!(dto.prepared_at_millis, 1_000);
        assert_eq!(dto.resolved_classification, "internal");
        assert_eq!(dto.policy_result, "allowed");
        assert_eq!(dto.authority, "head_of_products");
        assert!(dto.support.is_none());
        let PreparedOperationDto::AcceptActionRequest {
            request_id,
            request_version,
            action_id,
            intended_due_at_millis,
            ..
        } = &dto.operation
        else {
            panic!("an Accept preview must map to the Accept operation");
        };
        assert_eq!(request_id, "request-1");
        assert_eq!(*request_version, 2);
        assert_eq!(action_id, "action-1");
        assert_eq!(*intended_due_at_millis, 300_000);
        assert_eq!(dto.targets.len(), 1);
        assert_eq!(dto.targets[0].kind, "action_request");
        assert_eq!(dto.targets[0].expected_version, 2);
        let effects: Vec<&str> = dto.declared_effects.iter().map(|e| e.kind).collect();
        assert_eq!(
            effects,
            vec![
                "accept_action_request",
                "create_action",
                "link_action_request_to_action"
            ]
        );
        assert_eq!(dto.classification_sources.len(), 2);
    }

    #[test]
    fn a_snapshot_load_refusal_is_retryable_only_when_the_store_was_unavailable() {
        let correlation = host_correlation();
        let unavailable = flow_error(
            ActionRequestFlowError::Load(ActionPersistenceLoadError::StorageUnavailable),
            &correlation,
        );
        assert!(unavailable.retryable);
        let invalid = flow_error(
            ActionRequestFlowError::Load(ActionPersistenceLoadError::InvalidActionSnapshot),
            &correlation,
        );
        assert!(!invalid.retryable);
        assert_eq!(invalid.message_key, "desktop.snapshot_invalid");
        let schema = flow_error(
            ActionRequestFlowError::Load(ActionPersistenceLoadError::UnsupportedSchema {
                found: 1,
            }),
            &correlation,
        );
        assert!(!schema.retryable);
    }

    /// An Issue Judgment arrives as a pair: both halves or neither. Half a
    /// pair, or a classification the host does not know, is a validation
    /// refusal that echoes nothing; the disposition is always the host's.
    #[test]
    fn an_issue_judgment_is_both_halves_or_neither() {
        let correlation = host_correlation();
        assert_eq!(
            optional_issue_judgment(None, None, &correlation).unwrap(),
            None
        );
        let judgment = optional_issue_judgment(
            Some("The file was read; pinning waits.".to_owned()),
            Some("confidential".to_owned()),
            &correlation,
        )
        .unwrap()
        .expect("a full pair is a Judgment");
        assert_eq!(judgment.rationale(), "The file was read; pinning waits.");
        assert_eq!(judgment.classification(), DataClassification::Confidential);
        assert_eq!(
            judgment.disposition(),
            HumanJudgmentDisposition::ProceedWithDocumentedRationale
        );
        for (rationale, classification) in [
            (Some("Only the words.".to_owned()), None),
            (None, Some("internal".to_owned())),
            (Some("Words.".to_owned()), Some("top-secret".to_owned())),
            (Some(String::new()), Some("internal".to_owned())),
        ] {
            let error =
                optional_issue_judgment(rationale, classification, &correlation).unwrap_err();
            assert_eq!(error.error_code, "VALIDATION_INVALID_FIELD");
            assert_eq!(error.message_key, "desktop.invalid_argument");
            assert!(error.message_params.is_empty());
        }
    }

    #[test]
    fn a_bad_argument_is_a_validation_refusal_that_echoes_nothing() {
        let correlation = host_correlation();
        let error = parse_argument(ActionRequestId::parse(""), &correlation).unwrap_err();
        assert_eq!(error.error_code, "VALIDATION_INVALID_FIELD");
        assert_eq!(error.message_key, "desktop.invalid_argument");
        assert!(!error.retryable);
        assert!(error.message_params.is_empty());
    }

    #[test]
    fn a_cancel_preview_maps_its_reason_and_bound_evidence_classifications() {
        let prepared = WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("prepared-2").unwrap(),
            WorkManagementOperation::CancelAction {
                action_id: ActionId::parse("action-1").unwrap(),
                action_version: AggregateVersion::new(3).unwrap(),
                reason: ActionDetails::parse("Scope moved.").unwrap(),
                evidence_classifications: vec![
                    pmc_domain::work_management::EvidenceClassificationBinding::new(
                        EvidenceReferenceId::parse("evidence-1").unwrap(),
                        DataClassification::Confidential,
                    ),
                ],
            },
            DataClassification::Confidential,
            None,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
        let dto = prepared_intent_dto(&prepared, &host_correlation()).unwrap();
        assert_eq!(dto.intent_type, "cancel_action");
        let PreparedOperationDto::CancelAction {
            action_id,
            action_version,
            reason,
            evidence_classifications,
        } = &dto.operation
        else {
            panic!("a Cancel preview must map to the Cancel operation");
        };
        assert_eq!(action_id, "action-1");
        assert_eq!(*action_version, 3);
        assert_eq!(reason, "Scope moved.");
        assert_eq!(evidence_classifications.len(), 1);
        assert_eq!(evidence_classifications[0].classification, "confidential");
        assert_eq!(dto.resolved_classification, "confidential");
    }

    /// An occurrence preview is the only place the person sees the identity
    /// the Ledger is about to create on their say-so.
    #[test]
    fn an_occurrence_preview_names_the_issue_it_will_create() {
        let prepared = WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("prepared-risk-1").unwrap(),
            WorkManagementOperation::RecordRiskOccurrence {
                risk_id: RiskId::parse("risk-1").unwrap(),
                risk_version: AggregateVersion::new(2).unwrap(),
                issue_id: IssueId::parse("issue-from-occurrence").unwrap(),
                issue_classification: DataClassification::Internal,
            },
            DataClassification::Internal,
            None,
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
        let dto = prepared_intent_dto(&prepared, &host_correlation()).unwrap();
        assert_eq!(dto.intent_type, "record_risk_occurrence");
        let PreparedOperationDto::RecordRiskOccurrence {
            risk_id,
            risk_version,
            issue_id,
            issue_classification,
        } = &dto.operation
        else {
            panic!("an occurrence preview must map to the occurrence operation");
        };
        assert_eq!(risk_id, "risk-1");
        assert_eq!(*risk_version, 2);
        assert_eq!(issue_id, "issue-from-occurrence");
        assert_eq!(*issue_classification, "internal");
        assert!(!dto.payload_digest.is_empty());
    }

    #[test]
    fn a_reopen_issue_preview_carries_the_persons_own_rationale() {
        let prepared = WorkManagementPreparedIntent::prepare(
            PreparedIntentId::parse("prepared-issue-1").unwrap(),
            WorkManagementOperation::ReopenIssue {
                issue_id: IssueId::parse("issue-1").unwrap(),
                issue_version: AggregateVersion::new(2).unwrap(),
                rationale: WorkManagementRationale::parse("The fix did not hold.").unwrap(),
            },
            DataClassification::Internal,
            Some(
                pmc_domain::work_management::EvidenceOrJudgment::new(
                    vec![EvidenceReferenceMetadata::new(
                        EvidenceReferenceId::parse("evidence-failed-verification").unwrap(),
                        AggregateVersion::initial(),
                        DataClassification::Internal,
                        EvidenceRole::IssueFailedVerification,
                        EvidenceVerification::Verified {
                            verified_at: UtcTimestamp::from_unix_millis(1_000),
                            integrity_digest: pmc_domain::work_management::IntegrityDigest::parse(
                                "a".repeat(64),
                            )
                            .unwrap(),
                        },
                    )],
                    vec![],
                )
                .unwrap()
                .evaluate_evidence_required()
                .unwrap(),
            ),
            UtcTimestamp::from_unix_millis(2_000),
        )
        .unwrap();
        let dto = prepared_intent_dto(&prepared, &host_correlation()).unwrap();
        assert_eq!(dto.intent_type, "reopen_issue");
        let support = dto
            .support
            .as_ref()
            .expect("every Issue transition is Evidence-gated, so the sheet shows the support");
        assert_eq!(support.evidence.len(), 1);
        let PreparedOperationDto::ReopenIssue {
            issue_id,
            issue_version,
            rationale,
        } = &dto.operation
        else {
            panic!("a reopen preview must map to the reopen operation");
        };
        assert_eq!(issue_id, "issue-1");
        assert_eq!(*issue_version, 2);
        assert_eq!(rationale, "The fix did not hold.");
    }
}
