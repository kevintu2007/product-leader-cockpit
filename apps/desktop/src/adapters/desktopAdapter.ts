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
  RiskOutcomeDto,
  ResolvedDecisionDto,
  WorkItemKind,
  WorkQueueDto,
} from "../routes/cockpitContract";
import type { ResolveDecisionRequestInput } from "../routes/ipcCockpit";

/**
 * Everything the React surface may ask the desktop host for, one explicit
 * method per canonical query (and, in later slices, per canonical domain
 * intent). There is deliberately no `invoke(name, payload)`: a route can
 * only reach what this interface names, and a test adapter can stand in for
 * the host without a Tauri runtime.
 *
 * Rejections are the host's safe error envelope (`SafeErrorDto`) when the
 * host produced them; anything else is a transport failure and is treated
 * as an unknown error with no correlation id.
 */
export interface DesktopAdapter {
  readonly loadExecutiveCockpit: () => Promise<ExecutiveCockpitDto>;
  readonly loadPortfolioOverview: (offset: number, limit: number) => Promise<PortfolioOverviewDto>;
  readonly loadPeopleDirectory: (offset: number, limit: number) => Promise<PeopleDirectoryDto>;
  readonly loadWorkQueue: (
    offset: number,
    limit: number,
    kinds: readonly WorkItemKind[],
    onlyFlagged: boolean,
  ) => Promise<WorkQueueDto>;
  readonly loadProductDetail: (productId: string) => Promise<ProductDetailDto>;
  // S03 write path: one method per reviewed host command.
  readonly prepareAcceptActionRequest: (
    requestId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly approveAndExecuteAcceptActionRequest: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<AcceptedActionDto>;
  readonly rejectPreparedAcceptActionRequest: (
    preparedIntentId: string,
    clientRequestId: string,
  ) => Promise<RejectedPreparedIntentDto>;
  readonly declineActionRequest: (
    requestId: string,
    expectedVersion: number,
    rationale: string,
    clientRequestId: string,
  ) => Promise<ActionRequestOutcomeDto>;
  readonly withdrawActionRequest: (
    requestId: string,
    expectedVersion: number,
    rationale: string,
    clientRequestId: string,
  ) => Promise<ActionRequestOutcomeDto>;
  readonly startAction: (
    actionId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<ActionOutcomeDto>;
  // Action lifecycle.
  readonly loadActionCompletionContext: (actionId: string) => Promise<ActionCompletionContextDto>;
  readonly linkActionCompletionEvidence: (
    actionId: string,
    expectedVersion: number,
    evidenceId: string,
    clientRequestId: string,
  ) => Promise<ActionOutcomeDto>;
  readonly prepareCompleteAction: (
    actionId: string,
    expectedVersion: number,
    judgmentRationale: string | null,
    judgmentClassification: string | null,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly prepareCancelAction: (
    actionId: string,
    expectedVersion: number,
    reason: string,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly prepareReopenAction: (
    actionId: string,
    expectedVersion: number,
    mode: string,
    reason: string,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly approveAndExecuteCompleteAction: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<ActionOutcomeDto>;
  readonly approveAndExecuteCancelAction: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<ActionOutcomeDto>;
  readonly approveAndExecuteReopenAction: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<ActionOutcomeDto>;
  readonly rejectPreparedActionIntent: (
    preparedIntentId: string,
    clientRequestId: string,
  ) => Promise<RejectedPreparedIntentDto>;
  // Decision Requests.
  readonly loadEvidenceReferences: () => Promise<EvidenceReferencesDto>;
  readonly withdrawDecisionRequest: (
    requestId: string,
    expectedVersion: number,
    rationale: string,
    clientRequestId: string,
  ) => Promise<DecisionRequestOutcomeDto>;
  readonly prepareResolveDecisionRequest: (
    requestId: string,
    expectedVersion: number,
    input: ResolveDecisionRequestInput,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly approveAndExecuteResolveDecisionRequest: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<ResolvedDecisionDto>;
  readonly rejectPreparedDecisionIntent: (
    preparedIntentId: string,
    clientRequestId: string,
  ) => Promise<RejectedPreparedIntentDto>;
  // Evidence writes from the Product inspector. Neither takes a path: the host uses the one the
  // Evidence record already stores.
  readonly pinEvidenceFingerprint: (
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<EvidenceWriteOutcomeDto>;
  readonly reobserveEvidenceVerification: (
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<EvidenceWriteOutcomeDto>;
  readonly loadVaultStatus: () => Promise<VaultStatusDto>;
  /** Read-only: the opened Ledger's schema version and commit revision. */
  readonly loadLedgerStatus: () => Promise<LedgerStatusDto>;
  readonly linkEvidenceToProduct: (
    productId: string,
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
    expectedProductVersion?: number,
  ) => Promise<EvidenceLinkOutcomeDto>;
  // S03 Risk and Issue H2a. The occurrence preview names the
  // Issue the host will create; the webview never supplies that identity.
  readonly prepareRecordRiskOccurrence: (
    riskId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly prepareCloseRisk: (
    riskId: string,
    expectedVersion: number,
    rationale: string,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly approveAndExecuteRecordRiskOccurrence: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<OccurredRiskDto>;
  readonly approveAndExecuteCloseRisk: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<RiskOutcomeDto>;
  readonly rejectPreparedRiskIntent: (
    preparedIntentId: string,
    clientRequestId: string,
  ) => Promise<RejectedPreparedIntentDto>;
  readonly prepareResolveIssue: (
    issueId: string,
    expectedVersion: number,
    resolutionType: string,
    rationale: string,
    evidenceIds: readonly string[],
    judgmentRationale: string | null,
    judgmentClassification: string | null,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly prepareCloseIssue: (
    issueId: string,
    expectedVersion: number,
    evidenceIds: readonly string[],
    judgmentRationale: string | null,
    judgmentClassification: string | null,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  readonly prepareReopenIssue: (
    issueId: string,
    expectedVersion: number,
    rationale: string,
    evidenceIds: readonly string[],
    judgmentRationale: string | null,
    judgmentClassification: string | null,
    clientRequestId: string,
  ) => Promise<PreparedIntentDto>;
  /** One command for all three transitions: the host reads which one from
   * the preview it stored. */
  readonly approveAndExecuteIssueTransition: (
    preparedIntentId: string,
    acknowledgedPayloadDigest: string,
    clientRequestId: string,
  ) => Promise<IssueOutcomeDto>;
  readonly rejectPreparedIssueIntent: (
    preparedIntentId: string,
    clientRequestId: string,
  ) => Promise<RejectedPreparedIntentDto>;
}
