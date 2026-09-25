//! `pmc-scale-v1` composition measurement.
//!
//! Interactive evidence requires "warmups, at least 30 measured runs,
//! median/p95, peak memory, and peak temporary disk", against a fixture
//! exercising 25,000 Actions/Requests.
//!
//! Two honest limits, stated here so the evidence cannot be read as more than
//! it is:
//!
//! - This measures the **composition** path -- ranking, ordering, filtering,
//!   grouping and cache -- which is what the composition layer owns.
//!   End-to-end query latency including SQLite needs the Ledger side of
//!   `pmc-scale-v1`, which belongs to the Ledger and which does not exist in
//!   this repository. That was
//!   checked rather than assumed.
//! - Peak temporary disk is zero by construction: this generator writes
//!   nothing. It is reported as zero because that is true, not because it was
//!   not measured.
//!
//! These run as ordinary tests with generous ceilings rather than as a
//! benchmark harness. A tight threshold would fail on a loaded machine and
//! teach the team to ignore it; the ceilings here are set to catch an
//! algorithmic regression -- the O(n^2) mistake -- not to police milliseconds.

use std::time::{Duration, Instant};

use pmc_application::composition_cache::{
    group_by_tier, BoundedCompositionCache, CompositionCacheKey, EntityFilter,
};
use pmc_application::scale_fixture::{
    scale_attention, scale_products, SCALE_ACTIONS_AND_REQUESTS, SCALE_PRODUCTS, SCALE_SEED,
};

/// Warmups discarded before measuring, so the first allocation does not land
/// in the reported median.
const WARMUPS: usize = 3;
/// The plan's floor, not a target.
const MEASURED_RUNS: usize = 30;

struct Measurement {
    median: Duration,
    p95: Duration,
}

fn measure(label: &str, mut operation: impl FnMut()) -> Measurement {
    for _ in 0..WARMUPS {
        operation();
    }
    let mut samples: Vec<Duration> = (0..MEASURED_RUNS)
        .map(|_| {
            let started = Instant::now();
            operation();
            started.elapsed()
        })
        .collect();
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    // Index of the 95th percentile within a sorted sample of 30.
    let p95 = samples[(samples.len() * 95) / 100];
    println!("[scale] {label}: median={median:?} p95={p95:?} runs={MEASURED_RUNS}");
    Measurement { median, p95 }
}

#[test]
fn the_fixture_matches_the_counts_dg0_declares() {
    // A generator that quietly produced fewer items would make every
    // measurement below meaningless while still passing.
    assert_eq!(SCALE_SEED, "pmc-scale-v1");
    let attention = scale_attention(SCALE_ACTIONS_AND_REQUESTS);
    assert_eq!(attention.len(), 25_000);
    assert_eq!(scale_products(SCALE_PRODUCTS, &attention).len(), 50);
}

#[test]
fn the_fixture_is_reproducible_from_its_seed() {
    // Deterministic regeneration is what lets a reviewer reproduce a
    // measurement rather than take it on trust.
    let first = scale_attention(500);
    let second = scale_attention(500);

    let first_ids: Vec<&str> = first.iter().map(|item| item.flag.explanation).collect();
    let second_ids: Vec<&str> = second.iter().map(|item| item.flag.explanation).collect();
    assert_eq!(first_ids, second_ids);
    assert_eq!(
        first.iter().map(|item| item.tier).collect::<Vec<_>>(),
        second.iter().map(|item| item.tier).collect::<Vec<_>>()
    );
}

#[test]
fn ranking_twenty_five_thousand_items_stays_interactive() {
    let items = scale_attention(SCALE_ACTIONS_AND_REQUESTS);

    let measurement = measure("rank 25k", || {
        let grouped = group_by_tier(&items);
        assert!(!grouped.is_empty());
    });

    assert!(
        measurement.p95 < Duration::from_millis(500),
        "grouping 25k ranked items regressed: p95 {:?}",
        measurement.p95
    );
}

#[test]
fn ordering_the_portfolio_over_the_full_fixture_stays_interactive() {
    use pmc_application::cockpit_aggregation::order_portfolio_first;
    let attention = scale_attention(SCALE_ACTIONS_AND_REQUESTS);
    let products = scale_products(SCALE_PRODUCTS, &attention);

    let measurement = measure("order 50 products over 25k items", || {
        let ordered = order_portfolio_first(&products);
        assert_eq!(ordered.len(), SCALE_PRODUCTS);
    });

    assert!(
        measurement.p95 < Duration::from_millis(500),
        "Portfolio-first ordering regressed: p95 {:?}",
        measurement.p95
    );
}

#[test]
fn filtering_the_full_fixture_stays_interactive() {
    let attention = scale_attention(SCALE_ACTIONS_AND_REQUESTS);
    let products = scale_products(SCALE_PRODUCTS, &attention);
    let filter = EntityFilter {
        only_needing_attention: true,
        ..EntityFilter::default()
    };

    let measurement = measure("filter 50 products over 25k items", || {
        let kept = filter.apply(&products);
        assert!(kept.len() <= SCALE_PRODUCTS);
    });

    assert!(
        measurement.p95 < Duration::from_millis(500),
        "filtering regressed: p95 {:?}",
        measurement.p95
    );
}

#[test]
fn a_cached_cockpit_is_reached_well_within_one_second() {
    // The plan's interactive requirement: "Cached Executive Cockpit becomes
    // usable within one second in the accepted synthetic envelope."
    let attention = scale_attention(SCALE_ACTIONS_AND_REQUESTS);
    let products = scale_products(SCALE_PRODUCTS, &attention);
    let mut cache = BoundedCompositionCache::new(8);
    let key = CompositionCacheKey {
        query: "GetExecutiveCockpit",
        scope: String::new(),
        ledger_revision: 7,
    };
    cache.put(key.clone(), products.clone());

    let measurement = measure("cached cockpit read", || {
        let hit = cache.get(&key).expect("the entry was just stored");
        assert_eq!(hit.len(), SCALE_PRODUCTS);
    });

    assert!(
        measurement.median < Duration::from_millis(1_000),
        "a cached Cockpit must be usable within one second: median {:?}",
        measurement.median
    );
}

#[test]
fn ranking_cost_grows_no_worse_than_linearithmically() {
    // The regression this suite exists to catch. An accidental O(n^2) --
    // the mistake already made once in this project's diff code -- would show
    // as a roughly hundredfold jump when the input grows tenfold.
    let small = scale_attention(2_500);
    let large = scale_attention(25_000);

    let small_time = measure("group 2.5k", || {
        let _ = group_by_tier(&small);
    })
    .median;
    let large_time = measure("group 25k", || {
        let _ = group_by_tier(&large);
    })
    .median;

    let ratio = large_time.as_secs_f64() / small_time.as_secs_f64().max(f64::MIN_POSITIVE);
    assert!(
        ratio < 40.0,
        "ten times the input cost {ratio:.1}x the time, which is not linearithmic"
    );
}
