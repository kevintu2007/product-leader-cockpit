//! Transparent, deterministic ranking of attention flags.
//!
//! The accepted policy is recorded in
//! `docs/policies/attention-ranking.md`; this module is that
//! document in code. Ranking lives here rather than in `pmc-domain` because
//! cross-module composition, ranking and filtering belong to the application
//! layer while the domain remains the authority for the facts themselves.
//!
//! Two properties matter more than the ordering itself.
//!
//! **It invents nothing.** Every input is an `AttentionFlag` that
//! `pmc_domain::attention::derive_attention` already produced from
//! authoritative snapshots. Ranking adds no fact, no score and no judgement of
//! its own -- it decides sequence, and reports which existing fact decided it.
//!
//! **It is explainable one step at a time.** The comparison is lexicographic
//! over named tiers rather than a weighted score. A weighted score is a hidden
//! weight in substance even when the weights are printed: answering "why is
//! this above that" would require trusting arithmetic the reader did not
//! choose, and two items could swap places because of a coefficient rather
//! than because of anything true about the work. The design treats that as a
//! stop condition. Tiers answer the same question in words.

use pmc_domain::attention::{AttentionFlag, AttentionReason, AttentionTarget, Freshness};
use pmc_domain::time::UtcTimestamp;

/// Why an item sits where it does, in the order the comparison applies.
///
/// Deliberately **not** derived from `AttentionReason`'s own `Ord`. That
/// orders by declaration position, which is an accident of how the enum was
/// written; using it as a severity would be exactly the hidden weight the plan
/// forbids, and it would silently change if a variant were ever inserted in
/// the middle.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AttentionTier {
    /// A promise is already broken.
    BreachedCommitment,
    /// The work cannot proceed at all.
    Blocked,
    /// The basis of a conclusion is unsound.
    EvidenceIntegrity,
    /// A deadline is approaching but has not passed.
    ApproachingDeadline,
    /// Nobody is accountable for it.
    NoAccountableOwner,
    /// Everything else: needs-info, staleness, recurrence, exposure change.
    Other,
}

impl AttentionTier {
    /// The user-visible reason this tier outranks the ones below it.
    #[must_use]
    pub const fn why(self) -> &'static str {
        match self {
            Self::BreachedCommitment => "a commitment has already been missed",
            Self::Blocked => "the work cannot proceed until something is unblocked",
            Self::EvidenceIntegrity => "the evidence this relies on is missing or unsound",
            Self::ApproachingDeadline => "a deadline is approaching",
            Self::NoAccountableOwner => "no one is accountable for it yet",
            Self::Other => "it needs attention but nothing above applies",
        }
    }

    /// A stable identifier for the tier, for surfaces that need to name it
    /// without matching on the enum or parsing the prose in [`Self::why`].
    ///
    /// Separate from `why` on purpose: `why` is user-visible text that may be
    /// reworded, and a surface keying off reworded prose would break silently.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BreachedCommitment => "breachedCommitment",
            Self::Blocked => "blocked",
            Self::EvidenceIntegrity => "evidenceIntegrity",
            Self::ApproachingDeadline => "approachingDeadline",
            Self::NoAccountableOwner => "noAccountableOwner",
            Self::Other => "other",
        }
    }
}

/// Assigns every accepted attention reason to a tier.
///
/// Exhaustive by construction: a twenty-eighth `AttentionReason` will fail to
/// compile here rather than falling into a default bucket and acquiring a rank
/// nobody chose.
#[must_use]
pub const fn tier_of(reason: AttentionReason) -> AttentionTier {
    match reason {
        AttentionReason::ActionOverdue
        | AttentionReason::ActionRequestResponseOverdue
        | AttentionReason::DecisionRequestOverdue
        | AttentionReason::IssueResolutionOverdue => AttentionTier::BreachedCommitment,

        AttentionReason::ActionBlocked | AttentionReason::IssueBlocked => AttentionTier::Blocked,

        AttentionReason::ActionNeedsEvidence
        | AttentionReason::ActionEvidenceVerificationPending
        | AttentionReason::IssueNeedsEvidence
        | AttentionReason::RiskControlInvalid
        | AttentionReason::RiskEvidenceStale => AttentionTier::EvidenceIntegrity,

        AttentionReason::ActionAtRisk
        | AttentionReason::ActionRequestResponseDue
        | AttentionReason::DecisionRequestApproachingDeadline
        | AttentionReason::IssueResolutionDue
        | AttentionReason::RiskReviewDue => AttentionTier::ApproachingDeadline,

        AttentionReason::ActionRequestMissingIntendedOwner
        | AttentionReason::DecisionRequestMissingDecisionOwner
        | AttentionReason::RiskMissingOwner => AttentionTier::NoAccountableOwner,

        AttentionReason::ActionRequestNeedsInfo
        | AttentionReason::ActionRequestStale
        | AttentionReason::ActionSupersededPremise
        | AttentionReason::DecisionRequestNeedsInfo
        | AttentionReason::DecisionRequestStale
        | AttentionReason::RiskExposureIncreased
        | AttentionReason::IssueStale
        | AttentionReason::IssueRecurrence => AttentionTier::Other,
    }
}

/// A flag paired with the domain timestamp that makes it urgent.
///
/// The caller supplies `relevant_at` from the same snapshot the flags were
/// derived from -- `AttentionFlag` does not carry it, and the deadline fields
/// on the evaluator's inputs are private to `pmc-domain`. The pairing is built
/// once, beside the `AttentionInputs`, so a deadline cannot disagree with the
/// flag it belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankableAttentionItem {
    pub flag: AttentionFlag,
    /// Due, response-due, deadline or review-due, whichever the reason is
    /// about. `None` when the reason is not about a time at all.
    pub relevant_at: Option<UtcTimestamp>,
}

/// One ranked item, carrying the reason it landed where it did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankedAttentionItem {
    pub flag: AttentionFlag,
    pub relevant_at: Option<UtcTimestamp>,
    pub tier: AttentionTier,
    /// Why this item is ranked where it is, generated from the ordering
    /// itself rather than written separately, so the stated reason cannot
    /// drift from the reason actually used.
    pub rank_rationale: String,
}

/// The canonical identifier used as the final, stable tie-break.
#[must_use]
pub fn canonical_id_of(target: &AttentionTarget) -> &str {
    match target {
        AttentionTarget::ActionRequest(id) => id.as_str(),
        AttentionTarget::Action(id) => id.as_str(),
        AttentionTarget::DecisionRequest(id) => id.as_str(),
        AttentionTarget::Risk(id) => id.as_str(),
        AttentionTarget::Issue(id) => id.as_str(),
    }
}

/// Orders attention items by the accepted policy.
///
/// Total and deterministic: the same inputs always produce the same sequence,
/// so a rank never moves unless a fact moved.
///
/// Staleness never promotes. A stale or degraded item is placed by facts known
/// to be current and keeps its uncertainty attached in its rationale --
/// treating a stale "overdue" as certainly overdue would fabricate certainty,
/// which the Executive Cockpit requirements forbid in those words. It is not
/// demoted either; it is shown with the limit of what is known.
#[must_use]
pub fn rank_attention(items: &[RankableAttentionItem]) -> Vec<RankedAttentionItem> {
    let mut ranked: Vec<RankedAttentionItem> = items
        .iter()
        .map(|item| {
            let tier = tier_of(item.flag.reason);
            RankedAttentionItem {
                rank_rationale: rationale_for(tier, item),
                tier,
                flag: item.flag.clone(),
                relevant_at: item.relevant_at,
            }
        })
        .collect();
    ranked.sort_by(|left, right| {
        left.tier
            .cmp(&right.tier)
            // An absent deadline is not an early one, so it sorts after every
            // item that has one rather than before them as `None` would.
            .then_with(|| match (left.relevant_at, right.relevant_at) {
                (Some(a), Some(b)) => a.unix_millis().cmp(&b.unix_millis()),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| {
                canonical_id_of(&left.flag.target).cmp(canonical_id_of(&right.flag.target))
            })
    });
    ranked
}

fn rationale_for(tier: AttentionTier, item: &RankableAttentionItem) -> String {
    let mut rationale = format!("Ranked here because {}", tier.why());
    if item.relevant_at.is_some() {
        rationale.push_str("; within that group the earliest time comes first");
    }
    // The uncertainty is part of the reason, not a footnote to it.
    match item.flag.metadata.freshness {
        Freshness::Stale => rationale
            .push_str(". This is based on data known to be out of date, so it may no longer hold"),
        Freshness::Unknown => {
            rationale.push_str(". The freshness of this data is unknown, so it may no longer hold")
        }
        Freshness::Fresh => {}
    }
    if item.flag.metadata.degraded {
        rationale.push_str(". Some source context was unavailable when this was derived");
    }
    rationale
}
