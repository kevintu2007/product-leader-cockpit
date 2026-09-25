//! The recovery passphrase for Operational Backups (S7-A, ADR 0010 §7–8).
//!
//! The host generates the default passphrase (ten BIP-0039 words) and hands it
//! to the webview once, to be shown and written down. Whatever the person
//! confirms comes back once and is held here for this session; only when they
//! ask is it also stored in Windows Credential Manager so scheduled backups
//! can run unattended. It never goes to the Ledger, the settings document,
//! an archive or a log, and no command ever returns a stored passphrase.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use std::sync::{Arc, Mutex};

use pmc_platform::backup_archive::Passphrase;
use pmc_platform::recovery_passphrase::{
    check_strength, generate_recovery_passphrase as generate, PassphraseProblem,
};
use serde::Serialize;
use tauri::State;

use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

/// The passphrase confirmed in this session, if any.
pub struct BackupPassphraseState(pub Mutex<Option<Arc<Passphrase>>>);

/// Where the passphrase a backup would use comes from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassphraseSource {
    NotSet,
    Session,
    Remembered,
}

impl PassphraseSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotSet => "not_set",
            Self::Session => "session",
            Self::Remembered => "remembered",
        }
    }
}

impl BackupPassphraseState {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }

    fn session(&self) -> Option<Arc<Passphrase>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The passphrase for a backup now: this session's, else the one
    /// remembered on this Windows account. Reading Credential Manager shows
    /// no prompt, so this is safe on a background thread at startup.
    pub fn current(&self) -> Option<Arc<Passphrase>> {
        self.session()
            .or_else(|| remembered::read().map(|secret| Arc::new(Passphrase::new(secret))))
    }

    /// Where [`current`](Self::current) would take it from, without reading
    /// the secret.
    pub fn source(&self) -> PassphraseSource {
        if self.session().is_some() {
            PassphraseSource::Session
        } else if remembered::exists() == Some(true) {
            PassphraseSource::Remembered
        } else {
            PassphraseSource::NotSet
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassphraseStatusDto {
    /// A passphrase is available to this session (confirmed now, or
    /// remembered on this Windows account).
    pub available: bool,
    /// It is stored in Windows Credential Manager for unattended backups.
    pub remembered: bool,
}

fn host_error(code: &'static str, key: &'static str, retryable: bool) -> SafeErrorDto {
    SafeErrorDto::host(code, key, &host_correlation(), retryable)
}

fn credential_unavailable() -> SafeErrorDto {
    host_error("PLATFORM_INTERNAL", "desktop.credential_unavailable", true)
}

#[cfg(windows)]
mod remembered {
    use pmc_platform::credential::{CredentialId, WindowsCredentialStore};

    const SERVICE: &str = "backup";
    const ACCOUNT: &str = "recovery-passphrase";

    fn store_and_id() -> Option<(WindowsCredentialStore, CredentialId)> {
        let store = WindowsCredentialStore::open().ok()?;
        let id = CredentialId::new(SERVICE, ACCOUNT).ok()?;
        Some((store, id))
    }

    pub fn save(passphrase: &str) -> bool {
        store_and_id().is_some_and(|(store, id)| store.set(&id, passphrase.as_bytes()).is_ok())
    }

    /// Removed, or nothing was stored; `false` only when a stored passphrase
    /// could not be removed.
    pub fn forget() -> bool {
        store_and_id().is_some_and(|(store, id)| store.delete(&id).is_ok())
    }

    /// Whether one is stored, without reading the secret itself.
    pub fn exists() -> Option<bool> {
        let (store, id) = store_and_id()?;
        store.exists(&id).ok()
    }

    /// The stored passphrase, if any. `CredRead` never shows UI.
    pub fn read() -> Option<String> {
        let (store, id) = store_and_id()?;
        let secret = store.get(&id).ok()?;
        String::from_utf8(secret.expose().to_vec()).ok()
    }
}

#[cfg(not(windows))]
mod remembered {
    pub fn save(_passphrase: &str) -> bool {
        false
    }
    pub fn forget() -> bool {
        true
    }
    pub fn exists() -> Option<bool> {
        Some(false)
    }
    pub fn read() -> Option<String> {
        None
    }
}

/// Produce a new ten-word passphrase for the person to see and write down.
/// Nothing is stored until they confirm it with `set_backup_passphrase`.
#[tauri::command]
pub fn generate_recovery_passphrase() -> Result<String, SafeErrorDto> {
    generate().map_err(|_| host_error("PLATFORM_INTERNAL", "desktop.random_unavailable", true))
}

/// H1-User: accept the passphrase the person confirmed, for this session, and
/// — only when `remember` — in Windows Credential Manager. With `remember`
/// off, any previously remembered passphrase is removed.
#[tauri::command]
pub fn set_backup_passphrase(
    state: State<'_, BackupPassphraseState>,
    passphrase: String,
    remember: bool,
) -> Result<PassphraseStatusDto, SafeErrorDto> {
    check_strength(&passphrase).map_err(|problem| {
        host_error(
            "VALIDATION_INVALID_FIELD",
            match problem {
                PassphraseProblem::TooShort => "desktop.passphrase_too_short",
                PassphraseProblem::TooRepetitive => "desktop.passphrase_too_repetitive",
                PassphraseProblem::TooLong => "desktop.passphrase_too_long",
                PassphraseProblem::TooCommon => "desktop.passphrase_too_common",
            },
            false,
        )
    })?;
    if remember {
        if !remembered::save(&passphrase) {
            return Err(credential_unavailable());
        }
    } else if !remembered::forget() {
        // The old one is still stored: saying "not remembered" would be false.
        return Err(credential_unavailable());
    }
    *state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(Arc::new(Passphrase::new(passphrase)));
    Ok(PassphraseStatusDto {
        available: true,
        remembered: remember,
    })
}

/// H0: whether a passphrase is available and whether it is remembered.
#[tauri::command]
pub fn get_backup_passphrase_status(
    state: State<'_, BackupPassphraseState>,
) -> Result<PassphraseStatusDto, SafeErrorDto> {
    let in_session = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some();
    let remembered = remembered::exists().ok_or_else(credential_unavailable)?;
    Ok(PassphraseStatusDto {
        available: in_session || remembered,
        remembered,
    })
}
