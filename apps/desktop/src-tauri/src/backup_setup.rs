//! Where Operational Backups go (S7-A, ADR 0010, ADR 0011).
//!
//! The person chooses a folder through the host's native picker. The host
//! proves it usable, stores it in the settings document, and tells the
//! webview only whether a destination is set and available — never where it
//! is.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use pmc_platform::backup_destination::probe_destination;
use pmc_platform::settings::{OperationalPatch, PatchOutcome, SettingsPatch, ValuePatch};
use serde::Serialize;
use tauri::State;

use crate::display_settings::{unavailable, SettingsState};
use crate::native_dialogs::pick_folder;
use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

/// What the webview may know about the backup destination.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupDestinationDto {
    /// A destination is stored in settings.
    pub configured: bool,
    /// It is still a real directory this account can reach right now.
    pub available: bool,
    /// Only for `choose_backup_destination`: the person picked a folder in
    /// this call (false when they cancelled).
    pub chosen: bool,
}

fn destination_status(
    state: &SettingsState,
    chosen: bool,
) -> Result<BackupDestinationDto, SafeErrorDto> {
    let guard = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = guard.as_ref().ok_or_else(unavailable)?;
    let destination = store
        .read()
        .map_err(|_| unavailable())?
        .operational
        .backup_destination;
    Ok(BackupDestinationDto {
        configured: destination.is_some(),
        // Revalidated now: the folder may be on a drive that is unplugged.
        available: destination.is_some_and(|folder| folder.revalidate().is_ok()),
        chosen,
    })
}

fn unusable(correlation: &pmc_domain::identity::CorrelationId) -> SafeErrorDto {
    SafeErrorDto::host(
        "VALIDATION_INVALID_FIELD",
        "desktop.backup_destination_unusable",
        correlation,
        false,
    )
}

/// H0: whether a backup destination is set and reachable.
#[tauri::command]
pub fn get_backup_destination(
    settings: State<'_, SettingsState>,
) -> Result<BackupDestinationDto, SafeErrorDto> {
    destination_status(&settings, false)
}

/// H1-User: open the native folder picker; store the chosen folder if it is a
/// real directory this account can write to. `title` is the dialog's title in
/// the person's language, supplied by the webview (it is not a path).
///
/// Async: the dialog itself is dispatched onto the main thread by
/// `pick_folder`, and this task waits for the answer off that thread.
#[tauri::command]
pub async fn choose_backup_destination(
    window: tauri::Window,
    settings: State<'_, SettingsState>,
    title: String,
) -> Result<BackupDestinationDto, SafeErrorDto> {
    let correlation = host_correlation();
    let title = title.chars().take(120).collect::<String>();
    // No lock is held while the dialog is open.
    let Some(folder) = pick_folder(window, title).await else {
        return destination_status(&settings, false);
    };
    let destination = probe_destination(folder).map_err(|_| unusable(&correlation))?;
    {
        let guard = settings
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let store = guard.as_ref().ok_or_else(unavailable)?;
        let revision = store.read().map_err(|_| unavailable())?.revision;
        // Proven again under the lock, just before it is stored: the folder
        // may have gone or become a link since the probe. Backups revalidate
        // it again at every use.
        if destination.revalidate().is_err() {
            return Err(unusable(&correlation));
        }
        let patch = SettingsPatch::Operational(OperationalPatch {
            backup_destination: ValuePatch::Set(destination),
            ..OperationalPatch::default()
        });
        match store
            .apply_durable(revision, patch)
            .map_err(|_| unavailable())?
        {
            PatchOutcome::Committed { .. } => {}
            // Only another process could have written in between.
            PatchOutcome::Stale { .. } => return Err(unavailable()),
        }
    }
    destination_status(&settings, true)
}
