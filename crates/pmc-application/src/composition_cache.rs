//! Filters, grouping and bounded disposable caching for composition.
//!
//! The cache exists to make the Cockpit usable quickly, and the Cockpit's
//! design is unusually specific about what it may not become: invalidation "uses owner
//! revisions and may never create a competing freshness or authority clock",
//! the cache must stay "disposable and never authoritative", and it is a stop
//! condition if "a read model or cache becomes authoritative, stale results
//! are presented as current, or query failure changes state".
//!
//! Those constraints rule out the obvious design. **There is no TTL here.** A
//! time-to-live *is* a competing freshness clock: it invents a second opinion
//! about when a fact stopped being current, held by the cache rather than by
//! the Ledger, and the two can disagree. Instead the Ledger revision is part
//! of the cache key. A revision change turns every entry built at the old
//! revision into a miss automatically, with no expiry bookkeeping and no
//! second clock -- the Ledger remains the only authority on what is current.
//!
//! Two consequences worth stating because they are the point rather than a
//! side effect:
//!
//! - A stale entry is never *served with a warning*; it is simply not found.
//!   Serving it and labelling it would be presenting a stale result as
//!   current, dressed up.
//! - Dropping the whole cache can change how fast an answer arrives and never
//!   what the answer is. That is what "disposable" means, and it is asserted
//!   directly rather than assumed.

use std::collections::HashMap;

use pmc_domain::classification::DataClassification;

use crate::attention_ranking::{AttentionTier, RankedAttentionItem};
use crate::route_composition::ComposedEntity;

/// Identifies one cached composition.
///
/// The Ledger revision is part of the identity, not metadata beside it. That
/// is the whole invalidation mechanism: a composition built at revision 7 can
/// never be returned for a request at revision 8, because it is a different
/// key rather than an expired one.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CompositionCacheKey {
    /// Which composition query, in its own vocabulary.
    pub query: &'static str,
    /// What the query was scoped to -- a product id, a page, a filter
    /// fingerprint. Empty for a whole-portfolio query.
    pub scope: String,
    /// The authoritative revision the entry was composed from.
    pub ledger_revision: u64,
}

/// A bounded, disposable, in-memory composition cache.
///
/// Bounded by entry count, with least-recently-used eviction. Eviction order
/// is by use, deliberately not by age: an age-based policy would be another
/// clock, which is exactly what the plan forbids.
///
/// No new dependency, no remote state, nothing persisted -- the plan's safe
/// default for cache strategy is "rebuildable memory or disposable cache; no
/// Redis or remote state".
#[derive(Debug)]
pub struct BoundedCompositionCache<T> {
    capacity: usize,
    entries: HashMap<CompositionCacheKey, T>,
    /// Least-recently-used first.
    usage: Vec<CompositionCacheKey>,
}

impl<T> BoundedCompositionCache<T> {
    /// A cache holding at most `capacity` compositions.
    ///
    /// A zero capacity is legitimate and means "never cache". It must behave
    /// exactly like a cache that always misses rather than panicking, because
    /// disabling the cache is a supported way to run.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: HashMap::new(),
            usage: Vec::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns a composition built at exactly this revision, or nothing.
    ///
    /// There is no "stale hit" path by construction: an entry for another
    /// revision has a different key and is invisible here.
    pub fn get(&mut self, key: &CompositionCacheKey) -> Option<&T> {
        if !self.entries.contains_key(key) {
            return None;
        }
        self.touch(key);
        self.entries.get(key)
    }

    /// Stores a composition, evicting the least recently used entry if full.
    pub fn put(&mut self, key: CompositionCacheKey, value: T) {
        if self.capacity == 0 {
            return;
        }
        if !self.entries.contains_key(&key) && self.entries.len() == self.capacity {
            if let Some(evicted) = self.usage.first().cloned() {
                self.entries.remove(&evicted);
                self.usage.remove(0);
            }
        }
        self.entries.insert(key.clone(), value);
        self.touch(&key);
    }

    /// Discards everything. Always safe: the cache is never authoritative, so
    /// this can only cost time.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.usage.clear();
    }

    fn touch(&mut self, key: &CompositionCacheKey) {
        if let Some(position) = self.usage.iter().position(|entry| entry == key) {
            self.usage.remove(position);
        }
        self.usage.push(key.clone());
    }
}

/// A deterministic filter over composed entities.
///
/// Every criterion is a fact the entity already carries. There is no free-text
/// search and no relevance score: both would reintroduce an unexplainable
/// ordering, which the ranking policy exists to prevent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EntityFilter {
    /// Keep only entities at or below this classification. `None` keeps all.
    pub at_most: Option<DataClassification>,
    /// Keep only entities with at least one outstanding attention item.
    pub only_needing_attention: bool,
    /// Keep only entities whose source could not be fully consulted.
    pub only_degraded: bool,
}

impl EntityFilter {
    #[must_use]
    pub fn matches(&self, entity: &ComposedEntity) -> bool {
        if self.only_needing_attention && entity.attention.is_empty() {
            return false;
        }
        if self.only_degraded && !entity.degraded {
            return false;
        }
        match self.at_most {
            // `combine` returns the more restrictive of the two, so an entity
            // is within the bound exactly when combining changes nothing.
            Some(bound) => bound.combine(entity.classification) == bound,
            None => true,
        }
    }

    /// Applies the filter, preserving the input order.
    ///
    /// Order is preserved rather than recomputed because the caller has
    /// already ordered by the accepted policy; re-sorting here would let a
    /// filter silently change a rank.
    #[must_use]
    pub fn apply(&self, entities: &[ComposedEntity]) -> Vec<ComposedEntity> {
        entities
            .iter()
            .filter(|entity| self.matches(entity))
            .cloned()
            .collect()
    }

    /// A stable fingerprint for use in a cache key, so two different filters
    /// can never share a cached result.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        format!(
            "at_most={};attention={};degraded={}",
            self.at_most.map_or("any", DataClassification::as_persisted),
            self.only_needing_attention,
            self.only_degraded
        )
    }
}

/// Attention items grouped by tier, tiers in ranked order.
///
/// Grouping preserves the ranked order within each group and never merges
/// tiers, so a reader sees the same sequence they would see ungrouped. A
/// grouping that reordered items would quietly become a second ranking.
#[must_use]
pub fn group_by_tier(
    items: &[RankedAttentionItem],
) -> Vec<(AttentionTier, Vec<RankedAttentionItem>)> {
    let mut groups: Vec<(AttentionTier, Vec<RankedAttentionItem>)> = Vec::new();
    for item in items {
        match groups.last_mut() {
            Some((tier, bucket)) if *tier == item.tier => bucket.push(item.clone()),
            _ => groups.push((item.tier, vec![item.clone()])),
        }
    }
    groups
}
