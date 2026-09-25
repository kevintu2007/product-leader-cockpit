//! The production Work Queue adapter (S03).
//!
//! These check that the adapter reports what the Ledger actually holds, and
//! -- more importantly -- that it does not report what the Ledger does not.
//! The failure this project has repeatedly paid for is a value that exists in
//! a shape while no code path produces it, so several of these assert an
//! **absence** on purpose.

use pmc_application::attention_ranking::{canonical_id_of, rank_attention, RankableAttentionItem};
use pmc_application::cockpit_adapter::compose_cockpit_from_snapshots;
use pmc_application::cockpit_aggregation::NoApprovedPeriodPort;
use pmc_application::work_queue_adapter::{
    portfolio_attention_from_snapshots, work_item_facts_from_snapshots,
};
use pmc_application::work_queue_composition::{
    compose_work_queue, GetWorkQueue, WorkItemFacts, WorkItemKind, WorkQueueFilter,
};
use pmc_domain::attention::{AttentionReason, AttentionThresholds};
use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{
    ActionRequestReadRecord, DecisionRequestReadRecord, IssueReadRecord, LedgerCompositionSnapshot,
    RiskReadRecord,
};
use pmc_domain::identity::{ActionId, ActionRequestId, AggregateVersion, RiskId, StakeholderId};
use pmc_domain::projection_source::{
    ActionProjectionSource, LedgerProjectionSnapshot, RiskProjectionSource,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, ActionState, DecisionRequestState, IssueState, RiskState,
};

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

fn at(millis: i64) -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(millis)
}

fn version(value: u64) -> AggregateVersion {
    AggregateVersion::new(value).unwrap()
}

fn thresholds() -> AttentionThresholds {
    AttentionThresholds {
        decision_approaching_deadline_millis: 0,
        action_at_risk_millis: None,
    }
}

fn composition() -> LedgerCompositionSnapshot {
    LedgerCompositionSnapshot {
        schema_version: 42,
        ledger_revision: 7,
        ledger_as_of_utc: now(),
        stakeholders: Vec::new(),
        stakeholder_relationships: Vec::new(),
        milestones: Vec::new(),
        action_requests: vec![
            ActionRequestReadRecord {
                id: "request-open".to_owned(),
                title: "Synthetic open request".to_owned(),
                intended_owner_id: Some(StakeholderId::parse("stakeholder-a").unwrap()),
                state: ActionRequestState::Open,
                response_due_at: Some(at(9_000)),
                intended_action_due_at: None,
                classification: DataClassification::Internal,
                version: version(4),
            },
            ActionRequestReadRecord {
                id: "request-accepted".to_owned(),
                title: "Synthetic accepted request".to_owned(),
                intended_owner_id: Some(StakeholderId::parse("stakeholder-a").unwrap()),
                state: ActionRequestState::Accepted,
                response_due_at: None,
                intended_action_due_at: None,
                classification: DataClassification::Internal,
                version: version(5),
            },
        ],
        decision_requests: vec![DecisionRequestReadRecord {
            id: "decision-request-1".to_owned(),
            subject: "Synthetic decision subject".to_owned(),
            intended_owner_id: None,
            state: DecisionRequestState::Open,
            classification: DataClassification::Internal,
            version: version(6),
        }],
        risks: Vec::new(),
        issues: vec![IssueReadRecord {
            id: "issue-1".to_owned(),
            title: "Synthetic issue".to_owned(),
            state: IssueState::Open,
            source_risk_id: None,
            recurrence_of_id: None,
            classification: DataClassification::Internal,
            version: version(7),
        }],
        portfolio_relationships: Vec::new(),
        products: Vec::new(),
        initiatives: Vec::new(),
        projects: Vec::new(),
        roadmaps: Vec::new(),
        kpi_definitions: Vec::new(),
        evidence_links: Vec::new(),
        evidence_references: Vec::new(),
        kpi_observations: Vec::new(),
        work_owners: Vec::new(),
    }
}

fn projection() -> LedgerProjectionSnapshot {
    LedgerProjectionSnapshot {
        schema_version: 42,
        ledger_revision: 7,
        ledger_as_of_utc: now(),
        products: Vec::new(),
        projects: Vec::new(),
        actions: vec![ActionProjectionSource {
            id: ActionId::parse("action-1").unwrap(),
            classification: DataClassification::Internal,
            source_revision: version(8),
            state: ActionState::Open,
            // Before `now`, so it is genuinely overdue.
            due_at: at(1_000),
            source_request_id: ActionRequestId::parse("request-accepted").unwrap(),
            source_decision_id: None,
        }],
        risks: vec![RiskProjectionSource {
            id: RiskId::parse("risk-1").unwrap(),
            classification: DataClassification::Internal,
            source_revision: version(9),
            state: RiskState::Open,
            next_review_at: Some(at(2_000)),
        }],
        decisions: Vec::new(),
        kpis: Vec::new(),
    }
}

fn facts() -> Vec<WorkItemFacts> {
    work_item_facts_from_snapshots(&projection(), &composition(), thresholds())
}

fn find(facts: &[WorkItemFacts], kind: WorkItemKind, id: &str) -> WorkItemFacts {
    facts
        .iter()
        .find(|item| item.kind == kind && item.id == id)
        .unwrap_or_else(|| panic!("{} {id} is missing", kind.as_str()))
        .clone()
}

// ---------------------------------------------------------------------------
// The Cockpit and the Work Queue see the same attention (DG3 S01 amendment §7).
// ---------------------------------------------------------------------------

fn every_queue_flag() -> Vec<RankableAttentionItem> {
    facts()
        .into_iter()
        .flat_map(|item| item.attention)
        .collect()
}

/// The flagged Work Queue as a reader sees it: records in queue order, each
/// record's flags together.
fn flagged_queue_in_order(
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
) -> Vec<pmc_application::attention_ranking::RankedAttentionItem> {
    let facts = work_item_facts_from_snapshots(projection, composition, thresholds());
    compose_work_queue(
        &GetWorkQueue {
            as_of: now(),
            page: pmc_application::route_composition::PageState {
                offset: 0,
                limit: 100,
                total: 0,
            },
            filter: WorkQueueFilter {
                kinds: Vec::new(),
                only_flagged: true,
            },
        },
        &facts,
        composition.ledger_revision,
    )
    .body
    .items
    .into_iter()
    .flat_map(|item| item.attention)
    .collect()
}

#[test]
fn portfolio_attention_is_the_flagged_work_queue_in_its_own_order() {
    assert_eq!(
        portfolio_attention_from_snapshots(&projection(), &composition(), thresholds()),
        flagged_queue_in_order(&projection(), &composition())
    );
}

#[test]
fn one_records_reasons_stay_together_as_they_do_in_the_work_queue() {
    // A record with two flags. Ranked globally, its missing-owner flag would
    // drop below other records' breached commitments and the Cockpit would
    // read in a different order from the Work Queue for the same facts.
    let mut composition = composition();
    composition.action_requests.push(ActionRequestReadRecord {
        id: "request-ownerless".to_owned(),
        title: "Synthetic ownerless overdue request".to_owned(),
        intended_owner_id: None,
        state: ActionRequestState::Open,
        response_due_at: Some(at(500)),
        intended_action_due_at: None,
        classification: DataClassification::Internal,
        version: version(10),
    });

    let lens = portfolio_attention_from_snapshots(&projection(), &composition, thresholds());
    let order: Vec<(&str, AttentionReason)> = lens
        .iter()
        .map(|item| (canonical_id_of(&item.flag.target), item.flag.reason))
        .collect();

    assert_eq!(lens, flagged_queue_in_order(&projection(), &composition));
    assert_eq!(
        &order[..2],
        &[
            (
                "request-ownerless",
                AttentionReason::ActionRequestResponseOverdue
            ),
            (
                "request-ownerless",
                AttentionReason::ActionRequestMissingIntendedOwner
            ),
        ],
        "a record's reasons must stay together: {order:?}"
    );
    // And this scenario really does distinguish the two orderings.
    let every_flag: Vec<RankableAttentionItem> =
        work_item_facts_from_snapshots(&projection(), &composition, thresholds())
            .into_iter()
            .flat_map(|item| item.attention)
            .collect();
    assert_ne!(lens, rank_attention(&every_flag));
}

#[test]
fn the_cockpit_lists_exactly_the_flags_the_work_queue_shows() {
    // The defect this closes: the Cockpit read only the projection snapshot,
    // which has no Action Requests, and said nothing needed attention while
    // the Work Queue listed requests past their response deadline.
    let cockpit = compose_cockpit_from_snapshots(
        &projection(),
        &composition(),
        thresholds(),
        &NoApprovedPeriodPort,
    );

    assert_eq!(
        cockpit.body.exceptions,
        flagged_queue_in_order(&projection(), &composition())
    );
    // Every flag the adapter raised is there, and nothing else.
    assert_eq!(cockpit.body.exceptions.len(), every_queue_flag().len());
    assert!(
        cockpit.body.exceptions.iter().any(|item| {
            canonical_id_of(&item.flag.target) == "request-open"
                && item.flag.reason == AttentionReason::ActionRequestResponseOverdue
        }),
        "the overdue Action Request must reach the Cockpit"
    );
}

#[test]
fn the_cockpit_reports_the_composition_revision_it_was_built_at() {
    let cockpit = compose_cockpit_from_snapshots(
        &projection(),
        &composition(),
        thresholds(),
        &NoApprovedPeriodPort,
    );
    assert_eq!(cockpit.ledger_revision, composition().ledger_revision);
    assert_eq!(cockpit.as_of, composition().ledger_as_of_utc);
}

// ---------------------------------------------------------------------------
// Reading across two ports.
// ---------------------------------------------------------------------------

#[test]
fn all_five_lifecycle_types_are_read_although_they_live_behind_two_ports() {
    // Actions and Risks come from the projection snapshot; the other three
    // from the composition snapshot. A surface must not have to know which.
    let facts = facts();
    for kind in WorkItemKind::ALL {
        assert!(
            facts.iter().any(|item| item.kind == kind),
            "no {} was read at all",
            kind.as_str()
        );
    }
}

#[test]
fn each_item_carries_the_revision_of_its_own_record() {
    // Not the snapshot's revision. A single revision copied onto every item
    // would make two records look like they moved together when they did not.
    let facts = facts();
    assert_eq!(
        find(&facts, WorkItemKind::ActionRequest, "request-open").revision,
        4
    );
    assert_eq!(find(&facts, WorkItemKind::Action, "action-1").revision, 8);
    assert_eq!(
        find(&facts, WorkItemKind::DecisionRequest, "decision-request-1").revision,
        6
    );
    assert_eq!(find(&facts, WorkItemKind::Risk, "risk-1").revision, 9);
    assert_eq!(find(&facts, WorkItemKind::Issue, "issue-1").revision, 7);
}

// ---------------------------------------------------------------------------
// Intents are read from the domain, not restated here.
// ---------------------------------------------------------------------------

#[test]
fn the_intents_on_an_item_are_the_domains_own_table() {
    // Compared against the domain function rather than against a literal, so
    // this test cannot pass while the adapter drifts from the guards.
    let facts = facts();
    assert_eq!(
        find(&facts, WorkItemKind::ActionRequest, "request-open").lifecycle_legal_intents,
        pmc_domain::actions::request_allowed_intents(ActionRequestState::Open).to_vec()
    );
    assert_eq!(
        find(&facts, WorkItemKind::Action, "action-1").lifecycle_legal_intents,
        pmc_domain::actions::action_allowed_intents(ActionState::Open).to_vec()
    );
    assert_eq!(
        find(&facts, WorkItemKind::DecisionRequest, "decision-request-1").lifecycle_legal_intents,
        pmc_domain::state_intents::decision_request_state_intents(DecisionRequestState::Open)
            .to_vec()
    );
    assert_eq!(
        find(&facts, WorkItemKind::Risk, "risk-1").lifecycle_legal_intents,
        pmc_domain::state_intents::risk_state_intents(RiskState::Open).to_vec()
    );
    assert_eq!(
        find(&facts, WorkItemKind::Issue, "issue-1").lifecycle_legal_intents,
        pmc_domain::state_intents::issue_state_intents(IssueState::Open).to_vec()
    );
}

#[test]
fn a_terminal_record_is_read_but_composes_out_of_the_queue() {
    // The Accepted Action Request is read -- the adapter does not filter --
    // and then falls out of the queue because its state admits nothing. The
    // judgement lives in one place, visibly, rather than in both.
    let facts = facts();
    let accepted = find(&facts, WorkItemKind::ActionRequest, "request-accepted");
    assert!(accepted.lifecycle_legal_intents.is_empty());

    let composed = compose_work_queue(
        &GetWorkQueue {
            as_of: now(),
            page: pmc_application::route_composition::PageState {
                offset: 0,
                limit: 100,
                total: 0,
            },
            filter: WorkQueueFilter::default(),
        },
        &facts,
        7,
    );
    assert!(
        !composed
            .body
            .items
            .iter()
            .any(|item| item.id == "request-accepted"),
        "a record whose lifecycle admits nothing reached the queue"
    );
}

// ---------------------------------------------------------------------------
// Attention: derived where the facts exist, absent where they do not.
// ---------------------------------------------------------------------------

#[test]
fn an_overdue_action_is_genuinely_flagged_from_the_snapshot() {
    // The positive path. A test suite that only asserted "we never fabricate"
    // would pass with nothing derived at all.
    let facts = facts();
    let action = find(&facts, WorkItemKind::Action, "action-1");

    assert!(
        action
            .attention
            .iter()
            .any(|item| item.flag.reason == AttentionReason::ActionOverdue),
        "an action due at {:?} and read as_of {:?} was not flagged overdue",
        action.relevant_at,
        now()
    );
}

#[test]
fn a_flag_is_paired_with_the_deadline_of_the_record_it_came_from() {
    // So a rank can never be computed against a deadline belonging to some
    // other record.
    let facts = facts();
    let action = find(&facts, WorkItemKind::Action, "action-1");

    for item in &action.attention {
        assert_eq!(canonical_id_of(&item.flag.target), "action-1");
        assert_eq!(item.relevant_at, Some(at(1_000)));
    }
}

#[test]
fn a_risk_is_never_flagged_for_a_missing_owner_the_snapshot_cannot_report() {
    // The one unknown the adapter sets to `true`. `owner_present: false`
    // would CREATE a tier-5 flag and push the Risk up the queue for a reason
    // that may not be true. Absence of the fact must produce silence, not an
    // accusation.
    let facts = facts();
    let risk = find(&facts, WorkItemKind::Risk, "risk-1");

    assert!(
        !risk
            .attention
            .iter()
            .any(|item| item.flag.reason == AttentionReason::RiskMissingOwner),
        "a missing-owner flag was fabricated for a Risk whose owner the snapshot does not carry"
    );
}

#[test]
fn a_decision_request_missing_its_owner_is_flagged_because_that_fact_is_carried() {
    // The counterpart to the Risk case, and the reason that case is a
    // deliberate choice rather than a blanket "never flag missing owners".
    // The composition snapshot does carry `intended_owner_id`, so this flag
    // is derived from a real absence in the Ledger.
    let facts = facts();
    let request = find(&facts, WorkItemKind::DecisionRequest, "decision-request-1");

    assert!(
        request
            .attention
            .iter()
            .any(|item| item.flag.reason == AttentionReason::DecisionRequestMissingDecisionOwner),
        "a Decision Request with no owner in the Ledger was not flagged"
    );
}

#[test]
fn an_issue_is_never_flagged_for_evidence_the_adapter_cannot_inspect() {
    // `ResolvedIssueVerification::Pending` would raise an evidence-integrity
    // flag; the adapter cannot substantiate one, so it asserts nothing.
    let facts = facts();
    let issue = find(&facts, WorkItemKind::Issue, "issue-1");

    assert!(
        !issue.attention.iter().any(|item| matches!(
            item.flag.reason,
            AttentionReason::IssueNeedsEvidence | AttentionReason::IssueBlocked
        )),
        "an evidence or blocked flag was fabricated for an Issue"
    );
}

// ---------------------------------------------------------------------------
// Deadlines that do not exist stay absent.
// ---------------------------------------------------------------------------

#[test]
fn the_kinds_the_ledger_holds_no_deadline_for_report_none() {
    // Membership-and-ordering policy §4. Confirmed against the read records,
    // which carry no
    // deadline field at all for these two -- so this is asserting that the
    // adapter did not invent one from somewhere else.
    let facts = facts();
    assert_eq!(
        find(&facts, WorkItemKind::DecisionRequest, "decision-request-1").relevant_at,
        None
    );
    assert_eq!(
        find(&facts, WorkItemKind::Issue, "issue-1").relevant_at,
        None
    );
}

#[test]
fn no_deadline_flag_is_derived_for_the_kinds_that_have_no_deadline() {
    // The companion to the test above, and the one that actually covers the
    // dangerous path. `relevant_at` and the `AttentionInputs` are populated
    // separately, so a substituted deadline could reach `derive_attention`
    // and fabricate an overdue flag while `relevant_at` stayed honestly
    // `None`. This asserts the second path, not just the first.
    let facts = facts();
    let request = find(&facts, WorkItemKind::DecisionRequest, "decision-request-1");
    for item in &request.attention {
        assert!(
            !matches!(
                item.flag.reason,
                AttentionReason::DecisionRequestOverdue
                    | AttentionReason::DecisionRequestApproachingDeadline
            ),
            "a deadline flag was derived for a Decision Request the Ledger holds no deadline for"
        );
        assert_eq!(
            item.relevant_at, None,
            "a Decision Request flag was paired with a deadline that does not exist"
        );
    }

    let issue = find(&facts, WorkItemKind::Issue, "issue-1");
    for item in &issue.attention {
        assert!(
            !matches!(
                item.flag.reason,
                AttentionReason::IssueResolutionOverdue | AttentionReason::IssueResolutionDue
            ),
            "a resolution-deadline flag was derived for an Issue that has no resolution deadline"
        );
    }
}

#[test]
fn the_kinds_the_ledger_does_hold_a_deadline_for_report_it_unchanged() {
    let facts = facts();
    assert_eq!(
        find(&facts, WorkItemKind::ActionRequest, "request-open").relevant_at,
        Some(at(9_000))
    );
    assert_eq!(
        find(&facts, WorkItemKind::Action, "action-1").relevant_at,
        Some(at(1_000))
    );
    assert_eq!(
        find(&facts, WorkItemKind::Risk, "risk-1").relevant_at,
        Some(at(2_000))
    );
}

// ---------------------------------------------------------------------------
// Labels.
// ---------------------------------------------------------------------------

#[test]
fn a_label_is_the_records_own_title_where_the_ledger_carries_one() {
    let facts = facts();
    assert_eq!(
        find(&facts, WorkItemKind::ActionRequest, "request-open").label,
        "Synthetic open request"
    );
    assert_eq!(
        find(&facts, WorkItemKind::DecisionRequest, "decision-request-1").label,
        "Synthetic decision subject"
    );
    assert_eq!(
        find(&facts, WorkItemKind::Issue, "issue-1").label,
        "Synthetic issue"
    );
}

#[test]
fn a_label_names_the_type_and_identifier_where_no_title_exists() {
    // True and unhelpful, rather than helpful and invented. The projection
    // snapshot carries no title for Actions, and a Risk the composition
    // snapshot has no title for keeps its identifier.
    let facts = facts();
    assert_eq!(
        find(&facts, WorkItemKind::Action, "action-1").label,
        "Action action-1"
    );
    assert_eq!(
        find(&facts, WorkItemKind::Risk, "risk-1").label,
        "Risk risk-1"
    );
}

#[test]
fn a_risk_is_named_by_its_recorded_title() {
    let mut composition = composition();
    composition.risks.push(RiskReadRecord {
        id: "risk-1".to_owned(),
        title: "Synthetic supplier delay".to_owned(),
        classification: DataClassification::Internal,
        version: version(9),
    });

    let facts = work_item_facts_from_snapshots(&projection(), &composition, thresholds());

    assert_eq!(
        find(&facts, WorkItemKind::Risk, "risk-1").label,
        "Synthetic supplier delay"
    );
}

#[test]
fn a_follow_up_request_shows_the_date_its_work_is_promised_for() {
    // A Decision's follow-up request has no response deadline, only the date
    // the resulting work is promised for. The queue carries that date as its
    // own fact; it is not the request's deadline, so it neither ranks nor
    // flags the item.
    let mut composition = composition();
    composition.action_requests.push(ActionRequestReadRecord {
        id: "request-follow-up".to_owned(),
        title: "Synthetic follow-up request".to_owned(),
        intended_owner_id: Some(StakeholderId::parse("stakeholder-a").unwrap()),
        state: ActionRequestState::Open,
        response_due_at: None,
        intended_action_due_at: Some(at(1_700_000_900_000)),
        classification: DataClassification::Internal,
        version: version(11),
    });

    let facts = work_item_facts_from_snapshots(&projection(), &composition, thresholds());

    let follow_up = find(&facts, WorkItemKind::ActionRequest, "request-follow-up");
    assert_eq!(follow_up.promised_at, Some(at(1_700_000_900_000)));
    assert_eq!(follow_up.relevant_at, None);
    // A response deadline stays the request's deadline.
    let open = find(&facts, WorkItemKind::ActionRequest, "request-open");
    assert_eq!(open.relevant_at, Some(at(9_000)));
    assert_eq!(open.promised_at, None);
    // Only Action Requests carry a promised date.
    assert_eq!(find(&facts, WorkItemKind::Risk, "risk-1").promised_at, None);
    // And a promised date is not a missed response: no overdue flag.
    assert!(
        find(&facts, WorkItemKind::ActionRequest, "request-follow-up")
            .attention
            .is_empty()
    );
}

#[test]
fn every_item_states_the_lifecycle_state_that_produced_its_intents() {
    // A reader shown intents without the state cannot tell why those are the
    // options.
    for item in facts() {
        assert!(
            !item.state_label.is_empty(),
            "{} {} does not say what state it is in",
            item.kind.as_str(),
            item.id
        );
    }
}

// ---------------------------------------------------------------------------
// End to end through composition.
// ---------------------------------------------------------------------------

#[test]
fn the_queue_composed_from_real_snapshots_ranks_the_overdue_action_first() {
    let composed = compose_work_queue(
        &GetWorkQueue {
            as_of: now(),
            page: pmc_application::route_composition::PageState {
                offset: 0,
                limit: 100,
                total: 0,
            },
            filter: WorkQueueFilter::default(),
        },
        &facts(),
        7,
    );

    assert_eq!(
        composed.body.items.first().map(|item| item.id.as_str()),
        Some("action-1"),
        "the only breached commitment in the snapshot is not at the top"
    );
    assert_eq!(composed.ledger_revision, 7);
}
