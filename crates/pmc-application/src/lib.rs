#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

//! Domain-intent orchestration and application workflow boundaries.

pub mod action_lifecycle;
pub mod action_requests;
pub mod attention_ranking;
pub mod backup_service;
pub mod cockpit_adapter;
pub mod cockpit_aggregation;
pub mod composition_cache;
pub mod decision_requests;
pub mod delivery_entry;
pub mod desktop_runtime;
pub mod evidence_from_file;
pub mod evidence_writes;
pub mod executive_lens;
pub mod issue_lifecycle;
pub mod knowledge;
pub mod operational_backup;
pub mod people_adapter;
pub mod people_composition;
pub mod people_entry;
pub mod portfolio_entry;
pub mod product_adapter;
pub mod product_composition;
pub mod projection_publication;
pub mod record_entry;
pub mod restore_service;
pub mod risk_lifecycle;
pub mod route_composition;
pub mod sample_lifecycle;
pub mod sample_workspace;
pub mod scale_fixture;
pub mod vault_root_change;
pub mod work_entry;
pub mod work_management_authority;
pub mod work_queue_adapter;
pub mod work_queue_composition;
