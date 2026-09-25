import { useCallback, useEffect, useState } from "react";

import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import {
  classificationName,
  cockpitSentence,
  ownerLabel,
  reasonLabel,
  tierLabel,
  workItemKindLabel,
} from "../i18n/workLabels";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { ExecutiveCockpitDto, ProductDetailDto } from "./cockpitContract";
import { LensCanvas, LensTable } from "./ExecutiveLens";
import { GettingStarted, type GettingStartedSources } from "./GettingStarted";
import { LENS_MODE_COPY, LENS_MODES, type LensMode } from "./lensModes";
import { ProductAside } from "./ProductAside";
import type { EvidenceActions } from "./ProductInspector";
import type { Translator } from "../i18n/messages";
import { findRoute } from "../shell/routes";

/**
 * S01 Executive Cockpit.
 *
 * Renders the information sequence the DG1 brief fixes, in its order:
 * Portfolio health -- here the Portfolio Lens on the axes the 2026-09-15 S01
 * amendment accepted -- then the period comparison, then the pulse, then the
 * highest-impact attention items, then the leader briefing.
 *
 * The Lens has a persistent inspector beside it, as DG1 draws it: selecting a
 * Product shows its three measures with the records behind them, then the
 * same O01 inspector Portfolio uses.
 *
 * Two DG1 prohibitions shape what is deliberately absent here. There is no
 * progress bar or percentage anywhere: the composition carries counts and
 * ratios as whole numbers only. And the attention list is not a checklist:
 * every item shows why it is ranked where it is, which is what keeps the
 * Cockpit a judgement surface rather than the task list DG1 names as a
 * failure mode.
 */
export interface ExecutiveCockpitProps {
  /** Supplied by the caller so the route can be rendered from a fixture in
   * tests and from IPC in the app, without the component knowing which. */
  readonly load: () => Promise<ExecutiveCockpitDto>;
  /** When absent, the inspector shows the Lens measures only. */
  readonly loadDetail?: (productId: string) => Promise<ProductDetailDto>;
  readonly evidenceActions?: EvidenceActions;
  /** Your own workspace's setup checklist (getting-started amendment);
   * absent in the sample workspace. */
  readonly gettingStarted?: GettingStartedSources | undefined;
}

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly cockpit: ExecutiveCockpitDto };

/** The briefing in words, from the exceptions the host already ordered. */
function briefing(
  cockpit: ExecutiveCockpitDto,
  t: Translator,
): { conclusion: string; why: string | null } {
  const [top] = cockpit.exceptions;
  if (top === undefined) {
    return { conclusion: t("cockpit.briefingNone"), why: null };
  }
  // Counted by record, as the Work Queue badge counts: one record with two
  // reasons is one thing to look at.
  const records = new Set(cockpit.exceptions.map((item) => `${item.kind}:${item.id}`)).size;
  return {
    conclusion: t.plural("cockpit.briefingTop", records, {
      label: top.label,
      reason: reasonLabel(t, top.reason),
    }),
    why: t("cockpit.briefingWhy", { tier: tierLabel(t, top.tier) }),
  };
}

export function ExecutiveCockpit({
  load,
  loadDetail,
  evidenceActions,
  gettingStarted,
}: ExecutiveCockpitProps) {
  const t = useT();
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [mode, setMode] = useState<LensMode>("timing");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusKey, setFocusKey] = useState(0);

  // Only settles the outcome. It never sets `loading` itself, so the effect
  // below does not change state synchronously while rendering.
  const fetchCockpit = useCallback(() => {
    load().then(
      (cockpit) => {
        setState({ status: "ready", cockpit });
      },
      (reason: unknown) => {
        // A failed query reports failure. It never falls back to a previous
        // answer presented as current, and it changes no other state.
        setState({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [load, t]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    fetchCockpit();
  }, [fetchCockpit]);

  useEffect(() => {
    fetchCockpit();
  }, [fetchCockpit]);

  const select = useCallback((productId: string) => {
    setSelectedId(productId);
    setFocusKey((key) => key + 1);
  }, []);
  if (state.status === "loading") {
    return (
      <p className="pmc-cockpit-status" role="status">
        {t("route.loading", { route: findRoute("executive-cockpit").label })}
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
            route: findRoute("executive-cockpit").label,
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { cockpit } = state;
  const { pulse, lens } = cockpit;

  // The two Ledger reads disagreed, so nothing here is true at one revision.
  // Nothing is shown rather than a blend of two moments.
  if (cockpit.state === "outOfSync" || pulse === null || lens === null) {
    return (
      <div className="pmc-cockpit-status">
        <p role="status">{t("route.outOfSync")}</p>
        <button type="button" className="pmc-button" onClick={retry}>
          {t("route.reload")}
        </button>
      </div>
    );
  }

  const selected = lens.points.find((point) => point.productId === selectedId) ?? null;
  const brief = briefing(cockpit, t);
  const counts = [
    { key: "milestones", label: t("cockpit.pulse.milestones"), value: pulse.milestones },
    { key: "commitments", label: t("cockpit.pulse.commitments"), value: pulse.commitments },
    { key: "kpis", label: t("cockpit.pulse.kpis"), value: pulse.kpis },
  ];

  return (
    <div className="pmc-cockpit">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("cockpit.headline")}</h2>
          <p className="pmc-page-lede">{t("cockpit.lede")}</p>
        </div>
        <dl className="pmc-meta pmc-page-asof">
          <div>
            <dt>{t("route.ledgerRevision")}</dt>
            <dd>{String(cockpit.ledgerRevision)}</dd>
          </div>
          <div>
            <dt>{t("route.readAt")}</dt>
            <dd>
              <time>{formatReadAt(cockpit.asOfMillis)}</time>
            </dd>
          </div>
        </dl>
      </header>

      {gettingStarted !== undefined && (
        <GettingStarted {...gettingStarted} productCount={cockpit.products.length} />
      )}

      <section className="pmc-lens-layout" aria-labelledby="pmc-cockpit-lens">
        <div className="pmc-lens-surface">
          <div className="pmc-lens-head">
            <div>
              <h3 id="pmc-cockpit-lens">{t("noun.portfolioLens")}</h3>
              <p>{t(LENS_MODE_COPY[mode])}</p>
            </div>
            <div className="pmc-lens-modes" role="group" aria-label={t("cockpit.lensModes")}>
              {LENS_MODES.map((entry) => (
                <button
                  key={entry.mode}
                  type="button"
                  className="pmc-button"
                  aria-pressed={mode === entry.mode}
                  onClick={() => {
                    setMode(entry.mode);
                  }}
                >
                  {t(entry.label)}
                </button>
              ))}
            </div>
          </div>
          {lens.points.length === 0 ? (
            <p className="pmc-lens-empty">{t("cockpit.lensEmpty")}</p>
          ) : (
            <>
              <LensCanvas lens={lens} mode={mode} selectedId={selectedId} onSelect={select} />
              <p className="pmc-lens-legend">{t("cockpit.lensLegend")}</p>
              <LensTable points={lens.points} selectedId={selectedId} onSelect={select} />
            </>
          )}
        </div>

        <ProductAside
          ledgerRevision={lens.ledgerRevision}
          point={selected}
          focusKey={focusKey}
          emptyText={t("cockpit.asideEmpty")}
          {...(loadDetail === undefined ? {} : { loadDetail })}
          {...(evidenceActions === undefined ? {} : { evidenceActions })}
          onLedgerChanged={fetchCockpit}
        />
      </section>

      <div className="pmc-cockpit-band">
        <section aria-labelledby="pmc-cockpit-period">
          <h3 id="pmc-cockpit-period">{t("cockpit.period")}</h3>
          {/* Stated, never shown as a zero delta: a zero delta would claim
              "nothing changed" rather than "there is nothing to compare". */}
          <p>
            {cockpit.periodComparable
              ? cockpit.periodNote
              : t("cockpit.periodUnavailable", { reason: cockpitSentence(t, cockpit.periodNote) })}
          </p>
        </section>

        <section aria-labelledby="pmc-cockpit-pulse">
          <h3 id="pmc-cockpit-pulse">{t("cockpit.pulse")}</h3>
          <dl className="pmc-cockpit-pulse">
            {counts.map(({ key, label, value }) => (
              <div key={key} className="pmc-cockpit-count">
                <dt>{label}</dt>
                <dd>
                  <span className="pmc-cockpit-count-value">{String(value.count)}</span>
                  {/* DG1 requires inspectable definitions: a bare number invites
                      the reader to guess a denominator. */}
                  <span className="pmc-cockpit-count-definition">
                    {cockpitSentence(t, value.definition)}
                  </span>
                  <span className="pmc-cockpit-count-owner">
                    {t("cockpit.pulse.from", { owner: ownerLabel(t, value.owner) })}
                  </span>
                </dd>
              </div>
            ))}
          </dl>
        </section>
      </div>

      <section className="pmc-cockpit-attention" aria-labelledby="pmc-cockpit-attention">
        <h3 id="pmc-cockpit-attention">{t("cockpit.attention")}</h3>
        {cockpit.exceptions.length === 0 ? (
          <p>{t("cockpit.attentionNone")}</p>
        ) : (
          <ol className="pmc-cockpit-exceptions">
            {cockpit.exceptions.map((exception) => (
              <li
                key={`${exception.kind}:${exception.id}:${exception.reason}`}
                data-tier={exception.tier}
              >
                {/* The record first: a reason with no record behind it sends
                    the reader off to find it in the Work Queue. */}
                <p className="pmc-cockpit-exception-what">
                  <span className="pmc-kind" data-kind={exception.kind}>
                    {workItemKindLabel(t, exception.kind)}
                  </span>
                  <span className="pmc-cockpit-exception-label">{exception.label}</span>
                  <span
                    className="pmc-classification-badge"
                    data-classification={exception.classification}
                  >
                    {classificationName(t, exception.classification)}
                  </span>
                </p>
                <p className="pmc-cockpit-exception-reason">{reasonLabel(t, exception.reason)}</p>
                {/* Why it is ranked here, from the tier the ordering itself
                    applied, so the stated reason cannot drift from it. */}
                <p className="pmc-cockpit-exception-why">
                  {t("cockpit.placedBecause", { tier: tierLabel(t, exception.tier) })}
                </p>
              </li>
            ))}
          </ol>
        )}
      </section>

      <section className="pmc-cockpit-briefing" aria-labelledby="pmc-cockpit-briefing">
        <h3 id="pmc-cockpit-briefing">{t("cockpit.briefing")}</h3>
        <p className="pmc-cockpit-conclusion">{brief.conclusion}</p>
        {brief.why === null ? null : <p className="pmc-cockpit-intervention">{brief.why}</p>}
        {cockpit.periodComparable ? null : (
          <p className="pmc-cockpit-limit">{t("cockpit.briefingNoPeriod")}</p>
        )}
      </section>
    </div>
  );
}
