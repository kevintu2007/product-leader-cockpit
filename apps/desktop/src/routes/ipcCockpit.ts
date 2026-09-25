import { invoke } from "@tauri-apps/api/core";

import type {
  AcceptedActionDto,
  ActionCompletionContextDto,
  ActionOutcomeDto,
  DecisionRequestOutcomeDto,
  EvidenceLinkOutcomeDto,
  EvidenceReferencesDto,
  EvidenceWriteOutcomeDto,
  VaultStatusDto,
  LedgerStatusDto,
  ActionRequestOutcomeDto,
  ExecutiveCockpitDto,
  PeopleDirectoryDto,
  PortfolioOverviewDto,
  PreparedIntentDto,
  ProductDetailDto,
  IssueOutcomeDto,
  OccurredRiskDto,
  RejectedPreparedIntentDto,
  ResolvedDecisionDto,
  RiskOutcomeDto,
  ResultingActionRequestInput,
  WorkItemKind,
  WorkQueueDto,
} from "./cockpitContract";

/**
 * The production loader for the S01 Executive Cockpit.
 *
 * `asOfMillis` is supplied by the caller rather than read inside the command,
 * matching the rule that `pmc-ledger` never originates a timestamp: the whole
 * composition then agrees on one instant instead of reading a clock twice.
 */
export function loadExecutiveCockpit(): Promise<ExecutiveCockpitDto> {
  return invoke<ExecutiveCockpitDto>("get_executive_cockpit", {
    asOfMillis: Date.now(),
  });
}

/** The production loader for S02's list half. */
export function loadPortfolioOverview(
  offset: number,
  limit: number,
): Promise<PortfolioOverviewDto> {
  return invoke<PortfolioOverviewDto>("get_portfolio_overview", {
    asOfMillis: Date.now(),
    offset,
    limit,
  });
}

/** The production loader for S09. */
export function loadPeopleDirectory(offset: number, limit: number): Promise<PeopleDirectoryDto> {
  return invoke<PeopleDirectoryDto>("get_people_directory", {
    asOfMillis: Date.now(),
    offset,
    limit,
  });
}

/**
 * The production loader for S03 Work Queue.
 *
 * `kinds` empty means every kind, matching the composition default: an empty
 * filter shows the whole queue rather than nothing.
 */
export function loadWorkQueue(
  offset: number,
  limit: number,
  kinds: readonly WorkItemKind[],
  onlyFlagged: boolean,
): Promise<WorkQueueDto> {
  return invoke<WorkQueueDto>("get_work_queue", {
    asOfMillis: Date.now(),
    offset,
    limit,
    kinds,
    onlyFlagged,
  });
}

/** The production loader for the S02 detail half and the O01 inspector. */
export function loadProductDetail(productId: string): Promise<ProductDetailDto> {
  return invoke<ProductDetailDto>("get_product_detail", {
    asOfMillis: Date.now(),
    productId,
  });
}

// ---------------------------------------------------------------------------
// S03 write path. Each loader is exactly one reviewed
// host command; the route supplies only what it read, the person's own
// words, and its opaque client request id.
// ---------------------------------------------------------------------------

export function prepareAcceptActionRequest(
  requestId: string,
  expectedVersion: number,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_accept_action_request", {
    requestId,
    expectedVersion,
    clientRequestId,
  });
}

export function approveAndExecuteAcceptActionRequest(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<AcceptedActionDto> {
  return invoke<AcceptedActionDto>("approve_and_execute_accept_action_request", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function rejectPreparedAcceptActionRequest(
  preparedIntentId: string,
  clientRequestId: string,
): Promise<RejectedPreparedIntentDto> {
  return invoke<RejectedPreparedIntentDto>("reject_prepared_accept_action_request", {
    preparedIntentId,
    clientRequestId,
  });
}

export function declineActionRequest(
  requestId: string,
  expectedVersion: number,
  rationale: string,
  clientRequestId: string,
): Promise<ActionRequestOutcomeDto> {
  return invoke<ActionRequestOutcomeDto>("decline_action_request", {
    requestId,
    expectedVersion,
    rationale,
    clientRequestId,
  });
}

export function withdrawActionRequest(
  requestId: string,
  expectedVersion: number,
  rationale: string,
  clientRequestId: string,
): Promise<ActionRequestOutcomeDto> {
  return invoke<ActionRequestOutcomeDto>("withdraw_action_request", {
    requestId,
    expectedVersion,
    rationale,
    clientRequestId,
  });
}

export function startAction(
  actionId: string,
  expectedVersion: number,
  clientRequestId: string,
): Promise<ActionOutcomeDto> {
  return invoke<ActionOutcomeDto>("start_action", {
    actionId,
    expectedVersion,
    clientRequestId,
  });
}

// ---------------------------------------------------------------------------
// Action lifecycle.
// ---------------------------------------------------------------------------

export function loadActionCompletionContext(actionId: string): Promise<ActionCompletionContextDto> {
  return invoke<ActionCompletionContextDto>("get_action_completion_context", { actionId });
}

export function linkActionCompletionEvidence(
  actionId: string,
  expectedVersion: number,
  evidenceId: string,
  clientRequestId: string,
): Promise<ActionOutcomeDto> {
  return invoke<ActionOutcomeDto>("link_action_completion_evidence", {
    actionId,
    expectedVersion,
    evidenceId,
    clientRequestId,
  });
}

export function prepareCompleteAction(
  actionId: string,
  expectedVersion: number,
  judgmentRationale: string | null,
  judgmentClassification: string | null,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_complete_action", {
    actionId,
    expectedVersion,
    judgmentRationale,
    judgmentClassification,
    clientRequestId,
  });
}

export function prepareCancelAction(
  actionId: string,
  expectedVersion: number,
  reason: string,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_cancel_action", {
    actionId,
    expectedVersion,
    reason,
    clientRequestId,
  });
}

export function prepareReopenAction(
  actionId: string,
  expectedVersion: number,
  mode: string,
  reason: string,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_reopen_action", {
    actionId,
    expectedVersion,
    mode,
    reason,
    clientRequestId,
  });
}

export function approveAndExecuteCompleteAction(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<ActionOutcomeDto> {
  return invoke<ActionOutcomeDto>("approve_and_execute_complete_action", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function approveAndExecuteCancelAction(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<ActionOutcomeDto> {
  return invoke<ActionOutcomeDto>("approve_and_execute_cancel_action", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function approveAndExecuteReopenAction(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<ActionOutcomeDto> {
  return invoke<ActionOutcomeDto>("approve_and_execute_reopen_action", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function rejectPreparedActionIntent(
  preparedIntentId: string,
  clientRequestId: string,
): Promise<RejectedPreparedIntentDto> {
  return invoke<RejectedPreparedIntentDto>("reject_prepared_action_intent", {
    preparedIntentId,
    clientRequestId,
  });
}

// ---------------------------------------------------------------------------
// Decision Requests.
// ---------------------------------------------------------------------------

export function loadEvidenceReferences(): Promise<EvidenceReferencesDto> {
  return invoke<EvidenceReferencesDto>("get_evidence_references", {});
}

export function withdrawDecisionRequest(
  requestId: string,
  expectedVersion: number,
  rationale: string,
  clientRequestId: string,
): Promise<DecisionRequestOutcomeDto> {
  return invoke<DecisionRequestOutcomeDto>("withdraw_decision_request", {
    requestId,
    expectedVersion,
    rationale,
    clientRequestId,
  });
}

export interface ResolveDecisionRequestInput {
  readonly statement: string;
  readonly rationale: string;
  readonly impact: string;
  readonly evidenceIds: readonly string[];
  readonly judgmentRationale: string | null;
  readonly judgmentClassification: string | null;
  readonly resultingActionRequests: readonly ResultingActionRequestInput[];
}

export function prepareResolveDecisionRequest(
  requestId: string,
  expectedVersion: number,
  input: ResolveDecisionRequestInput,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_resolve_decision_request", {
    requestId,
    expectedVersion,
    statement: input.statement,
    rationale: input.rationale,
    impact: input.impact,
    evidenceIds: input.evidenceIds,
    judgmentRationale: input.judgmentRationale,
    judgmentClassification: input.judgmentClassification,
    resultingActionRequests: input.resultingActionRequests,
    clientRequestId,
  });
}

export function approveAndExecuteResolveDecisionRequest(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<ResolvedDecisionDto> {
  return invoke<ResolvedDecisionDto>("approve_and_execute_resolve_decision_request", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

/**
 * H1-User Evidence write from the inspector: pin a fingerprint onto an
 * Evidence reference created without one, and re-observe one against the
 * live filesystem.
 *
 * Neither takes a path. The host uses the path the Evidence record already
 * stores, so no Vault path crosses the IPC boundary in either direction --
 * the same rule every other write command here follows.
 */
export function pinEvidenceFingerprint(
  evidenceId: string,
  expectedVersion: number,
  clientRequestId: string,
): Promise<EvidenceWriteOutcomeDto> {
  return invoke<EvidenceWriteOutcomeDto>("pin_evidence_fingerprint", {
    evidenceId,
    expectedVersion,
    clientRequestId,
  });
}

export function reobserveEvidenceVerification(
  evidenceId: string,
  expectedVersion: number,
  clientRequestId: string,
): Promise<EvidenceWriteOutcomeDto> {
  return invoke<EvidenceWriteOutcomeDto>("reobserve_evidence_verification", {
    evidenceId,
    expectedVersion,
    clientRequestId,
  });
}

/** H0: can the Vault serve pin / re-observe right now, and if not, why. */
export function loadLedgerStatus(): Promise<LedgerStatusDto> {
  return invoke<LedgerStatusDto>("get_ledger_status");
}

export function loadVaultStatus(): Promise<VaultStatusDto> {
  return invoke<VaultStatusDto>("get_vault_status");
}

/**
 * H1-User Evidence write from the inspector: link an Evidence reference to
 * the inspected Product.
 * Ledger-only, so it works while the Vault is unavailable.
 * `expectedProductVersion`: the Product version the view read; when given,
 * the host refuses a Product that changed since.
 */
export function linkEvidenceToProduct(
  productId: string,
  evidenceId: string,
  expectedVersion: number,
  clientRequestId: string,
  expectedProductVersion?: number,
): Promise<EvidenceLinkOutcomeDto> {
  return invoke<EvidenceLinkOutcomeDto>("link_evidence_to_product", {
    productId,
    evidenceId,
    expectedVersion,
    clientRequestId,
    expectedProductVersion: expectedProductVersion ?? null,
  });
}

export function rejectPreparedDecisionIntent(
  preparedIntentId: string,
  clientRequestId: string,
): Promise<RejectedPreparedIntentDto> {
  return invoke<RejectedPreparedIntentDto>("reject_prepared_decision_intent", {
    preparedIntentId,
    clientRequestId,
  });
}

export function prepareRecordRiskOccurrence(
  riskId: string,
  expectedVersion: number,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_record_risk_occurrence", {
    riskId,
    expectedVersion,
    clientRequestId,
  });
}

export function prepareCloseRisk(
  riskId: string,
  expectedVersion: number,
  rationale: string,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_close_risk", {
    riskId,
    expectedVersion,
    rationale,
    clientRequestId,
  });
}

export function approveAndExecuteRecordRiskOccurrence(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<OccurredRiskDto> {
  return invoke<OccurredRiskDto>("approve_and_execute_record_risk_occurrence", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function approveAndExecuteCloseRisk(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<RiskOutcomeDto> {
  return invoke<RiskOutcomeDto>("approve_and_execute_close_risk", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function rejectPreparedRiskIntent(
  preparedIntentId: string,
  clientRequestId: string,
): Promise<RejectedPreparedIntentDto> {
  return invoke<RejectedPreparedIntentDto>("reject_prepared_risk_intent", {
    preparedIntentId,
    clientRequestId,
  });
}

export function prepareResolveIssue(
  issueId: string,
  expectedVersion: number,
  resolutionType: string,
  rationale: string,
  evidenceIds: readonly string[],
  judgmentRationale: string | null,
  judgmentClassification: string | null,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_resolve_issue", {
    issueId,
    expectedVersion,
    resolutionType,
    rationale,
    evidenceIds,
    judgmentRationale,
    judgmentClassification,
    clientRequestId,
  });
}

export function prepareCloseIssue(
  issueId: string,
  expectedVersion: number,
  evidenceIds: readonly string[],
  judgmentRationale: string | null,
  judgmentClassification: string | null,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_close_issue", {
    issueId,
    expectedVersion,
    evidenceIds,
    judgmentRationale,
    judgmentClassification,
    clientRequestId,
  });
}

export function prepareReopenIssue(
  issueId: string,
  expectedVersion: number,
  rationale: string,
  evidenceIds: readonly string[],
  judgmentRationale: string | null,
  judgmentClassification: string | null,
  clientRequestId: string,
): Promise<PreparedIntentDto> {
  return invoke<PreparedIntentDto>("prepare_reopen_issue", {
    issueId,
    expectedVersion,
    rationale,
    evidenceIds,
    judgmentRationale,
    judgmentClassification,
    clientRequestId,
  });
}

/** One command for all three Issue transitions: the host reads which one it
 * is from the preview it stored. */
export function approveAndExecuteIssueTransition(
  preparedIntentId: string,
  acknowledgedPayloadDigest: string,
  clientRequestId: string,
): Promise<IssueOutcomeDto> {
  return invoke<IssueOutcomeDto>("approve_and_execute_issue_transition", {
    preparedIntentId,
    acknowledgedPayloadDigest,
    clientRequestId,
  });
}

export function rejectPreparedIssueIntent(
  preparedIntentId: string,
  clientRequestId: string,
): Promise<RejectedPreparedIntentDto> {
  return invoke<RejectedPreparedIntentDto>("reject_prepared_issue_intent", {
    preparedIntentId,
    clientRequestId,
  });
}
