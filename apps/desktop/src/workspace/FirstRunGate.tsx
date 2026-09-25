import { useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { useT } from "../i18n/useT";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { WorkspaceActions, WorkspaceName } from "./workspaceIpc";

export interface FirstRunGateProps {
  readonly actions: Pick<WorkspaceActions, "chooseFirstWorkspace">;
}

type Phase =
  | { readonly kind: "choosing" }
  | { readonly kind: "working"; readonly choice: WorkspaceName }
  | { readonly kind: "restarting" };

/**
 * "Choose how to begin" (the accepted sample-workspace amendment §3): shown
 * once, to a profile that has never been used, before any workspace opens.
 * Both choices are equal and neither is preselected. Choosing saves the
 * choice and PMC restarts into it; sample data is prepared first, and if
 * that fails the choice stays unmade and can be made again.
 */
export function FirstRunGate({ actions }: FirstRunGateProps) {
  const t = useT();
  const [phase, setPhase] = useState<Phase>({ kind: "choosing" });
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  // Set before any re-render, so a double click makes one choice only.
  const inFlight = useRef(false);

  const choose = (choice: WorkspaceName) => {
    if (inFlight.current) {
      return;
    }
    inFlight.current = true;
    setError(null);
    setPhase({ kind: "working", choice });
    actions.chooseFirstWorkspace(choice).then(
      () => {
        setPhase({ kind: "restarting" });
      },
      (reason: unknown) => {
        inFlight.current = false;
        setError(resolveRejection(reason, t));
        setPhase({ kind: "choosing" });
      },
    );
  };

  const busy = phase.kind !== "choosing";

  return (
    <main className="pmc-upgrade-gate" aria-labelledby="pmc-first-run-title">
      <section className="pmc-lens-surface pmc-upgrade-panel">
        <h1
          id="pmc-first-run-title"
          className="pmc-page-headline"
          tabIndex={-1}
          ref={(node) => node?.focus()}
        >
          {t("firstRun.title")}
        </h1>
        <p className="pmc-page-lede">{t("firstRun.lede")}</p>
        <div className="pmc-first-run-choices">
          <button
            type="button"
            className="pmc-first-run-choice"
            disabled={busy}
            aria-busy={phase.kind === "working" && phase.choice === "live"}
            aria-describedby="pmc-first-run-live"
            onClick={() => {
              choose("live");
            }}
          >
            <span className="pmc-first-run-choice-title">{t("firstRun.live.title")}</span>
            <span id="pmc-first-run-live" className="pmc-first-run-choice-body">
              {t("firstRun.live.body")}
            </span>
            <span className="pmc-first-run-choice-note">{t("firstRun.live.setup")}</span>
          </button>
          <button
            type="button"
            className="pmc-first-run-choice"
            disabled={busy}
            aria-busy={phase.kind === "working" && phase.choice === "training"}
            aria-describedby="pmc-first-run-sample"
            onClick={() => {
              choose("training");
            }}
          >
            <span className="pmc-first-run-choice-title">{t("firstRun.sample.title")}</span>
            <span id="pmc-first-run-sample" className="pmc-first-run-choice-body">
              {t("firstRun.sample.body")}
            </span>
          </button>
        </div>
        {phase.kind === "working" && (
          <p role="status" className="pmc-backups-progress">
            {phase.choice === "training" ? t("firstRun.preparing") : t("firstRun.saving")}
          </p>
        )}
        {phase.kind === "restarting" && (
          <p role="status" className="pmc-backups-progress">
            {t("workspace.restarting")}
          </p>
        )}
        {error !== null && (
          <>
            <SafeErrorDetail
              message={error.message}
              correlationId={error.correlationId}
              retryable={error.retryable}
              errorCode={error.errorCode}
            />
            <p className="pmc-h2a-summary">{t("firstRun.tryAgain")}</p>
          </>
        )}
      </section>
    </main>
  );
}
