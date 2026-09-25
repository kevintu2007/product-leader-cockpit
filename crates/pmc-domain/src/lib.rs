#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

//! Authoritative Product Mission Control domain types and invariants.

pub mod actions;
pub mod attention;
pub mod audit;
pub mod classification;
pub mod classification_governance;
pub mod composition_source;
pub mod decisions;
pub mod delivery;
pub mod error;
pub mod evidence;
pub mod execution;
pub mod identity;
pub mod issues;
pub mod managed_projection_rebuild;
pub mod portfolio;
pub mod projection_source;
pub mod provenance;
pub mod relationships;
pub mod risks;
pub mod state_intents;
pub mod time;
pub mod work_management;
pub mod work_management_runtime;

mod value;

pub use value::{BoundedText, DomainValueError, ValueErrorKind};
