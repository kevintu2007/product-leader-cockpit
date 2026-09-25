# Work Queue membership and ordering

Which records the Work Queue shows, and in what order. The code is `crates/pmc-application/src/work_queue_composition.rs`, fed by `crates/pmc-application/src/work_queue_adapter.rs`. It reuses the [attention ranking policy](attention-ranking.md) unchanged and answers the two questions that policy does not: which records belong in the queue, and where records with no attention flag go.

## Why the ranking policy is not enough

The ranking policy orders attention flags, so every item it ranks is an item something has flagged. The Work Queue covers Action Requests, Actions, Decision Requests, Risks and Issues, with the attention reason as one column among several. It is a queue of work that can still be done, not a second copy of the Cockpit's exception list, so it also contains records nothing has flagged.

## Membership

A record is in the Work Queue while its lifecycle state admits at least one intent.

The intents come from the domain's own state tables (`pmc_domain::state_intents`, and `action_allowed_intents` and `request_allowed_intents` in `pmc_domain::actions`), not from a separate list of "active" states. A hand-written list would be a copy of the state machine that could drift from the guards that enforce it. A record leaves the queue exactly when the domain stops offering anything to do with it.

One consequence: a Completed or Cancelled Action stays in the queue, because it can be reopened.

The intents a record shows are what its state admits, which is weaker than what may be executed now. Preparation, classification, policy and approval are further checks.

Membership by "has an attention flag" was rejected: it would duplicate the Cockpit's exception list and hide ordinary unflagged work, which is what a daily queue exists to show.

## Ordering

| Position | Group                                                           | Order within the group                                                            |
| -------- | --------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| 1–6      | The six ranking tiers, applied to the record's most severe flag | Placing timestamp, earliest first, absent last; then record kind; then identifier |
| 7        | Unflagged: nothing has flagged this record                      | The same keys                                                                     |

- A record with several flags is placed by its most severe one. Its flags are listed in ranking order, so the first one shown is the one that placed it, and the record states why it sits where it does.
- The placing timestamp is the placing flag's timestamp when the record is flagged, and the record's own deadline when it is not. A Risk flagged for a reason unrelated to its review date is not ordered by that date.
- The final tie-break compares the record kind and then the identifier, so two records of different kinds that share an identifier still order the same way on every run.
- Group 7 applies the ranking policy's own reasoning one step further: an absent deadline is not an early one, and a record nothing has flagged is not more urgent than one something has. Removing group 7 would leave the flagged order exactly as it is.

The Cockpit's exception list is built from the same composition: it is the flagged-only queue with each record's flags listed together (`portfolio_attention_from_snapshots`). The two screens therefore show the same records in the same order.

## Deadlines that are not invented

| Kind             | Deadline used                  |
| ---------------- | ------------------------------ |
| Action           | due date                       |
| Action Request   | response-due date, if recorded |
| Risk             | next review date, if recorded  |
| Decision Request | none                           |
| Issue            | none                           |

The Ledger stores no decision deadline, and the Ledger read surface carries no Issue resolution deadline. Decision Requests and Issues therefore sort as having no timestamp. No default deadline is substituted: a made-up deadline would place a record on a timeline the Ledger never recorded, and the reader could not tell.

## Filters and counts

- Kind filter: any combination of the five kinds. Selecting none shows every kind.
- Flagged only: shows only records with at least one flag.
- Filters narrow the list and never reorder it, so a filtered queue is always a subsequence of the unfiltered one.
- Each kind button shows a count. The count says what choosing that kind would show: it counts records of that kind that pass the flagged-only narrowing, and ignores the kind selection itself (`counts_by_kind`). Counting after the kind filter would make every unselected kind read as zero. All five kinds are always listed, because zero and "not counted" are different facts.
- Paging applies after filtering and ordering. The desktop shows 25 records per page.

## Consequences

- Membership cannot drift from the domain, because it is computed from the same state tables the guards use.
- A record never appears without at least one thing the reader can attempt next.
- Ordering stays total, deterministic and explainable one step at a time.
- The membership rule and group 7 are consumed only in the composition module, so either can change without touching the ranking policy, the domain or the Ledger.
