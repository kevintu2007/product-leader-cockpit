import { useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import type { MessageKey } from "../i18n/messages";
import { formatReadAt } from "../i18n/time";
import { useT } from "../i18n/useT";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { BackupActions, BackupStatusDto } from "./backupIpc";
import { PassphraseDialog } from "./PassphraseDialog";
import type { RestoreActions, RestoreResultDto } from "./restoreIpc";
import { RestoreSheet } from "./RestoreSheet";

export interface BackupsSectionProps {
  readonly actions: BackupActions;
  /** The shared status (the policy strip reads the same one). */
  readonly status: BackupStatusDto | undefined;
  readonly onStatus: (status: BackupStatusDto) => void;
  readonly refresh: () => void;
  /** "Restore from a backup…" (DG3 restore amendment §2); absent where
   * restore does not apply. */
  readonly restore?:
    | {
        readonly actions: RestoreActions;
        readonly onFinished: (result: RestoreResultDto) => void;
      }
    | undefined;
  /** Only the folder and passphrase rows (the upgrade gate's "Backup
   * first", DG3 upgrade-gate amendment §3). */
  readonly setupOnly?: boolean | undefined;
}

const PASSPHRASE_LABEL = {
  not_set: "backup.passphrase.notSet",
  session: "backup.passphrase.session",
  remembered: "backup.passphrase.remembered",
} as const satisfies Record<BackupStatusDto["passphrase"], MessageKey>;

/**
 * Settings → Backups (DG3 backup-setup amendment §2, §4): where backups go,
 * the recovery passphrase, and the last verified backup with "Back up now".
 * The folder is named by its own last component only; nothing here shows a
 * path or a passphrase the host holds.
 */
export function BackupsSection({
  actions,
  status,
  onStatus,
  refresh,
  restore,
  setupOnly = false,
}: BackupsSectionProps) {
  const t = useT();
  const [choosing, setChoosing] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  const [settingUp, setSettingUp] = useState(false);
  const [justBackedUp, setJustBackedUp] = useState(false);
  const [restoring, setRestoring] = useState(false);

  const busy = running || status?.state === "backing_up";

  const chooseFolder = () => {
    if (choosing) {
      return;
    }
    setChoosing(true);
    setError(null);
    actions
      .chooseBackupDestination(t("backup.folder.dialogTitle"))
      .then(
        () => {
          refresh();
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
        },
      )
      .finally(() => {
        setChoosing(false);
      });
  };

  const backUpNow = () => {
    if (busy) {
      return;
    }
    setRunning(true);
    setError(null);
    setJustBackedUp(false);
    // The host marks the run as soon as it starts; read it back so the
    // policy strip says "Backing up" while it runs.
    const started = setTimeout(refresh, 500);
    actions
      .runBackupNow()
      .then(
        (next) => {
          onStatus(next);
          setJustBackedUp(true);
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
          refresh();
        },
      )
      .finally(() => {
        clearTimeout(started);
        setRunning(false);
      });
  };

  if (status === undefined) {
    return (
      <section className="pmc-lens-surface pmc-settings-section" aria-labelledby="pmc-backups">
        <h3 id="pmc-backups">{t("backup.headline")}</h3>
        <p role="status">{t("backup.loading")}</p>
      </section>
    );
  }

  const destinationPill =
    status.destination === "not_set" ? (
      <span className="pmc-status-pill" data-tone="warning">
        {t("backup.folder.notSet")}
      </span>
    ) : (
      <>
        <span className="pmc-status-pill" data-tone="neutral">
          {t("backup.folder.set")}
        </span>{" "}
        <span
          className="pmc-status-pill"
          data-tone={status.destination === "available" ? "success" : "warning"}
        >
          {status.destination === "available"
            ? t("backup.folder.available")
            : t("backup.folder.unavailable")}
        </span>
      </>
    );

  return (
    <section
      className="pmc-lens-surface pmc-settings-section pmc-backups-section"
      aria-labelledby="pmc-backups"
    >
      <h3 id="pmc-backups">{t("backup.headline")}</h3>
      {status.state === "due" && <p className="pmc-backups-due">{t("backup.dueLede")}</p>}
      <dl className="pmc-h2a-fields pmc-settings-facts">
        <div className="pmc-h2a-field">
          <dt>{t("backup.folder.label")}</dt>
          <dd>
            {destinationPill}
            {status.folderName !== null && (
              <span className="pmc-backups-folder"> {status.folderName}</span>
            )}
            <div>
              <button
                type="button"
                className="pmc-button"
                disabled={choosing}
                aria-busy={choosing}
                onClick={chooseFolder}
              >
                {t("backup.folder.choose")}
              </button>
            </div>
          </dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("backup.passphrase.label")}</dt>
          <dd>
            <span
              className="pmc-status-pill"
              data-tone={status.passphrase === "not_set" ? "warning" : "success"}
            >
              {t(PASSPHRASE_LABEL[status.passphrase])}
            </span>
            <div>
              <button
                type="button"
                className="pmc-button"
                onClick={() => {
                  setSettingUp(true);
                }}
              >
                {t("backup.passphrase.setUp")}
              </button>
            </div>
          </dd>
        </div>
        {!setupOnly && (
          <div className="pmc-h2a-field">
            <dt>{t("backup.last.label")}</dt>
            <dd>
              {status.lastVerifiedAtMillis === null
                ? t("backup.last.none")
                : t("backup.last.at", {
                    time: formatReadAt(status.lastVerifiedAtMillis),
                    next: formatReadAt(status.nextDueAtMillis ?? status.lastVerifiedAtMillis),
                  })}
              {status.orphanCount > 0 && (
                <p className="pmc-backups-orphans">
                  {t("backup.orphans", { count: String(status.orphanCount) })}
                </p>
              )}
              <div>
                <button
                  type="button"
                  className="pmc-button pmc-button-primary"
                  disabled={busy || status.state === "checking"}
                  aria-busy={busy}
                  onClick={backUpNow}
                >
                  {t("backup.run")}
                </button>
              </div>
              {busy && (
                <p role="status" className="pmc-backups-progress">
                  {t("backup.running")}
                </p>
              )}
              {!busy && justBackedUp && status.lastVerifiedAtMillis !== null && (
                <p role="status" className="pmc-backups-result">
                  {t("backup.done", { time: formatReadAt(status.lastVerifiedAtMillis) })}
                </p>
              )}
              {restore !== undefined && (
                <div>
                  <button
                    type="button"
                    className="pmc-button"
                    disabled={busy || status.state === "restoring"}
                    onClick={() => {
                      setRestoring(true);
                    }}
                  >
                    {t("restore.open")}
                  </button>
                </div>
              )}
            </dd>
          </div>
        )}
      </dl>
      {error !== null && (
        <SafeErrorDetail
          message={error.message}
          correlationId={error.correlationId}
          retryable={error.retryable}
        />
      )}
      {restoring && restore !== undefined && (
        <RestoreSheet
          actions={restore.actions}
          onClose={() => {
            setRestoring(false);
            refresh();
          }}
          onFinished={(result) => {
            setRestoring(false);
            restore.onFinished(result);
          }}
        />
      )}
      {settingUp && (
        <PassphraseDialog
          actions={actions}
          onDone={() => {
            setSettingUp(false);
            refresh();
          }}
          onClose={() => {
            setSettingUp(false);
          }}
        />
      )}
    </section>
  );
}
