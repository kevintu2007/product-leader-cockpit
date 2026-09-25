import { useEffect, useState } from "react";

import type { BackupStatusDto } from "../backup/backupIpc";
import type { MessageKey } from "../i18n/messages";
import { useT } from "../i18n/useT";
import type { VaultStatusDto } from "./cockpitContract";

/** What the checklist needs from outside the Cockpit. */
export interface GettingStartedSources {
  /** The backup status the app already reads; `undefined` while unknown. */
  readonly backup: BackupStatusDto | undefined;
  /** The newest backup-status read failed: unknown, not "not read yet". */
  readonly backupReadFailed: boolean;
  readonly loadVaultStatus: () => Promise<VaultStatusDto>;
  readonly onOpenSettings: () => void;
  readonly onOpenPortfolio: () => void;
}

export interface GettingStartedProps extends GettingStartedSources {
  /** How many Products the Cockpit's own read lists. */
  readonly productCount: number;
}

type StepState = "done" | "open" | "unknown";
type StepKey = "backupFolder" | "passphrase" | "firstBackup" | "vault" | "firstProduct";

interface Step {
  readonly key: StepKey;
  readonly state: StepState;
  readonly opens: "settings" | "portfolio";
}

const TITLE = {
  backupFolder: "gettingStarted.backupFolder",
  passphrase: "gettingStarted.passphrase",
  firstBackup: "gettingStarted.firstBackup",
  vault: "gettingStarted.vault",
  firstProduct: "gettingStarted.firstProduct",
} as const satisfies Record<StepKey, MessageKey>;

const WHY = {
  backupFolder: "gettingStarted.backupFolder.why",
  passphrase: "gettingStarted.passphrase.why",
  firstBackup: "gettingStarted.firstBackup.why",
  vault: "gettingStarted.vault.why",
  firstProduct: "gettingStarted.firstProduct.why",
} as const satisfies Record<StepKey, MessageKey>;

/** Each tick from a status the host reports; nothing here is stored. */
function steps(
  backup: BackupStatusDto | undefined,
  vault: VaultStatusDto | "unknown" | undefined,
  productCount: number,
): readonly Step[] {
  const fromBackup = (done: (status: BackupStatusDto) => boolean): StepState =>
    backup === undefined ? "unknown" : done(backup) ? "done" : "open";
  return [
    {
      key: "backupFolder",
      state: fromBackup((status) => status.destination !== "not_set"),
      opens: "settings",
    },
    {
      key: "passphrase",
      state: fromBackup((status) => status.passphrase !== "not_set"),
      opens: "settings",
    },
    {
      key: "firstBackup",
      state: fromBackup((status) => status.lastVerifiedAtMillis !== null),
      opens: "settings",
    },
    {
      key: "vault",
      state:
        vault === undefined || vault === "unknown" ? "unknown" : vault.configured ? "done" : "open",
      opens: "settings",
    },
    { key: "firstProduct", state: productCount > 0 ? "done" : "open", opens: "portfolio" },
  ];
}

/**
 * Getting started (the accepted getting-started amendment): the setup of
 * your own workspace in order. Every tick is read from the system's state;
 * only the first step not done can be opened, and the list is gone once all
 * five are done. No progress bar or count (DG1).
 */
export function GettingStarted({
  backup,
  backupReadFailed,
  loadVaultStatus,
  onOpenSettings,
  onOpenPortfolio,
  productCount,
}: GettingStartedProps) {
  const t = useT();
  const [vault, setVault] = useState<VaultStatusDto | "unknown" | undefined>(undefined);

  useEffect(() => {
    let live = true;
    loadVaultStatus().then(
      (status) => {
        if (live) setVault(status);
      },
      () => {
        if (live) setVault("unknown");
      },
    );
    return () => {
      live = false;
    };
  }, [loadVaultStatus]);

  // Nothing until both statuses have been read: a setup that is complete
  // must not flash a checklist on every visit. A read that failed shows as
  // "cannot tell yet" rather than as not done.
  if ((backup === undefined && !backupReadFailed) || vault === undefined) {
    return null;
  }
  // A failed newest read makes the backup steps unknown, even when an older
  // status is still held: it may no longer be true.
  const list = steps(backupReadFailed ? undefined : backup, vault, productCount);
  if (list.every((step) => step.state === "done")) {
    return null;
  }
  // The first step not done is the next one; a step PMC cannot read yet
  // claims nothing, and nothing after it is offered.
  const nextIndex = list.findIndex((step) => step.state !== "done");

  return (
    <section className="pmc-lens-surface pmc-getting-started" aria-labelledby="pmc-getting-started">
      <h3 id="pmc-getting-started">{t("gettingStarted.title")}</h3>
      <p className="pmc-h2a-summary">{t("gettingStarted.lede")}</p>
      <ol className="pmc-getting-started-steps">
        {list.map((step, index) => {
          const isNext = index === nextIndex;
          const status =
            step.state === "done"
              ? t("gettingStarted.status.done")
              : step.state === "unknown"
                ? t("gettingStarted.status.unknown")
                : isNext
                  ? t("gettingStarted.status.next")
                  : t("gettingStarted.status.waits");
          return (
            <li
              key={step.key}
              className="pmc-getting-started-step"
              data-state={step.state === "done" ? "done" : isNext ? "next" : "waiting"}
              aria-current={isNext && step.state === "open" ? "step" : undefined}
            >
              <span className="pmc-getting-started-mark" aria-hidden="true" />
              <div className="pmc-getting-started-text">
                <span className="pmc-getting-started-title">{t(TITLE[step.key])}</span>
                <span className="pmc-getting-started-why">{t(WHY[step.key])}</span>
              </div>
              <span className="pmc-getting-started-status">{status}</span>
              {isNext && step.state === "open" && (
                <button
                  type="button"
                  className="pmc-button pmc-button-primary"
                  onClick={step.opens === "settings" ? onOpenSettings : onOpenPortfolio}
                >
                  {step.opens === "settings"
                    ? t("gettingStarted.openSettings")
                    : t("gettingStarted.openPortfolio")}
                </button>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
