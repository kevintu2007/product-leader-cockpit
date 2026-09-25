import {
  type Dispatch,
  type ReactNode,
  type SetStateAction,
  type SyntheticEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import {
  newClientRequestId as newEntryRequestId,
  type EntryActions,
  type EntryOutcomeDto,
} from "../entry/entryIpc";
import {
  actionRequestFieldSpecs,
  actionRequestValues,
  decisionRequestFieldSpecs,
  decisionRequestValues,
  issueFieldSpecs,
  issueValues,
  kindWord,
  riskFieldSpecs,
  riskResponseFieldSpecs,
  riskResponseProblem,
  riskResponseValues,
  riskValues,
  type OwnerCandidate,
} from "../entry/entrySheets";
import { RecordSheet } from "../entry/RecordSheet";
import type { MessageKey, Translator } from "../i18n/messages";
import { useT } from "../i18n/useT";
import { formatLocalDate, formatReadAt } from "../i18n/time";
import {
  classificationName,
  freshnessLabel,
  intentLabel,
  ownerLabel,
  persistedStateLabel,
  placementLabel,
  reasonLabel,
  stateLabel,
  verificationLabel,
  workItemKindLabel,
} from "../i18n/workLabels";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { H2aFocusedReview, type H2aDecisionOutcome } from "../overlays/H2aFocusedReview";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type {
  AcceptedActionDto,
  ActionCompletionContextDto,
  ActionOutcomeDto,
  ActionRequestOutcomeDto,
  DecisionRequestOutcomeDto,
  EvidenceReferenceSummaryDto,
  EvidenceReferencesDto,
  IssueOutcomeDto,
  OccurredRiskDto,
  PreparedIntentDto,
  RejectedPreparedIntentDto,
  ResolvedDecisionDto,
  RiskOutcomeDto,
  ResultingActionRequestInput,
  WorkItemKind,
  WorkQueueDto,
  WorkQueueItemDto,
} from "./cockpitContract";
import type { ResolveDecisionRequestInput } from "./ipcCockpit";
import { findRoute } from "../shell/routes";

/**
 * The write path S03 may reach. Each method is
 * one host command; the route supplies only what it read (id + revision),
 * the person's own words, and one opaque client request id per submitted
 * command, reused verbatim on a retry of that same command.
 */
export interface WorkQueueActions {
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
  // Risk and Issue. The occurrence preview names the Issue the
  // host will create; this route never supplies that identity.
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

export interface WorkQueueProps {
  readonly load: (
    offset: number,
    limit: number,
    kinds: readonly WorkItemKind[],
    onlyFlagged: boolean,
  ) => Promise<WorkQueueDto>;
  /** Absent in a read-only host: the route then offers no actions at all. */
  readonly actions?: WorkQueueActions;
  /** The record-entry sheets (DG3 record-entry amendment, slice 6E): a new
   * Action Request or Decision Request draft and its submit, a new Risk or
   * Issue, and a Risk's response. Absent means none are offered. */
  readonly entryActions?: EntryActions;
  /** The workspace's configured zone dates are entered in (§3.5). */
  readonly timeZone?: string;
  /** Injectable request-id minting; production mints a random opaque id. */
  readonly newClientRequestId?: () => string;
  /** Injectable clock for O03's countdown. */
  readonly now?: () => number;
  /** Reviews closed without deciding, kept by a caller that outlives this
   * route: their prepared intents stay pending on the host after the route
   * unmounts, so forgetting them would let a row prepare a second approvable
   * preview on return. Absent, the route keeps them only while mounted. */
  readonly heldReviews?: HeldReviews;
  readonly onHeldReviewsChange?: Dispatch<SetStateAction<HeldReviews>>;
  /** Called after every successful read, so a caller showing a count from
   * the same Ledger (the rail badge) can read it again rather than go stale. */
  readonly onRead?: () => void;
}

/** An O03 review closed without deciding, by row. */
export type HeldReviews = ReadonlyMap<string, HeldReview>;
export type HeldReview = Review;

const PAGE_SIZE = 25;

const CLASSIFICATIONS = ["public", "internal", "confidential", "restricted"] as const;

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly queue: WorkQueueDto };

/** One per-row command: an H1 write, or the PREPARE step of an H2a loop. */
type RowCommand =
  | "decline"
  | "withdraw"
  | "start"
  | "prepare"
  | "link"
  | "prepareComplete"
  | "prepareCancel"
  | "prepareReopen"
  | "withdrawDecision"
  | "prepareResolve"
  | "prepareOccurrence"
  | "prepareCloseRisk"
  | "prepareResolveIssue"
  | "prepareCloseIssue"
  | "prepareReopenIssue";

interface PendingRow {
  readonly key: string;
  readonly command: RowCommand;
  readonly clientRequestId: string;
  readonly phase: "composing" | "sending" | "failed";
  /** The person's own words: a decline/withdraw/cancel/reopen reason, or a
   * completion Judgment rationale (empty = no Judgment). */
  readonly rationale: string;
  readonly judgmentClassification: string;
  readonly reopenMode: string;
  readonly evidenceId: string;
  /** For `link` and `prepareResolve`: the Evidence references, once loaded. */
  readonly evidence: readonly EvidenceReferenceSummaryDto[] | null;
  readonly linkedEvidenceIds: readonly string[];
  /** For `prepareResolveIssue`: how the Issue was put right. */
  readonly resolutionType: string;
  /** For `prepareResolve`: the person's resolution. */
  readonly statement: string;
  readonly impact: string;
  readonly judgmentRationale: string;
  readonly evidenceIds: readonly string[];
  readonly resulting: readonly ResultingActionRequestInput[];
  readonly error?: ResolvedSafeError;
}

/** An O03 session: the prepared contract and the two decision request ids. */
interface Review {
  readonly prepared: PreparedIntentDto;
  readonly item: WorkQueueItemDto;
  readonly approveRequestId: string;
  readonly rejectRequestId: string;
}

const OFFERS: readonly {
  readonly kind: WorkItemKind;
  readonly intent: string;
  readonly command: RowCommand;
  readonly label: MessageKey & `wq.offer.${string}`;
}[] = [
  {
    kind: "action_request",
    intent: "prepare_accept_action_request",
    command: "prepare",
    label: "wq.offer.prepareAccept",
  },
  {
    kind: "action_request",
    intent: "decline_action_request",
    command: "decline",
    label: "wq.offer.decline",
  },
  {
    kind: "action_request",
    intent: "withdraw_action_request",
    command: "withdraw",
    label: "wq.offer.withdraw",
  },
  { kind: "action", intent: "start_action", command: "start", label: "wq.offer.start" },
  {
    kind: "action",
    intent: "link_action_completion_evidence",
    command: "link",
    label: "wq.offer.link",
  },
  {
    kind: "action",
    intent: "prepare_complete_action",
    command: "prepareComplete",
    label: "wq.offer.prepareComplete",
  },
  {
    kind: "action",
    intent: "prepare_cancel_action",
    command: "prepareCancel",
    label: "wq.offer.prepareCancel",
  },
  {
    kind: "action",
    intent: "prepare_reopen_action",
    command: "prepareReopen",
    label: "wq.offer.prepareReopen",
  },
  {
    kind: "decision_request",
    intent: "withdraw_decision_request",
    command: "withdrawDecision",
    label: "wq.offer.withdraw",
  },
  {
    kind: "decision_request",
    intent: "prepare_resolve_decision_request",
    command: "prepareResolve",
    label: "wq.offer.prepareResolve",
  },
  {
    kind: "risk",
    intent: "prepare_record_risk_occurrence",
    command: "prepareOccurrence",
    label: "wq.offer.prepareOccurrence",
  },
  {
    kind: "risk",
    intent: "prepare_close_risk",
    command: "prepareCloseRisk",
    label: "wq.offer.prepareClose",
  },
  {
    kind: "issue",
    intent: "prepare_resolve_issue",
    command: "prepareResolveIssue",
    label: "wq.offer.prepareResolve",
  },
  {
    kind: "issue",
    intent: "prepare_close_issue",
    command: "prepareCloseIssue",
    label: "wq.offer.prepareClose",
  },
  {
    kind: "issue",
    intent: "prepare_reopen_issue",
    command: "prepareReopenIssue",
    label: "wq.offer.prepareReopen",
  },
];

const EMPTY_RESULTING: ResultingActionRequestInput = {
  subject: "",
  details: "",
  intendedOwner: "",
  dueAtMillis: 0,
  classification: "internal",
};

function defaultClientRequestId(): string {
  const random =
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `wq-${random}`;
}

/**
 * The deadline column. A request that carries a promised completion date but
 * no response deadline says exactly that, rather than "no deadline", because
 * the promised date is shown right under it.
 */
function formatDeadline(
  millis: number | null,
  t: Translator,
  promisedAtMillis?: number | null,
): string {
  if (millis === null) {
    return promisedAtMillis != null ? t("wq.noResponseDeadline") : t("wq.noDeadline");
  }
  return formatLocalDate(millis);
}

function rowKey(item: WorkQueueItemDto): string {
  return `${item.kind}:${item.id}`;
}

function blankPending(
  item: WorkQueueItemDto,
  command: RowCommand,
  clientRequestId: string,
): PendingRow {
  return {
    key: rowKey(item),
    command,
    clientRequestId,
    phase: "composing",
    rationale: "",
    judgmentClassification: "internal",
    reopenMode: item.stateLabel === "Cancelled" ? "restart_cancelled" : "reopen_completed",
    evidenceId: "",
    evidence: null,
    linkedEvidenceIds: [],
    resolutionType: "resolved",
    statement: "",
    impact: "",
    judgmentRationale: "",
    evidenceIds: [],
    resulting: [],
  };
}

/** What O03 calls the operation, per kind. */
function reviewCopy(
  review: Review,
  t: Translator,
): {
  readonly title: string;
  readonly summary: string;
  readonly approveLabel: string;
} {
  const { operation } = review.prepared;
  const label = review.item.label;
  const words = (title: string, summary: string, approveLabel: string) => ({
    title,
    summary,
    approveLabel,
  });
  switch (operation.kind) {
    case "acceptActionRequest":
      return words(
        t("wq.review.accept.title", { label }),
        t("wq.review.accept.summary"),
        t("wq.review.accept.approve"),
      );
    case "completeAction":
      return words(
        t("wq.review.complete.title", { label }),
        t("wq.review.complete.summary"),
        t("wq.review.complete.approve"),
      );
    case "cancelAction":
      return words(
        t("wq.review.cancel.title", { label }),
        t("wq.review.cancel.summary"),
        t("wq.review.cancel.approve"),
      );
    case "reopenAction":
      return words(
        t("wq.review.reopen.title", { label }),
        t("wq.review.reopen.summary"),
        t("wq.review.reopen.approve"),
      );
    case "resolveDecisionRequest":
      return words(
        t("wq.review.resolveDecision.title", { label }),
        t("wq.review.resolveDecision.summary"),
        t("wq.review.resolveDecision.approve"),
      );
    case "recordRiskOccurrence":
      return words(
        t("wq.review.occurrence.title", { label }),
        t("wq.review.occurrence.summary", { issue: operation.issueId }),
        t("wq.review.occurrence.approve"),
      );
    case "closeRisk":
      return words(
        t("wq.review.closeRisk.title", { label }),
        t("wq.review.closeRisk.summary"),
        t("wq.review.closeRisk.approve"),
      );
    case "resolveIssue":
      return words(
        t("wq.review.resolveIssue.title", { label }),
        t("wq.review.resolveIssue.summary"),
        t("wq.review.resolveIssue.approve"),
      );
    case "closeIssue":
      return words(
        t("wq.review.closeIssue.title", { label }),
        t("wq.review.closeIssue.summary"),
        t("wq.review.closeIssue.approve"),
      );
    case "reopenIssue":
      return words(
        t("wq.review.reopenIssue.title", { label }),
        t("wq.review.reopenIssue.summary"),
        t("wq.review.reopenIssue.approve"),
      );
  }
}

/** The one entry sheet open at a time (slice 6E), with its request id. */
type WorkSheet =
  | {
      readonly kind: "actionRequest";
      readonly owners: readonly OwnerCandidate[];
      readonly clientRequestId: string;
    }
  | {
      readonly kind: "decisionRequest";
      readonly owners: readonly OwnerCandidate[];
      readonly clientRequestId: string;
    }
  | { readonly kind: "risk"; readonly clientRequestId: string }
  | { readonly kind: "issue"; readonly clientRequestId: string }
  | {
      readonly kind: "riskResponse";
      readonly item: WorkQueueItemDto;
      readonly owners: readonly OwnerCandidate[];
      readonly clientRequestId: string;
    };

function browserTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

export function WorkQueue({
  load,
  actions,
  entryActions,
  timeZone = browserTimeZone(),
  newClientRequestId = defaultClientRequestId,
  now = Date.now,
  heldReviews,
  onHeldReviewsChange,
  onRead,
}: WorkQueueProps) {
  const t = useT();
  const [workSheet, setWorkSheet] = useState<WorkSheet | null>(null);
  const [entryOpening, setEntryOpening] = useState<ResolvedSafeError | null>(null);
  const [ownersLoading, setOwnersLoading] = useState(false);
  // A submit in flight, by row key; one at a time, like the row commands.
  const [submitting, setSubmitting] = useState<{
    readonly key: string;
    readonly clientRequestId: string;
    readonly error: ResolvedSafeError | null;
  } | null>(null);
  const [offset, setOffset] = useState(0);
  const [kinds, setKinds] = useState<readonly WorkItemKind[]>([]);
  const [onlyFlagged, setOnlyFlagged] = useState(false);
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [pending, setPending] = useState<PendingRow | null>(null);
  const [review, setReview] = useState<Review | null>(null);
  // Reviews closed without deciding, by row. The prepared intent is still
  // pending on the host, so the row must lead back to that same preview
  // rather than prepare a second approvable one beside it.
  const [ownHeld, setOwnHeld] = useState<HeldReviews>(() => new Map());
  const held = heldReviews ?? ownHeld;
  const setHeld = onHeldReviewsChange ?? setOwnHeld;
  // The row whose detail sheet is open, by row key.
  const [inspecting, setInspecting] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  // Every load is numbered; a response that is no longer the latest is
  // dropped, so a slow pre-write read cannot overwrite a post-write one.
  const latestLoad = useRef(0);

  const fetchPage = useCallback((): Promise<WorkQueueDto | null> => {
    latestLoad.current += 1;
    const sequence = latestLoad.current;
    return load(offset, PAGE_SIZE, kinds, onlyFlagged).then(
      (queue) => {
        if (sequence !== latestLoad.current) {
          return null;
        }
        setState({ status: "ready", queue });
        onRead?.();
        return queue;
      },
      (reason: unknown) => {
        if (sequence === latestLoad.current) {
          setState({ status: "error", error: resolveRejection(reason, t) });
        }
        return null;
      },
    );
  }, [load, offset, kinds, onlyFlagged, onRead, t]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    void fetchPage();
  }, [fetchPage]);

  useEffect(() => {
    void fetchPage();
  }, [fetchPage]);

  // -------------------------------------------------------------- actions

  function begin(item: WorkQueueItemDto, command: RowCommand) {
    if (!actions) {
      return;
    }
    setNotice(null);
    // One opaque id per submitted command, minted here and reused verbatim
    // by every retry of this same command.
    const clientRequestId = newClientRequestId();
    if (command === "start" || command === "prepare" || command === "prepareOccurrence") {
      send(item, command, blankPending(item, command, clientRequestId));
      return;
    }
    const composing = blankPending(item, command, clientRequestId);
    setPending(composing);
    if (
      command === "prepareResolve" ||
      command === "prepareResolveIssue" ||
      command === "prepareCloseIssue" ||
      command === "prepareReopenIssue"
    ) {
      actions.loadEvidenceReferences().then(
        (references) => {
          setPending((current) =>
            current?.key === composing.key && current.command === command
              ? { ...current, evidence: references.evidenceReferences }
              : current,
          );
        },
        (reason: unknown) => {
          setPending({ ...composing, phase: "failed", error: resolveRejection(reason, t) });
        },
      );
      return;
    }
    if (command === "link") {
      actions.loadActionCompletionContext(item.id).then(
        (context) => {
          setPending((current) =>
            current?.key === composing.key && current.command === "link"
              ? {
                  ...current,
                  evidence: context.evidenceReferences,
                  linkedEvidenceIds: context.linkedEvidenceIds,
                }
              : current,
          );
        },
        (reason: unknown) => {
          setPending({ ...composing, phase: "failed", error: resolveRejection(reason, t) });
        },
      );
    }
  }

  function send(item: WorkQueueItemDto, command: RowCommand, form: PendingRow) {
    if (!actions) {
      return;
    }
    const sending: PendingRow = { ...form, phase: "sending" };
    setPending(sending);
    const fail = (reason: unknown) => {
      setPending({ ...sending, phase: "failed", error: resolveRejection(reason, t) });
    };
    const done = (message: string) => {
      setPending(null);
      setInspecting(null);
      setNotice(message);
      void fetchPage();
    };
    const open = (prepared: PreparedIntentDto) => {
      setPending(null);
      setInspecting(null);
      setReview({
        prepared,
        item,
        approveRequestId: newClientRequestId(),
        rejectRequestId: newClientRequestId(),
      });
    };
    const { clientRequestId, rationale } = form;
    switch (command) {
      case "prepare":
        actions
          .prepareAcceptActionRequest(item.id, item.revision, clientRequestId)
          .then(open, fail);
        return;
      case "decline":
        actions
          .declineActionRequest(item.id, item.revision, rationale, clientRequestId)
          .then((outcome) => {
            done(
              t("wq.done.declined", {
                id: outcome.request.id,
                version: String(outcome.request.version),
              }),
            );
          }, fail);
        return;
      case "withdraw":
        actions
          .withdrawActionRequest(item.id, item.revision, rationale, clientRequestId)
          .then((outcome) => {
            done(
              t("wq.done.withdrawn", {
                id: outcome.request.id,
                version: String(outcome.request.version),
              }),
            );
          }, fail);
        return;
      case "start":
        actions.startAction(item.id, item.revision, clientRequestId).then((outcome) => {
          done(
            t("wq.done.started", {
              id: outcome.action.id,
              version: String(outcome.action.version),
            }),
          );
        }, fail);
        return;
      case "link":
        actions
          .linkActionCompletionEvidence(item.id, item.revision, form.evidenceId, clientRequestId)
          .then((outcome) => {
            done(
              t("wq.done.linked", {
                evidence: form.evidenceId,
                id: outcome.action.id,
                version: String(outcome.action.version),
              }),
            );
          }, fail);
        return;
      case "prepareComplete": {
        const judgment = rationale.trim().length === 0 ? null : rationale;
        actions
          .prepareCompleteAction(
            item.id,
            item.revision,
            judgment,
            judgment === null ? null : form.judgmentClassification,
            clientRequestId,
          )
          .then(open, fail);
        return;
      }
      case "prepareCancel":
        actions
          .prepareCancelAction(item.id, item.revision, rationale, clientRequestId)
          .then(open, fail);
        return;
      case "prepareReopen":
        actions
          .prepareReopenAction(item.id, item.revision, form.reopenMode, rationale, clientRequestId)
          .then(open, fail);
        return;
      case "prepareOccurrence":
        actions
          .prepareRecordRiskOccurrence(item.id, item.revision, clientRequestId)
          .then(open, fail);
        return;
      case "prepareCloseRisk":
        actions
          .prepareCloseRisk(item.id, item.revision, rationale, clientRequestId)
          .then(open, fail);
        return;
      case "prepareResolveIssue":
      case "prepareCloseIssue":
      case "prepareReopenIssue": {
        // The Judgment is separate from the transition rationale: it says
        // why partly verified Evidence is enough, not why the state changes.
        const judgment = form.judgmentRationale.trim().length === 0 ? null : form.judgmentRationale;
        const judgmentClassification = judgment === null ? null : form.judgmentClassification;
        const prepared =
          command === "prepareResolveIssue"
            ? actions.prepareResolveIssue(
                item.id,
                item.revision,
                form.resolutionType,
                rationale,
                form.evidenceIds,
                judgment,
                judgmentClassification,
                clientRequestId,
              )
            : command === "prepareCloseIssue"
              ? actions.prepareCloseIssue(
                  item.id,
                  item.revision,
                  form.evidenceIds,
                  judgment,
                  judgmentClassification,
                  clientRequestId,
                )
              : actions.prepareReopenIssue(
                  item.id,
                  item.revision,
                  rationale,
                  form.evidenceIds,
                  judgment,
                  judgmentClassification,
                  clientRequestId,
                );
        prepared.then(open, fail);
        return;
      }
      case "withdrawDecision":
        actions
          .withdrawDecisionRequest(item.id, item.revision, rationale, clientRequestId)
          .then((outcome) => {
            done(
              t("wq.done.withdrawn", {
                id: outcome.request.id,
                version: String(outcome.request.version),
              }),
            );
          }, fail);
        return;
      case "prepareResolve": {
        const judgment = form.judgmentRationale.trim().length === 0 ? null : form.judgmentRationale;
        actions
          .prepareResolveDecisionRequest(
            item.id,
            item.revision,
            {
              statement: form.statement,
              rationale,
              impact: form.impact,
              evidenceIds: form.evidenceIds,
              judgmentRationale: judgment,
              judgmentClassification: judgment === null ? null : form.judgmentClassification,
              resultingActionRequests: form.resulting,
            },
            clientRequestId,
          )
          .then(open, fail);
      }
    }
  }

  function reviewApprove(current: Review): Promise<H2aDecisionOutcome> {
    if (!actions) {
      return Promise.resolve({
        kind: "failed",
        error: { message: t("wq.noWritePath"), correlationId: "", retryable: false },
      });
    }
    const { preparedIntentId, payloadDigest, operation } = current.prepared;
    const id = current.approveRequestId;
    const failed = (reason: unknown): H2aDecisionOutcome => ({
      kind: "failed",
      error: resolveRejection(reason, t),
    });
    const settled =
      (key: "wq.settled.completed" | "wq.settled.cancelled" | "wq.settled.reopened") =>
      (outcome: ActionOutcomeDto): H2aDecisionOutcome => ({
        kind: "approved",
        summary: t(key, {
          id: outcome.action.id,
          version: String(outcome.action.version),
          state: persistedStateLabel(t, outcome.action.state),
        }),
      });
    switch (operation.kind) {
      case "acceptActionRequest":
        return actions
          .approveAndExecuteAcceptActionRequest(preparedIntentId, payloadDigest, id)
          .then(
            (accepted): H2aDecisionOutcome => ({
              kind: "approved",
              summary: t("wq.settled.accepted", {
                action: accepted.action.id,
                request: accepted.request.id,
                receipt: accepted.approvalReceiptId,
              }),
            }),
            failed,
          );
      case "completeAction":
        return actions
          .approveAndExecuteCompleteAction(preparedIntentId, payloadDigest, id)
          .then(settled("wq.settled.completed"), failed);
      case "cancelAction":
        return actions
          .approveAndExecuteCancelAction(preparedIntentId, payloadDigest, id)
          .then(settled("wq.settled.cancelled"), failed);
      case "reopenAction":
        return actions
          .approveAndExecuteReopenAction(preparedIntentId, payloadDigest, id)
          .then(settled("wq.settled.reopened"), failed);
      case "recordRiskOccurrence":
        return actions
          .approveAndExecuteRecordRiskOccurrence(preparedIntentId, payloadDigest, id)
          .then(
            (occurred): H2aDecisionOutcome => ({
              kind: "approved",
              summary: t("wq.settled.occurred", {
                issue: occurred.issue.id,
                version: String(occurred.risk.version),
              }),
            }),
            failed,
          );
      case "closeRisk":
        return actions.approveAndExecuteCloseRisk(preparedIntentId, payloadDigest, id).then(
          (outcome): H2aDecisionOutcome => ({
            kind: "approved",
            summary: t("wq.settled.riskClosed", {
              id: outcome.risk.id,
              version: String(outcome.risk.version),
            }),
          }),
          failed,
        );
      case "resolveIssue":
      case "closeIssue":
      case "reopenIssue":
        return actions.approveAndExecuteIssueTransition(preparedIntentId, payloadDigest, id).then(
          (outcome): H2aDecisionOutcome => ({
            kind: "approved",
            summary: t("wq.settled.issue", {
              id: outcome.issue.id,
              state: persistedStateLabel(t, outcome.issue.state),
              version: String(outcome.issue.version),
            }),
          }),
          failed,
        );
      case "resolveDecisionRequest":
        return actions
          .approveAndExecuteResolveDecisionRequest(preparedIntentId, payloadDigest, id)
          .then(
            (resolved): H2aDecisionOutcome => ({
              kind: "approved",
              summary: t.plural("wq.settled.decision", resolved.resultingActionRequestIds.length, {
                decision: resolved.decision.id,
                receipt: resolved.approvalReceiptId,
              }),
            }),
            failed,
          );
    }
  }

  function rejectDurably(current: Review): Promise<RejectedPreparedIntentDto> {
    if (!actions) {
      return Promise.reject(new Error("no write path"));
    }
    const { preparedIntentId, operation } = current.prepared;
    switch (operation.kind) {
      case "acceptActionRequest":
        return actions.rejectPreparedAcceptActionRequest(preparedIntentId, current.rejectRequestId);
      case "resolveDecisionRequest":
        return actions.rejectPreparedDecisionIntent(preparedIntentId, current.rejectRequestId);
      case "completeAction":
      case "cancelAction":
      case "reopenAction":
        return actions.rejectPreparedActionIntent(preparedIntentId, current.rejectRequestId);
      case "recordRiskOccurrence":
      case "closeRisk":
        return actions.rejectPreparedRiskIntent(preparedIntentId, current.rejectRequestId);
      case "resolveIssue":
      case "closeIssue":
      case "reopenIssue":
        return actions.rejectPreparedIssueIntent(preparedIntentId, current.rejectRequestId);
    }
  }

  function reviewReject(current: Review): Promise<H2aDecisionOutcome> {
    return rejectDurably(current).then(
      (rejected): H2aDecisionOutcome => ({
        kind: "rejected",
        expiredAtRejection: rejected.expiredAtRejection,
      }),
      (reason: unknown): H2aDecisionOutcome => ({
        kind: "failed",
        error: resolveRejection(reason, t),
      }),
    );
  }

  // "Prepare again" is not a local close: the pending preview is rejected
  // durably first, then the queue is re-read. Moving on even when that
  // rejection fails is safe: O03 offers this only for a preview that has
  // expired or that the host reported expired or changed, and execution
  // re-checks both expiry and the target's expected version, so the old
  // intent can no longer execute beside the new one. An Accept preview is
  // prepared afresh from the refreshed revision; the other kinds need the
  // person's inputs again, so their row is reopened for composing instead.
  function prepareAgain(current: Review) {
    if (!actions) {
      return;
    }
    setReview(null);
    const { kind } = current.prepared.operation;
    const key = rowKey(current.item);
    void rejectDurably(current)
      .then(
        () => undefined,
        () => undefined,
      )
      .then(fetchPage)
      .then((queue) => {
        if (queue === null) {
          return;
        }
        const item = queue.items.find((each) => rowKey(each) === key);
        if (!item) {
          setNotice(t("wq.notice.gone", { label: current.item.label }));
          return;
        }
        // An occurrence needs no words either, but it must not be
        // re-prepared silently: the first attempt named an Issue identity
        // the person read, and a fresh preview names a different one.
        if (kind === "acceptActionRequest") {
          if (item.lifecycleLegalIntents.includes("prepare_accept_action_request")) {
            send(item, "prepare", blankPending(item, "prepare", newClientRequestId()));
          } else {
            setNotice(t("wq.notice.notAcceptable", { id: item.id }));
          }
          return;
        }
        const command: RowCommand =
          kind === "completeAction"
            ? "prepareComplete"
            : kind === "cancelAction"
              ? "prepareCancel"
              : kind === "reopenAction"
                ? "prepareReopen"
                : kind === "recordRiskOccurrence"
                  ? "prepareOccurrence"
                  : kind === "closeRisk"
                    ? "prepareCloseRisk"
                    : kind === "resolveIssue"
                      ? "prepareResolveIssue"
                      : kind === "closeIssue"
                        ? "prepareCloseIssue"
                        : kind === "reopenIssue"
                          ? "prepareReopenIssue"
                          : "prepareResolve";
        setNotice(t("wq.notice.rejected"));
        setPending(blankPending(item, command, newClientRequestId()));
      });
  }

  // Reading an item is not acting on it: the detail sheet
  // opens with no host call, and its next steps are the same governed commands
  // the row offers. It shows only what the Work Queue read carries: `owner` is
  // the module that owns the record, never a person, and the host's English
  // rationale prose stays out in favour of the worded identifiers. Focus lands
  // on 關閉, so no key press on opening can start a command.
  function detailDialog(item: WorkQueueItemDto | undefined) {
    if (!item) {
      return null;
    }
    const close = () => {
      setInspecting(null);
    };
    return (
      // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- a pointer convenience only: Escape and 關閉 inside the dialog close it too.
      <div
        className="pmc-dialog-backdrop"
        onClick={(event) => {
          if (event.target === event.currentTarget) {
            close();
          }
        }}
      >
        <FocusTrapDialog
          titleId="pmc-work-item-detail-title"
          onEscape={close}
          className="pmc-h2a-review pmc-work-item-detail"
        >
          <div className="pmc-work-item-detail-head">
            <span className="pmc-kind" data-kind={item.kind}>
              {workItemKindLabel(t, item.kind)}
            </span>
            <button type="button" className="pmc-h2a-dismiss" onClick={close}>
              {t("wq.detail.close")}
            </button>
          </div>
          <h2 id="pmc-work-item-detail-title" className="pmc-section-title">
            {item.label}
          </h2>
          <p className="pmc-work-item-meta">
            <span>{stateLabel(t, item.stateLabel)}</span>
            <span className="pmc-classification-badge" data-classification={item.classification}>
              {classificationName(t, item.classification)}
            </span>
          </p>
          <dl className="pmc-h2a-fields">
            <div className="pmc-h2a-field">
              <dt>{t("wq.detail.attention")}</dt>
              <dd>
                {item.attention.length === 0 ? (
                  t("wq.detail.noAttention")
                ) : (
                  <ul className="pmc-work-queue-attention">
                    {item.attention.map((flag) => (
                      <li key={flag.reason} data-tier={flag.tier}>
                        {reasonLabel(t, flag.reason)}
                        {(flag.freshness !== "fresh" || flag.degraded) && (
                          <span className="pmc-work-queue-uncertainty">
                            {t(flag.degraded ? "wq.uncertaintyDegraded" : "wq.uncertainty", {
                              freshness: freshnessLabel(t, flag.freshness),
                            })}
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
              </dd>
            </div>
            <div className="pmc-h2a-field">
              <dt>{t("wq.detail.deadline")}</dt>
              <dd>{formatDeadline(item.relevantAtMillis, t, item.promisedAtMillis)}</dd>
            </div>
            {item.promisedAtMillis != null && (
              <div className="pmc-h2a-field">
                <dt>{t("wq.detail.promised")}</dt>
                <dd>{formatLocalDate(item.promisedAtMillis)}</dd>
              </div>
            )}
            <div className="pmc-h2a-field">
              <dt>{t("wq.detail.placement")}</dt>
              <dd>{placementLabel(t, item.placement)}</dd>
            </div>
            <div className="pmc-h2a-field">
              <dt>{t("wq.detail.allowed")}</dt>
              <dd>
                {item.lifecycleLegalIntents.length === 0
                  ? t("wq.none")
                  : item.lifecycleLegalIntents
                      .map((intent) => intentLabel(t, intent))
                      .join(t("common.idSeparator"))}
              </dd>
            </div>
            <div className="pmc-h2a-field">
              <dt>{t("wq.detail.owner")}</dt>
              <dd>{ownerLabel(t, item.owner)}</dd>
            </div>
            <div className="pmc-h2a-field">
              <dt>{t("wq.detail.version")}</dt>
              <dd>{String(item.revision)}</dd>
            </div>
            <div className="pmc-h2a-field">
              <dt>{t("route.readAt")}</dt>
              <dd>
                <time>{formatReadAt(item.asOfMillis)}</time>
              </dd>
            </div>
          </dl>
          {actions && (
            <div className="pmc-work-item-detail-next">
              <h3 className="pmc-work-item-detail-heading">{t("wq.detail.next")}</h3>
              {rowActions(item)}
            </div>
          )}
        </FocusTrapDialog>
      </div>
    );
  }

  /**
   * The entry offers on a row (slice 6E): a draft's submit, at the version
   * the row read, and a Risk's response. These are H1 writes with no
   * preview; one runs at a time, like the row commands.
   */
  function entryOffers(item: WorkQueueItemDto): ReactNode {
    if (entryActions === undefined) {
      return null;
    }
    const key = rowKey(item);
    const submit =
      item.kind === "action_request" && item.lifecycleLegalIntents.includes("submit_action_request")
        ? entryActions.submitActionRequest
        : item.kind === "decision_request" &&
            item.lifecycleLegalIntents.includes("submit_decision_request")
          ? entryActions.submitDecisionRequest
          : null;
    // A Risk still open (the same reading the close offer uses); the domain
    // refuses a response on a closed or occurred Risk either way.
    const respond =
      item.kind === "risk" && item.lifecycleLegalIntents.includes("prepare_close_risk");
    if (submit === null && !respond) {
      return null;
    }
    const mine = submitting?.key === key ? submitting : null;
    const busy =
      (submitting !== null && submitting.key !== key) || pending !== null || workSheet !== null;
    const send = (clientRequestId: string) => {
      if (submit === null) {
        return;
      }
      setSubmitting({ key, clientRequestId, error: null });
      submit(item.id, item.revision, clientRequestId).then(
        (outcome) => {
          setSubmitting(null);
          setNotice(
            t("entry.saved.submitted", { kind: kindWord(t, outcome.kind), id: outcome.id }),
          );
          void fetchPage();
        },
        (reason: unknown) => {
          setSubmitting({ key, clientRequestId, error: resolveRejection(reason, t) });
        },
      );
    };
    return (
      <>
        {submit !== null ? (
          <button
            type="button"
            className="pmc-button-primary"
            disabled={busy || (mine !== null && mine.error === null)}
            onClick={() => {
              // The same command, so the same request id on a retry.
              send(mine?.clientRequestId ?? newEntryRequestId());
            }}
          >
            {t("entry.submit")}
          </button>
        ) : null}
        {respond ? (
          <button
            type="button"
            disabled={busy || mine !== null}
            onClick={() => {
              openRiskResponse(item);
            }}
          >
            {t("entry.riskResponse")}
          </button>
        ) : null}
        {mine !== null && mine.error === null ? <span role="status">{t("wq.sending")}</span> : null}
        {mine?.error ? (
          <SafeErrorDetail
            message={t("wq.actionFailed", { message: mine.error.message })}
            correlationId={mine.error.correlationId}
            errorCode={mine.error.errorCode}
            retryable={mine.error.retryable}
            {...(mine.error.retryable
              ? {
                  onRetry: () => {
                    send(mine.clientRequestId);
                  },
                }
              : {})}
          />
        ) : null}
        {mine?.error ? (
          <button
            type="button"
            onClick={() => {
              setSubmitting(null);
            }}
          >
            {t("wq.abandon")}
          </button>
        ) : null}
      </>
    );
  }

  function rowActions(item: WorkQueueItemDto) {
    if (!actions) {
      return null;
    }
    const key = rowKey(item);
    const mine = pending?.key === key ? pending : null;
    const offered = OFFERS.filter(
      (offer) => offer.kind === item.kind && item.lifecycleLegalIntents.includes(offer.intent),
    );
    const entry = entryOffers(item);
    if (offered.length === 0) {
      return entry === null ? (
        <span>{t("wq.none")}</span>
      ) : (
        <div className="pmc-work-queue-actions">{entry}</div>
      );
    }
    return rowCommands(item, key, mine, offered, entry);
  }

  /**
   * The Stakeholders a request or response may name, read once when the
   * sheet opens (the domain refuses an unknown owner either way).
   */
  function loadOwners(): Promise<readonly OwnerCandidate[]> {
    if (entryActions === undefined) {
      return Promise.resolve([]);
    }
    return entryActions
      .listEntryRecords("stakeholder")
      .then((list) =>
        list.records.flatMap((record) =>
          record.kind === "stakeholder" ? [{ id: record.id, name: record.name }] : [],
        ),
      );
  }

  function openWithOwners(kind: "actionRequest" | "decisionRequest") {
    setEntryOpening(null);
    setOwnersLoading(true);
    loadOwners().then(
      (owners) => {
        setOwnersLoading(false);
        setWorkSheet({ kind, owners, clientRequestId: newEntryRequestId() });
      },
      (reason: unknown) => {
        setOwnersLoading(false);
        setEntryOpening(resolveRejection(reason, t));
      },
    );
  }

  function openRiskResponse(item: WorkQueueItemDto) {
    setEntryOpening(null);
    setOwnersLoading(true);
    loadOwners().then(
      (owners) => {
        setOwnersLoading(false);
        setWorkSheet({ kind: "riskResponse", item, owners, clientRequestId: newEntryRequestId() });
      },
      (reason: unknown) => {
        setOwnersLoading(false);
        setEntryOpening(resolveRejection(reason, t));
      },
    );
  }

  // One authoritative write at a time from this surface (§3.6): no sheet
  // opens while another is open or loading, while a row command is in
  // flight, or while a draft's submit is.
  const entryBusy = workSheet !== null || ownersLoading || pending !== null || submitting !== null;

  function entrySaved(
    outcome: EntryOutcomeDto,
    key: "entry.saved.created" | "entry.saved.responded",
  ) {
    setWorkSheet(null);
    setNotice(t(key, { kind: kindWord(t, outcome.kind), id: outcome.id }));
    void fetchPage();
  }

  function workSheetDialog(): ReactNode {
    if (entryActions === undefined || workSheet === null) {
      return null;
    }
    const actions = entryActions;
    const close = () => {
      setWorkSheet(null);
    };
    switch (workSheet.kind) {
      case "actionRequest":
        return (
          <RecordSheet
            title={t("entry.title.create.actionRequest")}
            fields={actionRequestFieldSpecs(t, workSheet.owners, timeZone)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              actions.createActionRequestDraft(
                actionRequestValues(values),
                workSheet.clientRequestId,
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "entry.saved.created");
            }}
            onClose={close}
          />
        );
      case "decisionRequest":
        return (
          <RecordSheet
            title={t("entry.title.create.decisionRequest")}
            fields={decisionRequestFieldSpecs(t, workSheet.owners)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              actions.createDecisionRequestDraft(
                decisionRequestValues(values),
                workSheet.clientRequestId,
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "entry.saved.created");
            }}
            onClose={close}
          />
        );
      case "risk":
        return (
          <RecordSheet
            title={t("entry.title.create.risk")}
            fields={riskFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) => actions.createRisk(riskValues(values), workSheet.clientRequestId)}
            onDone={(outcome) => {
              entrySaved(outcome, "entry.saved.created");
            }}
            onClose={close}
          />
        );
      case "issue":
        return (
          <RecordSheet
            title={t("entry.title.create.issue")}
            fields={issueFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) => actions.createIssue(issueValues(values), workSheet.clientRequestId)}
            onDone={(outcome) => {
              entrySaved(outcome, "entry.saved.created");
            }}
            onClose={close}
          />
        );
      case "riskResponse":
        return (
          <RecordSheet
            title={t("entry.title.riskResponse", { risk: workSheet.item.label })}
            fields={riskResponseFieldSpecs(t, workSheet.owners, timeZone)}
            initial={{}}
            submitLabel={t("entry.save")}
            validate={(values) => riskResponseProblem(t, values)}
            submit={(values) =>
              actions.updateRiskResponse(
                workSheet.item.id,
                workSheet.item.revision,
                riskResponseValues(values),
                workSheet.clientRequestId,
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "entry.saved.responded");
            }}
            onClose={close}
            onStale={() => {
              // The row read an older Risk: re-read the page, then offer again.
              setWorkSheet(null);
              void fetchPage();
            }}
          />
        );
    }
  }

  function rowCommands(
    item: WorkQueueItemDto,
    key: string,
    mine: PendingRow | null,
    offered: readonly (typeof OFFERS)[number][],
    entry: ReactNode,
  ) {
    // One row acts at a time: while any row has a command in flight,
    // composing, or failed-and-not-abandoned, the other rows wait.
    const busy = pending !== null && pending.key !== key;
    const waiting = held.get(key);
    return (
      <div className="pmc-work-queue-actions">
        {waiting && (
          <button
            type="button"
            className="pmc-button-primary"
            disabled={busy || review !== null}
            onClick={() => {
              setHeld((current) => {
                const next = new Map(current);
                next.delete(key);
                return next;
              });
              setNotice(null);
              setInspecting(null);
              setReview(waiting);
            }}
          >
            {t("wq.backToReview")}
          </button>
        )}
        {offered.map((offer) => (
          <button
            key={offer.command}
            type="button"
            disabled={busy || waiting !== undefined || (mine !== null && mine.phase !== "failed")}
            onClick={() => {
              begin(item, offer.command);
            }}
          >
            {t(offer.label)}
          </button>
        ))}
        {entry}
        {mine?.phase === "composing" && composeForm(item, mine)}
        {mine?.phase === "sending" && <span role="status">{t("wq.sending")}</span>}
        {mine?.phase === "failed" && mine.error && (
          <SafeErrorDetail
            message={t("wq.actionFailed", { message: mine.error.message })}
            correlationId={mine.error.correlationId}
            errorCode={mine.error.errorCode}
            retryable={mine.error.retryable}
            {...(mine.error.retryable
              ? {
                  onRetry: () => {
                    send(item, mine.command, mine);
                  },
                }
              : {})}
            nextActions={[
              {
                label: t("wq.abandon"),
                onSelect: () => {
                  setPending(null);
                },
              },
            ]}
          />
        )}
      </div>
    );
  }

  function composeForm(item: WorkQueueItemDto, mine: PendingRow) {
    const cancel = (
      <button
        type="button"
        onClick={() => {
          setPending(null);
        }}
      >
        {t("wq.cancel")}
      </button>
    );
    const submit = (event: SyntheticEvent) => {
      event.preventDefault();
      send(item, mine.command, mine);
    };
    switch (mine.command) {
      case "decline":
      case "withdraw":
      case "withdrawDecision":
      case "prepareCancel":
      case "prepareCloseRisk":
      case "prepareReopen": {
        const label =
          mine.command === "prepareCloseRisk"
            ? t("wq.reason.closeRisk")
            : mine.command === "decline"
              ? t("wq.reason.decline")
              : mine.command === "withdraw" || mine.command === "withdrawDecision"
                ? t("wq.reason.withdraw")
                : mine.command === "prepareCancel"
                  ? t("wq.reason.cancel")
                  : t("wq.reason.reopen");
        const confirm =
          mine.command === "prepareCloseRisk"
            ? t("wq.confirm.closePreview")
            : mine.command === "decline"
              ? t("wq.confirm.decline")
              : mine.command === "withdraw" || mine.command === "withdrawDecision"
                ? t("wq.confirm.withdraw")
                : mine.command === "prepareCancel"
                  ? t("wq.confirm.cancelPreview")
                  : t("wq.confirm.reopenPreview");
        return (
          <form className="pmc-work-queue-rationale" onSubmit={submit}>
            {mine.command === "prepareReopen" && (
              <label>
                {t("wq.reopenMode")}
                <select
                  value={mine.reopenMode}
                  onChange={(event) => {
                    setPending({ ...mine, reopenMode: event.target.value });
                  }}
                >
                  <option value="reopen_completed">{t("wq.reopenMode.completed")}</option>
                  <option value="restart_cancelled">{t("wq.reopenMode.cancelled")}</option>
                </select>
              </label>
            )}
            <label>
              {label}
              <textarea
                value={mine.rationale}
                onChange={(event) => {
                  setPending({ ...mine, rationale: event.target.value });
                }}
              />
            </label>
            <button type="submit" disabled={mine.rationale.trim().length === 0}>
              {confirm}
            </button>
            {cancel}
          </form>
        );
      }
      case "prepareResolveIssue":
      case "prepareCloseIssue":
      case "prepareReopenIssue": {
        const legend =
          mine.command === "prepareResolveIssue"
            ? t("wq.issueEvidence.resolve")
            : mine.command === "prepareCloseIssue"
              ? t("wq.issueEvidence.close")
              : t("wq.issueEvidence.reopen");
        const confirm =
          mine.command === "prepareResolveIssue"
            ? t("wq.confirm.resolvePreview")
            : mine.command === "prepareCloseIssue"
              ? t("wq.confirm.closePreview")
              : t("wq.confirm.reopenPreview");
        const needsRationale = mine.command !== "prepareCloseIssue";
        const update = (patch: Partial<PendingRow>) => {
          setPending({ ...mine, ...patch });
        };
        return (
          <form className="pmc-work-queue-rationale" onSubmit={submit}>
            {mine.command === "prepareResolveIssue" && (
              <label>
                {t("wq.resolutionType")}
                <select
                  value={mine.resolutionType}
                  onChange={(event) => {
                    update({ resolutionType: event.target.value });
                  }}
                >
                  <option value="resolved">{t("wq.resolutionType.resolved")}</option>
                  <option value="workaround">{t("wq.resolutionType.workaround")}</option>
                  <option value="accepted_impact">{t("wq.resolutionType.acceptedImpact")}</option>
                </select>
              </label>
            )}
            {needsRationale && (
              <label>
                {mine.command === "prepareResolveIssue"
                  ? t("wq.reason.resolve")
                  : t("wq.reason.reopen")}
                <textarea
                  value={mine.rationale}
                  onChange={(event) => {
                    update({ rationale: event.target.value });
                  }}
                />
              </label>
            )}
            <fieldset>
              <legend>{legend}</legend>
              {mine.evidence === null ? (
                <span role="status">{t("wq.evidenceLoading")}</span>
              ) : mine.evidence.length === 0 ? (
                <span>{t("wq.evidenceNone")}</span>
              ) : (
                mine.evidence.map((evidence) => (
                  <label key={evidence.id}>
                    <input
                      type="checkbox"
                      checked={mine.evidenceIds.includes(evidence.id)}
                      onChange={() => {
                        update({
                          evidenceIds: mine.evidenceIds.includes(evidence.id)
                            ? mine.evidenceIds.filter((each) => each !== evidence.id)
                            : [...mine.evidenceIds, evidence.id],
                        });
                      }}
                    />
                    {t(
                      evidence.pinned ? "wq.evidenceOption.pinned" : "wq.evidenceOption.unpinned",
                      {
                        id: evidence.id,
                        verification: verificationLabel(t, evidence.verification.kind),
                        classification: classificationName(t, evidence.classification),
                        version: String(evidence.version),
                      },
                    )}
                  </label>
                ))
              )}
            </fieldset>
            <label>
              {t("wq.judgment.issue")}
              <textarea
                value={mine.judgmentRationale}
                onChange={(event) => {
                  update({ judgmentRationale: event.target.value });
                }}
              />
            </label>
            {mine.judgmentRationale.trim().length > 0 && (
              <label>
                {t("wq.judgmentClassification")}
                <select
                  value={mine.judgmentClassification}
                  onChange={(event) => {
                    update({ judgmentClassification: event.target.value });
                  }}
                >
                  {CLASSIFICATIONS.map((classification) => (
                    <option key={classification} value={classification}>
                      {classificationName(t, classification)}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <button
              type="submit"
              disabled={
                mine.evidenceIds.length === 0 ||
                (needsRationale && mine.rationale.trim().length === 0)
              }
            >
              {confirm}
            </button>
            {cancel}
          </form>
        );
      }
      case "prepareComplete":
        return (
          <form className="pmc-work-queue-rationale" onSubmit={submit}>
            <label>
              {t("wq.judgment.complete")}
              <textarea
                value={mine.rationale}
                onChange={(event) => {
                  setPending({ ...mine, rationale: event.target.value });
                }}
              />
            </label>
            {mine.rationale.trim().length > 0 && (
              <label>
                {t("wq.judgmentClassification")}
                <select
                  value={mine.judgmentClassification}
                  onChange={(event) => {
                    setPending({ ...mine, judgmentClassification: event.target.value });
                  }}
                >
                  {CLASSIFICATIONS.map((classification) => (
                    <option key={classification} value={classification}>
                      {classificationName(t, classification)}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <button type="submit">{t("wq.confirm.completePreview")}</button>
            {cancel}
          </form>
        );
      case "link": {
        if (mine.evidence === null) {
          return <span role="status">{t("wq.linkLoading")}</span>;
        }
        const candidates = mine.evidence.filter(
          (evidence) => !mine.linkedEvidenceIds.includes(evidence.id),
        );
        return (
          <form className="pmc-work-queue-rationale" onSubmit={submit}>
            <label>
              {t("wq.linkChoose")}
              <select
                value={mine.evidenceId}
                onChange={(event) => {
                  setPending({ ...mine, evidenceId: event.target.value });
                }}
              >
                <option value="">{t("wq.linkPlaceholder")}</option>
                {candidates.map((evidence) => (
                  <option key={evidence.id} value={evidence.id}>
                    {t(
                      evidence.pinned ? "wq.evidenceOption.pinned" : "wq.evidenceOption.unpinned",
                      {
                        id: evidence.id,
                        verification: verificationLabel(t, evidence.verification.kind),
                        classification: classificationName(t, evidence.classification),
                        version: String(evidence.version),
                      },
                    )}
                  </option>
                ))}
              </select>
            </label>
            {candidates.length === 0 && <span>{t("wq.linkNone")}</span>}
            <button type="submit" disabled={mine.evidenceId === ""}>
              {t("wq.linkConfirm")}
            </button>
            {cancel}
          </form>
        );
      }
      case "prepareResolve":
        return resolveForm(mine, submit, cancel);
      default:
        return null;
    }
  }

  function resolveForm(
    mine: PendingRow,
    submit: (event: SyntheticEvent) => void,
    cancel: ReactNode,
  ) {
    const update = (patch: Partial<PendingRow>) => {
      setPending({ ...mine, ...patch });
    };
    const updateResulting = (index: number, patch: Partial<ResultingActionRequestInput>) => {
      update({
        resulting: mine.resulting.map((each, at) => (at === index ? { ...each, ...patch } : each)),
      });
    };
    const ready =
      mine.statement.trim().length > 0 &&
      mine.rationale.trim().length > 0 &&
      mine.impact.trim().length > 0 &&
      mine.resulting.every(
        (each) =>
          each.subject.trim().length > 0 &&
          each.details.trim().length > 0 &&
          each.intendedOwner.trim().length > 0 &&
          each.dueAtMillis > 0,
      );
    return (
      <form className="pmc-work-queue-rationale pmc-work-queue-resolve" onSubmit={submit}>
        <label>
          {t("wq.decision.statement")}
          <textarea
            value={mine.statement}
            onChange={(event) => {
              update({ statement: event.target.value });
            }}
          />
        </label>
        <label>
          {t("wq.decision.rationale")}
          <textarea
            value={mine.rationale}
            onChange={(event) => {
              update({ rationale: event.target.value });
            }}
          />
        </label>
        <label>
          {t("wq.decision.impact")}
          <textarea
            value={mine.impact}
            onChange={(event) => {
              update({ impact: event.target.value });
            }}
          />
        </label>
        <fieldset>
          <legend>{t("wq.decision.evidence")}</legend>
          {mine.evidence === null ? (
            <span role="status">{t("wq.evidenceLoading")}</span>
          ) : mine.evidence.length === 0 ? (
            <span>{t("wq.evidenceNone")}</span>
          ) : (
            mine.evidence.map((evidence) => (
              <label key={evidence.id}>
                <input
                  type="checkbox"
                  checked={mine.evidenceIds.includes(evidence.id)}
                  onChange={() => {
                    update({
                      evidenceIds: mine.evidenceIds.includes(evidence.id)
                        ? mine.evidenceIds.filter((each) => each !== evidence.id)
                        : [...mine.evidenceIds, evidence.id],
                    });
                  }}
                />
                {t(evidence.pinned ? "wq.evidenceOption.pinned" : "wq.evidenceOption.unpinned", {
                  id: evidence.id,
                  verification: verificationLabel(t, evidence.verification.kind),
                  classification: classificationName(t, evidence.classification),
                  version: String(evidence.version),
                })}
              </label>
            ))
          )}
        </fieldset>
        <label>
          {t("wq.judgment.decision")}
          <textarea
            value={mine.judgmentRationale}
            onChange={(event) => {
              update({ judgmentRationale: event.target.value });
            }}
          />
        </label>
        <fieldset>
          <legend>{t("wq.followUps")}</legend>
          {mine.resulting.map((each, index) => (
            <div key={String(index)} className="pmc-work-queue-resulting">
              <label>
                {t("wq.followUp.subject")}
                <input
                  value={each.subject}
                  onChange={(event) => {
                    updateResulting(index, { subject: event.target.value });
                  }}
                />
              </label>
              <label>
                {t("wq.followUp.details")}
                <textarea
                  value={each.details}
                  onChange={(event) => {
                    updateResulting(index, { details: event.target.value });
                  }}
                />
              </label>
              <label>
                {t("wq.followUp.owner")}
                <input
                  value={each.intendedOwner}
                  onChange={(event) => {
                    updateResulting(index, { intendedOwner: event.target.value });
                  }}
                />
              </label>
              <label>
                {t("wq.followUp.due")}
                <input
                  type="date"
                  onChange={(event) => {
                    const parsed = Date.parse(`${event.target.value}T00:00:00Z`);
                    updateResulting(index, { dueAtMillis: Number.isNaN(parsed) ? 0 : parsed });
                  }}
                />
              </label>
              <label>
                {t("wq.followUp.classification")}
                <select
                  value={each.classification}
                  onChange={(event) => {
                    updateResulting(index, { classification: event.target.value });
                  }}
                >
                  {CLASSIFICATIONS.map((classification) => (
                    <option key={classification} value={classification}>
                      {classificationName(t, classification)}
                    </option>
                  ))}
                </select>
              </label>
              <button
                type="button"
                onClick={() => {
                  update({ resulting: mine.resulting.filter((_, at) => at !== index) });
                }}
              >
                {t("wq.followUp.remove")}
              </button>
            </div>
          ))}
          <button
            type="button"
            onClick={() => {
              update({ resulting: [...mine.resulting, EMPTY_RESULTING] });
            }}
          >
            {t("wq.followUp.add")}
          </button>
        </fieldset>
        <button type="submit" disabled={!ready}>
          {t("wq.confirm.resolvePreview")}
        </button>
        {cancel}
      </form>
    );
  }

  // -------------------------------------------------------------- render

  if (state.status === "loading") {
    return (
      <p className="pmc-work-queue-status" role="status">
        {t("route.loading", { route: findRoute("work-queue").label })}
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="pmc-work-queue-status">
        <button type="button" onClick={retry}>
          {t("route.reload")}
        </button>
        <SafeErrorDetail
          message={t("route.unavailable", {
            route: findRoute("work-queue").label,
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { queue } = state;

  if (queue.state === "outOfSync") {
    return (
      <div className="pmc-work-queue-status" role="alert">
        <p>{t("wq.outOfSync")}</p>
        <button type="button" onClick={retry}>
          {t("route.reload")}
        </button>
      </div>
    );
  }

  const shownFrom = queue.items.length === 0 ? 0 : queue.offset + 1;
  const shownTo = queue.offset + queue.items.length;

  const toggleKind = (kind: WorkItemKind) => {
    setOffset(0);
    setKinds((current) =>
      current.includes(kind) ? current.filter((each) => each !== kind) : [...current, kind],
    );
  };

  const copy = review ? reviewCopy(review, t) : null;

  return (
    <div className="pmc-work-queue">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("wq.headline")}</h2>
          <p className="pmc-page-lede">{t("wq.lede")}</p>
        </div>
        <dl className="pmc-meta pmc-page-asof">
          <div>
            <dt>{t("route.ledgerRevision")}</dt>
            <dd>{String(queue.ledgerRevision)}</dd>
          </div>
          <div>
            <dt>{t("route.readAt")}</dt>
            <dd>
              <time>{formatReadAt(queue.asOfMillis)}</time>
            </dd>
          </div>
        </dl>
      </header>

      {notice !== null && (
        <p className="pmc-work-queue-notice" role="status">
          {notice}
        </p>
      )}
      {entryOpening !== null ? (
        <SafeErrorDetail
          message={t("entry.readFailed", { message: entryOpening.message })}
          correlationId={entryOpening.correlationId}
          retryable={entryOpening.retryable}
        />
      ) : null}
      {entryActions === undefined ? null : (
        <div
          className="pmc-entry-actions pmc-work-queue-entry"
          role="group"
          aria-label={t("entry.work.actions")}
        >
          <button
            type="button"
            className="pmc-button pmc-button-primary"
            disabled={entryBusy}
            onClick={() => {
              openWithOwners("actionRequest");
            }}
          >
            {t("entry.new.actionRequest")}
          </button>
          <button
            type="button"
            className="pmc-button pmc-button-primary"
            disabled={entryBusy}
            onClick={() => {
              openWithOwners("decisionRequest");
            }}
          >
            {t("entry.new.decisionRequest")}
          </button>
          <button
            type="button"
            className="pmc-button pmc-button-primary"
            disabled={entryBusy}
            onClick={() => {
              setWorkSheet({ kind: "risk", clientRequestId: newEntryRequestId() });
            }}
          >
            {t("entry.new.risk")}
          </button>
          <button
            type="button"
            className="pmc-button pmc-button-primary"
            disabled={entryBusy}
            onClick={() => {
              setWorkSheet({ kind: "issue", clientRequestId: newEntryRequestId() });
            }}
          >
            {t("entry.new.issue")}
          </button>
        </div>
      )}

      <fieldset className="pmc-work-queue-filters">
        <legend>{t("wq.filters")}</legend>
        {queue.countsByKind.map((entry) => (
          <label key={entry.kind} className="pmc-chip" data-checked={kinds.includes(entry.kind)}>
            <input
              type="checkbox"
              checked={kinds.includes(entry.kind)}
              onChange={() => {
                toggleKind(entry.kind);
              }}
            />
            {t("wq.filterChip", { kind: workItemKindLabel(t, entry.kind), count: entry.count })}
          </label>
        ))}
        <label className="pmc-chip pmc-chip-flag" data-checked={onlyFlagged}>
          <input
            type="checkbox"
            checked={onlyFlagged}
            onChange={() => {
              setOffset(0);
              setOnlyFlagged((current) => !current);
            }}
          />
          {t("wq.onlyFlagged")}
        </label>
      </fieldset>

      {queue.items.length === 0 ? (
        <p className="pmc-empty">{t("wq.empty")}</p>
      ) : (
        <>
          <table className="pmc-work-queue-table">
            <caption>
              {t.plural("workQueue.caption", queue.total, { from: shownFrom, to: shownTo })}
            </caption>
            <thead>
              <tr>
                <th scope="col">{t("wq.column.kind")}</th>
                <th scope="col">{t("wq.column.item")}</th>
                <th scope="col">{t("wq.column.attention")}</th>
                <th scope="col">{t("wq.column.deadline")}</th>
                <th scope="col">{t("wq.column.placement")}</th>
                {actions && <th scope="col">{t("wq.column.next")}</th>}
              </tr>
            </thead>
            <tbody>
              {queue.items.map((item) => (
                <tr key={rowKey(item)} data-flagged={item.attention.length > 0}>
                  {/* The type first, so the five lifecycles read as five and
                      not read as one undifferentiated pool. */}
                  <td>
                    <span className="pmc-kind" data-kind={item.kind}>
                      {workItemKindLabel(t, item.kind)}
                    </span>
                  </td>
                  <th scope="row">
                    <button
                      type="button"
                      className="pmc-work-item-title pmc-work-item-open"
                      onClick={() => {
                        setInspecting(rowKey(item));
                      }}
                    >
                      {item.label}
                    </button>
                    <span className="pmc-work-item-meta">
                      <span>{stateLabel(t, item.stateLabel)}</span>
                      <span
                        className="pmc-classification-badge"
                        data-classification={item.classification}
                      >
                        {classificationName(t, item.classification)}
                      </span>
                      {/* Per item, not per page: two of the five kinds come
                          from one snapshot and the other three from another,
                          so a single response-level timestamp would claim the
                          five types were read at one instant when they were
                          not. */}
                      <span>
                        {t("route.readAt")} <time>{formatReadAt(item.asOfMillis)}</time>
                      </span>
                    </span>
                    {/* Text, not buttons. Lifecycle-admissible is strictly
                        weaker than executable, and a button here would promise
                        an authority this line does not have. The governed paths
                        a host actually wires are offered in their own column,
                        and each of them still runs every further gate. */}
                    <span className="pmc-work-item-intents">
                      {t("wq.allowed", {
                        intents:
                          item.lifecycleLegalIntents.length === 0
                            ? t("wq.none")
                            : item.lifecycleLegalIntents
                                .map((intent) => intentLabel(t, intent))
                                .join(t("common.idSeparator")),
                      })}
                    </span>
                  </th>
                  {/* Every flag, in words keyed off its stable reason. A count
                      says several things are wrong without saying what, and the
                      reader cannot act on that. An empty list says plainly that
                      nothing has flagged this record. */}
                  <td>
                    {item.attention.length === 0 ? (
                      <span className="pmc-work-item-clear">{t("wq.detail.noAttention")}</span>
                    ) : (
                      <ul className="pmc-work-queue-attention">
                        {item.attention.map((flag) => (
                          <li key={flag.reason} data-tier={flag.tier}>
                            {reasonLabel(t, flag.reason)}
                            {/* Staleness never improves an item's position and
                                is never hidden either: it is shown with the
                                limit of what is known attached. */}
                            {(flag.freshness !== "fresh" || flag.degraded) && (
                              <span className="pmc-work-queue-uncertainty">
                                {t(flag.degraded ? "wq.uncertaintyDegraded" : "wq.uncertainty", {
                                  freshness: freshnessLabel(t, flag.freshness),
                                })}
                              </span>
                            )}
                          </li>
                        ))}
                      </ul>
                    )}
                  </td>
                  <td className="pmc-work-item-deadline">
                    {formatDeadline(item.relevantAtMillis, t, item.promisedAtMillis)}
                    {/* A promised completion date is its own fact, not the
                        request's deadline, so it sits under it, labelled. */}
                    {item.promisedAtMillis != null && (
                      <span className="pmc-work-item-promised">
                        {t("wq.promised", { date: formatLocalDate(item.promisedAtMillis) })}
                      </span>
                    )}
                  </td>
                  <td className="pmc-work-item-placement">{placementLabel(t, item.placement)}</td>
                  {/* While this row's detail sheet is open its commands live
                      there alone, so a form or error is never rendered twice. */}
                  {actions && (
                    <td>
                      {inspecting === rowKey(item) ? (
                        <span className="pmc-work-item-clear">{t("wq.inDetail")}</span>
                      ) : (
                        rowActions(item)
                      )}
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>

          <div className="pmc-work-queue-paging">
            <button
              type="button"
              className="pmc-button"
              disabled={queue.offset === 0}
              onClick={() => {
                setOffset(Math.max(0, queue.offset - PAGE_SIZE));
              }}
            >
              {t("wq.previous")}
            </button>
            <button
              type="button"
              className="pmc-button"
              disabled={!queue.hasMore}
              onClick={() => {
                setOffset(queue.offset + PAGE_SIZE);
              }}
            >
              {t("wq.next")}
            </button>
          </div>
        </>
      )}

      {review === null &&
        inspecting !== null &&
        detailDialog(queue.items.find((each) => rowKey(each) === inspecting))}

      {review && copy && (
        <H2aFocusedReview
          title={copy.title}
          summary={copy.summary}
          approveLabel={copy.approveLabel}
          prepared={review.prepared}
          now={now}
          onApprove={() => reviewApprove(review)}
          onReject={() => reviewReject(review)}
          onPrepareAgain={() => {
            prepareAgain(review);
          }}
          onDone={() => {
            setReview(null);
            void fetchPage();
          }}
          onDismiss={() => {
            const key = rowKey(review.item);
            setHeld((current) => new Map(current).set(key, review));
            setReview(null);
            setNotice(t("wq.notice.held", { label: review.item.label }));
          }}
        />
      )}

      {review === null && workSheetDialog()}
    </div>
  );
}
