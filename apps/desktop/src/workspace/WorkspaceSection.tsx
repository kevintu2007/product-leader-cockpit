import { useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { useT } from "../i18n/useT";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { SampleDeleteSheet } from "./SampleDeleteSheet";
import {
  newClientRequestId,
  type SampleDeletedDto,
  type WorkspaceActions,
  type WorkspaceStatusDto,
} from "./workspaceIpc";

export interface WorkspaceSectionProps {
  readonly status: WorkspaceStatusDto;
  readonly actions: WorkspaceActions;
  /** Something about the sample changed; read the status again. */
  readonly onChanged: () => void;
}

type Pending =
  | { readonly kind: "none" }
  | { readonly kind: "confirmSwitch" }
  | { readonly kind: "switching" }
  | { readonly kind: "restarting" }
  | { readonly kind: "confirmReset"; readonly clientRequestId: string }
  | { readonly kind: "resetting"; readonly clientRequestId: string }
  | { readonly kind: "reset" }
  | { readonly kind: "deleting" }
  | { readonly kind: "deleted"; readonly result: SampleDeletedDto };

/**
 * Settings → Workspace (the accepted sample-workspace amendment §4 and §8):
 * which workspace is open, switching to the other (PMC restarts), and — from
 * your own workspace only — resetting (H1) or deleting (H2b) the sample data.
 */
export function WorkspaceSection({ status, actions, onChanged }: WorkspaceSectionProps) {
  const t = useT();
  const [pending, setPending] = useState<Pending>({ kind: "none" });
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  const inFlight = useRef(false);
  const sampleOpen = status.open === "training";
  const target = sampleOpen ? "live" : "training";
  const busy =
    pending.kind === "switching" || pending.kind === "restarting" || pending.kind === "resetting";

  const switchWorkspace = () => {
    if (inFlight.current) {
      return;
    }
    inFlight.current = true;
    setError(null);
    setPending({ kind: "switching" });
    actions.switchWorkspace(target).then(
      () => {
        setPending({ kind: "restarting" });
      },
      (reason: unknown) => {
        inFlight.current = false;
        setError(resolveRejection(reason, t));
        setPending({ kind: "none" });
      },
    );
  };

  const reset = (clientRequestId: string) => {
    if (inFlight.current) {
      return;
    }
    inFlight.current = true;
    setError(null);
    setPending({ kind: "resetting", clientRequestId });
    actions
      .resetSampleData(clientRequestId)
      .then(
        () => {
          setPending({ kind: "reset" });
          onChanged();
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
          // The same request again, so a retry is the same reset.
          setPending({ kind: "confirmReset", clientRequestId });
        },
      )
      .finally(() => {
        inFlight.current = false;
      });
  };

  return (
    <section
      className="pmc-lens-surface pmc-settings-section pmc-workspace-section"
      aria-labelledby="pmc-settings-workspace-choice"
    >
      <h3 id="pmc-settings-workspace-choice">{t("workspace.title")}</h3>
      <p className="pmc-h2a-summary">
        {sampleOpen ? t("workspace.openSample") : t("workspace.openLive")}
      </p>
      {status.sampleFellBack && <p className="pmc-h2a-summary">{t("workspace.sampleFellBack")}</p>}
      {/* Switching would be refused: say why before it is tried. */}
      {!sampleOpen && status.sample === "foreign" && (
        <p className="pmc-h2a-summary">{t("health.sample.foreign")}</p>
      )}
      {!sampleOpen && status.sample === "unresolved" && (
        <p className="pmc-h2a-summary">{t("health.sample.unresolved")}</p>
      )}

      {pending.kind === "confirmSwitch" ? (
        <div className="pmc-workspace-confirm" role="group" aria-label={t("workspace.title")}>
          <p className="pmc-h2a-summary">
            {sampleOpen
              ? t("workspace.switchToLive.confirm")
              : t("workspace.switchToSample.confirm")}
          </p>
          <div className="pmc-h2a-buttons">
            <button
              type="button"
              className="pmc-button pmc-button-primary"
              onClick={switchWorkspace}
            >
              {t("workspace.switchAndRestart")}
            </button>
            <button
              type="button"
              className="pmc-button"
              onClick={() => {
                setPending({ kind: "none" });
              }}
            >
              {t("sampleWorkspace.cancel")}
            </button>
          </div>
        </div>
      ) : (
        <div className="pmc-h2a-buttons">
          <button
            type="button"
            className="pmc-button"
            disabled={busy}
            onClick={() => {
              setError(null);
              setPending({ kind: "confirmSwitch" });
            }}
          >
            {sampleOpen ? t("workspace.switchToLive") : t("workspace.switchToSample")}
          </button>
        </div>
      )}
      {pending.kind === "switching" && (
        <p role="status" className="pmc-backups-progress">
          {target === "training" ? t("firstRun.preparing") : t("firstRun.saving")}
        </p>
      )}
      {pending.kind === "restarting" && (
        <p role="status" className="pmc-backups-progress">
          {t("workspace.restarting")}
        </p>
      )}

      <h4 className="pmc-workspace-subtitle">{t("sampleWorkspace.manage")}</h4>
      {sampleOpen ? (
        <p className="pmc-h2a-summary">{t("sampleWorkspace.fromLiveOnly")}</p>
      ) : (
        <>
          {pending.kind === "confirmReset" || pending.kind === "resetting" ? (
            <div
              className="pmc-workspace-confirm"
              role="group"
              aria-label={t("sampleWorkspace.reset")}
            >
              <p className="pmc-h2a-summary">{t("sampleWorkspace.reset.confirm")}</p>
              <div className="pmc-h2a-buttons">
                <button
                  type="button"
                  className="pmc-button pmc-button-primary"
                  disabled={pending.kind === "resetting"}
                  aria-busy={pending.kind === "resetting"}
                  onClick={() => {
                    reset(pending.clientRequestId);
                  }}
                >
                  {t("sampleWorkspace.reset")}
                </button>
                <button
                  type="button"
                  className="pmc-button"
                  disabled={pending.kind === "resetting"}
                  onClick={() => {
                    setError(null);
                    setPending({ kind: "none" });
                  }}
                >
                  {t("sampleWorkspace.cancel")}
                </button>
              </div>
              {pending.kind === "resetting" && (
                <p role="status" className="pmc-backups-progress">
                  {t("sampleWorkspace.resetting")}
                </p>
              )}
            </div>
          ) : (
            <div className="pmc-h2a-buttons">
              <button
                type="button"
                className="pmc-button"
                disabled={busy}
                onClick={() => {
                  setError(null);
                  setPending({ kind: "confirmReset", clientRequestId: newClientRequestId() });
                }}
              >
                {t("sampleWorkspace.reset.open")}
              </button>
              <button
                type="button"
                className="pmc-button"
                disabled={busy}
                onClick={() => {
                  setError(null);
                  setPending({ kind: "deleting" });
                }}
              >
                {t("sampleWorkspace.delete.open")}
              </button>
            </div>
          )}
          {pending.kind === "reset" && (
            <p role="status" className="pmc-backups-result">
              {t("sampleWorkspace.reset.done")}
            </p>
          )}
          {pending.kind === "deleted" && (
            <p role="status" className="pmc-backups-result">
              {pending.result.outcome === "deleted"
                ? t("sampleWorkspace.delete.deleted")
                : t("sampleWorkspace.delete.notDeleted")}
            </p>
          )}
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

      {pending.kind === "deleting" && (
        <SampleDeleteSheet
          actions={actions}
          onClose={() => {
            setPending({ kind: "none" });
          }}
          onFinished={(result) => {
            setPending({ kind: "deleted", result });
            onChanged();
          }}
        />
      )}
    </section>
  );
}
