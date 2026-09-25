import { useCallback, useEffect, useState } from "react";

import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { cockpitSentence, reasonLabel, tierLabel, workItemKindLabel } from "../i18n/workLabels";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { ExecutiveCockpitDto } from "./cockpitContract";
import { findRoute } from "../shell/routes";

/**
 * S04 Reviews & Reports, limited to what the host can already answer.
 *
 * It lists what the Work Queue currently flags, in the Cockpit's attention
 * order, and sends the person to the Work Queue
 * to act. It does not claim these gate a review: no review workflow exists
 * yet to gate.
 *
 * Review periods, the Fact Pack and report approval have no host command
 * yet. The route says so in one sentence instead of drawing a workspace that
 * cannot do anything.
 */
export interface ReviewsProps {
  readonly load: () => Promise<ExecutiveCockpitDto>;
  readonly onOpenWorkQueue: () => void;
}

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly cockpit: ExecutiveCockpitDto };

export function Reviews({ load, onOpenWorkQueue }: ReviewsProps) {
  const t = useT();
  const [state, setState] = useState<LoadState>({ status: "loading" });

  const fetchReview = useCallback(() => {
    load().then(
      (cockpit) => {
        setState({ status: "ready", cockpit });
      },
      (reason: unknown) => {
        setState({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [load, t]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    fetchReview();
  }, [fetchReview]);

  useEffect(() => {
    fetchReview();
  }, [fetchReview]);

  if (state.status === "loading") {
    return (
      <p className="pmc-cockpit-status" role="status">
        {t("route.loading", { route: t("reviews.route") })}
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
            route: findRoute("reviews-and-reports").label,
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { cockpit } = state;

  if (cockpit.state === "outOfSync") {
    return (
      <div className="pmc-cockpit-status">
        <p role="status">{t("route.outOfSync")}</p>
        <button type="button" className="pmc-button" onClick={retry}>
          {t("route.reload")}
        </button>
      </div>
    );
  }

  // One entry per record, keeping the first (highest-placed) reason, so the
  // count agrees with the Work Queue badge.
  const records = cockpit.exceptions.filter(
    (item, index, all) =>
      all.findIndex((other) => other.kind === item.kind && other.id === item.id) === index,
  );

  return (
    <div className="pmc-reviews">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("reviews.headline")}</h2>
          <p className="pmc-page-lede">{t("reviews.lede")}</p>
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

      <div className="pmc-cockpit-band">
        <section aria-labelledby="pmc-reviews-period">
          <h3 id="pmc-reviews-period">{t("reviews.period")}</h3>
          <p>
            {cockpit.periodComparable
              ? cockpit.periodNote
              : t("reviews.noPeriod", { reason: cockpitSentence(t, cockpit.periodNote) })}
          </p>
          <p className="pmc-cockpit-limit">{t("reviews.notYet")}</p>
        </section>

        <section aria-labelledby="pmc-reviews-open">
          <h3 id="pmc-reviews-open">{t("reviews.open")}</h3>
          {records.length === 0 ? (
            <p>{t("reviews.openNone")}</p>
          ) : (
            <>
              <p>{t.plural("reviews.count", records.length)}</p>
              <ol className="pmc-cockpit-exceptions">
                {records.map((item) => (
                  <li key={`${item.kind}:${item.id}`} data-tier={item.tier}>
                    <p className="pmc-cockpit-exception-what">
                      <span className="pmc-kind" data-kind={item.kind}>
                        {workItemKindLabel(t, item.kind)}
                      </span>
                      <span className="pmc-cockpit-exception-label">{item.label}</span>
                    </p>
                    <p className="pmc-cockpit-exception-reason">{reasonLabel(t, item.reason)}</p>
                    <p className="pmc-cockpit-exception-why">
                      {t("cockpit.placedBecause", { tier: tierLabel(t, item.tier) })}
                    </p>
                  </li>
                ))}
              </ol>
              <div className="pmc-lens-paging">
                <button
                  type="button"
                  className="pmc-button pmc-button-primary"
                  onClick={onOpenWorkQueue}
                >
                  {t("reviews.goToQueue")}
                </button>
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  );
}
