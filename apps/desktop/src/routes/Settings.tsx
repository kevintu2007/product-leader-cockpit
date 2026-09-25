import { useCallback, useEffect, useState } from "react";

import {
  FOLLOW_SYSTEM,
  LANGUAGE_NAMES,
  preferenceFrom,
  type LocalePreference,
} from "../i18n/displayLocale";
import { SUPPORTED_LOCALES } from "../i18n/locale";
import { useLocaleSetting } from "../i18n/localeSetting";
import type { Translator } from "../i18n/messages";
import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import {
  applyTextScale,
  readStoredTextScale,
  type AppTextScaleStep,
} from "../overlays/appTextScale";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { TextScalePreview } from "../overlays/TextScalePreview";
import type { LedgerStatusDto, VaultStatusDto } from "./cockpitContract";
import { findRoute } from "../shell/routes";
import { BackupsSection, type BackupsSectionProps } from "../backup/BackupsSection";
import type { VaultRootActions } from "../vault/vaultRootIpc";
import { VaultRootSheet } from "../vault/VaultRootSheet";
import { WorkspaceSection, type WorkspaceSectionProps } from "../workspace/WorkspaceSection";

/**
 * S10 Settings, limited to what the host can already answer: the UI
 * language (stored in the platform settings document, not the Ledger), the
 * app text scale (O06), and the state of the Ledger and the Vault this
 * workspace opened. No path is shown -- DG3 forbids surfacing one -- and
 * nothing here writes to the Ledger.
 */
export interface SettingsProps {
  readonly loadLedgerStatus: () => Promise<LedgerStatusDto>;
  readonly loadVaultStatus: () => Promise<VaultStatusDto>;
  /** Settings → Backups; absent in tests that do not exercise it. */
  readonly backups?: BackupsSectionProps | undefined;
  /** Choosing the Live Vault folder (item ⑦, H2b). Absent for the Training
   * workspace, whose Vault the person does not choose. */
  readonly vaultRoot?: VaultRootActions | undefined;
  /** Settings → Workspace (item ⑨): which workspace is open, switching,
   * and the sample's reset and delete. Absent in tests that do not use it. */
  readonly workspace?: WorkspaceSectionProps | undefined;
}

type Facts =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | {
      readonly status: "ready";
      readonly ledger: LedgerStatusDto;
      readonly vault: VaultStatusDto;
    };

/** Why the Vault can't be used, in words; an unknown reason says so. */
function vaultReason(t: Translator, reason: string | null): string {
  return reason === "notConfigured" || reason === "invalidRoot" || reason === "changeUnresolved"
    ? t(`vault.reason.${reason}`)
    : t("vault.reason.unknown");
}

export function Settings({
  loadLedgerStatus,
  loadVaultStatus,
  backups,
  vaultRoot,
  workspace,
}: SettingsProps) {
  const t = useT();
  const [scale, setScale] = useState<AppTextScaleStep>(readStoredTextScale);
  const [facts, setFacts] = useState<Facts>({ status: "loading" });
  const [changingVault, setChangingVault] = useState(false);

  const fetchFacts = useCallback(() => {
    Promise.all([loadLedgerStatus(), loadVaultStatus()]).then(
      ([ledger, vault]) => {
        setFacts({ status: "ready", ledger, vault });
      },
      (reason: unknown) => {
        setFacts({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [loadLedgerStatus, loadVaultStatus, t]);

  useEffect(() => {
    fetchFacts();
  }, [fetchFacts]);

  const changeScale = (step: AppTextScaleStep) => {
    setScale(step);
    applyTextScale(step);
  };

  const language = useLocaleSetting();
  const [languageError, setLanguageError] = useState<ResolvedSafeError | null>(null);
  // One change at a time: a second choice while the first is being stored
  // could otherwise land first and be overwritten by the older one.
  const [savingLanguage, setSavingLanguage] = useState(false);
  const changeLanguage = (next: LocalePreference) => {
    if (language === null || savingLanguage) {
      return;
    }
    setLanguageError(null);
    setSavingLanguage(true);
    language
      .choose(next)
      .catch((reason: unknown) => {
        setLanguageError(resolveRejection(reason, t));
      })
      .finally(() => {
        setSavingLanguage(false);
      });
  };

  return (
    <div className="pmc-settings">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("settings.headline")}</h2>
          <p className="pmc-page-lede">{t("settings.lede")}</p>
        </div>
      </header>

      <div className="pmc-settings-grid">
        {workspace !== undefined && <WorkspaceSection {...workspace} />}
        {backups !== undefined && <BackupsSection {...backups} />}
        {language !== null && (
          <section
            className="pmc-lens-surface pmc-settings-section"
            aria-labelledby="pmc-settings-language"
          >
            <h3 id="pmc-settings-language">{t("settings.language")}</h3>
            <select
              className="pmc-settings-select"
              aria-labelledby="pmc-settings-language"
              value={language.preference}
              disabled={savingLanguage}
              aria-busy={savingLanguage}
              onChange={(event) => {
                changeLanguage(preferenceFrom(event.target.value));
              }}
            >
              <option value={FOLLOW_SYSTEM}>
                {t("settings.languageSystem", {
                  language: LANGUAGE_NAMES[language.systemLocale],
                })}
              </option>
              {SUPPORTED_LOCALES.map((locale) => (
                <option key={locale} value={locale} lang={locale}>
                  {LANGUAGE_NAMES[locale]}
                </option>
              ))}
            </select>
            {languageError !== null && (
              <SafeErrorDetail
                message={t("settings.languageFailed", { message: languageError.message })}
                correlationId={languageError.correlationId}
                retryable={languageError.retryable}
              />
            )}
          </section>
        )}

        <section
          className="pmc-lens-surface pmc-settings-section"
          aria-label={t("settings.textSize")}
        >
          <TextScalePreview
            value={scale}
            onChange={changeScale}
            sampleContent={
              <div className="pmc-text-scale-sample-content">
                <p className="pmc-route-title">{findRoute("executive-cockpit").label}</p>
                <p className="pmc-text-scale-sample-body">{t("settings.sampleBody")}</p>
                <span className="pmc-text-scale-sample-label">{t("settings.sampleLabel")}</span>
              </div>
            }
          />
        </section>

        <section
          className="pmc-lens-surface pmc-settings-section"
          aria-labelledby="pmc-settings-workspace"
        >
          <h3 id="pmc-settings-workspace">{t("settings.dataSources")}</h3>
          {facts.status === "loading" ? (
            <p role="status">{t("settings.loading")}</p>
          ) : facts.status === "error" ? (
            <>
              <SafeErrorDetail
                message={t("settings.unavailable", { message: facts.error.message })}
                correlationId={facts.error.correlationId}
                retryable={facts.error.retryable}
              />
              <button
                type="button"
                className="pmc-button"
                onClick={() => {
                  setFacts({ status: "loading" });
                  fetchFacts();
                }}
              >
                {t("route.reload")}
              </button>
            </>
          ) : (
            <dl className="pmc-h2a-fields pmc-settings-facts">
              <div className="pmc-h2a-field">
                <dt>{t("noun.productLedger")}</dt>
                <dd>
                  {t("settings.ledgerReadable", {
                    schema: String(facts.ledger.schemaVersion),
                    revision: String(facts.ledger.revision),
                  })}
                </dd>
              </div>
              <div className="pmc-h2a-field">
                <dt>{t("noun.productVault")}</dt>
                <dd>
                  {!facts.vault.configured ? (
                    <span className="pmc-status-pill" data-tone="warning">
                      {t("settings.vaultNotSet")}
                    </span>
                  ) : (
                    <>
                      <span
                        className="pmc-status-pill"
                        data-tone={facts.vault.available ? "success" : "warning"}
                      >
                        {facts.vault.available
                          ? t("settings.vaultAvailable")
                          : t("settings.vaultUnavailable")}
                      </span>
                      {/* The folder's own name in both states: a folder that
                          is set but unreachable is still the folder that is
                          set. Training's Vault has no name to show, because
                          the person did not choose it. */}
                      {facts.vault.folderName !== null && ` ${facts.vault.folderName}`}
                      {!facts.vault.available && (
                        <>
                          {" "}
                          {t("vault.pausedBecause", {
                            reason: vaultReason(t, facts.vault.reason ?? null),
                          })}
                        </>
                      )}
                    </>
                  )}
                  {vaultRoot !== undefined && (
                    <div>
                      <button
                        type="button"
                        className="pmc-button"
                        onClick={() => {
                          setChangingVault(true);
                        }}
                      >
                        {facts.vault.configured
                          ? t("vaultRoot.open.change")
                          : t("vaultRoot.open.choose")}
                      </button>
                    </div>
                  )}
                </dd>
              </div>
            </dl>
          )}
        </section>
      </div>
      {changingVault && vaultRoot !== undefined && (
        <VaultRootSheet
          actions={vaultRoot}
          backups={backups}
          onClose={() => {
            setChangingVault(false);
          }}
          onFinished={() => {
            setChangingVault(false);
            // What is set now is the host's to say, not the sheet's.
            setFacts({ status: "loading" });
            fetchFacts();
          }}
        />
      )}
    </div>
  );
}
