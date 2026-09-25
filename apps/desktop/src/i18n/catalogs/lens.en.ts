/**
 * English copy for the Portfolio Lens, shared by the Executive Cockpit and
 * Portfolio so a Product reads the same on both. Ratios are counts ("2/3"),
 * never percentages, and nothing here compares one moment with another.
 */
export const LENS_EN = {
  "lens.happened.unknown": "No linked milestones, so there's no schedule to read",
  "lens.happened.later.one": "Every milestone is more than {days} day away",
  "lens.happened.later.other": "Every milestone is more than {days} days away",
  "lens.happened.dueSoon.one": "A milestone is due within {days} day",
  "lens.happened.dueSoon.other": "A milestone is due within {days} days",
  "lens.happened.datePassed": "A milestone date has passed",
  "lens.happened.withEvidence": "{timing}; some Evidence is: {verification}",
  "lens.timing.none": "No linked milestones",
  "lens.timing.line": "{state}, earliest {date}, {count} in total",
  "lens.observability.none": "No KPI definitions",
  "lens.observability.line": "{observed}/{defined} KPIs observed",
  "lens.observability.lineLatest": "{observed}/{defined} KPIs observed, latest {date}",
  "lens.coverage.none": "No linked Evidence",
  "lens.coverage.line": "{verified}/{linked} verified",

  "lens.mode.timing": "Milestones",
  "lens.mode.observability": "Outcome observability",
  "lens.mode.evidence": "Evidence",
  "lens.modeCopy.timing":
    "Marks Products with a milestone date that has passed. The horizontal axis is milestone dates; it doesn't mean work is overdue.",
  "lens.modeCopy.observability":
    "Marks Products with KPI definitions where fewer than half have an observation.",
  "lens.modeCopy.evidence":
    "Marks Products with Evidence that doesn't match its record, or hasn't been verified.",

  "lens.bubble.label":
    "{product}. {happened}. Milestones: {timing}. Outcome observability: {observability}. Verified Evidence: {coverage}.",
  // Each tooltip line is one sentence; {label} is the bold label element.
  "lens.tooltip.happened": "What happened",
  "lens.tooltip.happenedLine": "{label}: {happened}",
  "lens.tooltip.impact": "Impact",
  "lens.tooltip.impactLine": "{label}: not assessed by anyone yet",
  "lens.tooltip.next": "Next step",
  "lens.tooltip.nextLine": "{label}: select it to look closer on the right",
  "lens.tooltip.timing": "Milestones: {value}",
  "lens.tooltip.observability": "Outcome observability: {value}",
  "lens.tooltip.coverage": "Verified Evidence: {value}",
  "lens.canvas.label": "Portfolio Lens quadrant chart; the table below has the same data",
  "lens.axis.y": "Outcome observability →",
  "lens.band.noMilestones": "No milestones",
  "lens.band.noKpis": "No KPI definitions",
  "lens.axis.x": "Milestone dates: {later} → {dueSoon} → {datePassed}",

  "lens.table.caption": "Sorted by Product name: an order to browse by, not a priority.",
  "lens.table.product": "Product",
  "lens.table.timing": "Milestones",
  "lens.table.observability": "Outcome observability",
  "lens.table.coverage": "Verified Evidence",
  "lens.table.quadrant": "Quadrant",
  "lens.table.view": "View",
  "lens.table.notEnoughData": "Not enough data",
  "lens.table.selected": "Selected",
  "lens.table.viewProduct": "View {product}",
  "lens.table.selectedProduct": "Selected {product}",

  "lens.contribution.versionless":
    "the link has no version of its own; read at snapshot {revision}",
  "lens.contribution.version": "version {version}",
  "lens.contribution.line": "{kind} {id} ({version}, {classification})",
  "lens.measures.heading": "Lens measures for {product}",
  "lens.measures.ownClassification": "The Product's own classification",
  "lens.measures.classificationFrom": "Set by {kind} {id}",
  "lens.measures.quadrant": "Quadrant",
  "lens.measures.noQuadrant": "Not enough data to place it in a quadrant",
  "lens.measures.timing": "Milestones",
  "lens.measures.observability": "Outcome observability",
  "lens.measures.coverage": "Verified Evidence",
  "lens.measures.stateCount": "{state}: {count}",
  "lens.measures.sharedProjects": "Shared Projects",
  "lens.measures.sharedProjectList": "{ids} (also linked to other Products)",
  "lens.measures.sources": "Where these numbers come from ({count} records)",
  "lens.measures.noSources": "No linked records.",
} as const;
