#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

//! Product Vault knowledge: Vault configuration/health, Evidence
//! availability/freshness observation, narrow Obsidian URI intents, and
//! deterministic managed Markdown projections.

pub mod evidence;
pub mod obsidian_uri;
pub mod projections;
pub mod vault;
