import { useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { useT } from "../i18n/useT";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type {
  CheckedBackupDto,
  RestoreActions,
  RestorePreviewDto,
  RestoreResultDto,
} from "./restoreIpc";

export interface RestoreSheetProps {
  readonly actions: RestoreActions;
  /** The sheet closed without replacing anything. */
  readonly onClose: () => void;
  /** A restore ran; the app reloads everything it shows. */
  readonly onFinished: (result: RestoreResultDto) => void;
  /** The current Ledger is not open and cannot be inspected (System
   * Health, an unsupported-old gate): what is kept first is a preservation
   * copy of its files, not a backup (restore-unopened amendment §1). */
  readonly currentUnavailable?: boolean | undefined;
  /** A failed restore's recovery backup, offered first (System Health,
   * restore-unopened amendment §2); any other backup can still be chosen. */
  readonly recoveryBackupName?: string | undefined;
}

type Step =
  | { readonly kind: "choose" }
  | { readonly kind: "passphrase"; readonly token: string; readonly fileName: string }
  | { readonly kind: "checking"; readonly token: string; readonly fileName: string }
  | {
      readonly kind: "backingUp";
      readonly token: string;
      readonly fileName: string;
      readonly checked: CheckedBackupDto;
    }
  | { readonly kind: "preview"; readonly preview: RestorePreviewDto }
  | { readonly kind: "replacing"; readonly preview: RestorePreviewDto }
  | {
      readonly kind: "result";
      readonly preview: RestorePreviewDto;
      readonly result: RestoreResultDto;
    };

function newClientRequestId(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `restore-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

function atRfc3339(value: string): string {
  const millis = Date.parse(value);
  return Number.isNaN(millis) ? value : formatReadAt(millis);
}

/**
 * Settings → Backups → "Restore from a backup…" (DG3 restore amendment §3,
 * O04 H2b): choose the file, open it with its passphrase, check it, back up
 * the current workspace, show the exact preview, then replace only after the
 * backup's creation date is typed. No cancel once replacing has started.
 */
export function RestoreSheet({
  actions,
  onClose,
  onFinished,
  currentUnavailable = false,
  recoveryBackupName,
}: RestoreSheetProps) {
  const t = useT();
  const [step, setStep] = useState<Step>({ kind: "choose" });
  const [picking, setPicking] = useState(false);
  const [passphrase, setPassphrase] = useState("");
  const [typedDate, setTypedDate] = useState("");
  const [rejecting, setRejecting] = useState(false);
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  // One request id per preview, so a retried approval is the same approval.
  const [clientRequestId] = useState(newClientRequestId);

  // The recovery backup and replacing both run to their end once started.
  const locked = step.kind === "backingUp" || step.kind === "replacing" || rejecting;

  const reject = () => {
    if (step.kind !== "preview" || rejecting) {
      return;
    }
    setRejecting(true);
    actions
      .rejectPreparedRestore(step.preview.preparedIntentId)
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
    if (step.kind === "preview") {
      reject();
      return;
    }
    // Checking can be cancelled: the host finishes and discards it.
    void actions.discardRestoreSelection().catch(() => undefined);
    onClose();
  };

  const choose = () => {
    if (picking) {
      return;
    }
    setPicking(true);
    setError(null);
    actions
      .chooseRestoreArchive(t("restore.choose.dialogTitle"), t("restore.choose.filterName"))
      .then(
        (chosen) => {
          if (chosen.chosen && chosen.token !== null && chosen.fileName !== null) {
            setPassphrase("");
            setStep({ kind: "passphrase", token: chosen.token, fileName: chosen.fileName });
          }
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
        },
      )
      .finally(() => {
        setPicking(false);
      });
  };

  const chooseRecovery = () => {
    if (picking) {
      return;
    }
    setPicking(true);
    setError(null);
    actions
      .chooseRecoveryArchive()
      .then(
        (chosen) => {
          if (chosen.chosen && chosen.token !== null && chosen.fileName !== null) {
            setPassphrase("");
            setStep({ kind: "passphrase", token: chosen.token, fileName: chosen.fileName });
          }
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
        },
      )
      .finally(() => {
        setPicking(false);
      });
  };

  const check = () => {
    if (step.kind !== "passphrase" || passphrase.length === 0) {
      return;
    }
    const { token, fileName } = step;
    setError(null);
    setStep({ kind: "checking", token, fileName });
    actions.checkRestoreArchive(token, passphrase).then(
      (checked) => {
        setPassphrase("");
        setStep({ kind: "backingUp", token, fileName, checked });
        actions.prepareRestore(token).then(
          (preview) => {
            setTypedDate("");
            setStep({ kind: "preview", preview });
          },
          (reason: unknown) => {
            setError(resolveRejection(reason, t));
            setStep({ kind: "passphrase", token, fileName });
          },
        );
      },
      (reason: unknown) => {
        setError(resolveRejection(reason, t));
        setStep({ kind: "passphrase", token, fileName });
      },
    );
  };

  const replace = () => {
    if (step.kind !== "preview" || typedDate !== step.preview.confirmationDate) {
      return;
    }
    const { preview } = step;
    setError(null);
    setStep({ kind: "replacing", preview });
    actions
      .approveAndExecuteRestore(
        preview.preparedIntentId,
        preview.payloadSha256,
        typedDate,
        clientRequestId,
      )
      .then(
        (result) => {
          setStep({ kind: "result", preview, result });
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
          setStep({ kind: "preview", preview });
        },
      );
  };

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId="pmc-restore-title"
        descriptionId="pmc-restore-lede"
        onEscape={close}
        className="pmc-h2b-approval pmc-restore-sheet"
      >
        <h2 id="pmc-restore-title" className="pmc-section-title">
          {t("restore.title")}
        </h2>

        {step.kind === "choose" && (
          <>
            <p id="pmc-restore-lede" className="pmc-h2a-summary">
              {t("restore.choose.lede")}
            </p>
            <div className="pmc-h2a-actions">
              {recoveryBackupName !== undefined && (
                <button
                  type="button"
                  className="pmc-h2a-approve"
                  disabled={picking}
                  aria-busy={picking}
                  onClick={chooseRecovery}
                >
                  {t("restore.choose.recovery", { name: recoveryBackupName })}
                </button>
              )}
              <button
                type="button"
                className={recoveryBackupName === undefined ? "pmc-h2a-approve" : "pmc-h2a-reject"}
                disabled={picking}
                aria-busy={picking}
                onClick={choose}
              >
                {recoveryBackupName === undefined
                  ? t("restore.choose.button")
                  : t("restore.choose.other")}
              </button>
              <button type="button" className="pmc-h2a-reject" onClick={close}>
                {t("restore.cancel")}
              </button>
            </div>
          </>
        )}

        {(step.kind === "passphrase" || step.kind === "checking") && (
          <>
            <p id="pmc-restore-lede" className="pmc-h2a-summary">
              {t("restore.file", { name: step.fileName })}
            </p>
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-restore-passphrase">
              {t("restore.passphrase.label")}
            </label>
            <input
              id="pmc-restore-passphrase"
              className="pmc-h2b-confirmation-input"
              type="password"
              autoComplete="off"
              disabled={step.kind === "checking"}
              value={passphrase}
              onChange={(event) => {
                setPassphrase(event.target.value);
              }}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  check();
                }
              }}
            />
            {step.kind === "checking" && (
              <p role="status" className="pmc-backups-progress">
                {t("restore.checking")}
              </p>
            )}
            <div className="pmc-h2a-actions">
              <button
                type="button"
                className="pmc-h2a-approve"
                disabled={step.kind === "checking" || passphrase.length === 0}
                aria-busy={step.kind === "checking"}
                onClick={check}
              >
                {t("restore.passphrase.check")}
              </button>
              <button type="button" className="pmc-h2a-reject" onClick={close}>
                {t("restore.cancel")}
              </button>
            </div>
          </>
        )}

        {step.kind === "backingUp" && (
          <>
            <p id="pmc-restore-lede" className="pmc-h2a-summary">
              {t("restore.backup", {
                time: atRfc3339(step.checked.createdAt),
                schema: String(step.checked.schemaVersion),
                count: String(step.checked.recordCount),
              })}
            </p>
            <p role="status" className="pmc-backups-progress">
              {currentUnavailable ? t("restore.preserve.running") : t("restore.recovery.running")}
            </p>
          </>
        )}

        {(step.kind === "preview" || step.kind === "replacing") && (
          <RestorePreview preview={step.preview} />
        )}

        {step.kind === "preview" && (
          <>
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-restore-date">
              {t("restore.confirm.label", { date: step.preview.confirmationDate })}
            </label>
            <input
              id="pmc-restore-date"
              className="pmc-h2b-confirmation-input"
              type="text"
              inputMode="numeric"
              autoComplete="off"
              spellCheck={false}
              placeholder="YYYY-MM-DD"
              value={typedDate}
              onChange={(event) => {
                setTypedDate(event.target.value.trim());
              }}
            />
            <div className="pmc-h2a-actions">
              <button
                type="button"
                className="pmc-h2a-approve pmc-restore-replace"
                disabled={typedDate !== step.preview.confirmationDate || rejecting}
                onClick={replace}
              >
                {t("restore.confirm.button")}
              </button>
              <button
                type="button"
                className="pmc-h2a-reject"
                disabled={rejecting}
                aria-busy={rejecting}
                onClick={reject}
              >
                {t("restore.reject")}
              </button>
            </div>
          </>
        )}

        {step.kind === "replacing" && (
          <p role="status" className="pmc-backups-progress">
            {t("restore.replacing")}
          </p>
        )}

        {step.kind === "result" && (
          <RestoreResult
            preview={step.preview}
            result={step.result}
            onContinue={() => {
              onFinished(step.result);
            }}
          />
        )}

        {error !== null && (
          <SafeErrorDetail
            message={error.message}
            correlationId={error.correlationId}
            retryable={error.retryable}
            errorCode={error.errorCode}
          />
        )}
      </FocusTrapDialog>
    </div>
  );
}

function RestorePreview({ preview }: { readonly preview: RestorePreviewDto }) {
  const t = useT();
  return (
    <>
      <p id="pmc-restore-lede" className="pmc-h2a-summary">
        {t("restore.preview.lede")}
      </p>
      <dl className="pmc-h2a-fields">
        <div className="pmc-h2a-field">
          <dt>{t("restore.preview.backup")}</dt>
          <dd>
            {t("restore.backup", {
              time: atRfc3339(preview.archiveCreatedAt),
              schema: String(preview.archiveSchemaVersion),
              count: String(preview.archiveRecordCount),
            })}
          </dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("restore.preview.now")}</dt>
          <dd>
            {preview.currentRecordCount === null
              ? t("restore.now.unavailable")
              : preview.currentLastChangeAtMillis === null
                ? t("restore.now.noChange", { count: String(preview.currentRecordCount) })
                : t("restore.now", {
                    count: String(preview.currentRecordCount),
                    time: formatReadAt(preview.currentLastChangeAtMillis),
                  })}
          </dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("restore.preview.replaced")}</dt>
          <dd>{t("restore.replaced")}</dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("restore.preview.kept")}</dt>
          <dd>{t("restore.kept")}</dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("restore.preview.recovery")}</dt>
          <dd>
            {preview.recoveryKind === "preservation_copy"
              ? t("restore.recovery.preserved", { name: preview.recoveryName })
              : t("restore.recovery.done", {
                  time: formatReadAt(preview.recoveryVerifiedAtMillis),
                })}
          </dd>
        </div>
      </dl>
      {preview.needsUpgrade && <p className="pmc-h2a-summary">{t("restore.needsUpgrade")}</p>}
    </>
  );
}

function RestoreResult({
  preview,
  result,
  onContinue,
}: {
  readonly preview: RestorePreviewDto;
  readonly result: RestoreResultDto;
  readonly onContinue: () => void;
}) {
  const t = useT();
  // A preservation copy is named as such, never called a backup (§3.7).
  const preserved = preview.recoveryKind === "preservation_copy";
  const message =
    result.outcome === "restored"
      ? preserved
        ? t("restore.result.restoredPreserved", {
            time: atRfc3339(preview.archiveCreatedAt),
            name: preview.recoveryName,
          })
        : t("restore.result.restored", {
            time: atRfc3339(preview.archiveCreatedAt),
            recovery: formatReadAt(preview.recoveryVerifiedAtMillis),
          })
      : result.outcome === "failed_before_replacement"
        ? result.sourceChanged
          ? t("restore.result.sourceChanged")
          : t("restore.result.unchanged")
        : result.outcome === "recovery_put_back"
          ? preserved
            ? t("restore.result.putBackPreserved")
            : t("restore.result.putBack")
          : result.outcome === "interrupted"
            ? t("restore.result.interrupted")
            : t("restore.result.recoveryFailed");
  return (
    <>
      <p role="status" className="pmc-backups-result" tabIndex={-1} ref={(node) => node?.focus()}>
        {message}
      </p>
      <div className="pmc-h2a-actions">
        <button type="button" className="pmc-h2a-approve" onClick={onContinue}>
          {result.ledger !== "ready" ? t("restore.openSystemHealth") : t("restore.continue")}
        </button>
      </div>
    </>
  );
}
