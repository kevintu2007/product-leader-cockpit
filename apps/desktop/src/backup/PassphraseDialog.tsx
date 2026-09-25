import { useEffect, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { useT } from "../i18n/useT";
import type { BackupActions, PassphraseStatusDto } from "./backupIpc";

export interface PassphraseDialogProps {
  readonly actions: Pick<BackupActions, "generateRecoveryPassphrase" | "setBackupPassphrase">;
  readonly onDone: (status: PassphraseStatusDto) => void;
  /** Closing without confirming changes nothing. */
  readonly onClose: () => void;
}

type Mode = "generated" | "own";

/**
 * Passphrase setup (DG3 backup-setup amendment §3; ADR 0010 §7–§8). The
 * default is a generated ten-word passphrase shown once; the person types it
 * back and acknowledges that PMC cannot recover it. "Use my own instead"
 * swaps to two fields. "Remember" is unticked by default. The host checks
 * strength and stores it; this dialog only relays what the person typed.
 */
export function PassphraseDialog({ actions, onDone, onClose }: PassphraseDialogProps) {
  const t = useT();
  const [mode, setMode] = useState<Mode>("generated");
  const [generated, setGenerated] = useState<string | null>(null);
  const [generateError, setGenerateError] = useState<ResolvedSafeError | null>(null);
  const [retyped, setRetyped] = useState("");
  const [own, setOwn] = useState("");
  const [ownAgain, setOwnAgain] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);
  const [remember, setRemember] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<ResolvedSafeError | null>(null);

  // A new passphrase each time the generated path is shown: the one shown
  // before is never shown again (DG3 amendment §3, "shown once").
  const [round, setRound] = useState(0);

  // Only while the generated path is open: leaving it discards any answer
  // still on its way, so an old passphrase can never reappear.
  useEffect(() => {
    if (mode !== "generated") {
      return;
    }
    let live = true;
    actions.generateRecoveryPassphrase().then(
      (words) => {
        if (live) {
          setGenerated(words);
        }
      },
      (reason: unknown) => {
        if (live) {
          setGenerateError(resolveRejection(reason, t));
        }
      },
    );
    return () => {
      live = false;
    };
  }, [actions, t, round, mode]);

  // While the host is storing it, the dialog cannot be closed.
  const close = () => {
    if (!saving) {
      onClose();
    }
  };

  const chosen = mode === "generated" ? generated : own;
  const matches =
    mode === "generated"
      ? generated !== null && retyped === generated
      : own.length > 0 && own === ownAgain;
  const canConfirm = matches && acknowledged && !saving && chosen !== null;

  const confirm = () => {
    if (!canConfirm) {
      return;
    }
    setSaving(true);
    setSaveError(null);
    actions.setBackupPassphrase(chosen, remember).then(
      (status) => {
        onDone(status);
      },
      (reason: unknown) => {
        setSaving(false);
        setSaveError(resolveRejection(reason, t));
      },
    );
  };

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId="pmc-passphrase-title"
        descriptionId="pmc-passphrase-lede"
        onEscape={close}
        className="pmc-h2b-approval pmc-passphrase-dialog"
      >
        <h2 id="pmc-passphrase-title" className="pmc-section-title">
          {t("backup.passphrase.title")}
        </h2>
        {mode === "generated" ? (
          <>
            <p id="pmc-passphrase-lede" className="pmc-h2a-summary">
              {t("backup.passphrase.generatedLede")}
            </p>
            {generated !== null ? (
              <p className="pmc-passphrase-words" lang="en">
                {generated}
              </p>
            ) : generateError !== null ? (
              <SafeErrorDetail
                message={generateError.message}
                correlationId={generateError.correlationId}
                retryable={generateError.retryable}
              />
            ) : (
              <p role="status">{t("backup.passphrase.generating")}</p>
            )}
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-passphrase-retype">
              {t("backup.passphrase.retype")}
            </label>
            <input
              id="pmc-passphrase-retype"
              className="pmc-h2b-confirmation-input"
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={retyped}
              onChange={(event) => {
                setRetyped(event.target.value);
              }}
            />
            <button
              type="button"
              className="pmc-link-button"
              onClick={() => {
                setMode("own");
                setRetyped("");
                setGenerated(null);
              }}
            >
              {t("backup.passphrase.useOwn")}
            </button>
          </>
        ) : (
          <>
            <p id="pmc-passphrase-lede" className="pmc-h2a-summary">
              {t("backup.passphrase.ownRule")}
            </p>
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-passphrase-own">
              {t("backup.passphrase.own")}
            </label>
            <input
              id="pmc-passphrase-own"
              className="pmc-h2b-confirmation-input"
              type="password"
              autoComplete="new-password"
              value={own}
              onChange={(event) => {
                setOwn(event.target.value);
              }}
            />
            <label className="pmc-h2b-confirmation-label" htmlFor="pmc-passphrase-own-again">
              {t("backup.passphrase.ownAgain")}
            </label>
            <input
              id="pmc-passphrase-own-again"
              className="pmc-h2b-confirmation-input"
              type="password"
              autoComplete="new-password"
              value={ownAgain}
              onChange={(event) => {
                setOwnAgain(event.target.value);
              }}
            />
            <button
              type="button"
              className="pmc-link-button"
              onClick={() => {
                setGenerated(null);
                setGenerateError(null);
                setMode("generated");
                setOwn("");
                setOwnAgain("");
                setRetyped("");
                setRound((previous) => previous + 1);
              }}
            >
              {t("backup.passphrase.useGenerated")}
            </button>
          </>
        )}
        <label className="pmc-passphrase-check">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(event) => {
              setAcknowledged(event.target.checked);
            }}
          />
          {t("backup.passphrase.acknowledge")}
        </label>
        <label className="pmc-passphrase-check">
          <input
            type="checkbox"
            checked={remember}
            onChange={(event) => {
              setRemember(event.target.checked);
            }}
          />
          {t("backup.passphrase.remember")}
        </label>
        {saveError !== null && (
          <SafeErrorDetail
            message={saveError.message}
            correlationId={saveError.correlationId}
            retryable={saveError.retryable}
          />
        )}
        <div className="pmc-h2a-actions">
          <button
            type="button"
            className="pmc-h2a-approve"
            disabled={!canConfirm}
            aria-busy={saving}
            onClick={confirm}
          >
            {t("backup.passphrase.confirm")}
          </button>
          <button type="button" className="pmc-h2a-reject" disabled={saving} onClick={close}>
            {t("backup.passphrase.cancel")}
          </button>
        </div>
      </FocusTrapDialog>
    </div>
  );
}
