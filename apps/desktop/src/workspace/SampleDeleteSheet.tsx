import { useEffect, useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { useT } from "../i18n/useT";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import {
  newClientRequestId,
  type SampleDeletePreviewDto,
  type SampleDeletedDto,
  type WorkspaceActions,
} from "./workspaceIpc";

export interface SampleDeleteSheetProps {
  readonly actions: Pick<
    WorkspaceActions,
    "prepareSampleDelete" | "rejectSampleDelete" | "approveSampleDelete"
  >;
  /** Closed without deleting. */
  readonly onClose: () => void;
  /** An approval ran; the caller reads the workspace status again. */
  readonly onFinished: (result: SampleDeletedDto) => void;
}

type Step =
  | { readonly kind: "preparing" }
  | { readonly kind: "preview"; readonly preview: SampleDeletePreviewDto }
  | { readonly kind: "deleting"; readonly preview: SampleDeletePreviewDto }
  | { readonly kind: "result"; readonly result: SampleDeletedDto };

/** The one operation and effect this sheet can put into words. A preview
 * that binds anything else is shown as such and cannot be approved here. */
const DELETE_OPERATION = "delete_sample_workspace";
const DELETE_EFFECT = "irreversible_live_unaffected";

function describable(preview: SampleDeletePreviewDto): boolean {
  return preview.operation === DELETE_OPERATION && preview.effect === DELETE_EFFECT;
}

/** The typed text, compared as the person means it — the host checks again. */
function normalized(text: string): string {
  return text.trim().replace(/\s+/g, " ").toLocaleUpperCase();
}

/**
 * Settings → Workspace → "Delete sample data…" (the accepted sample-workspace
 * amendment §8, H2b): the exact preview of what exists and what the delete
 * does, then the typed phrase. Reject — or leaving — changes nothing.
 */
export function SampleDeleteSheet({ actions, onClose, onFinished }: SampleDeleteSheetProps) {
  const t = useT();
  const [step, setStep] = useState<Step>({ kind: "preparing" });
  const [typed, setTyped] = useState("");
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  const [rejecting, setRejecting] = useState(false);
  // One request id per preview, so a retried approval is the same approval.
  const [clientRequestId] = useState(newClientRequestId);
  // A preview no screen shows would block every later operation: withdraw
  // the one this sheet holds when it goes away.
  const heldIntent = useRef<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    actions.prepareSampleDelete().then(
      (preview) => {
        if (!mounted.current) {
          void actions.rejectSampleDelete(preview.preparedIntentId).catch(() => undefined);
          return;
        }
        heldIntent.current = preview.preparedIntentId;
        setStep({ kind: "preview", preview });
      },
      (reason: unknown) => {
        if (mounted.current) {
          setError(resolveRejection(reason, t));
        }
      },
    );
    return () => {
      mounted.current = false;
      if (heldIntent.current !== null) {
        void actions.rejectSampleDelete(heldIntent.current).catch(() => undefined);
        heldIntent.current = null;
      }
    };
    // Prepared once per sheet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const phrase = t("sampleWorkspace.delete.phrase");
  const known = step.kind !== "preview" || describable(step.preview);
  const confirmed = step.kind === "preview" && known && normalized(typed) === normalized(phrase);
  const locked = step.kind === "deleting" || rejecting;

  const reject = () => {
    if (step.kind !== "preview" || rejecting) {
      onClose();
      return;
    }
    setRejecting(true);
    heldIntent.current = null;
    actions
      .rejectSampleDelete(step.preview.preparedIntentId)
      .catch(() => undefined)
      .finally(() => {
        setRejecting(false);
        onClose();
      });
  };

  const close = () => {
    if (locked) {
      return;
    }
    if (step.kind === "result") {
      onFinished(step.result);
      return;
    }
    reject();
  };

  const approve = () => {
    if (step.kind !== "preview" || !confirmed) {
      return;
    }
    const { preview } = step;
    setError(null);
    // Approving: from here the host decides; leaving must not reject it.
    heldIntent.current = null;
    setStep({ kind: "deleting", preview });
    actions
      .approveSampleDelete(preview.preparedIntentId, preview.payloadSha256, typed, clientRequestId)
      .then(
        (result) => {
          setStep({ kind: "result", result });
        },
        (reason: unknown) => {
          if (!mounted.current) {
            // Refused after the sheet went away: no screen shows this
            // preview any more, so withdraw it (the host may already have).
            void actions.rejectSampleDelete(preview.preparedIntentId).catch(() => undefined);
            return;
          }
          heldIntent.current = preview.preparedIntentId;
          setError(resolveRejection(reason, t));
          setStep({ kind: "preview", preview });
        },
      );
  };

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId="pmc-sample-delete-title"
        descriptionId="pmc-sample-delete-lede"
        onEscape={close}
        className="pmc-h2b-approval pmc-sample-delete-sheet"
      >
        <h2 id="pmc-sample-delete-title" className="pmc-section-title">
          {t("sampleWorkspace.delete.title")}
        </h2>

        {step.kind === "preparing" && error === null && (
          <p id="pmc-sample-delete-lede" role="status" className="pmc-backups-progress">
            {t("sampleWorkspace.delete.preparing")}
          </p>
        )}

        {(step.kind === "preview" || step.kind === "deleting") && (
          <>
            <p id="pmc-sample-delete-lede" className="pmc-h2a-summary">
              {t("sampleWorkspace.delete.lede")}
            </p>
            <dl className="pmc-h2a-fields">
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.what")}</dt>
                <dd>
                  {t("sampleWorkspace.delete.seed", {
                    seedId: step.preview.seedId,
                    version: String(step.preview.seedVersion),
                  })}
                </dd>
              </div>
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.parts")}</dt>
                <dd>
                  <ul className="pmc-sample-delete-parts">
                    {step.preview.hasLedger && <li>{t("sampleWorkspace.delete.part.ledger")}</li>}
                    {step.preview.hasVault && <li>{t("sampleWorkspace.delete.part.vault")}</li>}
                    {step.preview.hasGeneratedFiles && (
                      <li>{t("sampleWorkspace.delete.part.generated")}</li>
                    )}
                  </ul>
                </dd>
              </div>
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.effect")}</dt>
                <dd>{t("sampleWorkspace.delete.effectValue")}</dd>
              </div>
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.expires")}</dt>
                <dd>{formatReadAt(step.preview.expiresAtMillis)}</dd>
              </div>
              {/* The rest of what this approval is bound to: if any of it
                  changes before approval, the host refuses it. */}
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.settingsRevision")}</dt>
                <dd>{String(step.preview.settingsRevision)}</dd>
              </div>
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.inventory")}</dt>
                <dd className="pmc-sample-digest">{step.preview.inventorySha256}</dd>
              </div>
              <div className="pmc-h2a-field">
                <dt>{t("sampleWorkspace.delete.payload")}</dt>
                <dd className="pmc-sample-digest">{step.preview.payloadSha256}</dd>
              </div>
            </dl>
          </>
        )}

        {step.kind === "preview" && !known && (
          <p className="pmc-h2a-summary" role="alert">
            {t("sampleWorkspace.delete.unknown")}
          </p>
        )}

        {step.kind === "preview" && (
          <>
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-sample-delete-confirm">
              {t("sampleWorkspace.delete.confirmLabel", { phrase })}
            </label>
            <input
              id="pmc-sample-delete-confirm"
              className="pmc-h2b-confirmation-input"
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={typed}
              onChange={(event) => {
                setTyped(event.target.value);
              }}
            />
            <div className="pmc-h2a-actions">
              <button
                type="button"
                className="pmc-h2a-approve"
                disabled={!confirmed || rejecting}
                onClick={approve}
              >
                {t("sampleWorkspace.delete.confirm")}
              </button>
              <button
                type="button"
                className="pmc-h2a-reject"
                disabled={rejecting}
                aria-busy={rejecting}
                onClick={reject}
              >
                {t("sampleWorkspace.delete.reject")}
              </button>
            </div>
          </>
        )}

        {step.kind === "deleting" && (
          <p role="status" className="pmc-backups-progress">
            {t("sampleWorkspace.delete.deleting")}
          </p>
        )}

        {step.kind === "result" && (
          <>
            <p
              role="status"
              className="pmc-backups-result"
              tabIndex={-1}
              ref={(node) => node?.focus()}
            >
              {step.result.outcome === "deleted"
                ? t("sampleWorkspace.delete.deleted")
                : t("sampleWorkspace.delete.notDeleted")}
            </p>
            <div className="pmc-h2a-actions">
              <button
                type="button"
                className="pmc-h2a-approve"
                onClick={() => {
                  onFinished(step.result);
                }}
              >
                {t("sampleWorkspace.done")}
              </button>
            </div>
          </>
        )}

        {error !== null && (
          <>
            <SafeErrorDetail
              message={error.message}
              correlationId={error.correlationId}
              retryable={error.retryable}
              errorCode={error.errorCode}
            />
            {step.kind === "preparing" && (
              <div className="pmc-h2a-actions">
                <button type="button" className="pmc-h2a-reject" onClick={onClose}>
                  {t("sampleWorkspace.close")}
                </button>
              </div>
            )}
          </>
        )}
      </FocusTrapDialog>
    </div>
  );
}
