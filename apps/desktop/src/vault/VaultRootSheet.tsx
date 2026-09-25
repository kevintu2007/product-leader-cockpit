import { useEffect, useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { BackupsSection, type BackupsSectionProps } from "../backup/BackupsSection";
import { formatReadAt } from "../i18n/time";
import { useT } from "../i18n/useT";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { VaultRootActions, VaultRootPreviewDto, VaultRootResultDto } from "./vaultRootIpc";

export interface VaultRootSheetProps {
  readonly actions: VaultRootActions;
  /** The sheet closed without changing anything. */
  readonly onClose: () => void;
  /** An approval ran; the caller reads the Vault's state again. */
  readonly onFinished: (result: VaultRootResultDto) => void;
  /** Settings → Backups, so a person who has not set up backups yet can do
   * it here (product owner, 2026-09-23): the change backs up this workspace
   * first and cannot start without a folder and a passphrase. Absent in
   * tests that do not exercise it. */
  readonly backups?: BackupsSectionProps | undefined;
}

type Step =
  | { readonly kind: "choose" }
  | { readonly kind: "preparing"; readonly folderName: string }
  | { readonly kind: "preview"; readonly preview: VaultRootPreviewDto }
  | { readonly kind: "changing"; readonly preview: VaultRootPreviewDto }
  | {
      readonly kind: "result";
      readonly preview: VaultRootPreviewDto;
      readonly result: VaultRootResultDto;
    };

function newClientRequestId(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `vault-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** The typed text, compared as the person means it: spaces collapsed, and
 * letters in either case. */
function normalized(text: string): string {
  return text.trim().replace(/\s+/g, " ").toLocaleUpperCase();
}

/**
 * Settings → Product Vault → "Choose folder…" (DG3 Vault-root amendment §3,
 * O04 H2b): choose the folder, back up this workspace and check every
 * Evidence file under it, show the exact preview, then change the setting
 * only after the fixed phrase and this preview's code are typed.
 *
 * The sheet only enables the button when the text matches; the host checks
 * the phrase and the code again, because the confirmation is part of the
 * approval and the approval is the host's.
 */
export function VaultRootSheet({ actions, onClose, onFinished, backups }: VaultRootSheetProps) {
  const t = useT();
  const [step, setStep] = useState<Step>({ kind: "choose" });
  const [picking, setPicking] = useState(false);
  const [typed, setTyped] = useState("");
  const [rejecting, setRejecting] = useState(false);
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  // One request id per preview, so a retried approval is the same approval.
  const [clientRequestId, setClientRequestId] = useState(newClientRequestId);
  // Once the sheet is gone, a late answer from the picker must not start a
  // preview no screen could then reject.
  const mounted = useRef(true);
  // The preview this sheet holds and has not yet approved or rejected. When
  // the sheet goes away with one (the person navigated elsewhere), it is
  // withdrawn: a preview no screen shows would otherwise block every later
  // change until PMC restarts.
  const heldIntent = useRef<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (heldIntent.current !== null) {
        void actions.rejectVaultRootChange(heldIntent.current).catch(() => undefined);
        heldIntent.current = null;
      }
    };
  }, [actions]);

  // The native picker is open while `picking`: closing the sheet then would
  // leave its answer with nowhere to go.
  const locked = picking || step.kind === "preparing" || step.kind === "changing" || rejecting;
  const phrase = t("vaultRoot.confirm.phrase");
  // What the host's recovery backup needs before it can run. Only a guide:
  // the host refuses a change without them either way. Without the backup
  // status (tests), nothing is held back here.
  const backupReady =
    backups === undefined ||
    (backups.status?.destination === "available" && backups.status.passphrase !== "not_set");
  // When setup finishes here, the rows (and the button that had focus) go
  // away: hand focus to the choice they were holding back.
  const chooseButton = useRef<HTMLButtonElement>(null);
  const wasReady = useRef(backupReady);
  useEffect(() => {
    if (backupReady && !wasReady.current) {
      chooseButton.current?.focus();
    }
    wasReady.current = backupReady;
  }, [backupReady]);

  const reject = () => {
    if (step.kind !== "preview" || rejecting) {
      return;
    }
    setRejecting(true);
    heldIntent.current = null;
    actions
      .rejectVaultRootChange(step.preview.preparedIntentId)
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
    onClose();
  };

  const choose = () => {
    if (picking || !backupReady) {
      return;
    }
    setPicking(true);
    setError(null);
    actions
      .chooseVaultFolder(t("vaultRoot.choose.dialogTitle"))
      .then(
        (chosen) => {
          if (!mounted.current || !chosen.chosen || chosen.token === null) {
            return;
          }
          const folderName = chosen.folderName ?? "";
          setStep({ kind: "preparing", folderName });
          actions.prepareVaultRootChange(chosen.token).then(
            (preview) => {
              if (!mounted.current) {
                // Prepared after the sheet went away: withdraw it at once.
                void actions.rejectVaultRootChange(preview.preparedIntentId).catch(() => undefined);
                return;
              }
              heldIntent.current = preview.preparedIntentId;
              setTyped("");
              setClientRequestId(newClientRequestId());
              setStep({ kind: "preview", preview });
            },
            (reason: unknown) => {
              setError(resolveRejection(reason, t));
              setStep({ kind: "choose" });
            },
          );
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
        },
      )
      .finally(() => {
        setPicking(false);
      });
  };

  const expected =
    step.kind === "preview" ? normalized(`${phrase} ${step.preview.confirmationCode}`) : "";
  const confirmed = step.kind === "preview" && normalized(typed) === expected;

  const change = () => {
    if (step.kind !== "preview" || !confirmed) {
      return;
    }
    const { preview } = step;
    setError(null);
    // Approving: from here the host decides; leaving must not reject it.
    heldIntent.current = null;
    setStep({ kind: "changing", preview });
    actions
      // The whole confirmation as typed: the host checks the phrase and the
      // code itself.
      .approveVaultRootChange(
        preview.preparedIntentId,
        preview.payloadSha256,
        typed,
        clientRequestId,
      )
      .then(
        (result) => {
          setStep({ kind: "result", preview, result });
        },
        (reason: unknown) => {
          // Back on the preview, so held again: a refusal before the host
          // claimed it leaves it prepared. (Withdrawing one the host already
          // finished is refused harmlessly.)
          heldIntent.current = preview.preparedIntentId;
          setError(resolveRejection(reason, t));
          setStep({ kind: "preview", preview });
        },
      );
  };

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId="pmc-vault-root-title"
        descriptionId="pmc-vault-root-lede"
        onEscape={close}
        className="pmc-h2b-approval pmc-restore-sheet"
      >
        <h2 id="pmc-vault-root-title" className="pmc-section-title">
          {t("vaultRoot.title")}
        </h2>

        {step.kind === "choose" && (
          <>
            <p id="pmc-vault-root-lede" className="pmc-h2a-summary">
              {t("vaultRoot.choose.lede")}
            </p>
            {!backupReady && (
              <>
                <p className="pmc-h2a-summary">{t("vaultRoot.backupFirst")}</p>
                <BackupsSection {...backups} setupOnly />
              </>
            )}
            <div className="pmc-h2a-actions">
              <button
                ref={chooseButton}
                type="button"
                className="pmc-h2a-approve"
                disabled={picking || !backupReady}
                aria-busy={picking}
                onClick={choose}
              >
                {t("vaultRoot.choose.button")}
              </button>
              <button type="button" className="pmc-h2a-reject" onClick={close}>
                {t("vaultRoot.cancel")}
              </button>
            </div>
          </>
        )}

        {step.kind === "preparing" && (
          <>
            <p id="pmc-vault-root-lede" className="pmc-h2a-summary">
              {t("vaultRoot.folder", { name: step.folderName })}
            </p>
            <p role="status" className="pmc-backups-progress">
              {t("vaultRoot.preparing")}
            </p>
          </>
        )}

        {(step.kind === "preview" || step.kind === "changing") && (
          <VaultRootPreview preview={step.preview} />
        )}

        {step.kind === "preview" && (
          <>
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-vault-root-confirm">
              {t("vaultRoot.confirm.label", {
                phrase,
                code: step.preview.confirmationCode,
              })}
            </label>
            <input
              id="pmc-vault-root-confirm"
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
                onClick={change}
              >
                {t("vaultRoot.confirm.button")}
              </button>
              <button
                type="button"
                className="pmc-h2a-reject"
                disabled={rejecting}
                aria-busy={rejecting}
                onClick={reject}
              >
                {t("vaultRoot.reject")}
              </button>
            </div>
          </>
        )}

        {step.kind === "changing" && (
          <p role="status" className="pmc-backups-progress">
            {t("vaultRoot.changing")}
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
              {step.result.outcome === "changed"
                ? t("vaultRoot.result.changed", { name: step.preview.proposedFolderName })
                : t("vaultRoot.result.notChanged")}
            </p>
            <div className="pmc-h2a-actions">
              <button
                type="button"
                className="pmc-h2a-approve"
                onClick={() => {
                  onFinished(step.result);
                }}
              >
                {t("vaultRoot.done")}
              </button>
            </div>
          </>
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

function VaultRootPreview({ preview }: { readonly preview: VaultRootPreviewDto }) {
  const t = useT();
  return (
    <>
      <p id="pmc-vault-root-lede" className="pmc-h2a-summary">
        {t("vaultRoot.preview.lede")}
      </p>
      <dl className="pmc-h2a-fields">
        <div className="pmc-h2a-field">
          <dt>{t("vaultRoot.preview.now")}</dt>
          <dd>{preview.previousFolderName ?? t("settings.vaultNotSet")}</dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("vaultRoot.preview.new")}</dt>
          <dd>{preview.proposedFolderName}</dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("vaultRoot.preview.evidence")}</dt>
          <dd>
            {preview.evidenceCount === 0
              ? t("vaultRoot.preview.noEvidence")
              : t("vaultRoot.preview.resolved", {
                  resolved: String(preview.resolvedCount),
                  count: String(preview.evidenceCount),
                })}
          </dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("vaultRoot.preview.recovery")}</dt>
          <dd>
            {t("vaultRoot.preview.recoveryVerified", {
              time: formatReadAt(preview.recoveryVerifiedAtMillis),
            })}
          </dd>
        </div>
      </dl>
    </>
  );
}
