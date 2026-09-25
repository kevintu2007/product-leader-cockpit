//! The Work Queue route composition (S03).
//!
//! The Work Queue requires filtering and sorting across Action Requests,
//! Actions, Decision Requests, Risks and Issues "while preserving separate
//! lifecycle types". That phrase drives the whole shape of this module: the five types
//! share one ordered list, and the type each item belongs to is a required
//! field on every item rather than something a reader has to infer. An Issue
//! and an Action are not interchangeable work, and nothing here lets them
//! render as though they were.
//!
//! Two things are deliberately **not** invented here.
//!
//! Ordering is the accepted policy in
//! `docs/policies/attention-ranking.md`, whose scope section
//! says the Work Queue "reuses the same ordering". This module therefore calls
//! [`crate::attention_ranking::rank_attention`] rather than tiering flags
//! itself; a second implementation of the tiers would be a second policy, and
//! the one that shipped would be whichever the reader happened to be looking
//! at.
//!
//! Membership and the placement of unflagged items are the one question that
//! reuse does not answer, because the accepted policy ranks flags and a daily
//! queue also contains work nothing has flagged. Those two choices are
//! recorded and accepted in
//! `docs/policies/work-queue-ordering.md`. They are
//! consumed here and nowhere else, so either can be replaced without touching
//! the domain, the Ledger read surfaces, or the accepted ranking policy.
//!
//! Nothing here writes, and nothing here offers a write. The intents an item
//! carries say what its lifecycle admits, which is strictly weaker than what
//! may be executed now -- see [`pmc_domain::state_intents`].

use pmc_domain::attention::{AttentionTarget, Freshness};
use pmc_domain::classification::DataClassification;
use pmc_domain::time::UtcTimestamp;

use crate::attention_ranking::{
    rank_attention, AttentionTier, RankableAttentionItem, RankedAttentionItem,
};
use crate::route_composition::{CompositionResult, OwnerModule, PageState, RouteState};

/// Which lifecycle a work item belongs to.
///
/// The five DG0 names the Work Queue over, and the same five
/// `pmc_domain::attention::AttentionTarget` already distinguishes. Kept as a
/// required field on every item so the types cannot be collapsed into one
/// undifferentiated list: "preserving separate lifecycle types" is the
/// acceptance criterion, and a shared list without a discriminant would fail
/// it while looking correct.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WorkItemKind {
    ActionRequest,
    Action,
    DecisionRequest,
    Risk,
    Issue,
}

impl WorkItemKind {
    /// Every kind, in a fixed order, so a caller enumerating them cannot omit
    /// one by hand.
    pub const ALL: [Self; 5] = [
        Self::ActionRequest,
        Self::Action,
        Self::DecisionRequest,
        Self::Risk,
        Self::Issue,
    ];

    /// The kind of record an attention flag was raised against. The one
    /// place this mapping lives, so that everything deriving an owner from a
    /// flag goes through [`Self::owner`] rather than a second table.
    #[must_use]
    pub const fn of_target(target: &AttentionTarget) -> Self {
        match target {
            AttentionTarget::ActionRequest(_) => Self::ActionRequest,
            AttentionTarget::Action(_) => Self::Action,
            AttentionTarget::DecisionRequest(_) => Self::DecisionRequest,
            AttentionTarget::Risk(_) => Self::Risk,
            AttentionTarget::Issue(_) => Self::Issue,
        }
    }

    /// The module that owns this kind's records. Composition reads across
    /// these; it never becomes one of them.
    #[must_use]
    pub const fn owner(self) -> OwnerModule {
        match self {
            Self::ActionRequest | Self::Action => OwnerModule::ActionManagement,
            Self::DecisionRequest => OwnerModule::Decisions,
            Self::Risk => OwnerModule::Risks,
            Self::Issue => OwnerModule::Issues,
        }
    }

    /// A stable identifier for the kind, for filters and for surfaces that
    /// need to name it without matching on the enum.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActionRequest => "action_request",
            Self::Action => "action",
            Self::DecisionRequest => "decision_request",
            Self::Risk => "risk",
            Self::Issue => "issue",
        }
    }
}

/// Where an item sits relative to the accepted tiers.
///
/// `Flagged` carries the tier of the item's **most severe** flag, which is
/// what placed it. `Unflagged` is the appended group from the accepted
/// membership-and-ordering policy: nothing has flagged this item, so it sorts
/// below every flagged one by the accepted policy's own reasoning that an
/// absent signal is not an urgent one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkItemPlacement {
    Flagged(AttentionTier),
    Unflagged,
}

impl WorkItemPlacement {
    /// Compares two placements: the accepted tier order among flagged items,
    /// and unflagged below all of them.
    ///
    /// Written as an explicit comparison rather than a numeric cast. Casting
    /// `AttentionTier as u8` would make the order depend on the enum's
    /// discriminants, which is the accidental ordering the accepted policy
    /// §5 refuses for exactly this reason -- inserting a variant in the
    /// middle would silently re-rank the queue.
    fn compare(self, other: Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Flagged(left), Self::Flagged(right)) => left.cmp(&right),
            (Self::Flagged(_), Self::Unflagged) => std::cmp::Ordering::Less,
            (Self::Unflagged, Self::Flagged(_)) => std::cmp::Ordering::Greater,
            (Self::Unflagged, Self::Unflagged) => std::cmp::Ordering::Equal,
        }
    }

    /// A stable identifier for the placement, so a surface can key off it
    /// without matching on the enum or parsing the prose in [`Self::why`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flagged(tier) => tier.as_str(),
            Self::Unflagged => "unflagged",
        }
    }

    /// Why the item is where it is, in words rather than a number.
    #[must_use]
    pub fn why(self) -> String {
        match self {
            Self::Flagged(tier) => tier.why().to_owned(),
            Self::Unflagged => "nothing has flagged it, so it sorts below flagged work".to_owned(),
        }
    }
}

/// The facts one work item carries into composition.
///
/// Supplied by the adapter from real Ledger records. Composition orders,
/// filters and pages these; it does not reinterpret them and does not add
/// any fact of its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkItemFacts {
    pub kind: WorkItemKind,
    pub id: String,
    /// The record's own title or subject where the Ledger carries one. Where
    /// it does not, the adapter supplies the type and identifier, which is
    /// true and unhelpful rather than helpful and invented.
    pub label: String,
    /// The lifecycle state's own name, so a reader is never shown an item
    /// without knowing what state produced its intents.
    pub state_label: &'static str,
    /// What this item's state admits, read from `pmc_domain::state_intents`.
    /// Not permissions: preparation, classification, policy and approval are
    /// all further gates.
    pub lifecycle_legal_intents: Vec<&'static str>,
    /// Due, response-due or review-due, whichever this kind has. `None` for
    /// Decision Requests and Issues, which the Ledger persists no deadline
    /// for at all -- see §4 of the membership-and-ordering policy. Never
    /// defaulted.
    pub relevant_at: Option<UtcTimestamp>,
    /// When the work an Action Request asks for is promised to be done. Kept
    /// apart from `relevant_at` on purpose: a request not yet accepted has
    /// no commitment deadline, so this date neither ranks nor flags the item.
    /// `None` for every other kind.
    pub promised_at: Option<UtcTimestamp>,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
    pub freshness: Freshness,
    pub degraded: bool,
    /// Flags raised against this record, unranked. Empty is a fact --
    /// "nothing has flagged this" -- not a missing value.
    pub attention: Vec<RankableAttentionItem>,
}

/// One item as the Work Queue presents it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkQueueItem {
    pub kind: WorkItemKind,
    pub id: String,
    pub label: String,
    pub state_label: &'static str,
    pub lifecycle_legal_intents: Vec<&'static str>,
    pub relevant_at: Option<UtcTimestamp>,
    pub promised_at: Option<UtcTimestamp>,
    pub classification: DataClassification,
    pub revision: u64,
    pub as_of: UtcTimestamp,
    pub freshness: Freshness,
    pub degraded: bool,
    /// The item's flags, ordered by the accepted policy. The first, when
    /// present, is the one that placed the item.
    pub attention: Vec<RankedAttentionItem>,
    pub placement: WorkItemPlacement,
    /// Why this item is ranked where it is, generated from the placement
    /// itself so the stated reason cannot drift from the reason used.
    pub placement_rationale: String,
}

impl WorkQueueItem {
    /// The timestamp that placed the item within its group: the placing
    /// flag's where there is one, the record's own otherwise.
    ///
    /// These differ on purpose. A Risk whose review is due next week can also
    /// be flagged for something with no time at all; ordering it by the
    /// review date would place it by a fact that is not the reason it is
    /// there.
    #[must_use]
    pub fn placing_timestamp(&self) -> Option<UtcTimestamp> {
        self.attention
            .first()
            .map_or(self.relevant_at, |ranked| ranked.relevant_at)
    }
}

/// Which items a caller wants.
///
/// DG0 lists "filters" as Work Queue content. Both filters below narrow what
/// is shown and neither reorders anything, so a filtered queue is always a
/// subsequence of the unfiltered one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkQueueFilter {
    /// Kinds to include. Empty means every kind, so the default filter shows
    /// the whole queue rather than nothing.
    pub kinds: Vec<WorkItemKind>,
    /// When true, only items something has flagged. The Executive Exception
    /// Lens (S01) is the curated view of exceptions; this is the same queue
    /// narrowed, and is not a substitute for it.
    pub only_flagged: bool,
}

impl WorkQueueFilter {
    fn admits_flag(&self, item: &WorkQueueItem) -> bool {
        !self.only_flagged || !item.attention.is_empty()
    }

    fn admits(&self, item: &WorkQueueItem) -> bool {
        self.admits_flag(item) && (self.kinds.is_empty() || self.kinds.contains(&item.kind))
    }
}

/// The S03 query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GetWorkQueue {
    pub as_of: UtcTimestamp,
    pub page: PageState,
    pub filter: WorkQueueFilter,
}

/// How many items of each kind choosing that kind would show, before paging:
/// counted after the flagged-only narrowing but ignoring the kind selection.
///
/// Reported for all five kinds even when a kind has none, because zero and
/// absent are different facts: a kind missing from this list would read as
/// "not counted" rather than "none outstanding".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KindCount {
    pub kind: WorkItemKind,
    pub count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkQueueBody {
    pub items: Vec<WorkQueueItem>,
    pub counts_by_kind: Vec<KindCount>,
    pub page: PageState,
}

/// Whether an item belongs in the queue at all.
///
/// Membership-and-ordering policy §2: an item is in the queue while its
/// lifecycle admits at
/// least one intent. Derived from the state tables rather than from a second
/// list of "active" states, so membership cannot drift from the guards that
/// enforce the transitions.
#[must_use]
pub fn is_outstanding(facts: &WorkItemFacts) -> bool {
    !facts.lifecycle_legal_intents.is_empty()
}

fn to_item(facts: &WorkItemFacts) -> WorkQueueItem {
    let attention = rank_attention(&facts.attention);
    let placement = attention
        .first()
        .map_or(WorkItemPlacement::Unflagged, |ranked| {
            WorkItemPlacement::Flagged(ranked.tier)
        });
    WorkQueueItem {
        kind: facts.kind,
        id: facts.id.clone(),
        label: facts.label.clone(),
        state_label: facts.state_label,
        lifecycle_legal_intents: facts.lifecycle_legal_intents.clone(),
        relevant_at: facts.relevant_at,
        promised_at: facts.promised_at,
        classification: facts.classification,
        revision: facts.revision,
        as_of: facts.as_of,
        freshness: facts.freshness,
        degraded: facts.degraded,
        attention,
        placement,
        placement_rationale: placement.why(),
    }
}

/// Composes the Work Queue: membership, ordering, filtering, paging.
///
/// Ordering is total and deterministic. Two items can tie only if they share
/// a placement, share a placing timestamp, and share an identifier, which
/// cannot happen across distinct records because the identifier tie-break
/// includes the kind.
#[must_use]
pub fn compose_work_queue(
    request: &GetWorkQueue,
    facts: &[WorkItemFacts],
    ledger_revision: u64,
) -> CompositionResult<WorkQueueBody> {
    let outstanding: Vec<WorkQueueItem> = facts
        .iter()
        .filter(|candidate| is_outstanding(candidate))
        .map(to_item)
        .collect();

    // Each kind's count is what choosing that kind would show: the kind
    // selection itself is not applied, the flagged-only narrowing is. Counting
    // after the kind filter made every unselected kind read as zero.
    let counts_by_kind = WorkItemKind::ALL
        .iter()
        .map(|kind| KindCount {
            kind: *kind,
            count: outstanding
                .iter()
                .filter(|item| item.kind == *kind && request.filter.admits_flag(item))
                .count(),
        })
        .collect();

    let mut items: Vec<WorkQueueItem> = outstanding
        .into_iter()
        .filter(|item| request.filter.admits(item))
        .collect();

    items.sort_by(|left, right| {
        left.placement
            .compare(right.placement)
            // An absent timestamp is not an early one, so it sorts after
            // every item that has one. This is the accepted policy's rule,
            // applied to the timestamp that actually placed each item.
            .then_with(
                || match (left.placing_timestamp(), right.placing_timestamp()) {
                    (Some(left_at), Some(right_at)) => left_at.cmp(&right_at),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
            )
            // Stable final tie-break. Includes the kind so two records of
            // different types that share an identifier still order the same
            // way on every run.
            .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });

    let total = items.len();
    let paged: Vec<WorkQueueItem> = items
        .into_iter()
        .skip(request.page.offset)
        .take(request.page.limit)
        .collect();

    CompositionResult {
        state: if paged.is_empty() {
            RouteState::Empty
        } else {
            RouteState::Success
        },
        as_of: request.as_of,
        ledger_revision,
        body: WorkQueueBody {
            items: paged,
            counts_by_kind,
            page: PageState {
                offset: request.page.offset,
                limit: request.page.limit,
                total,
            },
        },
    }
}
