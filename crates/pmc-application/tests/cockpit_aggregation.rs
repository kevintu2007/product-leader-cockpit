//! Executive Cockpit aggregation and the Portfolio pair.

use pmc_application::attention_ranking::{
    rank_attention, RankableAttentionItem, RankedAttentionItem,
};
use pmc_application::cockpit_aggregation::{
    aggregate_cockpit, compose_portfolio_overview, health_reasons, leader_briefing,
    order_portfolio_first, period_change, portfolio_pulse, CockpitFacts, NoApprovedPeriodPort,
    ReasonSource, ReviewReadinessPort,
};
use pmc_application::route_composition::{
    ComposedEntity, ComposedEntityKind, OwnerModule, PageState, PeriodChange, PeriodReadiness,
    RouteState,
};
use pmc_domain::attention::{
    AttentionFlag, AttentionMetadata, AttentionReason, AttentionTarget, Freshness,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::ActionId;
use pmc_domain::time::UtcTimestamp;

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

fn attention(id: &str, reason: AttentionReason) -> RankedAttentionItem {
    let items = [RankableAttentionItem {
        flag: AttentionFlag {
            target: AttentionTarget::Action(ActionId::parse(id).unwrap()),
            reason,
            metadata: AttentionMetadata {
                classification: DataClassification::Internal,
                freshness: Freshness::Fresh,
                degraded: false,
            },
            explanation: "the action passed its due date",
            failed_verification_guidance: None,
        },
        relevant_at: Some(now()),
    }];
    rank_attention(&items).remove(0)
}

fn product(id: &str, attention_items: Vec<RankedAttentionItem>) -> ComposedEntity {
    ComposedEntity {
        kind: ComposedEntityKind::Product,
        id: id.to_owned(),
        owner: OwnerModule::Portfolio,
        revision: 7,
        as_of: now(),
        classification: DataClassification::Internal,
        freshness: Freshness::Fresh,
        degraded: false,
        attention: attention_items,
        lifecycle_legal_intents: Vec::new(),
    }
}

fn facts<'a>(
    products: &'a [ComposedEntity],
    attention: &'a [RankedAttentionItem],
    milestones: &'a [String],
    accepted: &'a [String],
    requests: &'a [String],
    kpis: &'a [String],
) -> CockpitFacts<'a> {
    CockpitFacts {
        as_of: now(),
        ledger_revision: 7,
        products,
        milestone_ids: milestones,
        accepted_action_ids: accepted,
        action_request_ids: requests,
        kpi_ids: kpis,
        attention,
    }
}

fn ids(entities: &[ComposedEntity]) -> Vec<&str> {
    entities.iter().map(|entity| entity.id.as_str()).collect()
}

#[test]
fn an_action_request_is_never_counted_as_a_commitment() {
    // The DG1 brief: submitting a request "does not create an Action or
    // commitment". Counting requests here would make a request appear as a
    // commitment prematurely, which is the exact failure that story names.
    let requests: Vec<String> = (0..9).map(|n| format!("action-request-{n}")).collect();
    let accepted = vec!["action-1".to_owned()];
    let facts = facts(&[], &[], &[], &accepted, &requests, &[]);

    let pulse = portfolio_pulse(&facts);

    assert_eq!(pulse.commitments.count(), 1, "only accepted Actions count");
    assert!(
        pulse
            .commitments
            .definition()
            .contains("not counted until it is accepted"),
        "the definition must make the distinction inspectable: {}",
        pulse.commitments.definition()
    );
}

#[test]
fn every_pulse_count_states_what_it_counted_and_where_it_came_from() {
    // DG1 requires "inspectable definitions". A bare number invites the
    // reader to guess a denominator, which is how a count becomes a
    // fabricated progress claim.
    let milestones = vec!["milestone-1".to_owned()];
    let kpis = vec!["kpi-1".to_owned(), "kpi-2".to_owned()];
    let facts = facts(&[], &[], &milestones, &[], &[], &kpis);

    let pulse = portfolio_pulse(&facts);

    for counted in [&pulse.milestones, &pulse.commitments, &pulse.kpis] {
        assert!(!counted.definition().is_empty());
        assert_eq!(counted.provenance().source_revision(), 7);
        assert_eq!(counted.provenance().as_of(), now());
    }
    assert_eq!(pulse.milestones.count(), 1);
    assert_eq!(pulse.kpis.count(), 2);
    assert_eq!(pulse.milestones.provenance().owner(), OwnerModule::Delivery);
    assert_eq!(pulse.kpis.provenance().owner(), OwnerModule::Kpi);
}

#[test]
fn the_worst_problem_decides_the_order_not_the_number_of_problems() {
    // Portfolio-first is a judgement surface, not a task list. One breached
    // commitment must outrank nine routine flags.
    let one_breach = product(
        "product-breached",
        vec![attention("action-1", AttentionReason::ActionOverdue)],
    );
    let many_minor = product(
        "product-noisy",
        (0..9)
            .map(|n| attention(&format!("action-{n}0"), AttentionReason::IssueStale))
            .collect(),
    );

    let ordered = order_portfolio_first(&[many_minor, one_breach]);

    assert_eq!(ids(&ordered), vec!["product-breached", "product-noisy"]);
}

#[test]
fn a_healthy_product_sorts_last_rather_than_first() {
    // `Option::None` is naturally the minimum, which would have floated
    // problem-free Products to the top of a judgement surface.
    let healthy = product("product-healthy", Vec::new());
    let troubled = product(
        "product-troubled",
        vec![attention("action-1", AttentionReason::IssueStale)],
    );

    let ordered = order_portfolio_first(&[healthy, troubled]);

    assert_eq!(ids(&ordered), vec!["product-troubled", "product-healthy"]);
}

#[test]
fn products_with_equal_severity_stay_in_a_stable_order() {
    let a = product(
        "product-a",
        vec![attention("action-1", AttentionReason::ActionOverdue)],
    );
    let b = product(
        "product-b",
        vec![attention("action-2", AttentionReason::ActionOverdue)],
    );

    assert_eq!(
        ids(&order_portfolio_first(&[b.clone(), a.clone()])),
        ids(&order_portfolio_first(&[a, b]))
    );
}

#[test]
fn no_approved_period_is_stated_rather_than_shown_as_a_zero_delta() {
    // A zero delta claims "nothing changed", which is a stronger and
    // different assertion than "there is nothing to compare against".
    let change = period_change(&PeriodReadiness::NoApprovedPeriod);

    match change {
        PeriodChange::NotComparable { because } => {
            assert!(because.contains("nothing to compare against"));
        }
        PeriodChange::Comparable { .. } => panic!("there is no approved period to compare against"),
    }
}

#[test]
fn an_approved_period_becomes_comparable() {
    let change = period_change(&PeriodReadiness::Approved {
        period_id: "period-2026-w36".to_owned(),
        approved_at: now(),
    });

    match change {
        PeriodChange::Comparable { period_id, since } => {
            assert_eq!(period_id, "period-2026-w36");
            assert_eq!(since, now());
        }
        PeriodChange::NotComparable { .. } => panic!("an approved period must be comparable"),
    }
}

#[test]
fn the_briefing_says_nothing_needs_attention_rather_than_reassuring() {
    let facts = facts(&[], &[], &[], &[], &[], &[]);

    let briefing = leader_briefing(&facts, &period_change(&PeriodReadiness::NoApprovedPeriod));

    assert!(briefing.conclusion.contains("Nothing"));
    assert_eq!(
        briefing.intervention, None,
        "with nothing wrong there is no intervention to invent"
    );
}

#[test]
fn the_briefing_carries_the_limit_of_what_is_known_in_the_conclusion() {
    // Not in a footnote: a conclusion that omits its own uncertainty reads as
    // more certain than the facts support.
    let attention_items = vec![attention("action-1", AttentionReason::ActionOverdue)];
    let products = vec![product("product-1", attention_items.clone())];
    let facts = facts(&products, &attention_items, &[], &[], &[], &[]);

    let briefing = leader_briefing(&facts, &period_change(&PeriodReadiness::NoApprovedPeriod));

    assert!(briefing.conclusion.contains("no period comparison"));
    assert!(briefing.intervention.is_some());
    assert_eq!(
        briefing.basis.len(),
        1,
        "the conclusion must name the facts it rests on"
    );
}

#[test]
fn the_cockpit_composes_the_whole_dg1_sequence() {
    let attention_items = vec![attention("action-1", AttentionReason::ActionOverdue)];
    let products = vec![product("product-1", attention_items.clone())];
    let accepted = vec!["action-1".to_owned()];
    let facts = facts(&products, &attention_items, &[], &accepted, &[], &[]);

    let result = aggregate_cockpit(&facts, &NoApprovedPeriodPort);

    assert_eq!(result.state, RouteState::Success);
    assert_eq!(result.ledger_revision, 7);
    assert_eq!(result.body.products.len(), 1);
    assert!(matches!(
        result.body.period_change,
        PeriodChange::NotComparable { .. }
    ));
    assert_eq!(result.body.pulse.commitments.count(), 1);
    assert_eq!(result.body.exceptions.len(), 1);
    assert!(result.body.leader_briefing.intervention.is_some());
}

#[test]
fn an_empty_portfolio_reports_empty_rather_than_success() {
    let facts = facts(&[], &[], &[], &[], &[], &[]);

    let result = aggregate_cockpit(&facts, &NoApprovedPeriodPort);

    assert_eq!(result.state, RouteState::Empty);
}

#[test]
fn a_degraded_product_makes_the_whole_route_degraded() {
    let mut degraded = product("product-1", Vec::new());
    degraded.degraded = true;
    let products = vec![degraded];
    let facts = facts(&products, &[], &[], &[], &[], &[]);

    let result = aggregate_cockpit(&facts, &NoApprovedPeriodPort);

    assert_eq!(result.state, RouteState::Degraded);
}

#[test]
fn a_custom_readiness_port_replaces_the_no_period_answer() {
    // Proves the port is a real seam, so Reviews & Reports can supply
    // approved periods without this contract changing shape.
    struct ApprovedPort;
    impl ReviewReadinessPort for ApprovedPort {
        fn readiness(&self) -> PeriodReadiness {
            PeriodReadiness::Approved {
                period_id: "period-2026-w36".to_owned(),
                approved_at: now(),
            }
        }
    }
    let products = vec![product("product-1", Vec::new())];
    let facts = facts(&products, &[], &[], &[], &[], &[]);

    let result = aggregate_cockpit(&facts, &ApprovedPort);

    assert!(matches!(
        result.body.period_change,
        PeriodChange::Comparable { .. }
    ));
}

#[test]
fn the_portfolio_overview_pages_within_the_ordered_list() {
    let products: Vec<ComposedEntity> = (0..5)
        .map(|n| product(&format!("product-{n}"), Vec::new()))
        .collect();
    let facts = facts(&products, &[], &[], &[], &[], &[]);

    let result = compose_portfolio_overview(
        &facts,
        PageState {
            offset: 2,
            limit: 2,
            total: 5,
        },
    );

    assert_eq!(ids(&result.body.products), vec!["product-2", "product-3"]);
    assert!(result.body.page.has_more());
}

#[test]
fn health_reasons_are_attributed_to_the_record_that_raised_them() {
    // The Product is Portfolio's, at revision 7, read at now(). The overdue
    // Action is Action Management's, at revision 3, read earlier. The reason
    // must carry the Action's provenance on every field. The previous form
    // of this test asserted the Product's owner and identifier instead --
    // and passed -- which is how provenance laundering was being guarded as
    // if it were correct.
    let entity = product(
        "product-1",
        vec![attention("action-1", AttentionReason::ActionOverdue)],
    );
    let earlier = UtcTimestamp::from_unix_millis(1_600_000_000_000);
    let sources = [ReasonSource {
        target: AttentionTarget::Action(ActionId::parse("action-1").unwrap()),
        revision: 3,
        as_of: earlier,
    }];

    let reasons = health_reasons(&entity, &sources).expect("every flag has a source");

    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].value(), "the action passed its due date");
    let provenance = reasons[0].provenance();
    assert_eq!(provenance.owner(), OwnerModule::ActionManagement);
    assert_eq!(provenance.source_record_id(), "action-1");
    assert_eq!(provenance.source_revision(), 3);
    assert_eq!(provenance.as_of(), earlier);
    // None of it is the Product's, on any axis.
    assert_ne!(provenance.owner(), entity.owner);
    assert_ne!(provenance.source_record_id(), entity.id);
    assert_ne!(provenance.source_revision(), entity.revision);
    assert_ne!(provenance.as_of(), entity.as_of);
}

#[test]
fn a_reason_with_no_supplied_source_is_refused_rather_than_charged_to_the_product() {
    // The honest answer to "where did this come from" is never "the thing it
    // is displayed under". Silence here is what would let a Product inherit
    // its accountable people's facts as its own.
    let entity = product(
        "product-1",
        vec![attention("action-1", AttentionReason::ActionOverdue)],
    );

    let refused = health_reasons(&entity, &[])
        .expect_err("a flag without a source must not become a Product fact");

    assert_eq!(
        refused.target,
        AttentionTarget::Action(ActionId::parse("action-1").unwrap())
    );
}
