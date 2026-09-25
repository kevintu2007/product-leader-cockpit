/**
 * The Executive Cockpit transport shape.
 *
 * Mirrors the `ExecutiveCockpitDto` the Tauri command returns. Declared here
 * rather than inferred so a change on the Rust side that this file does not
 * follow becomes a TypeScript error rather than a field that silently reads
 * as `undefined` in the UI.
 *
 * Nothing here carries a SQLite row, filesystem path, shell string or private
 * diagnostic, which the frozen DG3 query contract forbids surfacing. There is
 * deliberately no progress or percentage field: the composition computes
 * counts only, and a field that does not exist cannot be rendered.
 */

/** The Ledger this workspace opened: its schema version and commit revision. */
export interface LedgerStatusDto {
  readonly schemaVersion: number;
  readonly revision: number;
}

export interface CountDto {
  readonly count: number;
  /** What exactly was counted, so the number can be inspected rather than
   * guessed at. */
  readonly definition: string;
  /** The module that owns the counted fact. */
  readonly owner: string;
}

export interface PulseDto {
  readonly milestones: CountDto;
  readonly commitments: CountDto;
  readonly kpis: CountDto;
}

export interface ProductDto {
  readonly id: string;
  readonly classification: string;
  readonly revision: number;
  readonly owner: string;
  readonly degraded: boolean;
  readonly attentionCount: number;
}

/** One Portfolio exception: the record it is about, and why it is here. */
export interface ExceptionDto {
  /** The lifecycle type and identifier of the flagged record. */
  readonly kind: WorkItemKind;
  readonly id: string;
  /** The same label the Work Queue shows for that record. */
  readonly label: string;
  /** The reason identifier, stable across rewording of `explanation`. */
  readonly reason: string;
  readonly explanation: string;
  /** The tier identifier, and why it outranks the tiers below it. */
  readonly tier: string;
  readonly tierWhy: string;
  /** The deadline the flag is about, or `null` when it is not about a time. */
  readonly relevantAtMillis: number | null;
  /** Why this item is ranked where it is. */
  readonly rankRationale: string;
  readonly classification: string;
  readonly freshness: string;
  readonly degraded: boolean;
}

export interface ExecutiveCockpitDto {
  readonly state: string;
  readonly asOfMillis: number;
  readonly ledgerRevision: number;
  /** False when no review period has been approved, with the reason in
   * `periodNote`. Never a zero delta, which would claim nothing changed. */
  readonly periodComparable: boolean;
  readonly periodNote: string;
  /** `null` only when `state` is `outOfSync`: the two Ledger reads disagreed,
   * so no count is true at a single revision. */
  readonly pulse: PulseDto | null;
  readonly products: readonly ProductDto[];
  readonly exceptions: readonly ExceptionDto[];
  readonly leaderConclusion: string;
  readonly leaderIntervention: string | null;
  /** The Executive Lens on the amended S01 axes; `null` only when `state` is
   * `outOfSync`. Every judgement in it was made on the host. */
  readonly lens: ExecutiveLensDto | null;
}

/** One record behind a Lens measure. `version` is `null` for an Evidence
 * link, which has no identity or version of its own. */
export interface LensContributionDto {
  readonly kind: string;
  readonly id: string;
  readonly version: number | null;
  readonly classification: string;
  readonly role: string;
}

export type LensTimingState = "unknown" | "later" | "dueSoon" | "datePassed";

export interface LensTimingDto {
  readonly state: LensTimingState;
  /** `null` when the state is `unknown`: Unknown is neither half. */
  readonly high: boolean | null;
  readonly earliestDueAtMillis: number | null;
  readonly milestoneCount: number;
  readonly contributions: readonly LensContributionDto[];
}

export interface LensObservabilityDto {
  readonly observed: number;
  /** Zero means Unknown, never 0%. */
  readonly defined: number;
  readonly known: boolean;
  /** `null` when not `known`. */
  readonly high: boolean | null;
  readonly latestObservedAtMillis: number | null;
  readonly contributions: readonly LensContributionDto[];
}

export interface LensStateCountDto {
  readonly state: string;
  readonly count: number;
}

export interface LensCoverageDto {
  readonly verified: number;
  /** Zero means "no linked Evidence", never 0%. */
  readonly linked: number;
  /** Most severe first, by the amendment's frozen order. */
  readonly byState: readonly LensStateCountDto[];
  readonly worst: string | null;
  readonly contributions: readonly LensContributionDto[];
}

export type LensQuadrant =
  "keepMomentum" | "monitorClosely" | "exploreAndValidate" | "prioritizeNow";

export interface LensPointDto {
  readonly productId: string;
  readonly productName: string;
  readonly productVersion: number;
  readonly productClassification: string;
  readonly timing: LensTimingDto;
  readonly observability: LensObservabilityDto;
  readonly coverage: LensCoverageDto;
  /** `null` whenever either axis is Unknown. */
  readonly quadrant: LensQuadrant | null;
  readonly effectiveClassification: string;
  readonly classificationForcedBy: LensContributionDto | null;
  readonly sharedProjectIds: readonly string[];
}

export interface ExecutiveLensDto {
  readonly asOfMillis: number;
  readonly ledgerRevision: number;
  readonly dueSoonWindowMillis: number;
  /** In Product name order: navigation, not priority. */
  readonly points: readonly LensPointDto[];
}

/** One Portfolio row: the Product's Lens measures and the flagged work the
 * people accountable for it carry. */
export interface PortfolioRowDto {
  readonly point: LensPointDto;
  /** Distinct flagged work items, carried by the people, never the Product's. */
  readonly flaggedWorkCount: number;
  /** The point's classification folded with every counted item's. */
  readonly classification: string;
}

export interface PortfolioOverviewDto {
  readonly state: string;
  readonly asOfMillis: number;
  readonly ledgerRevision: number;
  readonly dueSoonWindowMillis: number;
  /** In Product name order: navigation, not priority. Empty when `outOfSync`. */
  readonly rows: readonly PortfolioRowDto[];
  readonly offset: number;
  readonly limit: number;
  readonly total: number;
  readonly hasMore: boolean;
}

export interface PersonDto {
  readonly id: string;
  /** The effective classification: the most restrictive of the person and
   * everything the entry exposes, never the record's own weaker label. */
  readonly classification: string;
  readonly revision: number;
  readonly owner: string;
  /** Counted separately from dependencies: being responsible for something
   * and depending on it are different relationships. */
  readonly responsibilityCount: number;
  readonly dependencyCount: number;
  /** Open requests only. An accepted request has become an Action and is a
   * commitment rather than a request. */
  readonly outstandingRequestCount: number;
  readonly displayName: string;
  /** `person` or `organization`. */
  readonly kind: string;
  /** Already folded into `classification`. */
  readonly relationships: readonly PersonRelationshipDto[];
  readonly outstandingRequests: readonly PersonRequestDto[];
}

export interface PersonRelationshipDto {
  readonly subjectId: string;
  readonly subjectLabel: string;
  /** `responsibility` or `dependency`. */
  readonly purpose: string;
}

export interface PersonRequestDto {
  readonly id: string;
  readonly label: string;
}

export interface PeopleDirectoryDto {
  readonly state: string;
  readonly asOfMillis: number;
  readonly ledgerRevision: number;
  readonly people: readonly PersonDto[];
  readonly offset: number;
  readonly limit: number;
  readonly total: number;
  readonly hasMore: boolean;
}

/** The five lifecycle types the Work Queue (S03) spans. */
export type WorkItemKind = "action_request" | "action" | "decision_request" | "risk" | "issue";

export interface WorkQueueItemDto {
  /** Required on every item. The Work Queue requires the five lifecycle types
   * to stay separate, so a surface must never render a queue row without the
   * type it belongs to. */
  readonly kind: WorkItemKind;
  readonly id: string;
  /** The record title or subject where the Ledger carries one, and the type
   * plus identifier where it does not. Never an invented name. */
  readonly label: string;
  readonly stateLabel: string;
  readonly classification: string;
  readonly revision: number;
  readonly owner: string;
  /** When the Ledger row behind this item was read. Per item, because the
   * five kinds come from two snapshots read in two separate transactions, and
   * the envelope `asOfMillis` is the caller request time rather than a read
   * time. Together with `owner`, `id` and `revision` this is the provenance
   * the Work Queue (S03) must preserve. */
  readonly asOfMillis: number;
  /** What the lifecycle admits -- NOT what may be executed now. Preparation,
   * classification, policy and approval are all further gates, so these must
   * be presented as possible next steps and never as authorized ones. */
  readonly lifecycleLegalIntents: readonly string[];
  /** `null` where the Ledger holds no deadline for this kind at all. Decision
   * Requests and Issues persist none, so theirs is always null, and no
   * default is substituted anywhere. */
  readonly relevantAtMillis: number | null;
  /** When the work an Action Request asks for is promised to be done. Not the
   * request's own deadline, so it never ranks the item. Absent or `null` for
   * every other kind and for a request that names no date. */
  readonly promisedAtMillis?: number | null;
  readonly placement: string;
  /** Why the item is ranked where it is, produced from the ordering itself. */
  readonly placementRationale: string;
  /** Every flag raised against this record, ordered by the accepted ranking
   * policy; the first is the one that placed the item. DG0 lists "attention
   * reasons" as Work Queue content, and a count would tell a reader that two
   * things are wrong without telling them what. Empty means nothing has
   * flagged this record, which is a fact rather than a missing value. */
  readonly attention: readonly WorkQueueAttentionDto[];
}

export interface WorkQueueAttentionDto {
  /** The reason identifier. Stable across rewording of the prose below. */
  readonly reason: string;
  readonly explanation: string;
  readonly tier: string;
  readonly tierWhy: string;
  /** `null` when the reason is not about a time at all. Never substituted. */
  readonly relevantAtMillis: number | null;
  readonly rankRationale: string;
  readonly classification: string;
  readonly freshness: string;
  /** Shown, never used to improve the position: a stale fact is not a more
   * certain one. */
  readonly degraded: boolean;
}

export interface WorkQueueKindCountDto {
  readonly kind: WorkItemKind;
  readonly count: number;
}

export interface WorkQueueDto {
  readonly state: string;
  readonly asOfMillis: number;
  readonly ledgerRevision: number;
  readonly items: readonly WorkQueueItemDto[];
  /** Every kind, including those with none, so a zero and an uncounted kind
   * stay distinguishable. */
  readonly countsByKind: readonly WorkQueueKindCountDto[];
  readonly offset: number;
  readonly limit: number;
  readonly total: number;
  readonly hasMore: boolean;
}

/** The Product detail and O01 inspector, per the DG3 O01 amendment. */
export interface ProductSummaryDto {
  readonly id: string;
  readonly label: string;
  /** The effective classification: the most restrictive of the Product and
   * everything the inspector exposes. `classificationForcedBy` says why. */
  readonly classification: string;
  readonly revision: number;
  readonly owner: string;
  readonly asOfMillis: number;
}

export interface ClassificationFoldDto {
  readonly forcedByKind: string;
  readonly forcedById: string;
  readonly classification: string;
}

export interface StructureEntryDto {
  readonly kind: string;
  readonly id: string;
  readonly label: string;
  readonly classification: string;
  readonly revision: number;
  /** Set only for an Initiative: the Project it was reached through. An
   * association, never containment. */
  readonly via: string | null;
}

/** The stable safe error envelope every command rejects with (DG3 Error
 * Contract). `messageKey` is resolved through the zh-TW catalog and never
 * shown raw; `privateDetailRef` is an opaque local reference whose contents
 * never reach the surface. */
export interface SafeErrorDto {
  readonly errorCode: string;
  readonly messageKey: string;
  readonly messageParams: readonly MessageParamDto[];
  readonly correlationId: string;
  readonly retryable: boolean;
  readonly privateDetailRef?: string;
  readonly extensions: readonly SafeErrorExtensionDto[];
}

export interface MessageParamDto {
  readonly key: string;
  readonly value: SafeParamValueDto;
}

export type SafeParamValueDto =
  | { readonly type: "identifier"; readonly value: string }
  | { readonly type: "fieldKey"; readonly value: string }
  | { readonly type: "unsigned"; readonly value: number }
  | { readonly type: "boolean"; readonly value: boolean };

export type SafeErrorExtensionDto =
  | { readonly kind: "currentVersion"; readonly version: number }
  | {
      readonly kind: "fieldErrors";
      readonly errors: readonly { readonly fieldKey: string; readonly reasonKey: string }[];
    };

/**
 * What one Evidence write from the inspector left behind.
 *
 * `changed` is the honest half. A re-observation that finds the live state
 * already matching what was stored writes nothing, so no version moves and
 * no audit event exists; a surface that reported every call as a write would
 * be claiming a Ledger effect that never happened.
 */
export interface EvidenceWriteOutcomeDto {
  readonly changed: boolean;
  readonly evidence: EvidenceReferenceSummaryDto;
  readonly correlationId: string;
}

/**
 * Whether the Vault can serve the filesystem Evidence actions right now
 * (H0). `reason` is `notConfigured` (no Vault folder is set) or
 * `invalidRoot` (one is set but is missing, not a directory, or a link);
 * `null` when available. `configured` false means none is set — a folder on
 * an unplugged drive is configured and unavailable, not unset.
 * `folderName` is the chosen folder's own name, or `null` for Training, for
 * a drive root, and when none is set; no path ever crosses.
 */
export interface VaultStatusDto {
  readonly configured: boolean;
  readonly available: boolean;
  readonly reason: "notConfigured" | "invalidRoot" | "changeUnresolved" | null;
  readonly folderName: string | null;
  readonly correlationId: string;
}

/** What linking an Evidence reference to a Product left behind. */
export interface EvidenceLinkOutcomeDto {
  readonly evidenceId: string;
  readonly productId: string;
  readonly classificationAtLink: string;
  readonly correlationId: string;
}

export interface EvidenceEntryDto {
  readonly id: string;
  readonly role: string | null;
  readonly verification: string;
  readonly verifiedAtMillis: number | null;
  /** Whether a fingerprint is pinned. A separate fact from `verification`. */
  readonly pinned: boolean;
  readonly classification: string;
  readonly classificationAtLink: string;
  readonly linkedAtMillis: number;
  readonly revision: number;
}

export interface CarriedWorkDto {
  readonly kind: string;
  readonly id: string;
  readonly label: string;
  readonly stateLabel: string;
  readonly classification: string;
  readonly revision: number;
  readonly asOfMillis: number;
  /** What the lifecycle admits -- not what may be executed now. */
  readonly lifecycleLegalIntents: readonly string[];
  readonly attention: readonly WorkQueueAttentionDto[];
}

export interface AccountablePersonDto {
  readonly id: string;
  readonly displayName: string;
  readonly purpose: string;
  readonly classification: string;
  readonly revision: number;
  readonly otherProductsAccountableFor: number;
  /** Carried by the person. Never presented as the Product's own work. */
  readonly carried: readonly CarriedWorkDto[];
}

export interface HealthReasonDto {
  readonly text: string;
  /** Verification kind or attention reason behind `text`; `null` when the
   * host could not read it back, in which case `text` is shown as is. */
  readonly reasonCode: string | null;
  /** `evidence` or a Work Queue kind. */
  readonly subjectKind: string | null;
  readonly subjectLabel: string | null;
  readonly owner: string;
  readonly sourceRecordId: string;
  readonly sourceField: string;
  readonly sourceRevision: number;
  readonly asOfMillis: number;
}

export interface ProductDetailDto {
  readonly state: string;
  readonly asOfMillis: number;
  readonly ledgerRevision: number;
  /** `null` only when `state` is `outOfSync`. */
  readonly product: ProductSummaryDto | null;
  readonly classificationForcedBy: ClassificationFoldDto | null;
  readonly structure: readonly StructureEntryDto[];
  readonly evidence: readonly EvidenceEntryDto[];
  readonly people: readonly AccountablePersonDto[];
  /** 發生: current conditions, each attributed to what raised it. */
  readonly healthReasons: readonly HealthReasonDto[];
  /** 影響: "unassessed" until a person records a judgment. */
  readonly impact: string;
}

// ---------------------------------------------------------------------------
// S03 write path: the typed prepared contract and the
// authoritative outcomes. Mirrors `apps/desktop/src-tauri/src/write_commands.rs`.
// ---------------------------------------------------------------------------

export interface PreparedTargetDto {
  readonly kind: string;
  readonly id: string;
  readonly expectedVersion: number;
}

export interface PreparedEffectDto {
  readonly kind: string;
  readonly ids: readonly string[];
}

export interface ClassificationSourceDto {
  readonly role: string;
  readonly id: string | null;
  readonly classification: string;
}

export interface EvidenceVerificationDto {
  readonly kind: string;
  readonly atMillis: number | null;
  readonly integrityDigest: string | null;
}

export interface SupportEvidenceDto {
  readonly id: string;
  /** The Evidence reference's aggregate version bound at prepare time. */
  readonly sourceVersion: number;
  readonly classification: string;
  readonly role: string;
  readonly verification: EvidenceVerificationDto;
}

export interface SupportJudgmentDto {
  readonly disposition: string;
  readonly rationale: string;
  readonly classification: string;
}

export interface SupportWitnessDto {
  readonly disposition: string;
  readonly classification: string;
  readonly evidence: readonly SupportEvidenceDto[];
  readonly judgments: readonly SupportJudgmentDto[];
}

export interface AcceptActionRequestOperationDto {
  readonly kind: "acceptActionRequest";
  readonly requestId: string;
  readonly requestVersion: number;
  readonly actionId: string;
  readonly actionClassification: string;
  readonly actionSubject: string;
  readonly commitmentDetails: string;
  readonly intendedOwner: string;
  readonly intendedDueAtMillis: number;
}

export interface CompleteActionOperationDto {
  readonly kind: "completeAction";
  readonly actionId: string;
  readonly actionVersion: number;
}

/** One Evidence reference's classification as the preview bound it. */
export interface EvidenceClassificationDto {
  readonly evidenceId: string;
  readonly classification: string;
}

export interface CancelActionOperationDto {
  readonly kind: "cancelAction";
  readonly actionId: string;
  readonly actionVersion: number;
  readonly reason: string;
  readonly evidenceClassifications: readonly EvidenceClassificationDto[];
}

export interface ReopenActionOperationDto {
  readonly kind: "reopenAction";
  readonly actionId: string;
  readonly actionVersion: number;
  readonly mode: string;
  readonly reason: string;
  readonly evidenceClassifications: readonly EvidenceClassificationDto[];
}

/** One resulting Action Request as the Resolve preview declares it. */
export interface ResultingActionRequestDto {
  readonly id: string;
  readonly subject: string;
  readonly details: string;
  readonly intendedOwner: string;
  readonly dueAtMillis: number;
  readonly classification: string;
}

export interface ResolveDecisionRequestOperationDto {
  readonly kind: "resolveDecisionRequest";
  readonly requestId: string;
  readonly requestVersion: number;
  readonly decisionId: string;
  readonly decisionClassification: string;
  readonly statement: string;
  readonly rationale: string;
  readonly impact: string;
  readonly decisionOwner: string;
  readonly decidedAtMillis: number;
  readonly resultingActionRequests: readonly ResultingActionRequestDto[];
}

/** The exact operation payload, discriminated by `kind`: the five H2a
 * kinds S03 wires. */
export interface RecordRiskOccurrenceOperationDto {
  readonly kind: "recordRiskOccurrence";
  readonly riskId: string;
  readonly riskVersion: number;
  /** The Issue this occurrence will create: host-minted, and bound by the
   * digest the person acknowledges. */
  readonly issueId: string;
  readonly issueClassification: string;
}

export interface CloseRiskOperationDto {
  readonly kind: "closeRisk";
  readonly riskId: string;
  readonly riskVersion: number;
  readonly rationale: string;
}

export interface ResolveIssueOperationDto {
  readonly kind: "resolveIssue";
  readonly issueId: string;
  readonly issueVersion: number;
  readonly resolutionType: string;
  readonly rationale: string;
}

export interface CloseIssueOperationDto {
  readonly kind: "closeIssue";
  readonly issueId: string;
  readonly issueVersion: number;
}

export interface ReopenIssueOperationDto {
  readonly kind: "reopenIssue";
  readonly issueId: string;
  readonly issueVersion: number;
  readonly rationale: string;
}

export type PreparedOperationDto =
  | AcceptActionRequestOperationDto
  | CompleteActionOperationDto
  | CancelActionOperationDto
  | ReopenActionOperationDto
  | ResolveDecisionRequestOperationDto
  | RecordRiskOccurrenceOperationDto
  | CloseRiskOperationDto
  | ResolveIssueOperationDto
  | CloseIssueOperationDto
  | ReopenIssueOperationDto;

/** What the webview supplies for one resulting Action Request; the host
 * mints the id. */
export interface ResultingActionRequestInput {
  readonly subject: string;
  readonly details: string;
  readonly intendedOwner: string;
  readonly dueAtMillis: number;
  readonly classification: string;
}

export interface DecisionRequestSummaryDto {
  readonly id: string;
  readonly subject: string;
  readonly details: string;
  readonly intendedOwner: string | null;
  readonly classification: string;
  readonly state: string;
  readonly version: number;
  readonly withdrawalRationale: string | null;
  readonly linkedDecisionId: string | null;
}

export interface DecisionSummaryDto {
  readonly id: string;
  readonly sourceRequestId: string | null;
  readonly statement: string;
  readonly rationale: string;
  readonly impact: string;
  readonly owner: string;
  readonly decidedAtMillis: number;
  readonly classification: string;
  readonly state: string;
  readonly version: number;
  readonly resultingActionRequestIds: readonly string[];
}

export interface DecisionRequestOutcomeDto {
  readonly request: DecisionRequestSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly correlationId: string;
}

export interface ResolvedDecisionDto {
  readonly request: DecisionRequestSummaryDto;
  readonly decision: DecisionSummaryDto;
  readonly resultingActionRequestIds: readonly string[];
  readonly auditEventIds: readonly string[];
  readonly approvalReceiptId: string;
  readonly correlationId: string;
}

export interface EvidenceReferencesDto {
  readonly ledgerRevision: number;
  readonly evidenceReferences: readonly EvidenceReferenceSummaryDto[];
  readonly correlationId: string;
}

/** One Evidence reference as the Ledger holds it now; never a Vault path. */
export interface EvidenceReferenceSummaryDto {
  readonly id: string;
  readonly role: string | null;
  readonly verification: EvidenceVerificationDto;
  readonly pinned: boolean;
  readonly classification: string;
  readonly version: number;
}

export interface ActionCompletionContextDto {
  readonly action: ActionSummaryDto;
  readonly linkedEvidenceIds: readonly string[];
  readonly ledgerRevision: number;
  readonly evidenceReferences: readonly EvidenceReferenceSummaryDto[];
  readonly correlationId: string;
}

export interface PreparedIntentDto {
  readonly preparedIntentId: string;
  readonly contractVersion: number;
  readonly intentType: string;
  readonly operation: PreparedOperationDto;
  readonly targets: readonly PreparedTargetDto[];
  readonly declaredEffects: readonly PreparedEffectDto[];
  /** Full, never truncated: shown whole and sent back whole on Approve. */
  readonly payloadDigest: string;
  readonly resolvedClassification: string;
  readonly classificationSources: readonly ClassificationSourceDto[];
  readonly support: SupportWitnessDto | null;
  readonly policyResult: string;
  readonly authority: string;
  readonly preparedAtMillis: number;
  readonly expiresAtMillis: number;
  readonly cancellationPolicy: string;
  readonly correlationId: string;
}

export interface ActionRequestSummaryDto {
  readonly id: string;
  readonly title: string;
  readonly details: string;
  readonly intendedOwner: string | null;
  readonly responseDueAtMillis: number | null;
  readonly intendedActionDueAtMillis: number | null;
  readonly classification: string;
  readonly state: string;
  readonly version: number;
  readonly terminalRationale: string | null;
  readonly linkedActionId: string | null;
}

export interface ActionSummaryDto {
  readonly id: string;
  readonly sourceRequestId: string;
  readonly title: string;
  readonly details: string;
  readonly owner: string;
  readonly dueAtMillis: number;
  readonly classification: string;
  readonly state: string;
  readonly version: number;
}

export interface AcceptedActionDto {
  readonly request: ActionRequestSummaryDto;
  readonly action: ActionSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly approvalReceiptId: string;
  readonly correlationId: string;
}

export interface ActionRequestOutcomeDto {
  readonly request: ActionRequestSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly correlationId: string;
}

export interface ActionOutcomeDto {
  readonly action: ActionSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly correlationId: string;
}

export interface RejectedPreparedIntentDto {
  readonly preparedIntentId: string;
  readonly disposition: "rejected";
  readonly rejectedAtMillis: number;
  readonly expiredAtRejection: boolean;
  readonly correlationId: string;
}

/** A Risk as the Ledger holds it after a transition. */
export interface RiskSummaryDto {
  readonly id: string;
  readonly title: string;
  readonly details: string;
  readonly classification: string;
  readonly state: string;
  readonly version: number;
}

/** An Issue as the Ledger holds it after a transition. */
export interface IssueSummaryDto {
  readonly id: string;
  readonly title: string;
  readonly details: string;
  readonly classification: string;
  readonly state: string;
  readonly version: number;
  readonly sourceRiskId: string | null;
  readonly resolutionType: string | null;
}

/** Recording an occurrence transitions the Risk and creates the Issue the
 * preview named. */
export interface OccurredRiskDto {
  readonly risk: RiskSummaryDto;
  readonly issue: IssueSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly approvalReceiptId: string;
  readonly correlationId: string;
}

export interface RiskOutcomeDto {
  readonly risk: RiskSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly approvalReceiptId: string | null;
  readonly correlationId: string;
}

export interface IssueOutcomeDto {
  readonly issue: IssueSummaryDto;
  readonly auditEventIds: readonly string[];
  readonly approvalReceiptId: string | null;
  readonly correlationId: string;
}
