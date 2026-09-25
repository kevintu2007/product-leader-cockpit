/**
 * English copy for the routes: what every route says while it reads, when it
 * can't, and when the Ledger moved under it -- then each route's own words.
 */
export const ROUTES_EN = {
  // Shared by every route.
  "route.loading": "Reading {route}.",
  "route.reload": "Read again",
  "route.unavailable": "{route} can't be read right now. Nothing old is shown. {message}",
  "route.outOfSync":
    "The Ledger changed while it was being read, so the two reads disagree and nothing is shown this time.",
  "route.ledgerRevision": "Ledger version",
  "route.readAt": "Read at",

  // S01, the Executive Cockpit.
  "cockpit.headline": "Good morning. Start with what really moves results today.",
  "cockpit.lede":
    "Every Product by milestone dates, outcome observability and verified Evidence. No ranking, no confidence estimate.",
  "cockpit.lensModes": "Marking",
  "cockpit.lensEmpty":
    "There are no Products in the Ledger yet. Once you create one, it's placed here by its milestones, KPIs and Evidence.",
  "cockpit.lensLegend":
    "A bigger circle means a larger share of verified Evidence; a dashed circle means no linked Evidence. A Product is placed in a quadrant only when both axes have data.",
  "cockpit.asideEmpty":
    "Pick a Product in the chart or the table to see its measures, the records behind them and where it stands.",
  "cockpit.period": "Period comparison",
  "cockpit.periodUnavailable": "Can't compare: {reason}",
  "cockpit.pulse": "Portfolio today",
  "cockpit.pulse.milestones": "Milestones",
  "cockpit.pulse.commitments": "Commitments",
  "cockpit.pulse.kpis": "KPIs",
  "cockpit.pulse.from": "From {owner}",
  "cockpit.attention": "Needs attention",
  "cockpit.attentionNone": "Nothing needs attention right now.",
  "cockpit.placedBecause": "Placed here: {tier}",
  "cockpit.briefing": "Your summary",
  "cockpit.briefingNone": "Nothing across the Portfolio needs your attention right now.",
  "cockpit.briefingTop.one": "{count} thing needs attention. The first is “{label}”: {reason}.",
  "cockpit.briefingTop.other": "{count} things need attention. The first is “{label}”: {reason}.",
  "cockpit.briefingWhy": "It comes first because: {tier}.",
  "cockpit.briefingNoPeriod":
    "There's no period to compare with yet, so you can't tell whether these are getting better or worse.",

  // S02, Portfolio.
  "portfolio.headline": "The Product portfolio",
  "portfolio.lede":
    "Each Product's milestones, outcome observability and verified Evidence, and the flagged work carried by the people responsible for it.",
  "portfolio.showing": "{total} in total, showing {from} to {to}.",
  "portfolio.empty":
    "There are no Products in the Ledger yet. Once you create some, they're listed here.",
  "portfolio.emptyPage": "No Products on this page; there are {total} in total.",
  "portfolio.firstPage": "Back to the first page",
  "portfolio.nextPage": "Show the next page",
  "portfolio.column.flagged": "Flagged work of the people responsible",
  "portfolio.column.classification": "Classification",
  "portfolio.flaggedNone": "None",
  "portfolio.flaggedCount": "{count}",
  "portfolio.asideEmpty":
    "Pick a Product in the table to see its measures, the records behind them and where it stands.",
  "productAside.label": "Selected Product",
} as const;
