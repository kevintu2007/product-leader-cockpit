//! The production Cockpit adapter.
//!
//! Turns a real `LedgerProjectionSnapshot` -- read from SQLite by
//! `pmc-ledger`'s `LedgerSnapshotForProjectionPort` -- into the Cockpit
//! composition. The Cockpit must be backed by this production adapter, as
//! opposed to the synthetic fixtures the other tests use.
//!
//! **What the snapshot can and cannot support.** The projection snapshot was
//! shaped for the managed projections' field allowlist, so it carries identity,
//! classification, revision, lifecycle state and the timestamps that matter
//! for projection -- and deliberately not much else. Several
//! `ActionAttentionInput` fields therefore have no source in it:
//! `blocked`, `at_risk`, `evidence_state` and `superseded_premise` are set to
//! their unknown-but-safe values rather than guessed. The consequence is
//! stated plainly rather than hidden: this adapter can currently derive
//! overdue and review-due attention, and not blocked, at-risk or
//! evidence-integrity attention, because the facts behind those are not in
//! the snapshot. Widening the snapshot is S2's to do, and doing it here would
//! mean inventing facts the Ledger never returned.
//!
//! Nothing here writes. The whole path is a query, and evaluating attention
//! is a pure derivation that cannot mutate lifecycle state or advance a clock.

use pmc_domain::attention::{
    derive_attention, ActionAttentionInput, AttentionInputs, AttentionMetadata,
    AttentionThresholds, EvidenceAttentionState, Freshness,
};
use pmc_domain::composition_source::LedgerCompositionSnapshot;
use pmc_domain::projection_source::LedgerProjectionSnapshot;

use crate::attention_ranking::{rank_attention, RankableAttentionItem, RankedAttentionItem};
use crate::cockpit_aggregation::{aggregate_cockpit, CockpitFacts, ReviewReadinessPort};
use crate::route_composition::{
    ComposedEntity, ComposedEntityKind, CompositionResult, ExecutiveCockpitBody, OwnerModule,
};

/// Derives ranked attention from a real Ledger snapshot.
///
/// The deadline paired with each flag comes from the same snapshot record the
/// flag was derived from, so a rank can never be computed against a deadline
/// that disagrees with the fact beside it.
#[must_use]
pub fn ranked_attention_from_snapshot(
    snapshot: &LedgerProjectionSnapshot,
    thresholds: AttentionThresholds,
) -> Vec<RankedAttentionItem> {
    let inputs = AttentionInputs {
        as_of: snapshot.ledger_as_of_utc,
        thresholds,
        action_requests: Vec::new(),
        actions: snapshot
            .actions
            .iter()
            .map(|action| ActionAttentionInput {
                id: action.id.clone(),
                state: action.state,
                due_at: action.due_at,
                // Not carried by the projection snapshot. Set to the
                // unknown-but-safe value rather than guessed; see the module
                // note.
                blocked: false,
                at_risk: false,
                evidence_state: EvidenceAttentionState::None,
                superseded_premise: false,
                metadata: AttentionMetadata {
                    classification: action.classification,
                    freshness: Freshness::Fresh,
                    degraded: false,
                },
            })
            .collect(),
        decision_requests: Vec::new(),
        risks: Vec::new(),
        issues: Vec::new(),
    };
    let result = derive_attention(&inputs);

    // Pair each flag with the deadline of the record it came from.
    let rankable: Vec<RankableAttentionItem> = result
        .flags
        .into_iter()
        .map(|flag| {
            let relevant_at = snapshot
                .actions
                .iter()
                .find(|action| {
                    matches!(
                        &flag.target,
                        pmc_domain::attention::AttentionTarget::Action(id) if id == &action.id
                    )
                })
                .map(|action| action.due_at);
            RankableAttentionItem { flag, relevant_at }
        })
        .collect();
    rank_attention(&rankable)
}

/// Composes every Product in the snapshot, carrying the attention that
/// belongs to it.
///
/// Attention is attached to the Portfolio as a whole rather than split across
/// Products, because the projection snapshot does not record which Product an
/// Action belongs to. Distributing them by guess would produce a
/// Portfolio-first ordering built on an invented relationship, which is worse
/// than an ordering that honestly has nothing to distinguish Products by yet.
#[must_use]
pub fn products_from_snapshot(snapshot: &LedgerProjectionSnapshot) -> Vec<ComposedEntity> {
    snapshot
        .products
        .iter()
        .map(|product| ComposedEntity {
            kind: ComposedEntityKind::Product,
            id: product.id.as_str().to_owned(),
            owner: OwnerModule::Portfolio,
            revision: product.source_revision.get(),
            as_of: snapshot.ledger_as_of_utc,
            classification: product.classification,
            freshness: Freshness::Fresh,
            degraded: false,
            attention: Vec::new(),
            lifecycle_legal_intents: Vec::new(),
        })
        .collect()
}

/// The Cockpit composed from the projection snapshot alone.
///
/// This cannot see Action Requests, Decision Requests or Issues -- the
/// projection snapshot does not carry them -- so it reports a Portfolio with
/// nothing to attend to while the Work Queue lists overdue requests. The
/// desktop uses [`compose_cockpit_from_snapshots`]; this remains for the
/// projection-only tests that pin what that snapshot alone can support.
#[must_use]
pub fn compose_cockpit_from_snapshot(
    snapshot: &LedgerProjectionSnapshot,
    thresholds: AttentionThresholds,
    readiness: &dyn ReviewReadinessPort,
) -> CompositionResult<ExecutiveCockpitBody> {
    let attention = ranked_attention_from_snapshot(snapshot, thresholds);
    let products = products_from_snapshot(snapshot);
    let milestone_ids: Vec<String> = Vec::new();
    let accepted_action_ids: Vec<String> = snapshot
        .actions
        .iter()
        .map(|action| action.id.as_str().to_owned())
        .collect();
    // The snapshot carries accepted Actions only -- an Action Request has no
    // projection source -- so this is empty rather than approximated. The
    // commitment count reads accepted Actions regardless, so an empty request
    // list cannot inflate it.
    let action_request_ids: Vec<String> = Vec::new();
    let kpi_ids: Vec<String> = snapshot
        .kpis
        .iter()
        .map(|kpi| kpi.id.as_str().to_owned())
        .collect();

    let facts = CockpitFacts {
        as_of: snapshot.ledger_as_of_utc,
        ledger_revision: snapshot.ledger_revision,
        products: &products,
        milestone_ids: &milestone_ids,
        accepted_action_ids: &accepted_action_ids,
        action_request_ids: &action_request_ids,
        kpi_ids: &kpi_ids,
        attention: &attention,
    };
    aggregate_cockpit(&facts, readiness)
}

/// The production composition: a matched pair of Ledger snapshots in, the
/// Cockpit out.
///
/// Attention comes from
/// [`crate::work_queue_adapter::portfolio_attention_from_snapshots`], the
/// derivation the Work Queue itself stands on, so the Cockpit can no longer say
/// nothing needs attention while the Work Queue lists overdue requests (DG3 S01
/// amendment §7). Milestones and Action Requests are counted from the
/// composition snapshot, which is where the Ledger reads them.
///
/// The two snapshots must share a Ledger revision. Only the caller holds both
/// ports, so the caller checks and reports `OutOfSync` otherwise; this function
/// reports the composition snapshot's revision and time.
#[must_use]
pub fn compose_cockpit_from_snapshots(
    projection: &LedgerProjectionSnapshot,
    composition: &LedgerCompositionSnapshot,
    thresholds: AttentionThresholds,
    readiness: &dyn ReviewReadinessPort,
) -> CompositionResult<ExecutiveCockpitBody> {
    let attention = crate::work_queue_adapter::portfolio_attention_from_snapshots(
        projection,
        composition,
        thresholds,
    );
    let products = products_from_snapshot(projection);
    let milestone_ids: Vec<String> = composition
        .milestones
        .iter()
        .map(|milestone| milestone.id.as_str().to_owned())
        .collect();
    let accepted_action_ids: Vec<String> = projection
        .actions
        .iter()
        .map(|action| action.id.as_str().to_owned())
        .collect();
    let action_request_ids: Vec<String> = composition
        .action_requests
        .iter()
        .map(|request| request.id.clone())
        .collect();
    let kpi_ids: Vec<String> = projection
        .kpis
        .iter()
        .map(|kpi| kpi.id.as_str().to_owned())
        .collect();

    let facts = CockpitFacts {
        as_of: composition.ledger_as_of_utc,
        ledger_revision: composition.ledger_revision,
        products: &products,
        milestone_ids: &milestone_ids,
        accepted_action_ids: &accepted_action_ids,
        action_request_ids: &action_request_ids,
        kpi_ids: &kpi_ids,
        attention: &attention,
    };
    aggregate_cockpit(&facts, readiness)
}

/// The production Portfolio overview (S02 list half).
///
/// Reads the same real snapshot as the Cockpit and pages the Portfolio-first
/// ordering. Deliberately reuses `products_from_snapshot` rather than shaping
/// products a second way: two orderings of the same records would eventually
/// disagree, and a reader moving between the Cockpit and Portfolio would see
/// the same Products in a different order for no stated reason.
#[must_use]
pub fn compose_portfolio_from_snapshot(
    snapshot: &LedgerProjectionSnapshot,
    thresholds: AttentionThresholds,
    page: crate::route_composition::PageState,
) -> CompositionResult<crate::route_composition::PortfolioOverviewBody> {
    let attention = ranked_attention_from_snapshot(snapshot, thresholds);
    let products = products_from_snapshot(snapshot);
    let milestone_ids: Vec<String> = Vec::new();
    let accepted_action_ids: Vec<String> = Vec::new();
    let action_request_ids: Vec<String> = Vec::new();
    let kpi_ids: Vec<String> = Vec::new();
    let facts = CockpitFacts {
        as_of: snapshot.ledger_as_of_utc,
        ledger_revision: snapshot.ledger_revision,
        products: &products,
        milestone_ids: &milestone_ids,
        accepted_action_ids: &accepted_action_ids,
        action_request_ids: &action_request_ids,
        kpi_ids: &kpi_ids,
        attention: &attention,
    };
    crate::cockpit_aggregation::compose_portfolio_overview(&facts, page)
}
