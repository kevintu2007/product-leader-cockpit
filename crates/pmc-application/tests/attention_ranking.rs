//! Transparent attention ranking.
//!
//! These assert the accepted policy in
//! `docs/policies/attention-ranking.md`.

use pmc_application::attention_ranking::{
    canonical_id_of, rank_attention, tier_of, AttentionTier, RankableAttentionItem,
};
use pmc_domain::attention::{
    AttentionFlag, AttentionMetadata, AttentionReason, AttentionTarget, Freshness,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::ActionId;
use pmc_domain::time::UtcTimestamp;

/// Every accepted reason, so the mapping test cannot silently miss one.
const ALL_REASONS: [AttentionReason; 27] = [
    AttentionReason::ActionRequestNeedsInfo,
    AttentionReason::ActionRequestStale,
    AttentionReason::ActionRequestMissingIntendedOwner,
    AttentionReason::ActionRequestResponseDue,
    AttentionReason::ActionRequestResponseOverdue,
    AttentionReason::ActionBlocked,
    AttentionReason::ActionOverdue,
    AttentionReason::ActionAtRisk,
    AttentionReason::ActionNeedsEvidence,
    AttentionReason::ActionEvidenceVerificationPending,
    AttentionReason::ActionSupersededPremise,
    AttentionReason::DecisionRequestNeedsInfo,
    AttentionReason::DecisionRequestMissingDecisionOwner,
    AttentionReason::DecisionRequestApproachingDeadline,
    AttentionReason::DecisionRequestOverdue,
    AttentionReason::DecisionRequestStale,
    AttentionReason::RiskReviewDue,
    AttentionReason::RiskExposureIncreased,
    AttentionReason::RiskEvidenceStale,
    AttentionReason::RiskControlInvalid,
    AttentionReason::RiskMissingOwner,
    AttentionReason::IssueBlocked,
    AttentionReason::IssueResolutionDue,
    AttentionReason::IssueResolutionOverdue,
    AttentionReason::IssueNeedsEvidence,
    AttentionReason::IssueStale,
    AttentionReason::IssueRecurrence,
];

fn metadata(freshness: Freshness, degraded: bool) -> AttentionMetadata {
    AttentionMetadata {
        classification: DataClassification::Internal,
        freshness,
        degraded,
    }
}

fn item(id: &str, reason: AttentionReason, relevant_at: Option<i64>) -> RankableAttentionItem {
    item_with(id, reason, relevant_at, Freshness::Fresh, false)
}

fn item_with(
    id: &str,
    reason: AttentionReason,
    relevant_at: Option<i64>,
    freshness: Freshness,
    degraded: bool,
) -> RankableAttentionItem {
    RankableAttentionItem {
        flag: AttentionFlag {
            target: AttentionTarget::Action(ActionId::parse(id).unwrap()),
            reason,
            metadata: metadata(freshness, degraded),
            explanation: "fixture",
            failed_verification_guidance: None,
        },
        relevant_at: relevant_at.map(UtcTimestamp::from_unix_millis),
    }
}

fn ids(ranked: &[pmc_application::attention_ranking::RankedAttentionItem]) -> Vec<&str> {
    ranked
        .iter()
        .map(|entry| canonical_id_of(&entry.flag.target))
        .collect()
}

#[test]
fn every_accepted_reason_maps_to_the_tier_the_policy_names() {
    // The mapping is exhaustive at compile time; this proves it is also the
    // mapping the accepted document describes, not merely *a* mapping.
    for reason in ALL_REASONS {
        let expected = match reason {
            AttentionReason::ActionOverdue
            | AttentionReason::ActionRequestResponseOverdue
            | AttentionReason::DecisionRequestOverdue
            | AttentionReason::IssueResolutionOverdue => AttentionTier::BreachedCommitment,
            AttentionReason::ActionBlocked | AttentionReason::IssueBlocked => {
                AttentionTier::Blocked
            }
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
            _ => AttentionTier::Other,
        };
        assert_eq!(
            tier_of(reason),
            expected,
            "{reason:?} landed in the wrong tier"
        );
    }
}

#[test]
fn a_breached_commitment_outranks_every_lower_tier() {
    // Deliberately supplied in reverse order, and with the overdue item given
    // the *latest* deadline, so only the tier can be producing the result.
    let ranked = rank_attention(&[
        item(
            "action-6",
            AttentionReason::ActionSupersededPremise,
            Some(10),
        ),
        item("action-5", AttentionReason::RiskMissingOwner, Some(10)),
        item("action-4", AttentionReason::ActionAtRisk, Some(10)),
        item("action-3", AttentionReason::ActionNeedsEvidence, Some(10)),
        item("action-2", AttentionReason::ActionBlocked, Some(10)),
        item("action-1", AttentionReason::ActionOverdue, Some(9_999)),
    ]);

    assert_eq!(
        ids(&ranked),
        vec!["action-1", "action-2", "action-3", "action-4", "action-5", "action-6"]
    );
}

#[test]
fn within_a_tier_the_earliest_time_comes_first() {
    let ranked = rank_attention(&[
        item("action-late", AttentionReason::ActionOverdue, Some(3_000)),
        item("action-early", AttentionReason::ActionOverdue, Some(1_000)),
        item("action-mid", AttentionReason::ActionOverdue, Some(2_000)),
    ]);

    assert_eq!(
        ids(&ranked),
        vec!["action-early", "action-mid", "action-late"]
    );
}

#[test]
fn an_absent_deadline_sorts_after_every_item_that_has_one() {
    // An absent deadline is not an early one. Treating `None` as the minimum,
    // which is its natural Ord, would put undated items at the very top.
    let ranked = rank_attention(&[
        item("action-none", AttentionReason::ActionOverdue, None),
        item("action-dated", AttentionReason::ActionOverdue, Some(9_999)),
    ]);

    assert_eq!(ids(&ranked), vec!["action-dated", "action-none"]);
}

#[test]
fn the_canonical_identifier_is_the_final_tie_break() {
    let ranked = rank_attention(&[
        item("action-c", AttentionReason::ActionOverdue, Some(1_000)),
        item("action-a", AttentionReason::ActionOverdue, Some(1_000)),
        item("action-b", AttentionReason::ActionOverdue, Some(1_000)),
    ]);

    assert_eq!(ids(&ranked), vec!["action-a", "action-b", "action-c"]);
}

#[test]
fn the_same_facts_always_produce_the_same_order() {
    // Stability is what lets a rank be trusted: it must never move unless a
    // fact moved.
    let forward = [
        item("action-a", AttentionReason::ActionOverdue, Some(1_000)),
        item("action-b", AttentionReason::ActionBlocked, Some(500)),
        item("action-c", AttentionReason::RiskReviewDue, None),
    ];
    let mut reversed = forward.clone();
    reversed.reverse();

    assert_eq!(
        ids(&rank_attention(&forward)),
        ids(&rank_attention(&reversed))
    );
}

#[test]
fn stale_data_never_promotes_an_item() {
    // A stale "overdue" is not known to still be overdue. Letting staleness
    // lift it above a fresh item with an earlier deadline would fabricate
    // certainty, which the Executive Cockpit forbids in those words ("never
    // fabricates progress or certainty").
    let ranked = rank_attention(&[
        item_with(
            "action-stale",
            AttentionReason::ActionOverdue,
            Some(2_000),
            Freshness::Stale,
            false,
        ),
        item("action-fresh", AttentionReason::ActionOverdue, Some(1_000)),
    ]);

    assert_eq!(ids(&ranked), vec!["action-fresh", "action-stale"]);
}

#[test]
fn a_stale_item_carries_its_uncertainty_in_its_rationale() {
    let ranked = rank_attention(&[item_with(
        "action-1",
        AttentionReason::ActionOverdue,
        Some(1_000),
        Freshness::Stale,
        true,
    )]);

    let rationale = &ranked[0].rank_rationale;
    assert!(
        rationale.contains("out of date"),
        "a stale item must say so: {rationale}"
    );
    assert!(
        rationale.contains("unavailable"),
        "a degraded item must say so: {rationale}"
    );
}

#[test]
fn every_ranked_item_states_why_it_is_ranked_where_it_is() {
    // Every attention item must answer why it is ranked where it is. The rationale is generated from the tier actually used, so it
    // cannot drift from the ordering.
    for reason in ALL_REASONS {
        let ranked = rank_attention(&[item("action-1", reason, Some(1_000))]);
        let entry = &ranked[0];
        assert!(
            entry.rank_rationale.contains(entry.tier.why()),
            "{reason:?} must state its own tier's reason"
        );
    }
}

#[test]
fn no_rank_claims_a_score_or_a_model() {
    // The stop condition names an unexplained model score. Nothing in a
    // rationale may read as one.
    for reason in ALL_REASONS {
        let ranked = rank_attention(&[item("action-1", reason, Some(1_000))]);
        let rationale = ranked[0].rank_rationale.to_lowercase();
        for forbidden in ["score", "confidence", "probability", "ai ", "model"] {
            assert!(
                !rationale.contains(forbidden),
                "{reason:?} rationale must not imply a {forbidden}: {rationale}"
            );
        }
    }
}

#[test]
fn an_empty_input_ranks_to_nothing_rather_than_inventing_a_row() {
    assert!(rank_attention(&[]).is_empty());
}

#[test]
fn every_reason_has_a_distinct_stable_identifier() {
    // The Work Queue (S03) must preserve attention flags, so a
    // surface has to name each reason. Two reasons sharing an identifier
    // would render as the same problem, and the reader would act on one
    // while the other stayed invisible.
    let mut seen = std::collections::HashSet::new();
    for reason in ALL_REASONS {
        assert!(
            seen.insert(reason.as_str()),
            "{reason:?} shares an identifier with another reason"
        );
    }
    assert_eq!(seen.len(), ALL_REASONS.len());
}

#[test]
fn a_reason_identifier_is_not_its_debug_rendering_or_its_prose() {
    // Both of those move. `Debug` is a derived convenience with no contract,
    // and an explanation is user-visible text that may be reworded; a surface
    // keying off either would break silently rather than fail to compile.
    for reason in ALL_REASONS {
        let identifier = reason.as_str();
        assert_ne!(identifier, format!("{reason:?}"));
        assert!(
            identifier
                .chars()
                .all(|character| character.is_ascii_lowercase() || character == '_'),
            "{reason:?} identifier {identifier} is not a stable snake_case name"
        );
        assert!(!identifier.is_empty(), "{reason:?} has an empty identifier");
    }
}

#[test]
fn naming_a_reason_says_nothing_about_its_severity() {
    // The accepted policy section 5 refuses `AttentionReason`'s derived `Ord`
    // because declaration order is an accident. Identifiers must not
    // reintroduce that: sorting by name must NOT reproduce the tier order,
    // or a surface could start ordering by the string and appear correct.
    let mut by_identifier: Vec<AttentionReason> = ALL_REASONS.to_vec();
    by_identifier.sort_by_key(|reason| reason.as_str());
    let tiers_in_name_order: Vec<AttentionTier> = by_identifier
        .iter()
        .map(|reason| tier_of(*reason))
        .collect();
    let mut sorted_tiers = tiers_in_name_order.clone();
    sorted_tiers.sort();
    assert_ne!(
        tiers_in_name_order, sorted_tiers,
        "sorting reasons by identifier reproduced the tier order, which invites \
         a surface to rank by name"
    );
}
