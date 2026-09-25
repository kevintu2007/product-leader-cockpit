import { useCallback, useEffect, useState } from "react";

import type { Translator } from "../i18n/messages";
import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatLocalDate } from "../i18n/time";
import { classificationName, evidenceRoleLabel, verificationLabel } from "../i18n/workLabels";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { EvidenceReferencesDto, VaultStatusDto } from "./cockpitContract";
import { findRoute } from "../shell/routes";
import { EvidenceFromFileSheet } from "../evidence/EvidenceFromFileSheet";
import type { EvidenceFileActions } from "../evidence/evidenceFileIpc";

/**
 * S05 Product Vault, limited to what the host can already answer: whether
 * the Vault can be read right now, and the state of every Evidence reference
 * the Ledger holds. No Vault path reaches this surface. Its one write is
 * "Add Evidence from a file…" (DG3 Vault-root and Evidence-from-file
 * amendment §4), create only; pinning, re-observing and linking stay in the
 * O01 inspector, next to the Product the Evidence is linked to.
 *
 * Projection health is not shown: no command reports it yet, and a status
 * this surface cannot back would be a claim, not a fact.
 */
export interface VaultProps {
  readonly loadVaultStatus: () => Promise<VaultStatusDto>;
  readonly loadEvidenceReferences: () => Promise<EvidenceReferencesDto>;
  /** Absent means this page offers no write. */
  readonly evidenceFile?: EvidenceFileActions | undefined;
}

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | {
      readonly status: "ready";
      readonly vault: VaultStatusDto;
      readonly evidence: EvidenceReferencesDto;
    };

/** Why the Vault can't be used, in words; an unknown reason says so. */
function vaultReason(t: Translator, reason: string | null): string {
  return reason === "notConfigured" || reason === "invalidRoot" || reason === "changeUnresolved"
    ? t(`vault.reason.${reason}`)
    : t("vault.reason.unknown");
}

function formatDate(millis: number | null): string {
  return millis === null ? "" : formatLocalDate(millis);
}

export function Vault({ loadVaultStatus, loadEvidenceReferences, evidenceFile }: VaultProps) {
  const t = useT();
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [fileSheetOpen, setFileSheetOpen] = useState(false);

  const fetchAll = useCallback(() => {
    Promise.all([loadVaultStatus(), loadEvidenceReferences()]).then(
      ([vault, evidence]) => {
        setState({ status: "ready", vault, evidence });
      },
      (reason: unknown) => {
        setState({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [loadVaultStatus, loadEvidenceReferences, t]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    fetchAll();
  }, [fetchAll]);

  useEffect(() => {
    fetchAll();
  }, [fetchAll]);

  if (state.status === "loading") {
    return (
      <p className="pmc-cockpit-status" role="status">
        {t("route.loading", { route: findRoute("product-vault").label })}
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="pmc-cockpit-status">
        <button type="button" className="pmc-button" onClick={retry}>
          {t("route.reload")}
        </button>
        <SafeErrorDetail
          message={t("route.unavailable", {
            route: findRoute("product-vault").label,
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { vault, evidence } = state;
  const references = evidence.evidenceReferences;

  return (
    <div className="pmc-vault">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("vault.headline")}</h2>
          <p className="pmc-page-lede">{t("vault.lede")}</p>
        </div>
        <dl className="pmc-meta pmc-page-asof">
          <div>
            <dt>{t("route.ledgerRevision")}</dt>
            <dd>{String(evidence.ledgerRevision)}</dd>
          </div>
        </dl>
      </header>

      <p className="pmc-vault-availability" role="status" data-available={vault.available}>
        <span className="pmc-status-pill" data-tone={vault.available ? "success" : "warning"}>
          {vault.available ? t("vault.available") : t("vault.unavailable")}
        </span>
        {vault.available
          ? t("vault.availableHint")
          : t("vault.pausedBecause", { reason: vaultReason(t, vault.reason ?? null) })}
      </p>

      <section className="pmc-lens-surface" aria-labelledby="pmc-vault-evidence">
        <div className="pmc-lens-head">
          <div>
            <h3 id="pmc-vault-evidence">{t("noun.evidence")}</h3>
            <p>{t("vault.count", { count: references.length })}</p>
          </div>
          {evidenceFile !== undefined ? (
            <button
              type="button"
              className="pmc-button"
              disabled={!vault.available}
              onClick={() => {
                setFileSheetOpen(true);
              }}
            >
              {t("evidenceFile.open")}
            </button>
          ) : null}
        </div>
        {references.length === 0 ? (
          <p className="pmc-lens-empty">{t("vault.empty")}</p>
        ) : (
          <div className="pmc-lens-table">
            <table>
              <thead>
                <tr>
                  <th scope="col">{t("vault.column.evidence")}</th>
                  <th scope="col">{t("vault.column.role")}</th>
                  <th scope="col">{t("vault.column.verification")}</th>
                  <th scope="col">{t("vault.column.fingerprint")}</th>
                  <th scope="col">{t("vault.column.classification")}</th>
                  <th scope="col">{t("vault.column.version")}</th>
                </tr>
              </thead>
              <tbody>
                {[...references]
                  .sort((left, right) => left.id.localeCompare(right.id))
                  .map((reference) => (
                    <tr key={reference.id} data-verification={reference.verification.kind}>
                      <th scope="row">{reference.id}</th>
                      <td>
                        {reference.role === null
                          ? t("vault.roleUnset")
                          : evidenceRoleLabel(t, reference.role)}
                      </td>
                      <td>
                        {reference.verification.atMillis === null
                          ? verificationLabel(t, reference.verification.kind)
                          : t("vault.verificationOn", {
                              verification: verificationLabel(t, reference.verification.kind),
                              date: formatDate(reference.verification.atMillis),
                            })}
                      </td>
                      <td>{reference.pinned ? t("vault.pinned") : t("vault.notPinned")}</td>
                      <td>
                        <span
                          className="pmc-classification-badge"
                          data-classification={reference.classification}
                        >
                          {classificationName(t, reference.classification)}
                        </span>
                      </td>
                      <td>{String(reference.version)}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {fileSheetOpen && evidenceFile !== undefined ? (
        <EvidenceFromFileSheet
          actions={evidenceFile}
          onClose={() => {
            setFileSheetOpen(false);
          }}
          onFinished={() => {
            setFileSheetOpen(false);
            fetchAll();
          }}
        />
      ) : null}
    </div>
  );
}
