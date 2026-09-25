import { formatLocalDate } from "../i18n/time";
import { timingStateLabel, verificationLabel } from "../i18n/workLabels";
import type { LensPointDto } from "./cockpitContract";
import type { Translator } from "../i18n/messages";

/**
 * The Lens measures in words, shared by the Cockpit and Portfolio so a
 * Product reads the same on both. Ratios are counts ("2／3"), never
 * percentages, and nothing here compares one moment with another.
 */
function formatDate(millis: number | null): string {
  // Non-breaking hyphens, so a date never wraps in the middle of a table cell.
  return millis === null ? "" : formatLocalDate(millis).replaceAll("-", "‑");
}

export function windowDays(lens: { readonly dueSoonWindowMillis: number }): number {
  return Math.round(lens.dueSoonWindowMillis / (24 * 60 * 60 * 1000));
}

/** The measures restated as a current condition. Never "worsened": nothing
 * here compares one moment with another. */
export function happened(point: LensPointDto, days: number, t: Translator): string {
  const timing = {
    unknown: () => t("lens.happened.unknown"),
    later: () => t.plural("lens.happened.later", days, { days }),
    dueSoon: () => t.plural("lens.happened.dueSoon", days, { days }),
    datePassed: () => t("lens.happened.datePassed"),
  }[point.timing.state]();
  const worst = point.coverage.worst;
  return worst !== null && worst !== "verified"
    ? t("lens.happened.withEvidence", { timing, verification: verificationLabel(t, worst) })
    : timing;
}

export function timingLine(point: LensPointDto, t: Translator): string {
  const { timing } = point;
  return timing.state === "unknown"
    ? t("lens.timing.none")
    : t("lens.timing.line", {
        state: timingStateLabel(t, timing.state),
        date: formatDate(timing.earliestDueAtMillis),
        count: timing.milestoneCount,
      });
}

export function observabilityLine(point: LensPointDto, t: Translator): string {
  const { observability } = point;
  if (!observability.known) {
    return t("lens.observability.none");
  }
  const counts = { observed: observability.observed, defined: observability.defined };
  return observability.latestObservedAtMillis === null
    ? t("lens.observability.line", counts)
    : t("lens.observability.lineLatest", {
        ...counts,
        date: formatDate(observability.latestObservedAtMillis),
      });
}

export function coverageLine(point: LensPointDto, t: Translator): string {
  const { coverage } = point;
  return coverage.linked === 0
    ? t("lens.coverage.none")
    : t("lens.coverage.line", { verified: coverage.verified, linked: coverage.linked });
}
