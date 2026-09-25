import { type ReactNode, useId } from "react";

import {
  classificationName,
  contributionLabel,
  quadrantLabel,
  timingStateLabel,
  verificationLabel,
} from "../i18n/workLabels";
import type {
  ExecutiveLensDto,
  LensContributionDto,
  LensPointDto,
  LensTimingState,
} from "./cockpitContract";
import { coverageLine, happened, observabilityLine, timingLine, windowDays } from "./lensText";
import type { LensMode } from "./lensModes";
import { Slotted } from "../i18n/Slotted";
import { useT } from "../i18n/useT";
import type { Translator } from "../i18n/messages";

/**
 * The Executive Lens (S01) on the axes the DG3 S01 amendment of 2026-09-15
 * accepted: Milestone Timing Exposure across, Outcome Observability up, and
 * Verified Evidence Coverage as the size of each point.
 *
 * Every judgement -- the timing state, high or low, the quadrant, the
 * classification fold -- arrives from the host. This file only places what it
 * is given: a categorical X, the observed share as Y, and a size. A Product
 * the host marks Unknown on an axis sits in that axis's labelled band outside
 * the quadrants, never at a reassuring position inside them.
 *
 * Products are placed, not ranked. The table lists them in the host's name
 * order and says so.
 */

function emphasised(point: LensPointDto, mode: LensMode): boolean {
  switch (mode) {
    case "timing":
      return point.timing.state === "datePassed";
    case "observability":
      return point.observability.high === false;
    case "evidence":
      return point.coverage.worst === "integrity_mismatch" || point.coverage.worst === "unverified";
  }
}

const X_POSITION: Readonly<Record<Exclude<LensTimingState, "unknown">, number>> = {
  later: 25,
  dueSoon: 62,
  datePassed: 85,
};

type Region = "plot" | "unknownX" | "unknownY" | "corner";

interface Placement {
  readonly region: Region;
  /** Percent of the region's width and height, from the top-left. */
  readonly left: number;
  readonly top: number;
}

function basePlacement(point: LensPointDto): Placement {
  const { timing, observability } = point;
  const x = timing.state === "unknown" ? null : X_POSITION[timing.state];
  // The share places the point; the host's `high` decides which side of the
  // divider it is on, so the picture can never contradict the quadrant.
  const share = observability.known
    ? 92 - (observability.observed / observability.defined) * 84
    : null;
  const y =
    share === null || observability.high === null
      ? null
      : observability.high
        ? Math.min(share, 44)
        : Math.max(share, 56);
  if (x === null && y === null) {
    return { region: "corner", left: 50, top: 50 };
  }
  if (x === null) {
    return { region: "unknownX", left: 50, top: y ?? 50 };
  }
  if (y === null) {
    return { region: "unknownY", left: x, top: 50 };
  }
  return { region: "plot", left: x, top: y };
}

/** Points sharing a position are spread in name order, so the picture only
 * changes when a fact changes. */
function placeAll(points: readonly LensPointDto[]): Map<string, Placement> {
  const groups = new Map<string, LensPointDto[]>();
  for (const point of points) {
    const base = basePlacement(point);
    const key = `${base.region}:${base.left.toFixed(1)}:${base.top.toFixed(1)}`;
    groups.set(key, [...(groups.get(key) ?? []), point]);
  }
  const placed = new Map<string, Placement>();
  for (const group of groups.values()) {
    const [first] = group;
    if (first === undefined) {
      continue;
    }
    const base = basePlacement(first);
    // A shared position becomes a small grid around it, kept on the same side
    // of the dividers: a point spread across a divider would read as the
    // other quadrant. The bottom bands are short, so they use one row.
    const oneRow = base.region === "unknownY" || base.region === "corner";
    const columns = oneRow
      ? group.length
      : base.region === "unknownX"
        ? 1
        : Math.min(group.length, 2);
    const rows = Math.ceil(group.length / columns);
    const [top, bottom] = oneRow ? [50, 50] : base.top <= 50 ? [8, 44] : [56, 92];
    const rowStep = rows > 1 ? Math.min(22, (bottom - top) / (rows - 1)) : 0;
    const firstRow = Math.min(Math.max(base.top, top), bottom - (rows - 1) * rowStep);
    const columnStep = base.region === "corner" ? 30 : 18;
    // The right edge carries the Y axis name, so points stop short of it.
    const [left, right] = base.region === "corner" ? [20, 80] : [12, 84];
    const firstColumn = Math.min(
      Math.max(base.left - ((columns - 1) * columnStep) / 2, left),
      right - (columns - 1) * columnStep,
    );
    group.forEach((point, index) => {
      placed.set(point.productId, {
        region: base.region,
        left: base.region === "unknownX" ? base.left : firstColumn + (index % columns) * columnStep,
        top: oneRow ? base.top : firstRow + Math.floor(index / columns) * rowStep,
      });
    });
  }
  return placed;
}

function bubbleSize(point: LensPointDto): number {
  const { coverage } = point;
  return coverage.linked === 0 ? 30 : 30 + (coverage.verified / coverage.linked) * 26;
}

export interface LensCanvasProps {
  readonly lens: ExecutiveLensDto;
  readonly mode: LensMode;
  readonly selectedId: string | null;
  readonly onSelect: (productId: string) => void;
}

export function LensCanvas({ lens, mode, selectedId, onSelect }: LensCanvasProps) {
  const t = useT();
  const idBase = useId();
  const placements = placeAll(lens.points);
  const days = windowDays(lens);

  const bubbles = (region: Region) =>
    lens.points
      .filter((point) => placements.get(point.productId)?.region === region)
      .map((point) => {
        const placement = placements.get(point.productId);
        if (!placement) {
          return null;
        }
        const tooltipId = `${idBase}-tip-${point.productId}`;
        const size = bubbleSize(point);
        const selected = point.productId === selectedId;
        return (
          <button
            key={point.productId}
            type="button"
            className="pmc-lens-bubble"
            data-selected={selected}
            data-emphasised={emphasised(point, mode)}
            data-hollow={point.coverage.linked === 0}
            data-tooltip-below={placement.top < 35}
            style={{
              left: `${String(placement.left)}%`,
              top: `${String(placement.top)}%`,
              width: `${String(size)}px`,
              height: `${String(size)}px`,
            }}
            aria-pressed={selected}
            aria-label={t("lens.bubble.label", {
              product: point.productName,
              happened: happened(point, days, t),
              timing: timingLine(point, t),
              observability: observabilityLine(point, t),
              coverage: coverageLine(point, t),
            })}
            aria-describedby={tooltipId}
            onClick={() => {
              onSelect(point.productId);
            }}
          >
            <span className="pmc-lens-bubble-label">{point.productName}</span>
            <span className="pmc-lens-tooltip" id={tooltipId} role="tooltip">
              <strong>{point.productName}</strong>
              <span>
                <Slotted
                  text={
                    t.lookup("lens.tooltip.happenedLine", {
                      happened: happened(point, days, t),
                    }) ?? ""
                  }
                  slots={{ label: <b>{t("lens.tooltip.happened")}</b> }}
                />
              </span>
              <span>
                <Slotted
                  text={t.lookup("lens.tooltip.impactLine", {}) ?? ""}
                  slots={{ label: <b>{t("lens.tooltip.impact")}</b> }}
                />
              </span>
              <span>
                <Slotted
                  text={t.lookup("lens.tooltip.nextLine", {}) ?? ""}
                  slots={{ label: <b>{t("lens.tooltip.next")}</b> }}
                />
              </span>
              <small>{t("lens.tooltip.timing", { value: timingLine(point, t) })}</small>
              <small>
                {t("lens.tooltip.observability", { value: observabilityLine(point, t) })}
              </small>
              <small>{t("lens.tooltip.coverage", { value: coverageLine(point, t) })}</small>
            </span>
          </button>
        );
      });

  return (
    <div className="pmc-lens-canvas" role="region" aria-label={t("lens.canvas.label")}>
      <div className="pmc-lens-plot">
        <span className="pmc-lens-quadrant" data-quadrant="keepMomentum">
          {quadrantLabel(t, "keepMomentum")}
        </span>
        <span className="pmc-lens-quadrant" data-quadrant="monitorClosely">
          {quadrantLabel(t, "monitorClosely")}
        </span>
        <span className="pmc-lens-quadrant" data-quadrant="exploreAndValidate">
          {quadrantLabel(t, "exploreAndValidate")}
        </span>
        <span className="pmc-lens-quadrant" data-quadrant="prioritizeNow">
          {quadrantLabel(t, "prioritizeNow")}
        </span>
        <span className="pmc-lens-axis pmc-lens-axis-y">{t("lens.axis.y")}</span>
        {bubbles("plot")}
      </div>
      <div className="pmc-lens-band pmc-lens-band-x">
        <span className="pmc-lens-band-label">{t("lens.band.noMilestones")}</span>
        {bubbles("unknownX")}
      </div>
      <div className="pmc-lens-band pmc-lens-band-y">
        <span className="pmc-lens-band-label">{t("lens.band.noKpis")}</span>
        <span className="pmc-lens-axis pmc-lens-axis-x">
          {t("lens.axis.x", {
            later: timingStateLabel(t, "later"),
            dueSoon: timingStateLabel(t, "dueSoon"),
            datePassed: timingStateLabel(t, "datePassed"),
          })}
        </span>
        {bubbles("unknownY")}
      </div>
      <div className="pmc-lens-band pmc-lens-band-corner">{bubbles("corner")}</div>
    </div>
  );
}

/** A column a route adds before 檢視, with a header that is also its key. */
export interface LensTableColumn {
  readonly header: string;
  readonly cell: (point: LensPointDto) => ReactNode;
}

export interface LensTableProps {
  readonly points: readonly LensPointDto[];
  readonly selectedId: string | null;
  readonly onSelect: (productId: string) => void;
  readonly extraColumns?: readonly LensTableColumn[];
}

export function LensTable({ points, selectedId, onSelect, extraColumns = [] }: LensTableProps) {
  const t = useT();
  return (
    <div className="pmc-lens-table">
      <table>
        <caption>{t("lens.table.caption")}</caption>
        <thead>
          <tr>
            <th scope="col">{t("lens.table.product")}</th>
            <th scope="col">{t("lens.table.timing")}</th>
            <th scope="col">{t("lens.table.observability")}</th>
            <th scope="col">{t("lens.table.coverage")}</th>
            <th scope="col">{t("lens.table.quadrant")}</th>
            {extraColumns.map((column) => (
              <th key={column.header} scope="col">
                {column.header}
              </th>
            ))}
            <th scope="col">{t("lens.table.view")}</th>
          </tr>
        </thead>
        <tbody>
          {points.map((point) => (
            <tr key={point.productId} data-selected={point.productId === selectedId}>
              <th scope="row">{point.productName}</th>
              <td>{timingLine(point, t)}</td>
              <td>{observabilityLine(point, t)}</td>
              <td>{coverageLine(point, t)}</td>
              <td>
                {point.quadrant === null
                  ? t("lens.table.notEnoughData")
                  : quadrantLabel(t, point.quadrant)}
              </td>
              {extraColumns.map((column) => (
                <td key={column.header}>{column.cell(point)}</td>
              ))}
              <td>
                <button
                  type="button"
                  className="pmc-button"
                  aria-pressed={point.productId === selectedId}
                  aria-label={t(
                    point.productId === selectedId
                      ? "lens.table.selectedProduct"
                      : "lens.table.viewProduct",
                    { product: point.productName },
                  )}
                  onClick={() => {
                    onSelect(point.productId);
                  }}
                >
                  {point.productId === selectedId ? t("lens.table.selected") : t("lens.table.view")}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function contributionText(
  contribution: LensContributionDto,
  revision: number,
  t: Translator,
): string {
  const version =
    contribution.version === null
      ? t("lens.contribution.versionless", { revision: String(revision) })
      : t("lens.contribution.version", { version: String(contribution.version) });
  return t("lens.contribution.line", {
    kind: contributionLabel(t, contribution.kind, contribution.role),
    id: contribution.id,
    version,
    classification: classificationName(t, contribution.classification),
  });
}

export interface LensMeasuresProps {
  /** The snapshot version an Evidence link is read at. */
  readonly ledgerRevision: number;
  readonly point: LensPointDto;
  readonly headingId: string;
}

/** The selected Product's three measures, each with the records behind it. */
export function LensMeasures({ ledgerRevision, point, headingId }: LensMeasuresProps) {
  const t = useT();
  const contributions = [
    ...point.timing.contributions,
    ...point.observability.contributions,
    ...point.coverage.contributions,
  ];
  const forced = point.classificationForcedBy;
  return (
    <section className="pmc-lens-measures" aria-labelledby={headingId}>
      <h3 id={headingId} tabIndex={-1}>
        {t("lens.measures.heading", { product: point.productName })}
      </h3>
      <p className="pmc-lens-measures-fold">
        <span
          className="pmc-classification-badge"
          data-classification={point.effectiveClassification}
        >
          {classificationName(t, point.effectiveClassification)}
        </span>
        <span>
          {forced === null
            ? t("lens.measures.ownClassification")
            : t("lens.measures.classificationFrom", {
                kind: contributionLabel(t, forced.kind, forced.role),
                id: forced.id,
              })}
        </span>
      </p>
      <dl className="pmc-h2a-fields">
        <div className="pmc-h2a-field">
          <dt>{t("lens.measures.quadrant")}</dt>
          <dd>
            {point.quadrant === null
              ? t("lens.measures.noQuadrant")
              : quadrantLabel(t, point.quadrant)}
          </dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("lens.measures.timing")}</dt>
          <dd>{timingLine(point, t)}</dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("lens.measures.observability")}</dt>
          <dd>{observabilityLine(point, t)}</dd>
        </div>
        <div className="pmc-h2a-field">
          <dt>{t("lens.measures.coverage")}</dt>
          <dd>
            {coverageLine(point, t)}
            {point.coverage.byState.length > 0 && (
              <ul className="pmc-lens-states">
                {point.coverage.byState.map((entry) => (
                  <li key={entry.state}>
                    {t("lens.measures.stateCount", {
                      state: verificationLabel(t, entry.state),
                      count: entry.count,
                    })}
                  </li>
                ))}
              </ul>
            )}
          </dd>
        </div>
        {point.sharedProjectIds.length > 0 && (
          <div className="pmc-h2a-field">
            <dt>{t("lens.measures.sharedProjects")}</dt>
            <dd>
              {t("lens.measures.sharedProjectList", {
                ids: point.sharedProjectIds.join(t("common.idSeparator")),
              })}
            </dd>
          </div>
        )}
      </dl>
      <details className="pmc-lens-sources">
        <summary>{t("lens.measures.sources", { count: contributions.length })}</summary>
        {contributions.length === 0 ? (
          <p>{t("lens.measures.noSources")}</p>
        ) : (
          <ul>
            {contributions.map((contribution) => (
              <li key={`${contribution.kind}:${contribution.id}`}>
                {contributionText(contribution, ledgerRevision, t)}
              </li>
            ))}
          </ul>
        )}
      </details>
    </section>
  );
}
