//! Filters, grouping and bounded disposable caching.

use pmc_application::attention_ranking::{
    rank_attention, AttentionTier, RankableAttentionItem, RankedAttentionItem,
};
use pmc_application::composition_cache::{
    group_by_tier, BoundedCompositionCache, CompositionCacheKey, EntityFilter,
};
use pmc_application::route_composition::{ComposedEntity, ComposedEntityKind, OwnerModule};
use pmc_domain::attention::{
    AttentionFlag, AttentionMetadata, AttentionReason, AttentionTarget, Freshness,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::ActionId;
use pmc_domain::time::UtcTimestamp;

fn now() -> UtcTimestamp {
    UtcTimestamp::from_unix_millis(1_700_000_000_000)
}

fn key(scope: &str, revision: u64) -> CompositionCacheKey {
    CompositionCacheKey {
        query: "GetExecutiveCockpit",
        scope: scope.to_owned(),
        ledger_revision: revision,
    }
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
            explanation: "fixture",
            failed_verification_guidance: None,
        },
        relevant_at: Some(now()),
    }];
    rank_attention(&items).remove(0)
}

fn entity(
    id: &str,
    classification: DataClassification,
    attention_items: Vec<RankedAttentionItem>,
    degraded: bool,
) -> ComposedEntity {
    ComposedEntity {
        kind: ComposedEntityKind::Product,
        id: id.to_owned(),
        owner: OwnerModule::Portfolio,
        revision: 7,
        as_of: now(),
        classification,
        freshness: Freshness::Fresh,
        degraded,
        attention: attention_items,
        lifecycle_legal_intents: Vec::new(),
    }
}

#[test]
fn a_composition_built_at_one_revision_is_invisible_at_the_next() {
    // Invalidation by owner revision, with no expiry bookkeeping and no
    // second clock. The old entry is not expired, it is a different key.
    let mut cache = BoundedCompositionCache::new(8);
    cache.put(key("portfolio", 7), "composed at 7");

    assert_eq!(cache.get(&key("portfolio", 7)), Some(&"composed at 7"));
    assert_eq!(
        cache.get(&key("portfolio", 8)),
        None,
        "a newer revision must never be served a composition built from an older one"
    );
}

#[test]
fn a_stale_entry_is_never_served_with_a_warning() {
    // Serving it and labelling it would be presenting a stale result as
    // current, which the stop conditions name directly. It is simply absent.
    let mut cache = BoundedCompositionCache::new(8);
    cache.put(key("portfolio", 7), "composed at 7");

    assert!(cache.get(&key("portfolio", 9)).is_none());
}

#[test]
fn different_scopes_never_share_a_cached_result() {
    let mut cache = BoundedCompositionCache::new(8);
    cache.put(key("product-1", 7), "detail for product-1");

    assert!(cache.get(&key("product-2", 7)).is_none());
}

#[test]
fn dropping_the_cache_changes_the_speed_and_never_the_answer() {
    // This is what "disposable" has to mean. Asserted rather than assumed.
    let compute = |revision: u64| format!("composition at revision {revision}");
    let mut cache = BoundedCompositionCache::new(8);

    let uncached: Vec<String> = (1..=5).map(compute).collect();
    let mut through_cache = Vec::new();
    for revision in 1..=5 {
        let entry_key = key("portfolio", revision);
        if cache.get(&entry_key).is_none() {
            cache.put(entry_key.clone(), compute(revision));
        }
        through_cache.push(cache.get(&entry_key).cloned().unwrap());
    }
    cache.clear();
    let after_clear: Vec<String> = (1..=5)
        .map(|revision| {
            let entry_key = key("portfolio", revision);
            if cache.get(&entry_key).is_none() {
                cache.put(entry_key.clone(), compute(revision));
            }
            cache.get(&entry_key).cloned().unwrap()
        })
        .collect();

    assert_eq!(through_cache, uncached);
    assert_eq!(after_clear, uncached);
}

#[test]
fn the_cache_stays_within_its_bound_and_evicts_least_recently_used() {
    let mut cache = BoundedCompositionCache::new(2);
    cache.put(key("a", 7), "a");
    cache.put(key("b", 7), "b");
    // Touch `a` so `b` becomes the least recently used.
    assert_eq!(cache.get(&key("a", 7)), Some(&"a"));

    cache.put(key("c", 7), "c");

    assert_eq!(cache.len(), 2, "the bound must hold");
    assert!(
        cache.get(&key("b", 7)).is_none(),
        "b was least recently used"
    );
    assert!(cache.get(&key("a", 7)).is_some());
    assert!(cache.get(&key("c", 7)).is_some());
}

#[test]
fn a_zero_capacity_cache_always_misses_rather_than_failing() {
    // Running with the cache disabled is a supported configuration, so it
    // must behave like a cache that never hits, not like a broken one.
    let mut cache = BoundedCompositionCache::new(0);
    cache.put(key("portfolio", 7), "composed");

    assert!(cache.get(&key("portfolio", 7)).is_none());
    assert!(cache.is_empty());
}

#[test]
fn re_storing_the_same_key_does_not_grow_the_cache() {
    let mut cache = BoundedCompositionCache::new(2);
    cache.put(key("a", 7), "first");
    cache.put(key("a", 7), "second");

    assert_eq!(cache.len(), 1);
    assert_eq!(cache.get(&key("a", 7)), Some(&"second"));
}

#[test]
fn a_classification_bound_keeps_only_what_is_within_it() {
    let entities = [
        entity(
            "product-public",
            DataClassification::Public,
            Vec::new(),
            false,
        ),
        entity(
            "product-internal",
            DataClassification::Internal,
            Vec::new(),
            false,
        ),
        entity(
            "product-restricted",
            DataClassification::Restricted,
            Vec::new(),
            false,
        ),
    ];
    let filter = EntityFilter {
        at_most: Some(DataClassification::Internal),
        ..EntityFilter::default()
    };

    let kept = filter.apply(&entities);

    let ids: Vec<&str> = kept.iter().map(|entity| entity.id.as_str()).collect();
    assert_eq!(ids, vec!["product-public", "product-internal"]);
}

#[test]
fn an_unclassified_entity_is_excluded_by_every_bound() {
    // `Unclassified` is the most restrictive rank, so it fails closed here
    // too rather than slipping through a Public bound.
    let entities = [entity(
        "product-unknown",
        DataClassification::Unclassified,
        Vec::new(),
        false,
    )];
    let filter = EntityFilter {
        at_most: Some(DataClassification::Restricted),
        ..EntityFilter::default()
    };

    assert!(filter.apply(&entities).is_empty());
}

#[test]
fn filtering_preserves_the_order_it_was_given() {
    // The caller has already ordered by the accepted policy. Re-sorting here
    // would let a filter silently change a rank.
    let entities = [
        entity("product-c", DataClassification::Internal, Vec::new(), false),
        entity("product-a", DataClassification::Internal, Vec::new(), false),
        entity("product-b", DataClassification::Internal, Vec::new(), false),
    ];

    let kept = EntityFilter::default().apply(&entities);

    let ids: Vec<&str> = kept.iter().map(|entity| entity.id.as_str()).collect();
    assert_eq!(ids, vec!["product-c", "product-a", "product-b"]);
}

#[test]
fn attention_and_degraded_filters_select_what_they_name() {
    let entities = [
        entity(
            "product-quiet",
            DataClassification::Internal,
            Vec::new(),
            false,
        ),
        entity(
            "product-flagged",
            DataClassification::Internal,
            vec![attention("action-1", AttentionReason::ActionOverdue)],
            false,
        ),
        entity(
            "product-degraded",
            DataClassification::Internal,
            Vec::new(),
            true,
        ),
    ];

    let flagged = EntityFilter {
        only_needing_attention: true,
        ..EntityFilter::default()
    }
    .apply(&entities);
    let degraded = EntityFilter {
        only_degraded: true,
        ..EntityFilter::default()
    }
    .apply(&entities);

    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].id, "product-flagged");
    assert_eq!(degraded.len(), 1);
    assert_eq!(degraded[0].id, "product-degraded");
}

#[test]
fn two_different_filters_never_share_a_cache_key() {
    let broad = EntityFilter::default();
    let narrow = EntityFilter {
        only_needing_attention: true,
        ..EntityFilter::default()
    };

    assert_ne!(broad.fingerprint(), narrow.fingerprint());
}

#[test]
fn grouping_keeps_the_ranked_order_and_never_merges_tiers() {
    // A grouping that reordered items would quietly become a second ranking.
    let items = vec![
        attention("action-1", AttentionReason::ActionOverdue),
        attention("action-2", AttentionReason::ActionOverdue),
        attention("action-3", AttentionReason::ActionBlocked),
        attention("action-4", AttentionReason::IssueStale),
    ];

    let groups = group_by_tier(&items);

    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].0, AttentionTier::BreachedCommitment);
    assert_eq!(groups[0].1.len(), 2);
    assert_eq!(groups[1].0, AttentionTier::Blocked);
    assert_eq!(groups[2].0, AttentionTier::Other);
    let flattened: Vec<&str> = groups
        .iter()
        .flat_map(|(_, bucket)| bucket.iter())
        .map(|item| item.flag.explanation)
        .collect();
    assert_eq!(flattened.len(), items.len(), "grouping must lose nothing");
}

#[test]
fn grouping_an_empty_list_produces_no_groups() {
    assert!(group_by_tier(&[]).is_empty());
}
