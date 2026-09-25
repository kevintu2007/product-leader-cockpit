//! The UI language preference, kept in the platform settings document
//! (`DisplaySettings::locale`) rather than in webview storage.
//!
//! The stored value is either one of the six UI languages or
//! [`FOLLOW_SYSTEM_LOCALE`], which the renderer resolves against the
//! operating system's languages at every launch. The host never resolves it
//! and never displays in it.
//!
//! First run is decided once, at startup, before the Ledger is opened: a
//! profile that has used the app before this setting existed keeps
//! Traditional Chinese, the only language the UI had; a new profile follows
//! the system. The whole first document is one atomic write, so a crash
//! cannot leave the wrong choice behind.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use std::sync::Mutex;

use pmc_platform::settings::{
    startup_selection, DisplayPatch, LoadDisposition, PatchOutcome, ProtectedSettingsRoot,
    SelectedWorkspace, SettingsPatch, SettingsStore, StartupSelection, FOLLOW_SYSTEM_LOCALE,
};
use tauri::State;

use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

/// The UI languages the renderer has catalogs for (`i18n/locale.ts`).
pub const UI_LOCALES: [&str; 6] = ["en", "zh-TW", "zh-CN", "ja", "ko", "es"];

/// The language every screen was written in before the setting existed.
const LEGACY_LOCALE: &str = "zh-TW";

/// The one managed settings store, shared by every command that reads or
/// writes settings (language, backup destination) so their writes are
/// serialized. `None` when the settings document could not be opened or
/// created; the renderer then follows the system language for this session,
/// and settings writes report `desktop.settings_unavailable`.
pub struct SettingsState(pub Mutex<Option<SettingsStore>>);

/// What a profile's first settings document stores as its language.
#[must_use]
pub const fn first_run_locale(used_before: bool) -> &'static str {
    if used_before {
        LEGACY_LOCALE
    } else {
        FOLLOW_SYSTEM_LOCALE
    }
}

/// Whether a stored or requested preference is one this app can honour.
#[must_use]
pub fn is_preference(locale: &str) -> bool {
    locale == FOLLOW_SYSTEM_LOCALE || UI_LOCALES.contains(&locale)
}

/// What a profile's first settings document records as its workspace
/// (sample-workspace amendment §2): Live for a profile used before, and for
/// settings PMC could not read (never a first-run choice, never sample data);
/// nothing yet for a genuinely new profile, which is asked.
#[must_use]
pub const fn first_run_workspace(
    used_before: bool,
    settings_were_unreadable: bool,
) -> Option<SelectedWorkspace> {
    if used_before || settings_were_unreadable {
        Some(SelectedWorkspace::Live)
    } else {
        None
    }
}

/// The settings as startup found them, and which workspace that means.
pub struct SettingsAtStartup {
    /// `None` when the document could not be opened or created.
    pub store: Option<SettingsStore>,
    /// Which workspace this run opens, or the first-run choice (§2).
    pub selection: StartupSelection,
    /// The document could not be read and was set aside; a fresh one was
    /// written (System Health says so).
    pub set_aside: bool,
}

/// Open the settings document, creating it on first run. A missing document
/// and a corrupt one (moved aside by the store) are both first runs; the
/// caller's `used_before` decides which language that first run stores, and
/// together with the disposition which workspace it records and opens
/// (sample-workspace amendment §2). Settings that cannot be opened at all
/// open Live — never the first-run choice, never sample data.
pub fn open(root: &ProtectedSettingsRoot, used_before: bool) -> SettingsAtStartup {
    let unavailable = SettingsAtStartup {
        store: None,
        selection: StartupSelection::Open(SelectedWorkspace::Live),
        set_aside: false,
    };
    let Ok(opened) = SettingsStore::open(root) else {
        return unavailable;
    };
    let disposition = opened.disposition();
    let selection = startup_selection(
        &disposition,
        opened.document().operational.selected_workspace,
        used_before,
    );
    let needs_document = !matches!(disposition, LoadDisposition::Loaded);
    let set_aside = matches!(disposition, LoadDisposition::InvalidPreserved { .. });
    let store = opened.into_store();
    if needs_document
        && store
            .initialize_first_run(
                first_run_locale(used_before),
                first_run_workspace(used_before, set_aside),
            )
            .is_err()
    {
        return SettingsAtStartup {
            set_aside,
            ..unavailable
        };
    }
    SettingsAtStartup {
        store: Some(store),
        selection,
        set_aside,
    }
}

pub(crate) fn unavailable() -> SafeErrorDto {
    SafeErrorDto::host(
        "PLATFORM_INTERNAL",
        "desktop.settings_unavailable",
        &host_correlation(),
        false,
    )
}

/// The stored language preference, verbatim: one of the six UI languages or
/// [`FOLLOW_SYSTEM_LOCALE`]. Anything else reads as following the system.
#[tauri::command]
pub fn get_display_locale(state: State<'_, SettingsState>) -> Result<String, SafeErrorDto> {
    let guard = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = guard.as_ref().ok_or_else(unavailable)?;
    let locale = store.read().map_err(|_| unavailable())?.display.locale;
    Ok(if is_preference(&locale) {
        locale
    } else {
        FOLLOW_SYSTEM_LOCALE.to_owned()
    })
}

/// The workspace's configured time zone (`DisplaySettings::timezone`, an
/// IANA name the settings document validated), the zone every entered date
/// is read in (DG3 record-entry amendment §3.5). The renderer falls back to
/// the browser's zone only when the settings document is unavailable.
#[tauri::command]
pub fn get_display_timezone(state: State<'_, SettingsState>) -> Result<String, SafeErrorDto> {
    let guard = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = guard.as_ref().ok_or_else(unavailable)?;
    Ok(store.read().map_err(|_| unavailable())?.display.timezone)
}

/// Store a new language preference. Writes go through the one managed store,
/// serialized by its mutex, against the revision just read.
#[tauri::command]
pub fn set_display_locale(
    state: State<'_, SettingsState>,
    locale: String,
) -> Result<String, SafeErrorDto> {
    if !is_preference(&locale) {
        return Err(SafeErrorDto::host(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            &host_correlation(),
            false,
        ));
    }
    let guard = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let store = guard.as_ref().ok_or_else(unavailable)?;
    let revision = store.read().map_err(|_| unavailable())?.revision;
    let patch = SettingsPatch::Display(DisplayPatch {
        locale: Some(locale.clone()),
        ..DisplayPatch::default()
    });
    match store
        .apply_durable(revision, patch)
        .map_err(|_| unavailable())?
    {
        PatchOutcome::Committed { .. } => Ok(locale),
        // Only another process could have written in between.
        PatchOutcome::Stale { .. } => Err(unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_used_before_keeps_traditional_chinese_and_a_new_one_follows_the_system() {
        assert_eq!(first_run_locale(true), "zh-TW");
        assert_eq!(first_run_locale(false), "und");
    }

    #[test]
    fn only_a_genuinely_new_profile_is_left_to_choose_its_workspace() {
        assert_eq!(first_run_workspace(false, false), None);
        assert_eq!(
            first_run_workspace(true, false),
            Some(SelectedWorkspace::Live)
        );
        // Unreadable settings never lead to the first-run choice.
        assert_eq!(
            first_run_workspace(false, true),
            Some(SelectedWorkspace::Live)
        );
    }

    #[test]
    fn only_the_six_languages_and_follow_system_are_preferences() {
        for locale in ["und", "en", "zh-TW", "zh-CN", "ja", "ko", "es"] {
            assert!(is_preference(locale), "{locale}");
        }
        for locale in ["", "fr", "zh", "zh-Hant", "EN", "en-US", "und-x"] {
            assert!(!is_preference(locale), "{locale}");
        }
    }
}
