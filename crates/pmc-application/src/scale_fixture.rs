//! Deterministic `pmc-scale-v1` composition fixtures.
//!
//! DG0 §8.2 names the scale baseline: 50 Products, 200 Projects, 25,000
//! Actions/Requests, 100,000 audit events, 10,000 notes and 1,000 attachments
//! totalling 5 GiB. It also fixes what the repository may hold -- "generator,
//! manifest, seed, expected counts/hashes and sampled public-safe assertions"
//! -- with large output produced on demand into a named workspace and never
//! committed. This module is the generator and the expected counts for the
//! composition layer; it produces composition inputs in memory and writes
//! nothing.
//!
//! **What this measures and what it does not.** The composition, ranking,
//! filtering and cache paths belong entirely to this layer, and they are what
//! this generator exercises at full scale. End-to-end query latency including
//! SQLite needs the Ledger side of `pmc-scale-v1`, which belongs to the Ledger
//! and which does not exist anywhere in the repository today -- that was
//! checked, not assumed. Reporting a composition measurement as if it were an
//! end-to-end one would be exactly the fabricated evidence this project's
//! governance exists to prevent, so the two are kept separate and the gap is
//! stated in the evidence rather than papered over.
//!
//! Everything here is synthetic and public-safe: identifiers are generated
//! from the seed, and no real name, path or payload appears.

use pmc_domain::attention::{
    AttentionFlag, AttentionMetadata, AttentionReason, AttentionTarget, Freshness,
};
use pmc_domain::classification::DataClassification;
use pmc_domain::identity::ActionId;
use pmc_domain::time::UtcTimestamp;

use crate::attention_ranking::{rank_attention, RankableAttentionItem, RankedAttentionItem};
use crate::route_composition::{ComposedEntity, ComposedEntityKind, OwnerModule};

/// The named seed. Changing it changes every generated identifier, so it is
/// part of the fixture's identity rather than a tuning knob.
pub const SCALE_SEED: &str = "pmc-scale-v1";

/// Counts DG0 §8.2 declares for the baseline. Held here so a drift between
/// the specification and the generator fails a test rather than passing
/// unnoticed.
pub const SCALE_PRODUCTS: usize = 50;
pub const SCALE_PROJECTS: usize = 200;
pub const SCALE_ACTIONS_AND_REQUESTS: usize = 25_000;

/// A tiny deterministic sequence. Not cryptographic and not trying to be:
/// its only job is to spread fixture values reproducibly so a measurement is
/// repeatable and a reviewer can regenerate the identical set from the seed.
fn deterministic(seed: u64, index: u64) -> u64 {
    let mut value = seed
        .wrapping_add(index.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn seed_value() -> u64 {
    SCALE_SEED.bytes().fold(0_u64, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(u64::from(byte))
    })
}

/// The reasons the generator cycles through, one per tier, so a measured set
/// exercises every branch of the ranking comparison rather than one.
const CYCLED_REASONS: [AttentionReason; 6] = [
    AttentionReason::ActionOverdue,
    AttentionReason::ActionBlocked,
    AttentionReason::ActionNeedsEvidence,
    AttentionReason::ActionAtRisk,
    AttentionReason::RiskMissingOwner,
    AttentionReason::IssueStale,
];

const CYCLED_CLASSIFICATIONS: [DataClassification; 4] = [
    DataClassification::Public,
    DataClassification::Internal,
    DataClassification::Confidential,
    DataClassification::Restricted,
];

const CYCLED_FRESHNESS: [Freshness; 3] = [Freshness::Fresh, Freshness::Stale, Freshness::Unknown];

/// Generates `count` ranked attention items spread across every tier,
/// classification and freshness state.
///
/// Returned already ranked, because that is the shape the Cockpit consumes
/// and measuring anything else would measure the wrong thing.
#[must_use]
pub fn scale_attention(count: usize) -> Vec<RankedAttentionItem> {
    let seed = seed_value();
    let items: Vec<RankableAttentionItem> = (0..count)
        .map(|index| {
            let noise = deterministic(seed, index as u64);
            let reason = CYCLED_REASONS[(noise % CYCLED_REASONS.len() as u64) as usize];
            let classification = CYCLED_CLASSIFICATIONS
                [((noise >> 8) % CYCLED_CLASSIFICATIONS.len() as u64) as usize];
            let freshness =
                CYCLED_FRESHNESS[((noise >> 16) % CYCLED_FRESHNESS.len() as u64) as usize];
            RankableAttentionItem {
                flag: AttentionFlag {
                    target: AttentionTarget::Action(
                        ActionId::parse(format!("action-{index:07}"))
                            .unwrap_or_else(|_| unreachable!("generated identifiers are valid")),
                    ),
                    reason,
                    metadata: AttentionMetadata {
                        classification,
                        freshness,
                        degraded: noise % 17 == 0,
                    },
                    explanation: "synthetic scale fixture item",
                    failed_verification_guidance: None,
                },
                // Spread deadlines so the within-tier time comparison does real
                // work instead of collapsing to the identifier tie-break.
                relevant_at: if noise % 11 == 0 {
                    None
                } else {
                    Some(UtcTimestamp::from_unix_millis(
                        1_700_000_000_000 + i64::try_from(noise % 5_000_000).unwrap_or(0),
                    ))
                },
            }
        })
        .collect();
    rank_attention(&items)
}

/// Generates `count` composed Products, each carrying a share of the
/// attention items so ordering has something real to order.
#[must_use]
pub fn scale_products(count: usize, attention: &[RankedAttentionItem]) -> Vec<ComposedEntity> {
    let seed = seed_value();
    let per_product = if count == 0 {
        0
    } else {
        attention.len() / count
    };
    (0..count)
        .map(|index| {
            let noise = deterministic(seed, 1_000_000 + index as u64);
            let start = index * per_product;
            let end = (start + per_product).min(attention.len());
            ComposedEntity {
                kind: ComposedEntityKind::Product,
                id: format!("product-{index:04}"),
                owner: OwnerModule::Portfolio,
                revision: 7,
                as_of: UtcTimestamp::from_unix_millis(1_700_000_000_000),
                classification: CYCLED_CLASSIFICATIONS
                    [(noise % CYCLED_CLASSIFICATIONS.len() as u64) as usize],
                freshness: Freshness::Fresh,
                degraded: noise % 13 == 0,
                attention: attention.get(start..end).unwrap_or(&[]).to_vec(),
                lifecycle_legal_intents: Vec::new(),
            }
        })
        .collect()
}
