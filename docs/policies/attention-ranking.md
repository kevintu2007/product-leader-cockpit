# Attention ranking policy

How the application orders attention flags. The code is `crates/pmc-application/src/attention_ranking.rs`. The Work Queue and the Executive Cockpit's exception list apply this order per record: a record is placed by its most severe flag, and its flags stay together. [Work Queue membership and ordering](work-queue-ordering.md) describes that step and extends it to unflagged work.

## What is ranked

Ranking orders the `AttentionFlag` values produced by `pmc_domain::attention::derive_attention`. That evaluator is a pure, read-only derivation over snapshots: evaluating attention cannot change lifecycle state, advance a clock or create audit records. Ranking keeps that property.

Every flag carries a `target`, a `reason` (one of 27 `AttentionReason` values), an `explanation`, and metadata with `classification`, `freshness` and `degraded`. Ranking adds no facts. It decides the order and reports which existing fact decided it.

## Tiers, not a weighted score

The order is lexicographic over named tiers. There is no weighted numeric score.

A weighted score hides its reasoning even when the weights are published: to answer "why is this above that" the reader has to trust arithmetic they did not choose, and two items can swap places because of a coefficient rather than anything true about the work. A tiered comparison answers the same question in words: this one is above that one because it is already overdue, and that one is only approaching its deadline. The comparison is total, deterministic and explainable one step at a time.

## The ordering

Tiers are compared first; the first tier that differs decides, and a lower tier number sorts higher.

| Tier | Meaning                                                  | Reasons                                                                                                                                                                                 |
| ---- | -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1    | Breached commitment: a promise is already broken         | `ActionOverdue`, `ActionRequestResponseOverdue`, `DecisionRequestOverdue`, `IssueResolutionOverdue`                                                                                     |
| 2    | Blocked: the work cannot proceed                         | `ActionBlocked`, `IssueBlocked`                                                                                                                                                         |
| 3    | Evidence integrity: the basis of a conclusion is unsound | `ActionNeedsEvidence`, `ActionEvidenceVerificationPending`, `IssueNeedsEvidence`, `RiskControlInvalid`, `RiskEvidenceStale`                                                             |
| 4    | Approaching a deadline                                   | `ActionAtRisk`, `ActionRequestResponseDue`, `DecisionRequestApproachingDeadline`, `IssueResolutionDue`, `RiskReviewDue`                                                                 |
| 5    | No accountable owner                                     | `ActionRequestMissingIntendedOwner`, `DecisionRequestMissingDecisionOwner`, `RiskMissingOwner`                                                                                          |
| 6    | Everything else                                          | `ActionRequestNeedsInfo`, `DecisionRequestNeedsInfo`, `ActionRequestStale`, `DecisionRequestStale`, `IssueStale`, `IssueRecurrence`, `RiskExposureIncreased`, `ActionSupersededPremise` |

Within a tier:

1. Earliest relevant timestamp first. The timestamp is the deadline of the record the flag was raised against: an Action's due date, an Action Request's response-due date, or a Risk's next review date. A flag with no timestamp sorts after every flag that has one, because an absent deadline is not an early one.
2. Canonical identifier ascending, as the final tie-break. The same inputs always produce the same sequence, so a rank never moves unless a fact moved.

## Staleness never promotes

`freshness` and `degraded` are reported and never improve an item's position.

If a fact is stale, the application does not know it is still true. Ranking a stale item as though its overdue status were current would present uncertainty as certainty. A stale or degraded item is therefore placed by the facts it carries, and its rank explanation says that the data is out of date, that its freshness is unknown, or that some source context was unavailable. Staleness does not hide an item either; it is shown with its uncertainty attached.

## Declaration order is not a severity

`AttentionReason` derives `Ord`, which orders variants by where they were written in the enum. That order is an accident, and using it as a severity would be a ranking nobody chose that would change silently if a variant were inserted. The tier of each reason is assigned by hand in `tier_of`.

## What every ranked item states

Each item states what happened, why it matters, the owner or that there is none, the due or review time, the next step the lifecycle allows, and why it is ranked where it is. The last is generated from the ordering itself (the tier that placed it, and whether a timestamp ordered it within that tier), so the stated reason cannot drift from the reason used.

No item claims progress or certainty that its source facts do not carry, and no item shows a model score, because none exists.

## What the application derives today

The ranking handles all 27 reasons, but the adapter that builds attention inputs from the Ledger (`crates/pmc-application/src/work_queue_adapter.rs`) can only derive flags from facts the Ledger read surfaces carry. Where a fact is missing, the input is set to whichever value produces no flag, so nothing is flagged for a reason that may not be true. As a result these reasons are never raised today:

- Action: blocked, needs evidence, evidence verification pending, superseded premise; and at risk, because no at-risk window is configured either.
- Action Request and Decision Request: needs info.
- Decision Request: approaching deadline and overdue, because the Ledger stores no decision deadline.
- Risk: exposure increased, evidence stale, control invalid, missing owner.
- Issue: blocked, needs evidence, resolution due and overdue, because the Ledger read surface carries no resolution deadline.
- Issue recurrence: the derivation exists, but the Ledger writers do not yet accept a recurrence link, so no stored Issue has one.
- Action Request, Decision Request and Issue staleness: these are raised only for stale data, and every snapshot row is reported as fresh and not degraded, because the Ledger returns no per-record read time.

The reasons that can appear today are: overdue Actions, Action Requests whose response is due or overdue, Action Requests and Decision Requests without an owner, and Risks whose review date has arrived.

## Consequences

- Adding a 28th `AttentionReason` does not compile until it is given a tier, so a new reason cannot fall into a default bucket.
- Reordering the enum cannot change any rank.
- Two items tie only if they share a tier, a relevant timestamp and a canonical identifier, which cannot happen for distinct records.
