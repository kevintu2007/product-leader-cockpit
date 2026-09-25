import { useEffect, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { BackupsSection, type BackupsSectionProps } from "../backup/BackupsSection";
import type { SystemHealthActions, SystemHealthDto } from "../backup/restoreIpc";
import { RestoreSheet } from "../backup/RestoreSheet";
import type { MessageKey } from "../i18n/messages";
import { useT } from "../i18n/useT";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";

export interface SystemHealthProps {
  readonly actions: SystemHealthActions;
  /** Settings → Backups, with its restore when the workspace is Live: "Restore
   * from a backup…" for a Ledger that does not open or a failed recovery left
   * uncertain (DG3 restore-unopened amendment §2). */
  readonly backups?: BackupsSectionProps | undefined;
}

const LEDGER_STATE = {
  ready: "health.ledger.ready",
  replacing: "health.ledger.replacing",
  restore_recovery_required: "health.ledger.restore_recovery_required",
  upgrade_required: "health.ledger.upgrade_required",
  unsupported_old: "health.ledger.unsupported_old",
  newer_version: "health.ledger.newer_version",
  open_failed: "health.ledger.open_failed",
  first_run: "health.ledger.first_run",
} as const satisfies Record<SystemHealthDto["ledger"], MessageKey>;

const SAMPLE_REASON = {
  unresolved: "health.sample.unresolved",
  missing: "health.sample.missing",
  foreign: "health.sample.foreign",
} as const satisfies Record<NonNullable<SystemHealthDto["sample"]>, MessageKey>;

type Loaded =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly health: SystemHealthDto };

/**
 * S11 System Health, limited to what the host can answer now: whether the
 * Product Ledger is open, why not, and — when a restore could not be put
 * back — which recovery backup to restore. "Quit" is offered whenever the
 * Ledger is not open; "Restore from a backup…" when it does not open or a
 * failed recovery left it uncertain, beside the backup folder and
 * passphrase rows while either is missing (restore-unopened amendment §2).
 */
export function SystemHealth({ actions, backups }: SystemHealthProps) {
  const t = useT();
  const [loaded, setLoaded] = useState<Loaded>({ status: "loading" });
  const [restoring, setRestoring] = useState(false);
  const restore = backups?.restore;
  const backupReady =
    backups?.status?.destination === "available" && backups.status.passphrase !== "not_set";

  useEffect(() => {
    let live = true;
    actions.loadSystemHealth().then(
      (health) => {
        if (live) {
          setLoaded({ status: "ready", health });
        }
      },
      (reason: unknown) => {
        if (live) {
          setLoaded({ status: "error", error: resolveRejection(reason, t) });
        }
      },
    );
    return () => {
      live = false;
    };
  }, [actions, t]);

  return (
    <div className="pmc-settings">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("health.headline")}</h2>
        </div>
      </header>
      <section
        className="pmc-lens-surface pmc-settings-section"
        aria-labelledby="pmc-health-ledger"
      >
        <h3 id="pmc-health-ledger">{t("health.ledger.label")}</h3>
        {loaded.status === "loading" && <p role="status">{t("health.loading")}</p>}
        {loaded.status === "error" && (
          <SafeErrorDetail
            message={t("health.unavailable", { message: loaded.error.message })}
            correlationId={loaded.error.correlationId}
            retryable={loaded.error.retryable}
          />
        )}
        {loaded.status === "ready" && (
          <>
            <p>
              <span
                className="pmc-status-pill"
                data-tone={loaded.health.ledger === "ready" ? "success" : "warning"}
              >
                {t(LEDGER_STATE[loaded.health.ledger])}
              </span>
            </p>
            {loaded.health.settings !== "ok" && (
              <p className="pmc-h2a-summary">
                {loaded.health.settings === "set_aside"
                  ? t("health.settings.setAside")
                  : t("health.settings.unavailable")}
              </p>
            )}
            {loaded.health.sample !== null && (
              <p className="pmc-h2a-summary">{t(SAMPLE_REASON[loaded.health.sample])}</p>
            )}
            {loaded.health.sampleCleanupPending > 0 && (
              <p className="pmc-h2a-summary">
                {t("health.sample.cleanupPending", {
                  count: String(loaded.health.sampleCleanupPending),
                })}
              </p>
            )}
            {loaded.health.recoveryBackup !== null && (
              <p className="pmc-health-recovery">
                {t("health.recoveryBackup", { name: loaded.health.recoveryBackup })}
              </p>
            )}
            {restore !== undefined &&
              backups !== undefined &&
              (loaded.health.ledger === "open_failed" ||
                loaded.health.ledger === "restore_recovery_required") && (
                <>
                  {!backupReady && (
                    <>
                      <p className="pmc-h2a-summary">{t("health.restore.backupFirst")}</p>
                      <BackupsSection {...backups} restore={undefined} setupOnly />
                    </>
                  )}
                  <div>
                    <button
                      type="button"
                      className="pmc-button pmc-button-primary"
                      disabled={!backupReady}
                      onClick={() => {
                        setRestoring(true);
                      }}
                    >
                      {t("restore.open")}
                    </button>
                  </div>
                </>
              )}
            {loaded.health.ledger !== "ready" && loaded.health.ledger !== "replacing" && (
              <div>
                <button
                  type="button"
                  className="pmc-button"
                  onClick={() => {
                    void actions.quit();
                  }}
                >
                  {t("health.quit")}
                </button>
              </div>
            )}
          </>
        )}
      </section>
      {restoring && restore !== undefined && (
        <RestoreSheet
          actions={restore.actions}
          currentUnavailable
          recoveryBackupName={
            loaded.status === "ready" &&
            loaded.health.ledger === "restore_recovery_required" &&
            loaded.health.recoveryBackup !== null
              ? loaded.health.recoveryBackup
              : undefined
          }
          onClose={() => {
            setRestoring(false);
            backups?.refresh();
          }}
          onFinished={() => {
            // Whatever happened, the next start classifies the Ledger again.
            window.location.reload();
          }}
        />
      )}
    </div>
  );
}
