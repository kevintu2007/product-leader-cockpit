import { useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { useT } from "../i18n/useT";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { BackupsSection, type BackupsSectionProps } from "./BackupsSection";
import type { RestoreActions, RestoreResultDto } from "./restoreIpc";
import { RestoreSheet } from "./RestoreSheet";
import type { UpgradeActions, UpgradeGateDto, UpgradeResultDto } from "./upgradeIpc";

export interface UpgradeGateProps {
  readonly gate: UpgradeGateDto;
  readonly actions: UpgradeActions;
  /** The backup folder and passphrase rows, as in Settings → Backups. */
  readonly backups: BackupsSectionProps;
  /** Upgraded and open: the shell takes over. */
  readonly onUpgraded: () => void;
  /** Upgraded but not openable, or the outcome is unknown. */
  readonly onOpenSystemHealth: () => void;
  /** The sample workspace's older data (`training_reset`): switch to your own
   * workspace, where the sample can be reset (item ⑨, §8). */
  readonly onSwitchToLive?: (() => Promise<unknown>) | undefined;
  /** "Restore from a backup…" beside Upgrade (DG3 restore-unopened
   * amendment §2, first cut): the Live workspace only. The host backs up the
   * closed Ledger file first, as the upgrade does. */
  readonly restore?:
    | {
        readonly actions: RestoreActions;
        readonly onFinished: (result: RestoreResultDto) => void;
      }
    | undefined;
}

type Phase =
  | { readonly kind: "ready" }
  | { readonly kind: "running" }
  | { readonly kind: "result"; readonly result: UpgradeResultDto };

/**
 * The Ledger upgrade gate (DG3 upgrade-gate amendment §2–§4), shown in place
 * of the shell. An older supported format gets the upgrade screen; an older
 * unsupported or a newer one says why it cannot open, with Quit.
 */
export function UpgradeGate({
  gate,
  actions,
  backups,
  onUpgraded,
  onOpenSystemHealth,
  restore,
  onSwitchToLive,
}: UpgradeGateProps) {
  const t = useT();
  const [phase, setPhase] = useState<Phase>({ kind: "ready" });
  const [restoring, setRestoring] = useState(false);
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  // Set before any re-render, so a double click starts one upgrade only.
  const inFlight = useRef(false);
  const status = backups.status;
  // Switching to your own workspace (`training_reset`): saved, then PMC
  // restarts; a failure says why and can be tried again.
  const [switching, setSwitching] = useState<"none" | "switching" | "restarting">("none");

  const quit = () => {
    void actions.quit();
  };

  const ready = status?.destination === "available" && status.passphrase !== "not_set";
  // An older unsupported Ledger does not inspect: a restore keeps a
  // preservation copy of its files first (restore-unopened amendment §2, §7).
  // Never for a newer one: no downgrade.
  const restorableBlocked = gate.state === "unsupported_old" && restore !== undefined;

  const switchToLive = () => {
    if (onSwitchToLive === undefined || inFlight.current) {
      return;
    }
    inFlight.current = true;
    setError(null);
    setSwitching("switching");
    onSwitchToLive().then(
      () => {
        setSwitching("restarting");
      },
      (reason: unknown) => {
        inFlight.current = false;
        setError(resolveRejection(reason, t));
        setSwitching("none");
      },
    );
  };

  if (gate.state !== "upgrade") {
    return (
      <main className="pmc-upgrade-gate" aria-labelledby="pmc-upgrade-title">
        <section className="pmc-lens-surface pmc-upgrade-panel">
          <h1 id="pmc-upgrade-title" className="pmc-page-headline" tabIndex={-1}>
            {t("upgrade.blocked.title")}
          </h1>
          <p className="pmc-page-lede">
            {gate.state === "newer_version"
              ? t("upgrade.newer")
              : gate.state === "training_reset"
                ? t("upgrade.trainingReset")
                : t("upgrade.unsupportedOld")}
          </p>
          {restorableBlocked && !ready && (
            <>
              <p className="pmc-h2a-summary">{t("health.restore.backupFirst")}</p>
              <BackupsSection {...backups} setupOnly />
            </>
          )}
          <div className="pmc-h2a-actions">
            {gate.state === "training_reset" && onSwitchToLive !== undefined && (
              <button
                type="button"
                className="pmc-h2a-approve"
                disabled={switching !== "none"}
                aria-busy={switching === "switching"}
                onClick={switchToLive}
              >
                {t("workspace.switchToLive")}
              </button>
            )}
            {restorableBlocked && (
              <button
                type="button"
                className="pmc-h2a-approve"
                disabled={!ready}
                onClick={() => {
                  setRestoring(true);
                }}
              >
                {t("restore.open")}
              </button>
            )}
            <button type="button" className="pmc-h2a-reject" onClick={quit}>
              {t("health.quit")}
            </button>
          </div>
          {switching === "switching" && (
            <p role="status" className="pmc-backups-progress">
              {t("firstRun.saving")}
            </p>
          )}
          {switching === "restarting" && (
            <p role="status" className="pmc-backups-progress">
              {t("workspace.restarting")}
            </p>
          )}
          {error !== null && (
            <SafeErrorDetail
              message={error.message}
              correlationId={error.correlationId}
              retryable={error.retryable}
              errorCode={error.errorCode}
            />
          )}
        </section>
        {restoring && restore !== undefined && (
          <RestoreSheet
            actions={restore.actions}
            currentUnavailable
            onClose={() => {
              setRestoring(false);
              backups.refresh();
            }}
            onFinished={(result) => {
              setRestoring(false);
              restore.onFinished(result);
            }}
          />
        )}
      </main>
    );
  }

  const upgrade = () => {
    if (!ready || inFlight.current) {
      return;
    }
    inFlight.current = true;
    setError(null);
    setPhase({ kind: "running" });
    // The host marks the backup as soon as it starts; read it back so the
    // screen can say which step is running.
    const started = setTimeout(backups.refresh, 500);
    actions
      .runUpgrade()
      .then(
        (result) => {
          setPhase({ kind: "result", result });
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
          setPhase({ kind: "ready" });
        },
      )
      .finally(() => {
        inFlight.current = false;
        clearTimeout(started);
        backups.refresh();
      });
  };

  return (
    <main className="pmc-upgrade-gate" aria-labelledby="pmc-upgrade-title">
      <section className="pmc-lens-surface pmc-upgrade-panel">
        <h1
          id="pmc-upgrade-title"
          className="pmc-page-headline"
          tabIndex={-1}
          ref={(node) => node?.focus()}
        >
          {t("upgrade.title")}
        </h1>
        <p className="pmc-page-lede">{t("upgrade.body", { version: gate.appVersion })}</p>
        <dl className="pmc-h2a-fields">
          <div className="pmc-h2a-field">
            <dt>{t("upgrade.fact.current")}</dt>
            <dd>{t("upgrade.schema", { schema: String(gate.fromSchema ?? "") })}</dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("upgrade.fact.new")}</dt>
            <dd>{t("upgrade.schema", { schema: String(gate.toSchema ?? "") })}</dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("upgrade.fact.records")}</dt>
            <dd>{String(gate.recordCount ?? 0)}</dd>
          </div>
          <div className="pmc-h2a-field">
            <dt>{t("upgrade.fact.lastBackup")}</dt>
            <dd>
              {status?.lastVerifiedAtMillis == null
                ? t("upgrade.fact.noBackup")
                : formatReadAt(status.lastVerifiedAtMillis)}
            </dd>
          </div>
        </dl>

        {phase.kind === "ready" && !ready && (
          <>
            <p className="pmc-h2a-summary">{t("upgrade.backupFirst")}</p>
            <BackupsSection {...backups} setupOnly />
          </>
        )}

        {phase.kind === "running" && (
          <p role="status" className="pmc-backups-progress">
            {status?.state === "backing_up" ? t("upgrade.backingUp") : t("upgrade.upgrading")}
          </p>
        )}

        {phase.kind === "result" && (
          <UpgradeResult
            result={phase.result}
            onUpgraded={onUpgraded}
            onRetry={upgrade}
            onQuit={quit}
            onOpenSystemHealth={onOpenSystemHealth}
            onRestore={
              restore === undefined
                ? undefined
                : () => {
                    setRestoring(true);
                  }
            }
          />
        )}

        {phase.kind === "ready" && (
          <div className="pmc-h2a-actions">
            <button type="button" className="pmc-h2a-approve" disabled={!ready} onClick={upgrade}>
              {t("upgrade.run")}
            </button>
            {restore !== undefined && (
              <button
                type="button"
                className="pmc-h2a-reject"
                disabled={!ready}
                onClick={() => {
                  setRestoring(true);
                }}
              >
                {t("restore.open")}
              </button>
            )}
            <button type="button" className="pmc-h2a-reject" onClick={quit}>
              {t("health.quit")}
            </button>
          </div>
        )}

        {error !== null && (
          <SafeErrorDetail
            message={error.message}
            correlationId={error.correlationId}
            retryable={error.retryable}
            errorCode={error.errorCode}
          />
        )}
      </section>
      {restoring && restore !== undefined && (
        <RestoreSheet
          actions={restore.actions}
          onClose={() => {
            setRestoring(false);
            backups.refresh();
          }}
          onFinished={(result) => {
            setRestoring(false);
            restore.onFinished(result);
          }}
        />
      )}
    </main>
  );
}

function UpgradeResult({
  result,
  onUpgraded,
  onRetry,
  onQuit,
  onOpenSystemHealth,
  onRestore,
}: {
  readonly result: UpgradeResultDto;
  readonly onUpgraded: () => void;
  readonly onRetry: () => void;
  readonly onQuit: () => void;
  readonly onOpenSystemHealth: () => void;
  /** Rolled back: the Ledger is as it was, and can be restored instead. */
  readonly onRestore?: (() => void) | undefined;
}) {
  const t = useT();
  const time = formatReadAt(result.backupVerifiedAtMillis);
  const message =
    result.outcome === "upgraded"
      ? t("upgrade.result.upgraded", { time })
      : result.outcome === "rolled_back"
        ? t("upgrade.result.rolledBack")
        : result.outcome === "upgraded_unreadable"
          ? t("upgrade.result.unreadable", { time })
          : t("upgrade.result.unknown", { time });
  return (
    <>
      <p role="status" className="pmc-backups-result" tabIndex={-1} ref={(node) => node?.focus()}>
        {message}
      </p>
      <div className="pmc-h2a-actions">
        {result.outcome === "upgraded" && result.ledger === "ready" && (
          <button type="button" className="pmc-h2a-approve" onClick={onUpgraded}>
            {t("restore.continue")}
          </button>
        )}
        {result.outcome === "rolled_back" && (
          <>
            <button type="button" className="pmc-h2a-approve" onClick={onRetry}>
              {t("upgrade.retry")}
            </button>
            {onRestore !== undefined && (
              <button type="button" className="pmc-h2a-reject" onClick={onRestore}>
                {t("restore.open")}
              </button>
            )}
            <button type="button" className="pmc-h2a-reject" onClick={onQuit}>
              {t("health.quit")}
            </button>
          </>
        )}
        {(result.outcome === "upgraded_unreadable" ||
          result.outcome === "outcome_unknown" ||
          (result.outcome === "upgraded" && result.ledger !== "ready")) && (
          <button type="button" className="pmc-h2a-approve" onClick={onOpenSystemHealth}>
            {t("restore.openSystemHealth")}
          </button>
        )}
      </div>
    </>
  );
}
