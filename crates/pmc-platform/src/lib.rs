// `deny`, not `forbid`: `windows_names` holds the one `unsafe` call the
// product owner approved (2026-09-23, option A for Evidence-from-a-file path
// identity). Every other module still refuses it; any further `unsafe` needs
// the product owner again.
#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

//! OS paths, settings, dialogs, credentials, and platform capability adapters.

pub mod authority_control;
pub mod backup_archive;
pub mod backup_destination;
pub mod backup_registry;
pub mod filesystem;
pub mod host_audit;
pub mod instance_lock;
pub mod local_time;
pub mod managed_publication;
pub mod paths;
pub mod preservation;
pub mod recovery_passphrase;
pub mod restore_control;
pub mod sample_control;
pub mod settings;
pub mod uri_launcher;
pub mod windows_names;
pub mod workspace;

#[cfg(target_os = "windows")]
pub mod credential;
