//! Executive Cockpit aggregation and the Portfolio composition pair.
//!
//! The DG1 brief fixes the Cockpit's information sequence: Portfolio and
//! Project health, then period-over-period changes, then Portfolio pulse and
//! measurable exceptions, then the highest-impact attention items, then the
//! leader briefing. Two of its constraints are prohibitions rather than
//! features, and both are enforced structurally here rather than by
//! convention:
//!
//! **No unsupported progress percentage.** [`PortfolioPulse`] holds counts and
//! nothing else. There is no percentage field to populate, so a surface
//! cannot render a completion figure the domain never computed. This is the
//! same requirement the Executive Cockpit states as "never fabricates
//! progress or certainty".
//!
//! **A request is not a commitment.** The DG1 brief is explicit that
//! submitting an Action Request "does not create an Action or commitment" --
//! accepting it later is H2a and is what creates the linked Action. So
//! [`CockpitFacts`] takes accepted Actions and Action Requests as two separate
//! inputs, and the commitment count reads only the first. A single combined
//! list would have made the premature-commitment bug a typo away.

use pmc_domain::attention::AttentionTarget;
use pmc_domain::time::UtcTimestamp;

use crate::attention_ranking::{canonical_id_of, RankedAttentionItem};
use crate::route_composition::{
    ComposedEntity, CompositionResult, CountedFact, ExecutiveCockpitBody, FieldProvenance,
    LeaderBriefing, OwnerModule, PageState, PeriodChange, PeriodReadiness, PortfolioOverviewBody,
    PortfolioPulse, RouteState, Sourced,
};
use crate::work_queue_composition::WorkItemKind;

/// Supplies approved-period identity and readiness.
///
/// Reviews & Reports owns these facts and has not been built; no
/// approved-period, carry-forward or review-readiness concept exists in the
/// domain today. The port is defined here, as owner ports are, so that
/// Reviews & Reports supplies a real implementation later without reshaping
/// this contract. Until then [`NoApprovedPeriodPort`] answers honestly.
pub trait ReviewReadinessPort {
    fn readiness(&self) -> PeriodReadiness;
}

/// The truthful answer while Reviews & Reports has no report data.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoApprovedPeriodPort;

impl ReviewReadinessPort for NoApprovedPeriodPort {
    fn readiness(&self) -> PeriodReadiness {
        PeriodReadiness::NoApprovedPeriod
    }
}

/// Everything the Cockpit is composed from, supplied by owner ports.
pub struct CockpitFacts<'a> {
    pub as_of: UtcTimestamp,
    pub ledger_revision: u64,
    pub products: &'a [ComposedEntity],
    pub milestone_ids: &'a [String],
    /// Accepted Actions. These are the commitments.
    pub accepted_action_ids: &'a [String],
    /// Submitted Action Requests. Deliberately a separate input, and
    /// deliberately never counted as commitments.
    pub action_request_ids: &'a [String],
    pub kpi_ids: &'a [String],
    pub attention: &'a [RankedAttentionItem],
}

/// Orders Products so the ones demanding judgement come first.
///
/// Portfolio-first means the Cockpit opens on portfolio health, not on a task
/// list, so the order is driven by each Product's worst outstanding attention
/// tier rather than by how many items it has. A Product with one breached
/// commitment outranks a Product with nine routine staleness flags, which is
/// the judgement a Head of Products actually makes. Products with nothing
/// outstanding sort last, and the canonical identifier keeps the order stable.
#[must_use]
pub fn order_portfolio_first(products: &[ComposedEntity]) -> Vec<ComposedEntity> {
    let mut ordered = products.to_vec();
    ordered.sort_by(|left, right| {
        // `None` is naturally the minimum, which would float problem-free
        // Products to the top of a judgement surface. Ordered explicitly so
        // "nothing outstanding" sorts last.
        match (worst_tier(left), worst_tier(right)) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| left.id.cmp(&right.id))
    });
    ordered
}

/// `None` sorts after every `Some`, so a healthy Product falls to the bottom
/// rather than the top.
fn worst_tier(entity: &ComposedEntity) -> Option<crate::attention_ranking::AttentionTier> {
    entity.attention.iter().map(|item| item.tier).min()
}

/// Counts the pulse from the facts, never from a caller-supplied total.
#[must_use]
pub fn portfolio_pulse(facts: &CockpitFacts<'_>) -> PortfolioPulse {
    let provenance = |owner, field| {
        FieldProvenance::new(
            owner,
            "portfolio",
            field,
            facts.ledger_revision,
            facts.as_of,
        )
    };
    PortfolioPulse {
        milestones: CountedFact::new(
            facts.milestone_ids.len(),
            "Milestones currently tracked across the Portfolio",
            provenance(OwnerModule::Delivery, "milestones"),
        ),
        // Reads accepted actions only. `facts.action_request_ids` is
        // deliberately not consulted here.
        commitments: CountedFact::new(
            facts.accepted_action_ids.len(),
            "Accepted Actions. A submitted Action Request is not counted until it is accepted",
            provenance(OwnerModule::ActionManagement, "accepted_actions"),
        ),
        kpis: CountedFact::new(
            facts.kpi_ids.len(),
            "KPIs with a definition in the Ledger",
            provenance(OwnerModule::Kpi, "kpis"),
        ),
    }
}

/// Reports the period comparison, or states honestly that none is possible.
#[must_use]
pub fn period_change(readiness: &PeriodReadiness) -> PeriodChange {
    match readiness {
        PeriodReadiness::NoApprovedPeriod => PeriodChange::NotComparable {
            because:
                "no review period has been approved yet, so there is nothing to compare against",
        },
        PeriodReadiness::Approved {
            period_id,
            approved_at,
        } => PeriodChange::Comparable {
            period_id: period_id.clone(),
            since: *approved_at,
        },
    }
}

/// Builds the one-line conclusion from the highest-ranked exception.
///
/// Says nothing the facts do not support. With no attention items the
/// conclusion is that nothing needs the leader's attention, and there is no
/// intervention -- rather than a reassuring sentence the data does not carry.
#[must_use]
pub fn leader_briefing(facts: &CockpitFacts<'_>, change: &PeriodChange) -> LeaderBriefing {
    let Some(top) = facts.attention.first() else {
        return LeaderBriefing {
            conclusion: "Nothing across the Portfolio currently needs your attention.".to_owned(),
            intervention: None,
            basis: Vec::new(),
        };
    };
    let mut conclusion = format!(
        "{} item{} need attention; the most urgent is because {}.",
        facts.attention.len(),
        if facts.attention.len() == 1 { "" } else { "s" },
        top.tier.why()
    );
    if let PeriodChange::NotComparable { because } = change {
        // The limit of what is known belongs in the conclusion, not beneath it.
        conclusion.push_str(&format!(
            " There is no period comparison because {because}."
        ));
    }
    LeaderBriefing {
        conclusion,
        intervention: Some(top.rank_rationale.clone()),
        basis: facts
            .products
            .iter()
            .map(|product| {
                FieldProvenance::new(
                    product.owner,
                    product.id.clone(),
                    "attention",
                    product.revision,
                    product.as_of,
                )
            })
            .collect(),
    }
}

/// Composes the Executive Cockpit.
#[must_use]
pub fn aggregate_cockpit(
    facts: &CockpitFacts<'_>,
    readiness: &dyn ReviewReadinessPort,
) -> CompositionResult<ExecutiveCockpitBody> {
    let products = order_portfolio_first(facts.products);
    let state = if products.is_empty() && facts.attention.is_empty() {
        RouteState::Empty
    } else if products.iter().any(|product| product.degraded) {
        RouteState::Degraded
    } else {
        RouteState::Success
    };
    let change = period_change(&readiness.readiness());
    let briefing = leader_briefing(facts, &change);
    CompositionResult {
        state,
        as_of: facts.as_of,
        ledger_revision: facts.ledger_revision,
        body: ExecutiveCockpitBody {
            products,
            period_change: change,
            pulse: portfolio_pulse(facts),
            exceptions: facts.attention.to_vec(),
            leader_briefing: briefing,
        },
    }
}

/// Composes the Portfolio overview list, Portfolio-first within the page.
#[must_use]
pub fn compose_portfolio_overview(
    facts: &CockpitFacts<'_>,
    page: PageState,
) -> CompositionResult<PortfolioOverviewBody> {
    let ordered = order_portfolio_first(facts.products);
    let products: Vec<ComposedEntity> = ordered
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect();
    CompositionResult {
        state: if products.is_empty() {
            RouteState::Empty
        } else {
            RouteState::Success
        },
        as_of: facts.as_of,
        ledger_revision: facts.ledger_revision,
        body: PortfolioOverviewBody { products, page },
    }
}

/// Where a health reason comes from: the record its flag was raised against,
/// at the revision and instant that record was read.
///
/// Supplied by the caller from the source snapshot. A flag does not carry
/// its record's revision or read time, and the Product the reason is shown
/// under is not the record the fact belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReasonSource {
    pub target: AttentionTarget,
    pub revision: u64,
    pub as_of: UtcTimestamp,
}

/// A flag on the Product whose source record was not supplied. Composition
/// refuses to attribute it rather than guess.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnattributableReason {
    pub target: AttentionTarget,
}

/// The health reasons shown for one Product, each attributed to the record
/// that actually raised it.
///
/// An earlier version of this function stamped every reason with the
/// Product's own owner, identifier, revision and read time. That is
/// provenance laundering: an overdue Action's fact would be presented as
/// Action Management's fact about the Product, under the Product's revision.
/// It was harmless only because no production path gave a Product any
/// attention -- the moment the O01 inspector aggregates the work of the
/// people accountable for a Product, it would have fabricated provenance for
/// every reason shown.
///
/// So the owner comes from the flag's target kind, the identifier from the
/// target, and the revision and read time from a [`ReasonSource`] the caller
/// took from the source record. A flag with no supplied source is an error,
/// not a reason attributed to the Product: the honest answer to "where did
/// this come from" is never "the thing it is displayed under".
pub fn health_reasons(
    product: &ComposedEntity,
    sources: &[ReasonSource],
) -> Result<Vec<Sourced<String>>, UnattributableReason> {
    product
        .attention
        .iter()
        .map(|item| {
            let source = sources
                .iter()
                .find(|source| source.target == item.flag.target)
                .ok_or_else(|| UnattributableReason {
                    target: item.flag.target.clone(),
                })?;
            Ok(Sourced::new(
                item.flag.explanation.to_owned(),
                FieldProvenance::new(
                    WorkItemKind::of_target(&item.flag.target).owner(),
                    canonical_id_of(&item.flag.target),
                    "attention",
                    source.revision,
                    source.as_of,
                ),
            ))
        })
        .collect()
}
