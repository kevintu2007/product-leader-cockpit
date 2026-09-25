//! Work Queue (S03) composition.
//!
//! These assert two documents at once: the Accepted ranking policy in
//! `docs/policies/attention-ranking.md`, whose ordering the
//! Work Queue reuses unchanged, and the membership-and-ordering
//! policy in `docs/policies/work-queue-ordering.md`,
//! which supplies only the two decisions reuse does not answer.
//!
//! Tests that pin the membership-and-ordering policy say so, so that if the
//! product owner amends it the tests to change are identifiable without
//! reading all of them. Both documents are Accepted.

use pmc_application::attention_ranking::{AttentionTier, RankableAttentionItem};
use pmc_application::route_composition::{PageState, RouteState};
use pmc_application::work_queue_composition::{
    compose_work_queue, is_outstanding, GetWorkQueue, WorkItemFacts, WorkItemKind,
    WorkItemPlacement, WorkQueueFilter,
};
use pmc_domain::attention::{
    AttentionFlag, AttentionMetadata, AttentionReason, AttentionTarget, Freshness,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{ActionId, ActionRequestId, DecisionRequestId, IssueId, RiskId};
use pmc_domain::time::UtcTimestamp;

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn target_for(kind: WorkItemKind, id: &str) -> AttentionTarget {
    match kind {
        WorkItemKind::ActionRequest => {
            AttentionTarget::ActionRequest(ActionRequestId::parse(id).unwrap())
        }
        WorkItemKind::Action => AttentionTarget::Action(ActionId::parse(id).unwrap()),
        WorkItemKind::DecisionRequest => {
            AttentionTarget::DecisionRequest(DecisionRequestId::parse(id).unwrap())
        }
        WorkItemKind::Risk => AttentionTarget::Risk(RiskId::parse(id).unwrap()),
        WorkItemKind::Issue => AttentionTarget::Issue(IssueId::parse(id).unwrap()),
    }
}

fn flag(
    kind: WorkItemKind,
    id: &str,
    reason: AttentionReason,
    relevant_at: Option<i64>,
) -> RankableAttentionItem {
    RankableAttentionItem {
        flag: AttentionFlag {
            target: target_for(kind, id),
            reason,
            metadata: AttentionMetadata {
                classification: DataClassification::Internal,
                freshness: Freshness::Fresh,
                degraded: false,
            },
            explanation: "fixture",
            failed_verification_guidance: None,
        },
        relevant_at: relevant_at.map(at),
    }
}

/// A queue-eligible item: it has at least one legal intent, so the
/// membership rule admits it.
fn facts(kind: WorkItemKind, id: &str, relevant_at: Option<i64>) -> WorkItemFacts {
    WorkItemFacts {
        kind,
        id: id.to_owned(),
        label: format!("{} {id}", kind.as_str()),
        state_label: "Open",
        lifecycle_legal_intents: vec!["prepare_something"],
        relevant_at: relevant_at.map(at),
        promised_at: None,
        classification: DataClassification::Internal,
        revision: 3,
        as_of: at(1_700_000_000_000),
        freshness: Freshness::Fresh,
        degraded: false,
        attention: Vec::new(),
    }
}

fn request(page: PageState) -> GetWorkQueue {
    GetWorkQueue {
        as_of: at(1_700_000_000_000),
        page,
        filter: WorkQueueFilter::default(),
    }
}

fn whole_queue() -> GetWorkQueue {
    request(PageState {
        offset: 0,
        limit: 100,
        total: 0,
    })
}

fn ids(body: &[pmc_application::work_queue_composition::WorkQueueItem]) -> Vec<&str> {
    body.iter().map(|item| item.id.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Membership -- membership-and-ordering policy §2.
// ---------------------------------------------------------------------------

#[test]
fn an_item_whose_lifecycle_admits_nothing_is_not_in_the_queue() {
    // Membership-and-ordering policy §2. A terminal record has nothing the reader can
    // attempt, so listing it would be listing work that cannot be done.
    let mut terminal = facts(WorkItemKind::Issue, "issue-1", None);
    terminal.lifecycle_legal_intents = Vec::new();
    assert!(!is_outstanding(&terminal));

    let composed = compose_work_queue(&whole_queue(), &[terminal], 9);

    assert!(composed.body.items.is_empty());
    assert_eq!(composed.state, RouteState::Empty);
}

#[test]
fn every_item_in_the_queue_offers_at_least_one_thing_to_attempt() {
    // The consequence the membership-and-ordering policy §5 claims. Stated as a property
    // over the composed output rather than trusted from the filter, because
    // the filter is what would be wrong if this were ever broken.
    let composed = compose_work_queue(
        &whole_queue(),
        &[
            facts(WorkItemKind::Action, "action-1", Some(10)),
            facts(WorkItemKind::Risk, "risk-1", None),
        ],
        9,
    );

    assert_eq!(composed.body.items.len(), 2);
    for item in &composed.body.items {
        assert!(
            !item.lifecycle_legal_intents.is_empty(),
            "{} is in the queue with nothing to attempt",
            item.id
        );
    }
}

// ---------------------------------------------------------------------------
// Ordering -- the Accepted policy, unchanged.
// ---------------------------------------------------------------------------

#[test]
fn a_breached_commitment_outranks_an_approaching_deadline() {
    // Accepted policy §3, tier 1 above tier 4. Reused, not reimplemented.
    let mut overdue = facts(WorkItemKind::Action, "action-2", Some(500));
    overdue.attention = vec![flag(
        WorkItemKind::Action,
        "action-2",
        AttentionReason::ActionOverdue,
        Some(500),
    )];
    let mut approaching = facts(WorkItemKind::Action, "action-1", Some(100));
    approaching.attention = vec![flag(
        WorkItemKind::Action,
        "action-1",
        AttentionReason::ActionAtRisk,
        Some(100),
    )];

    let composed = compose_work_queue(&whole_queue(), &[approaching, overdue], 9);

    // The overdue item wins despite the sooner timestamp and later id on the
    // other, so the tier -- not the clock or the id -- decided it.
    assert_eq!(ids(&composed.body.items), ["action-2", "action-1"]);
    assert_eq!(
        composed.body.items[0].placement,
        WorkItemPlacement::Flagged(AttentionTier::BreachedCommitment)
    );
}

#[test]
fn an_item_is_placed_by_its_most_severe_flag() {
    // Membership-and-ordering policy §3: several flags, placed by the worst. Ordering by the
    // first flag supplied, or by an average, would move an item because of a
    // fact that is not the reason it is there.
    let mut many = facts(WorkItemKind::Issue, "issue-2", None);
    many.attention = vec![
        flag(
            WorkItemKind::Issue,
            "issue-2",
            AttentionReason::IssueStale,
            None,
        ),
        flag(
            WorkItemKind::Issue,
            "issue-2",
            AttentionReason::IssueResolutionOverdue,
            Some(700),
        ),
    ];
    let mut blocked = facts(WorkItemKind::Issue, "issue-1", None);
    blocked.attention = vec![flag(
        WorkItemKind::Issue,
        "issue-1",
        AttentionReason::IssueBlocked,
        None,
    )];

    let composed = compose_work_queue(&whole_queue(), &[blocked, many], 9);

    // issue-2's worst flag is tier 1; issue-1's only flag is tier 2.
    assert_eq!(ids(&composed.body.items), ["issue-2", "issue-1"]);
    assert_eq!(
        composed.body.items[0].placement,
        WorkItemPlacement::Flagged(AttentionTier::BreachedCommitment)
    );
}

#[test]
fn a_flagged_item_sorts_above_an_unflagged_one() {
    // Membership-and-ordering policy §3, group 7. This is the appended rule
    // and the one to
    // revisit if the product owner amends the policy.
    let mut flagged = facts(WorkItemKind::Risk, "risk-9", Some(9_000));
    flagged.attention = vec![flag(
        WorkItemKind::Risk,
        "risk-9",
        // Tier 6 -- the weakest flag there is, so this test shows that ANY
        // flag outranks no flag, not merely that a severe one does.
        AttentionReason::RiskExposureIncreased,
        None,
    )];
    let unflagged = facts(WorkItemKind::Risk, "risk-1", Some(1));

    let composed = compose_work_queue(&whole_queue(), &[unflagged, flagged], 9);

    assert_eq!(ids(&composed.body.items), ["risk-9", "risk-1"]);
    assert_eq!(
        composed.body.items[0].placement,
        WorkItemPlacement::Flagged(AttentionTier::Other)
    );
    assert_eq!(
        composed.body.items[1].placement,
        WorkItemPlacement::Unflagged
    );
}

#[test]
fn an_absent_timestamp_sorts_after_a_present_one() {
    // Accepted policy §3 tie-break 1, applied within the unflagged group:
    // "an absent deadline is not an early one". This is what keeps Decision
    // Requests and Issues from floating to the top merely by having no
    // deadline to compare.
    let composed = compose_work_queue(
        &whole_queue(),
        &[
            facts(WorkItemKind::Action, "action-1", None),
            facts(WorkItemKind::Action, "action-2", Some(5_000)),
        ],
        9,
    );

    assert_eq!(ids(&composed.body.items), ["action-2", "action-1"]);
}

#[test]
fn the_deadlineless_kinds_are_not_given_a_substituted_deadline() {
    // Membership-and-ordering policy §4. The Ledger persists no deadline for Decision
    // Requests or Issues. Composition must carry that absence through rather
    // than defaulting it, because a substituted deadline would place a record
    // on a timeline the Ledger never recorded and the reader could not tell.
    let composed = compose_work_queue(
        &whole_queue(),
        &[
            facts(WorkItemKind::DecisionRequest, "decision-request-1", None),
            facts(WorkItemKind::Issue, "issue-1", None),
        ],
        9,
    );

    for item in &composed.body.items {
        assert_eq!(
            item.relevant_at, None,
            "{} was given a deadline the Ledger does not hold",
            item.id
        );
        assert_eq!(item.placing_timestamp(), None);
    }
}

#[test]
fn ordering_does_not_depend_on_the_order_facts_arrive_in() {
    // "Total and deterministic: the same inputs always produce the same
    // sequence, so a rank never moves unless a fact moved."
    let supplied = [
        facts(WorkItemKind::Issue, "issue-1", Some(300)),
        facts(WorkItemKind::Action, "action-1", Some(100)),
        facts(WorkItemKind::Risk, "risk-1", Some(200)),
    ];
    let reversed: Vec<WorkItemFacts> = supplied.iter().rev().cloned().collect();

    let first = compose_work_queue(&whole_queue(), &supplied, 9);
    let second = compose_work_queue(&whole_queue(), &reversed, 9);

    assert_eq!(ids(&first.body.items), ids(&second.body.items));
    assert_eq!(ids(&first.body.items), ["action-1", "risk-1", "issue-1"]);
}

#[test]
fn two_kinds_sharing_an_identifier_still_order_the_same_way_every_run() {
    // The final tie-break includes the kind. Without it, two records of
    // different types with the same identifier and timestamp would tie
    // completely and their order would depend on the sort's stability rather
    // than on anything true.
    let supplied = [
        facts(WorkItemKind::Risk, "shared-1", Some(100)),
        facts(WorkItemKind::Action, "shared-1", Some(100)),
    ];
    let reversed: Vec<WorkItemFacts> = supplied.iter().rev().cloned().collect();

    let first = compose_work_queue(&whole_queue(), &supplied, 9);
    let second = compose_work_queue(&whole_queue(), &reversed, 9);

    let kinds_of = |result: &pmc_application::route_composition::CompositionResult<
        pmc_application::work_queue_composition::WorkQueueBody,
    >| {
        result
            .body
            .items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds_of(&first), kinds_of(&second));
    assert_eq!(
        kinds_of(&first),
        [WorkItemKind::Action, WorkItemKind::Risk],
        "the kind tie-break should be alphabetical by its stable identifier"
    );
}

// ---------------------------------------------------------------------------
// Preserving separate lifecycle types -- a Work Queue requirement.
// ---------------------------------------------------------------------------

#[test]
fn every_item_states_which_lifecycle_it_belongs_to() {
    let composed = compose_work_queue(
        &whole_queue(),
        &WorkItemKind::ALL
            .iter()
            .map(|kind| facts(*kind, &format!("{}-1", kind.as_str()), Some(10)))
            .collect::<Vec<_>>(),
        9,
    );

    assert_eq!(composed.body.items.len(), 5);
    let kinds: std::collections::HashSet<_> =
        composed.body.items.iter().map(|item| item.kind).collect();
    assert_eq!(
        kinds.len(),
        5,
        "the five lifecycle types were collapsed into fewer"
    );
}

#[test]
fn every_kind_is_counted_even_when_it_has_none() {
    // Zero and absent are different facts. A kind missing from the counts
    // would read as "not counted" rather than "none outstanding", which is
    // exactly the fabricated-absence failure this project has paid for.
    let composed = compose_work_queue(
        &whole_queue(),
        &[facts(WorkItemKind::Action, "action-1", Some(10))],
        9,
    );

    assert_eq!(composed.body.counts_by_kind.len(), WorkItemKind::ALL.len());
    for kind in WorkItemKind::ALL {
        let counted = composed
            .body
            .counts_by_kind
            .iter()
            .find(|entry| entry.kind == kind)
            .unwrap_or_else(|| panic!("{} is not counted at all", kind.as_str()));
        let expected = usize::from(kind == WorkItemKind::Action);
        assert_eq!(counted.count, expected, "{} counted wrong", kind.as_str());
    }
}

#[test]
fn each_kind_is_owned_by_the_module_that_may_change_it() {
    // Composition reads across owners; it never becomes one. A kind
    // attributed to the wrong module would let a surface send a write intent
    // to a module with no authority over the record.
    assert_eq!(
        WorkItemKind::ActionRequest.owner(),
        WorkItemKind::Action.owner(),
        "Action Requests and Actions are both Action Management's"
    );
    let owners: std::collections::HashSet<_> = WorkItemKind::ALL
        .iter()
        .map(|kind| kind.owner().as_str())
        .collect();
    assert_eq!(owners.len(), 4, "five kinds across four owning modules");
}

#[test]
fn every_kind_has_a_distinct_stable_identifier() {
    let seen: std::collections::HashSet<_> =
        WorkItemKind::ALL.iter().map(|kind| kind.as_str()).collect();
    assert_eq!(seen.len(), WorkItemKind::ALL.len());
}

// ---------------------------------------------------------------------------
// Filters -- DG0 lists them as Work Queue content.
// ---------------------------------------------------------------------------

#[test]
fn a_kind_filter_narrows_without_reordering() {
    let supplied: Vec<WorkItemFacts> = vec![
        facts(WorkItemKind::Action, "action-1", Some(100)),
        facts(WorkItemKind::Issue, "issue-1", Some(200)),
        facts(WorkItemKind::Action, "action-2", Some(300)),
    ];
    let unfiltered = compose_work_queue(&whole_queue(), &supplied, 9);

    let mut narrowed = whole_queue();
    narrowed.filter.kinds = vec![WorkItemKind::Action];
    let filtered = compose_work_queue(&narrowed, &supplied, 9);

    assert_eq!(ids(&filtered.body.items), ["action-1", "action-2"]);
    // A filtered queue is a subsequence of the unfiltered one: the surviving
    // items keep their relative order.
    let unfiltered_order: Vec<&str> = ids(&unfiltered.body.items)
        .into_iter()
        .filter(|id| id.starts_with("action"))
        .collect();
    assert_eq!(ids(&filtered.body.items), unfiltered_order);
}

#[test]
fn an_empty_kind_filter_shows_everything_rather_than_nothing() {
    // The default filter must not be an accidental "show nothing".
    let composed = compose_work_queue(
        &whole_queue(),
        &[
            facts(WorkItemKind::Action, "action-1", Some(10)),
            facts(WorkItemKind::Risk, "risk-1", Some(20)),
        ],
        9,
    );

    assert!(whole_queue().filter.kinds.is_empty());
    assert_eq!(composed.body.items.len(), 2);
}

#[test]
fn the_flagged_only_filter_drops_exactly_the_unflagged() {
    let mut flagged = facts(WorkItemKind::Risk, "risk-1", Some(10));
    flagged.attention = vec![flag(
        WorkItemKind::Risk,
        "risk-1",
        AttentionReason::RiskMissingOwner,
        None,
    )];
    let supplied = vec![flagged, facts(WorkItemKind::Risk, "risk-2", Some(20))];

    let mut only_flagged = whole_queue();
    only_flagged.filter.only_flagged = true;
    let composed = compose_work_queue(&only_flagged, &supplied, 9);

    assert_eq!(ids(&composed.body.items), ["risk-1"]);
    assert_eq!(composed.body.page.total, 1);
}

fn count_of(
    body: &pmc_application::work_queue_composition::WorkQueueBody,
    kind: WorkItemKind,
) -> usize {
    body.counts_by_kind
        .iter()
        .find(|entry| entry.kind == kind)
        .map_or(0, |entry| entry.count)
}

#[test]
fn a_kind_count_says_what_choosing_that_kind_would_show() {
    // With only Issues selected, the Action chip must still say how many
    // Actions there are; a zero there reads as "there are none".
    let supplied = vec![
        facts(WorkItemKind::Action, "action-1", Some(10)),
        facts(WorkItemKind::Action, "action-2", Some(11)),
        facts(WorkItemKind::Issue, "issue-1", Some(12)),
    ];
    let mut issues_only = whole_queue();
    issues_only.filter.kinds = vec![WorkItemKind::Issue];

    let composed = compose_work_queue(&issues_only, &supplied, 9);

    assert_eq!(ids(&composed.body.items), ["issue-1"]);
    assert_eq!(count_of(&composed.body, WorkItemKind::Action), 2);
    assert_eq!(count_of(&composed.body, WorkItemKind::Issue), 1);
}

#[test]
fn kind_counts_still_follow_the_flagged_only_narrowing() {
    let mut flagged = facts(WorkItemKind::Risk, "risk-1", Some(10));
    flagged.attention = vec![flag(
        WorkItemKind::Risk,
        "risk-1",
        AttentionReason::RiskMissingOwner,
        None,
    )];
    let supplied = vec![
        flagged,
        facts(WorkItemKind::Risk, "risk-2", Some(20)),
        facts(WorkItemKind::Action, "action-1", Some(30)),
    ];
    let mut only_flagged = whole_queue();
    only_flagged.filter.only_flagged = true;

    let composed = compose_work_queue(&only_flagged, &supplied, 9);

    assert_eq!(count_of(&composed.body, WorkItemKind::Risk), 1);
    assert_eq!(count_of(&composed.body, WorkItemKind::Action), 0);
}

// ---------------------------------------------------------------------------
// Paging -- the Work Queue is a non-virtualized list with paging.
// ---------------------------------------------------------------------------

#[test]
fn the_page_reports_the_filtered_total_not_the_page_length() {
    // A total equal to the page length would tell the reader they are seeing
    // everything when they are seeing the first page, which is the kind of
    // quiet false completeness this project forbids.
    let supplied: Vec<WorkItemFacts> = (0..7)
        .map(|index| {
            facts(
                WorkItemKind::Action,
                &format!("action-{index}"),
                Some(i64::from(index)),
            )
        })
        .collect();

    let composed = compose_work_queue(
        &request(PageState {
            offset: 0,
            limit: 3,
            total: 0,
        }),
        &supplied,
        9,
    );

    assert_eq!(composed.body.items.len(), 3);
    assert_eq!(composed.body.page.total, 7);
    assert!(composed.body.page.has_more());
}

#[test]
fn counts_by_kind_describe_the_whole_filtered_queue_not_the_visible_page() {
    // The counts drive the filter UI. Counting only the visible page would
    // make a filter appear to have no matches when it has plenty.
    let supplied: Vec<WorkItemFacts> = (0..5)
        .map(|index| {
            facts(
                WorkItemKind::Issue,
                &format!("issue-{index}"),
                Some(i64::from(index)),
            )
        })
        .collect();

    let composed = compose_work_queue(
        &request(PageState {
            offset: 0,
            limit: 2,
            total: 0,
        }),
        &supplied,
        9,
    );

    assert_eq!(composed.body.items.len(), 2);
    let issues = composed
        .body
        .counts_by_kind
        .iter()
        .find(|entry| entry.kind == WorkItemKind::Issue)
        .unwrap();
    assert_eq!(issues.count, 5);
}

#[test]
fn a_page_past_the_end_is_empty_rather_than_wrong() {
    let composed = compose_work_queue(
        &request(PageState {
            offset: 50,
            limit: 10,
            total: 0,
        }),
        &[facts(WorkItemKind::Action, "action-1", Some(10))],
        9,
    );

    assert!(composed.body.items.is_empty());
    assert_eq!(composed.state, RouteState::Empty);
    // The total still describes the queue, so the surface can tell the reader
    // the page is out of range rather than that the queue is empty.
    assert_eq!(composed.body.page.total, 1);
}

// ---------------------------------------------------------------------------
// What every item must state.
// ---------------------------------------------------------------------------

#[test]
fn every_item_says_why_it_is_where_it_is() {
    // Accepted policy §6: every item answers "why is it ranked where it is",
    // produced from the ordering itself so the stated reason cannot drift
    // from the reason used.
    let mut flagged = facts(WorkItemKind::Action, "action-1", Some(10));
    flagged.attention = vec![flag(
        WorkItemKind::Action,
        "action-1",
        AttentionReason::ActionOverdue,
        Some(10),
    )];
    let composed = compose_work_queue(
        &whole_queue(),
        &[flagged, facts(WorkItemKind::Action, "action-2", Some(20))],
        9,
    );

    for item in &composed.body.items {
        assert!(
            !item.placement_rationale.is_empty(),
            "{} does not say why it is ranked where it is",
            item.id
        );
    }
    assert_eq!(
        composed.body.items[0].placement_rationale,
        AttentionTier::BreachedCommitment.why()
    );
    assert!(
        composed.body.items[1]
            .placement_rationale
            .contains("nothing has flagged it"),
        "an unflagged item must say that is why it is low, not stay silent"
    );
}

#[test]
fn the_composition_reports_the_revision_it_was_read_at() {
    // So a surface can tell that two fields were read from the same instant.
    let composed = compose_work_queue(
        &whole_queue(),
        &[facts(WorkItemKind::Action, "action-1", Some(10))],
        42,
    );

    assert_eq!(composed.ledger_revision, 42);
    assert_eq!(composed.as_of, at(1_700_000_000_000));
}
