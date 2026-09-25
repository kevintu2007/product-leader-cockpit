import { useEffect, useState } from "react";

import type { ResolvedSafeError } from "../adapters/safeError";
import {
  classificationName,
  dispositionLabel,
  effectLabel,
  evidenceRoleLabel,
  policyLabel,
  resolutionTypeLabel,
  sourceRoleLabel,
  targetKindLabel,
  verificationLabel,
} from "../i18n/workLabels";
import type {
  PreparedIntentDto,
  PreparedOperationDto,
  SupportWitnessDto,
} from "../routes/cockpitContract";
import { ClassificationBadge, type DataClassification } from "./ClassificationBadge";
import { FocusTrapDialog } from "./FocusTrapDialog";
import { SafeErrorDetail } from "./SafeErrorDetail";
import { Slotted } from "../i18n/Slotted";
import { useT } from "../i18n/useT";
import type { Translator } from "../i18n/messages";

/** A label/value pair, as O04 (H2b) renders its own preview. O03 renders
 * the typed contract directly and derives its rows from it. */
export interface H2aPreviewField {
  readonly label: string;
  readonly value: string;
}

/** What a decision resolved to, reported by the caller after the host answered. */
export type H2aDecisionOutcome =
  | { readonly kind: "approved"; readonly summary: string }
  | { readonly kind: "rejected"; readonly expiredAtRejection: boolean }
  | { readonly kind: "failed"; readonly error: ResolvedSafeError };

export interface H2aFocusedReviewProps {
  /** e.g. "核准並執行：接受 Action Request". */
  readonly title: string;
  /** One-line description of what approving this does. */
  readonly summary: string;
  /** Action-specific approve label, e.g. "核准並接受". */
  readonly approveLabel: string;
  /** The exact, typed prepared contract from the host. This component only
   * renders what it is given; it never derives or summarizes domain state. */
  readonly prepared: PreparedIntentDto;
  /** Approve = the person's explicit click; the caller sends the acknowledged
   * digest with the same client request id on every retry of this decision. */
  readonly onApprove: () => Promise<H2aDecisionOutcome>;
  /** Reject is durable: the caller records it in the Ledger. */
  readonly onReject: () => Promise<H2aDecisionOutcome>;
  /** Close without deciding, for a person who wants to look before deciding:
   * clicking outside the sheet, 先不決定, or Escape (the DG1 brief: Escape
   * closes without effects). Nothing is sent -- it is never a
   * rejection -- and the preview simply stays pending until it expires.
   * Absent, the sheet offers no such close. */
  readonly onDismiss?: () => void;
  /** The preview expired or changed: the caller prepares a fresh one. */
  readonly onPrepareAgain: () => void;
  /** After a terminal outcome (approved / rejected) the caller refreshes. */
  readonly onDone: () => void;
  /** Injectable clock for the countdown; production uses `Date.now`. */
  readonly now?: () => number;
}

type Phase =
  | { readonly status: "idle" }
  | { readonly status: "approving" }
  | { readonly status: "rejecting" }
  | { readonly status: "approved"; readonly summary: string }
  | { readonly status: "rejected"; readonly expiredAtRejection: boolean }
  | {
      readonly status: "failed";
      readonly attempt: "approve" | "reject";
      readonly error: ResolvedSafeError;
    };

const EXPIRED_OR_CHANGED = "SECURITY_PREVIEW_EXPIRED_OR_CHANGED";

function formatInstant(millis: number): string {
  return new Date(millis).toISOString().slice(0, 19).replace("T", " ") + " UTC";
}

function formatRemaining(millis: number, t: Translator): string {
  if (millis <= 0) {
    return t("review.expired");
  }
  const seconds = Math.ceil(millis / 1000);
  return t("review.remaining", { minutes: Math.floor(seconds / 60), seconds: seconds % 60 });
}

/** The linked Evidence and each one's classification, or that there is none. */
function evidenceBindings(
  bindings: readonly { readonly evidenceId: string; readonly classification: string }[],
  t: Translator,
): string {
  return bindings.length === 0
    ? t("review.noLinkedEvidence")
    : bindings
        .map((binding) =>
          t("review.evidenceBinding", {
            id: binding.evidenceId,
            classification: classificationName(t, binding.classification),
          }),
        )
        .join(t("review.listSeparator"));
}

/** The rows for each operation kind S03 wires: what the person is
 * approving, in the operation's own terms. Exhaustive over the union. */
function operationFields(
  operation: PreparedOperationDto,
  t: Translator,
): readonly H2aPreviewField[] {
  switch (operation.kind) {
    case "acceptActionRequest":
      return [
        {
          label: t("review.field.actionRequest"),
          value: t("review.recordWithVersion", {
            id: operation.requestId,
            version: String(operation.requestVersion),
          }),
        },
        { label: t("review.field.actionToCreate"), value: operation.actionId },
        { label: t("review.field.subject"), value: operation.actionSubject },
        { label: t("review.field.commitment"), value: operation.commitmentDetails },
        { label: t("review.field.owner"), value: operation.intendedOwner },
        { label: t("review.field.dueAt"), value: formatInstant(operation.intendedDueAtMillis) },
        {
          label: t("review.field.actionClassification"),
          value: classificationName(t, operation.actionClassification),
        },
      ];
    case "completeAction":
      return [
        {
          label: t("review.field.actionToComplete"),
          value: t("review.recordWithVersion", {
            id: operation.actionId,
            version: String(operation.actionVersion),
          }),
        },
      ];
    case "cancelAction":
      return [
        {
          label: t("review.field.actionToCancel"),
          value: t("review.recordWithVersion", {
            id: operation.actionId,
            version: String(operation.actionVersion),
          }),
        },
        { label: t("review.field.cancelReason"), value: operation.reason },
        {
          label: t("review.field.linkedEvidenceClassification"),
          value: evidenceBindings(operation.evidenceClassifications, t),
        },
      ];
    case "reopenAction":
      return [
        {
          label: t("review.field.actionToReopen"),
          value: t("review.recordWithVersion", {
            id: operation.actionId,
            version: String(operation.actionVersion),
          }),
        },
        {
          label: t("review.field.mode"),
          value:
            operation.mode === "reopen_completed"
              ? t("review.mode.reopenCompleted")
              : t("review.mode.restartCancelled"),
        },
        { label: t("review.field.reopenReason"), value: operation.reason },
        {
          label: t("review.field.linkedEvidenceClassification"),
          value: evidenceBindings(operation.evidenceClassifications, t),
        },
      ];
    case "resolveDecisionRequest":
      return [
        {
          label: t("review.field.decisionRequest"),
          value: t("review.recordWithVersion", {
            id: operation.requestId,
            version: String(operation.requestVersion),
          }),
        },
        { label: t("review.field.decisionToCreate"), value: operation.decisionId },
        { label: t("review.field.statement"), value: operation.statement },
        { label: t("review.field.decisionRationale"), value: operation.rationale },
        { label: t("review.field.impact"), value: operation.impact },
        { label: t("review.field.decisionOwner"), value: operation.decisionOwner },
        { label: t("review.field.decidedAt"), value: formatInstant(operation.decidedAtMillis) },
        {
          label: t("review.field.decisionClassification"),
          value: classificationName(t, operation.decisionClassification),
        },
        {
          label: t("review.field.followUps"),
          value:
            operation.resultingActionRequests.length === 0
              ? t("review.none")
              : operation.resultingActionRequests
                  .map((request) =>
                    t("review.followUp", {
                      id: request.id,
                      subject: request.subject,
                      owner: request.intendedOwner,
                      due: formatInstant(request.dueAtMillis),
                      classification: classificationName(t, request.classification),
                    }),
                  )
                  .join(t("review.listSeparator")),
        },
      ];
    case "recordRiskOccurrence":
      return [
        {
          label: t("review.field.riskOccurred"),
          value: t("review.recordWithVersion", {
            id: operation.riskId,
            version: String(operation.riskVersion),
          }),
        },
        { label: t("review.field.issueToCreate"), value: operation.issueId },
        {
          label: t("review.field.issueClassification"),
          value: classificationName(t, operation.issueClassification),
        },
      ];
    case "closeRisk":
      return [
        {
          label: t("review.field.riskToClose"),
          value: t("review.recordWithVersion", {
            id: operation.riskId,
            version: String(operation.riskVersion),
          }),
        },
        { label: t("review.field.closeReason"), value: operation.rationale },
      ];
    case "resolveIssue":
      return [
        {
          label: t("review.field.issueToResolve"),
          value: t("review.recordWithVersion", {
            id: operation.issueId,
            version: String(operation.issueVersion),
          }),
        },
        {
          label: t("review.field.resolutionType"),
          value: resolutionTypeLabel(t, operation.resolutionType),
        },
        { label: t("review.field.resolutionRationale"), value: operation.rationale },
      ];
    case "closeIssue":
      return [
        {
          label: t("review.field.issueToClose"),
          value: t("review.recordWithVersion", {
            id: operation.issueId,
            version: String(operation.issueVersion),
          }),
        },
      ];
    case "reopenIssue":
      return [
        {
          label: t("review.field.issueToReopen"),
          value: t("review.recordWithVersion", {
            id: operation.issueId,
            version: String(operation.issueVersion),
          }),
        },
        { label: t("review.field.reopenReason"), value: operation.rationale },
      ];
  }
}

/** One piece of Evidence as a whole sentence: the catalog has one per
 * combination of the optional verification time and digest, so every
 * language places those clauses itself. */
function evidenceLine(evidence: SupportWitnessDto["evidence"][number], t: Translator): string {
  const base = {
    id: evidence.id,
    role: evidenceRoleLabel(t, evidence.role),
    version: String(evidence.sourceVersion),
    classification: classificationName(t, evidence.classification),
    verification: verificationLabel(t, evidence.verification.kind),
  };
  const { atMillis, integrityDigest } = evidence.verification;
  if (atMillis !== null && integrityDigest !== null) {
    return t("review.support.evidenceAtDigest", {
      ...base,
      time: formatInstant(atMillis),
      digest: integrityDigest,
    });
  }
  if (atMillis !== null) {
    return t("review.support.evidenceAt", { ...base, time: formatInstant(atMillis) });
  }
  if (integrityDigest !== null) {
    return t("review.support.evidenceDigest", { ...base, digest: integrityDigest });
  }
  return t("review.support.evidence", base);
}

function SupportSection({ support }: { readonly support: SupportWitnessDto | null }) {
  const t = useT();
  if (support === null) {
    return <p className="pmc-h2a-support-none">{t("review.support.none")}</p>;
  }
  return (
    <div className="pmc-h2a-support">
      <p>
        {t("review.support.summary", {
          disposition: dispositionLabel(t, support.disposition),
          classification: classificationName(t, support.classification),
        })}
      </p>
      {support.evidence.length > 0 && (
        <ul>
          {support.evidence.map((evidence) => (
            <li key={evidence.id}>{evidenceLine(evidence, t)}</li>
          ))}
        </ul>
      )}
      {support.judgments.length > 0 && (
        <ul>
          {support.judgments.map((judgment) => (
            <li key={judgment.rationale}>
              {t("review.support.judgment", {
                disposition: dispositionLabel(t, judgment.disposition),
                classification: classificationName(t, judgment.classification),
                rationale: judgment.rationale,
              })}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * O03 H2a Focused Review (wired to the durable H2a loop): "Exact preview,
 * approve/reject, no typed phrase, atomic
 * single-use receipt on execute" (DG3 Overlay and Contextual Surface
 * Contract). Unlike O04 there is no confirmation phrase and no recovery
 * evidence.
 *
 * The preview is the host's whole typed contract: operation payload,
 * targets with expected versions, declared effects, classification and its
 * sources, Evidence/Judgment support with source revisions, and the full
 * payload digest the person acknowledges. Expiry is absolute with a live
 * countdown; an expired preview can no longer be approved but can still be
 * rejected (durably), or prepared again.
 *
 * Both decisions are asynchronous and durable. The actions disable only
 * while a call is in flight or after a terminal outcome; a failed call
 * leaves the person able to retry it (same decision, same request id) or,
 * when the host says the preview expired or changed, to prepare again.
 */
export function H2aFocusedReview({
  title,
  summary,
  approveLabel,
  prepared,
  onApprove,
  onReject,
  onPrepareAgain,
  onDone,
  onDismiss,
  now = Date.now,
}: H2aFocusedReviewProps) {
  const t = useT();
  const [phase, setPhase] = useState<Phase>({ status: "idle" });
  const [remaining, setRemaining] = useState(() => prepared.expiresAtMillis - now());

  useEffect(() => {
    const tick = () => {
      setRemaining(prepared.expiresAtMillis - now());
    };
    tick();
    const handle = setInterval(tick, 1000);
    return () => {
      clearInterval(handle);
    };
  }, [prepared.expiresAtMillis, now]);

  const expired = remaining <= 0;
  const busy = phase.status === "approving" || phase.status === "rejecting";
  const terminal = phase.status === "approved" || phase.status === "rejected";
  const canApprove = !busy && !terminal && !expired && phase.status !== "failed";
  const canReject = !busy && !terminal;

  function settle(outcome: H2aDecisionOutcome, attempt: "approve" | "reject") {
    if (outcome.kind === "approved") {
      setPhase({ status: "approved", summary: outcome.summary });
    } else if (outcome.kind === "rejected") {
      setPhase({ status: "rejected", expiredAtRejection: outcome.expiredAtRejection });
    } else {
      setPhase({ status: "failed", attempt, error: outcome.error });
    }
  }

  function approve() {
    if (!canApprove) {
      return;
    }
    setPhase({ status: "approving" });
    onApprove().then(
      (outcome) => {
        settle(outcome, "approve");
      },
      () => {
        setPhase({
          status: "failed",
          attempt: "approve",
          error: {
            message: t("review.unknownApproveOutcome"),
            correlationId: "",
            retryable: true,
          },
        });
      },
    );
  }

  function reject() {
    if (!canReject) {
      return;
    }
    setPhase({ status: "rejecting" });
    onReject().then(
      (outcome) => {
        settle(outcome, "reject");
      },
      () => {
        setPhase({
          status: "failed",
          attempt: "reject",
          error: {
            message: t("review.unknownRejectOutcome"),
            correlationId: "",
            retryable: true,
          },
        });
      },
    );
  }

  function retry() {
    if (phase.status !== "failed") {
      return;
    }
    if (phase.attempt === "approve") {
      setPhase({ status: "approving" });
      onApprove().then(
        (outcome) => {
          settle(outcome, "approve");
        },
        () => {
          setPhase({
            status: "failed",
            attempt: "approve",
            error: {
              message: t("review.unknownApproveOutcome"),
              correlationId: "",
              retryable: true,
            },
          });
        },
      );
    } else {
      setPhase({ status: "rejecting" });
      onReject().then(
        (outcome) => {
          settle(outcome, "reject");
        },
        () => {
          setPhase({
            status: "failed",
            attempt: "reject",
            error: {
              message: t("review.unknownRejectOutcome"),
              correlationId: "",
              retryable: true,
            },
          });
        },
      );
    }
  }

  // A close with no effect. While a decision is in flight its outcome is
  // not yet known, so the sheet stays; after one, closing is just 關閉.
  function dismiss() {
    if (busy) {
      return;
    }
    if (terminal) {
      onDone();
      return;
    }
    onDismiss?.();
  }

  const expiredOrChanged =
    phase.status === "failed" && phase.error.errorCode === EXPIRED_OR_CHANGED;
  const classification = prepared.resolvedClassification as DataClassification;

  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- a pointer convenience only: the same no-effect close is the 先不決定 button inside the dialog, reachable by keyboard.
    <div
      className="pmc-dialog-backdrop"
      onClick={(event) => {
        if (event.target === event.currentTarget) {
          dismiss();
        }
      }}
    >
      <FocusTrapDialog
        titleId="pmc-h2a-title"
        descriptionId="pmc-h2a-summary"
        onEscape={terminal ? onDone : onDismiss ? dismiss : reject}
        className="pmc-h2a-review"
      >
        <h2 id="pmc-h2a-title" className="pmc-section-title">
          {title}
        </h2>
        <p id="pmc-h2a-summary" className="pmc-h2a-summary">
          {summary}
        </p>
        <ClassificationBadge classification={classification} />

        <dl className="pmc-h2a-fields">
          {operationFields(prepared.operation, t).map((field) => (
            <div key={field.label} className="pmc-h2a-field">
              <dt>{field.label}</dt>
              <dd>{field.value}</dd>
            </div>
          ))}
          <div className="pmc-h2a-field">
            <dt>{t("review.field.targets")}</dt>
            <dd>
              {prepared.targets
                .map((target) =>
                  t("review.target", {
                    kind: targetKindLabel(t, target.kind),
                    id: target.id,
                    version: String(target.expectedVersion),
                  }),
                )
                .join(t("review.listSeparator"))}
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.effects")}</dt>
            <dd>
              {prepared.declaredEffects
                .map((effect) =>
                  t("review.effect", {
                    effect: effectLabel(t, effect.kind),
                    ids: effect.ids.join(t("review.idSeparator")),
                  }),
                )
                .join(t("review.listSeparator"))}
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.classificationSources")}</dt>
            <dd>
              {prepared.classificationSources
                .map((source) =>
                  source.id === null
                    ? t("review.classificationSource", {
                        role: sourceRoleLabel(t, source.role),
                        classification: classificationName(t, source.classification),
                      })
                    : t("review.classificationSourceWithId", {
                        role: sourceRoleLabel(t, source.role),
                        id: source.id,
                        classification: classificationName(t, source.classification),
                      }),
                )
                .join(t("review.listSeparator"))}
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.policy")}</dt>
            <dd>
              {[prepared.policyResult, prepared.authority, prepared.cancellationPolicy]
                .map((value) => policyLabel(t, value))
                .join(t("review.idSeparator"))}
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.support")}</dt>
            <dd>
              <SupportSection support={prepared.support} />
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.digest")}</dt>
            <dd>
              <code className="pmc-h2a-digest">{prepared.payloadDigest}</code>
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.preparation")}</dt>
            <dd>
              <dl className="pmc-meta">
                <div>
                  <dt>{t("review.meta.operation")}</dt>
                  <dd>{effectLabel(t, prepared.intentType)}</dd>
                </div>
                <div>
                  <dt>{t("review.meta.id")}</dt>
                  <dd>{prepared.preparedIntentId}</dd>
                </div>
                <div>
                  <dt>{t("review.meta.contractVersion")}</dt>
                  <dd>{String(prepared.contractVersion)}</dd>
                </div>
                <div>
                  <dt>{t("review.meta.preparedAt")}</dt>
                  <dd>{formatInstant(prepared.preparedAtMillis)}</dd>
                </div>
                <div>
                  <dt>{t("review.meta.correlation")}</dt>
                  <dd>{prepared.correlationId}</dd>
                </div>
              </dl>
            </dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("review.field.validUntil")}</dt>
            <dd>
              {/* Only the remaining time is live; the time and the brackets
                  around it are the catalog's sentence, not the timer's. */}
              <Slotted
                text={
                  t.lookup("review.validUntil", {
                    time: formatInstant(prepared.expiresAtMillis),
                  }) ?? ""
                }
                slots={{
                  remaining: (
                    <span role="timer" aria-live="polite">
                      {formatRemaining(remaining, t)}
                    </span>
                  ),
                }}
              />
            </dd>
          </div>
        </dl>

        {/* The outcome and any failure sit in the footer, above the buttons,
            so they are on screen whenever the buttons are -- a result
            printed below a sticky footer is covered by it. */}
        <div className="pmc-h2a-actions">
          <p role="status" className="pmc-h2a-decision-status" data-outcome={phase.status}>
            {phase.status === "approving" ? t("review.status.approving") : null}
            {phase.status === "rejecting" ? t("review.status.rejecting") : null}
            {phase.status === "approved"
              ? t("review.status.approved", { summary: phase.summary })
              : null}
            {phase.status === "rejected"
              ? phase.expiredAtRejection
                ? t("review.status.rejectedExpired")
                : t("review.status.rejected")
              : null}
            {phase.status === "idle" && expired ? t("review.status.expired") : null}
          </p>

          {phase.status === "failed" && (
            <SafeErrorDetail
              message={t(
                phase.attempt === "approve" ? "review.failed.approve" : "review.failed.reject",
                {
                  message: phase.error.message,
                },
              )}
              correlationId={phase.error.correlationId}
              errorCode={phase.error.errorCode}
              retryable={phase.error.retryable}
              {...(phase.error.retryable ? { onRetry: retry } : {})}
            />
          )}
          <div className="pmc-h2a-buttons">
            <button
              type="button"
              className="pmc-h2a-approve"
              disabled={!canApprove}
              onClick={approve}
            >
              {approveLabel}
            </button>
            <button type="button" className="pmc-h2a-reject" disabled={!canReject} onClick={reject}>
              {t("review.button.reject")}
            </button>
            {onDismiss && !terminal && (
              <button type="button" className="pmc-h2a-dismiss" disabled={busy} onClick={dismiss}>
                {t("review.button.later")}
              </button>
            )}
            {(expired || expiredOrChanged) && !terminal && (
              <button type="button" className="pmc-h2a-prepare-again" onClick={onPrepareAgain}>
                {t("review.button.prepareAgain")}
              </button>
            )}
            {terminal && (
              <button type="button" className="pmc-h2a-done" onClick={onDone}>
                {t("review.button.close")}
              </button>
            )}
          </div>
        </div>
      </FocusTrapDialog>
    </div>
  );
}
