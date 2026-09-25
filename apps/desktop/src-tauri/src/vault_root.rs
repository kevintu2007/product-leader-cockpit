//! Changing the Live workspace's Product Vault folder in the host (item ⑦,
//! H2b; the accepted DG3 Vault-root amendment §2–§3; ADR 0011, ADR 0012).
//!
//! The sheet's steps map to one command each: choose the folder (the host's
//! picker; the webview gets an opaque token and the folder's own name),
//! prepare (a fresh verified backup as recovery evidence, then the exact
//! preview), then approve or reject. The chosen folder stays here; no path
//! goes back over IPC.
//!
//! Approval holds the backup gate's admission and the Ledger's write lock
//! for the whole call — the same pair, in the same order, every Ledger write
//! takes — so nothing can write between the service's re-check of what the
//! preview bound and the settings write that commits the change.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use pmc_application::desktop_runtime::SystemClock;
use pmc_application::vault_root_change::{
    ApproveVaultRootChange, PrepareVaultRootChange, VaultRootChangeError, VaultRootService,
};
use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;
use pmc_platform::authority_control::{AuthorityOperation, VaultRootChangeOutcome};
use pmc_platform::backup_registry::new_archive_id;
use pmc_platform::settings::{CanonicalDirectoryPath, DeferredDirectoryPath};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::backup_gate::BackupGate;
use crate::backup_run::{run_backup, run_error, stored_destination};
use crate::display_settings::{unavailable, SettingsState};
use crate::ledger_state::{protected_root, LedgerState};
use crate::native_dialogs::pick_folder;
use crate::runtime::host_correlation;
use crate::safe_error::{MessageParamDto, SafeErrorDto, SafeParamValueDto};

/// How long a prepared preview may be approved (the H2b Prepared Intent).
const PREPARED_FOR_MILLIS: i64 = 15 * 60 * 1000;
/// Letters and digits a person cannot misread for one another: no I, L, O,
/// 0 or 1.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 6;
/// The fixed phrase of §3.3 in each UI language, exactly as the catalogs'
/// `vaultRoot.confirm.phrase` hold it (a renderer test keeps the two
/// equal). The host checks the phrase itself rather than trusting the
/// sheet: the confirmation is part of the approval, and the approval is the
/// host's authority.
const CONFIRMATION_PHRASES: [&str; 6] = [
    "CHANGE VAULT",
    "更換 Vault",
    "更换 Vault",
    "VAULT を変更",
    "VAULT 변경",
    "CAMBIAR VAULT",
];

/// The typed confirmation as the person means it: spaces collapsed and
/// letters in one case.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// The code the person typed after one of the fixed phrases, or `None` when
/// the text is not a phrase followed by one word. Whether that word is this
/// preview's code is the service's check.
fn typed_code(typed: &str) -> Option<String> {
    let typed = normalized(typed);
    let (phrase, code) = typed.rsplit_once(' ')?;
    CONFIRMATION_PHRASES
        .iter()
        .any(|candidate| normalized(candidate) == phrase)
        .then(|| code.to_owned())
}

/// Whether `folder` is, contains, or lies inside a folder PMC owns: its
/// protected settings root, or the backup folder as it is set now.
/// Checked when the folder is chosen and again at every step after, because
/// the backup folder can be changed in between.
fn in_use(app: &AppHandle, folder: &CanonicalDirectoryPath) -> bool {
    folder.overlaps(protected_root().path())
        || stored_destination(&app.state::<SettingsState>())
            .and_then(|destination| destination.revalidate().ok())
            .is_some_and(|destination| folder.overlaps(destination.as_path()))
}

fn in_use_error(correlation: &CorrelationId) -> SafeErrorDto {
    error(
        "VALIDATION_INVALID_FIELD",
        "desktop.vault_folder_in_use",
        correlation,
        false,
    )
}

/// This workspace's Vault-change service, managed as Tauri state.
pub struct VaultRootServiceState(pub VaultRootService);

#[derive(Default)]
struct Inner {
    chosen: Option<(String, CanonicalDirectoryPath)>,
    /// A preparation is running; a second is refused.
    busy: bool,
}

/// The folder the person picked, for this session.
pub struct VaultRootSession {
    live: bool,
    inner: Mutex<Inner>,
}

impl VaultRootSession {
    pub fn new(live: bool) -> Self {
        Self {
            live,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Marks the session busy; clears it however the step ends.
struct Busy<'a>(&'a VaultRootSession);

impl<'a> Busy<'a> {
    fn claim(
        session: &'a VaultRootSession,
        correlation: &CorrelationId,
    ) -> Result<Self, SafeErrorDto> {
        let mut inner = session.lock();
        if inner.busy {
            return Err(error(
                "VAULT_CHANGE_ACTIVE",
                "desktop.vault_change_active",
                correlation,
                true,
            ));
        }
        inner.busy = true;
        Ok(Self(session))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.lock().busy = false;
    }
}

fn now() -> UtcTimestamp {
    SystemClock.read()
}

fn error(
    code: &'static str,
    key: &'static str,
    correlation: &CorrelationId,
    retryable: bool,
) -> SafeErrorDto {
    SafeErrorDto::host(code, key, correlation, retryable)
}

fn with_count(mut dto: SafeErrorDto, count: u64) -> SafeErrorDto {
    dto.message_params.push(MessageParamDto {
        key: "count".to_owned(),
        value: SafeParamValueDto::Unsigned(count),
    });
    dto
}

fn live_only(session: &VaultRootSession, correlation: &CorrelationId) -> Result<(), SafeErrorDto> {
    if session.live {
        Ok(())
    } else {
        Err(error(
            "VAULT_NOT_LIVE",
            "desktop.vault_not_live",
            correlation,
            false,
        ))
    }
}

fn stale(correlation: &CorrelationId) -> SafeErrorDto {
    error(
        "VAULT_PREVIEW_STALE",
        "desktop.vault_preview_stale",
        correlation,
        false,
    )
}

/// DG3 Vault-root amendment §5: one key per reason.
fn change_error(failure: &VaultRootChangeError, correlation: &CorrelationId) -> SafeErrorDto {
    match failure {
        VaultRootChangeError::AnotherChangeActive => error(
            "VAULT_CHANGE_ACTIVE",
            "desktop.vault_change_active",
            correlation,
            true,
        ),
        VaultRootChangeError::NotLiveWorkspace => error(
            "VAULT_NOT_LIVE",
            "desktop.vault_not_live",
            correlation,
            false,
        ),
        VaultRootChangeError::ProposedRootUnusable => error(
            "VALIDATION_INVALID_FIELD",
            "desktop.vault_folder_unusable",
            correlation,
            false,
        ),
        VaultRootChangeError::ProposedRootUnchanged => error(
            "VALIDATION_INVALID_FIELD",
            "desktop.vault_folder_unchanged",
            correlation,
            false,
        ),
        VaultRootChangeError::RecoveryBackupStale => error(
            "VAULT_RECOVERY_BACKUP_STALE",
            "desktop.vault_recovery_backup_stale",
            correlation,
            true,
        ),
        VaultRootChangeError::UnpinnedEvidence { count } => with_count(
            error(
                "VAULT_EVIDENCE_UNPINNED",
                "desktop.vault_evidence_unpinned",
                correlation,
                false,
            ),
            *count,
        ),
        VaultRootChangeError::UnresolvedEvidence { count } => with_count(
            error(
                "VAULT_EVIDENCE_UNRESOLVED",
                "desktop.vault_evidence_unresolved",
                correlation,
                false,
            ),
            *count,
        ),
        VaultRootChangeError::NotPrepared
        | VaultRootChangeError::PayloadMismatch
        | VaultRootChangeError::Expired
        | VaultRootChangeError::Stale => stale(correlation),
        VaultRootChangeError::ConfirmationMismatch => error(
            "VALIDATION_INVALID_FIELD",
            "desktop.vault_confirmation_mismatch",
            correlation,
            false,
        ),
        VaultRootChangeError::ChangedButNotRecorded => error(
            "VAULT_CHANGED_NOT_RECORDED",
            "desktop.vault_changed_but_not_recorded",
            correlation,
            false,
        ),
        VaultRootChangeError::NotRecorded => error(
            "VAULT_CHANGE_NOT_RECORDED",
            "desktop.vault_change_not_recorded",
            correlation,
            true,
        ),
        VaultRootChangeError::Settings => unavailable(),
        VaultRootChangeError::Ledger(_) | VaultRootChangeError::Control(_) => error(
            "VAULT_CHANGE_FAILED",
            "desktop.vault_change_failed",
            correlation,
            true,
        ),
    }
}

/// A six-character code from the host's random source, or `None` when it
/// cannot supply one.
fn mint_code() -> Option<String> {
    let random = new_archive_id()?;
    let bytes = (0..CODE_LENGTH)
        .map(|index| u8::from_str_radix(random.get(index * 2..index * 2 + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some(
        bytes
            .iter()
            .map(|byte| char::from(CODE_ALPHABET[usize::from(*byte) % CODE_ALPHABET.len()]))
            .collect(),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenVaultFolderDto {
    /// False when the person cancelled the picker.
    pub chosen: bool,
    /// Opaque; names the picked folder in the next step.
    pub token: Option<String>,
    /// The folder's own name only.
    pub folder_name: Option<String>,
}

/// H1-User: open the host's folder picker (DG3 Vault-root amendment §2). The
/// folder is checked now — a directory, reached through no link, and not one
/// PMC already uses — and held behind a token.
#[tauri::command]
pub async fn choose_vault_folder(
    window: tauri::Window,
    app: AppHandle,
    title: String,
) -> Result<ChosenVaultFolderDto, SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<VaultRootSession>();
    live_only(&session, &correlation)?;
    let title = title.chars().take(120).collect::<String>();
    // No lock is held while the dialog is open.
    let Some(picked) = pick_folder(window, title).await else {
        return Ok(ChosenVaultFolderDto {
            chosen: false,
            token: None,
            folder_name: None,
        });
    };
    let folder = CanonicalDirectoryPath::new(picked).map_err(|_| {
        error(
            "VALIDATION_INVALID_FIELD",
            "desktop.vault_folder_unusable",
            &correlation,
            false,
        )
    })?;
    // §2: never PMC's own settings and workspace folder, and never the
    // backup folder, in either direction — a Vault inside a backup folder
    // would put Evidence among archives, and one around it the reverse.
    if in_use(&app, &folder) {
        return Err(in_use_error(&correlation));
    }
    let token = new_archive_id().ok_or_else(|| {
        error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        )
    })?;
    let folder_name = DeferredDirectoryPath::from(folder.clone()).folder_name();
    session.lock().chosen = Some((token.clone(), folder));
    Ok(ChosenVaultFolderDto {
        chosen: true,
        token: Some(token),
        folder_name,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultRootPreviewDto {
    pub prepared_intent_id: String,
    /// What the approval acknowledges; any change invalidates it.
    pub payload_sha256: String,
    pub proposed_folder_name: String,
    pub previous_folder_name: Option<String>,
    pub evidence_count: u64,
    pub resolved_count: u64,
    pub recovery_verified_at_millis: i64,
    /// The code the person types after the fixed phrase.
    pub confirmation_code: String,
    pub expires_at_millis: i64,
}

/// H1-User: a fresh verified backup of this workspace — the change's
/// recovery evidence (§3.2) — then the exact preview. Nothing is changed.
#[tauri::command]
pub async fn prepare_vault_root_change(
    app: AppHandle,
    token: String,
) -> Result<VaultRootPreviewDto, SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<VaultRootSession>();
    live_only(&session, &correlation)?;
    let _busy = Busy::claim(&session, &correlation)?;
    let folder = match &session.lock().chosen {
        Some((current, folder)) if *current == token => folder.clone(),
        _ => return Err(stale(&correlation)),
    };
    // Again: the backup folder may have changed since the folder was chosen.
    if in_use(&app, &folder) {
        return Err(in_use_error(&correlation));
    }
    let (Some(intent_id), Some(code)) = (new_archive_id(), mint_code()) else {
        return Err(error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        ));
    };
    // The change begins here: a backup verified before this instant may
    // carry settings that are not the ones being replaced.
    let started_at = now().unix_millis();
    let recovery = run_backup(&app)
        .await
        .map_err(|key| run_error(key, &correlation))?;

    let worker = app.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let ledger = worker.state::<LedgerState>();
        let ledger = ledger.read().map_err(PrepareFailure::Closed)?;
        let settings = worker.state::<SettingsState>();
        let document = settings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|store| store.read().ok())
            .ok_or(PrepareFailure::Settings)?;
        worker
            .state::<VaultRootServiceState>()
            .0
            .prepare(
                &PrepareVaultRootChange {
                    proposed_root: &folder,
                    ledger: &ledger,
                    settings: &document,
                    recovery: &recovery.record,
                    recovery_not_before_millis: started_at,
                    recovery_settings_revision: recovery.settings_revision,
                    prepared_intent_id: &intent_id,
                    confirmation_code: &code,
                    valid_for_millis: PREPARED_FOR_MILLIS,
                },
                now(),
            )
            .map_err(PrepareFailure::Change)
    })
    .await
    .map_err(|_| {
        error(
            "VAULT_CHANGE_FAILED",
            "desktop.vault_change_failed",
            &correlation,
            true,
        )
    })?;
    let preview = prepared.map_err(|failure| match failure {
        PrepareFailure::Closed(closed) => closed.to_safe_error(&correlation),
        PrepareFailure::Settings => unavailable(),
        PrepareFailure::Change(failure) => change_error(&failure, &correlation),
    })?;
    Ok(VaultRootPreviewDto {
        prepared_intent_id: preview.prepared_intent_id,
        payload_sha256: preview.payload_sha256,
        proposed_folder_name: preview.proposed_folder_name,
        previous_folder_name: preview.previous_folder_name,
        evidence_count: preview.evidence_count,
        resolved_count: preview.resolved_count,
        recovery_verified_at_millis: preview.recovery_verified_at_millis,
        confirmation_code: preview.confirmation_code,
        expires_at_millis: preview.expires_at_millis,
    })
}

enum PrepareFailure {
    Closed(crate::ledger_state::LedgerClosed),
    Settings,
    Change(VaultRootChangeError),
}

/// H2b reject: the preview is discarded, nothing changes, the choice is
/// recorded.
#[tauri::command]
pub fn reject_vault_root_change(
    app: AppHandle,
    prepared_intent_id: String,
) -> Result<(), SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<VaultRootSession>();
    live_only(&session, &correlation)?;
    let result = app
        .state::<VaultRootServiceState>()
        .0
        .reject(&prepared_intent_id, now());
    session.lock().chosen = None;
    result.map_err(|failure| change_error(&failure, &correlation))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultRootResultDto {
    /// `changed` or `not_changed`.
    pub outcome: &'static str,
}

/// H2b approve (§3.4–§3.5): the typed code and the acknowledged preview are
/// checked, everything the preview bound is proved again, and only then does
/// the setting move — all while this call holds the backup gate's admission
/// and the Ledger's write lock.
#[tauri::command]
pub async fn approve_vault_root_change(
    app: AppHandle,
    prepared_intent_id: String,
    payload_sha256: String,
    typed_confirmation: String,
    client_request_id: String,
) -> Result<VaultRootResultDto, SafeErrorDto> {
    let correlation = host_correlation();
    live_only(&app.state::<VaultRootSession>(), &correlation)?;
    // The whole confirmation, checked here: a fixed phrase, then one word.
    // The service then checks that word is this preview's code.
    let code = typed_code(&typed_confirmation).ok_or_else(|| {
        error(
            "VALIDATION_INVALID_FIELD",
            "desktop.vault_confirmation_mismatch",
            &correlation,
            false,
        )
    })?;
    if client_request_id.is_empty() || client_request_id.len() > 128 {
        return Err(error(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            &correlation,
            false,
        ));
    }
    let receipt_id = new_archive_id().ok_or_else(|| {
        error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        )
    })?;
    let worker = app.clone();
    let task_correlation = correlation.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let gate = worker.state::<BackupGate>();
        let ledgers = worker.state::<LedgerState>();
        // Gate admission, then the Ledger: held until this closure returns,
        // across the re-check and the settings write.
        let ledger = ledgers
            .write(&gate, now())
            .map_err(|refusal| refusal.to_safe_error(&task_correlation))?;
        let settings = worker.state::<SettingsState>();
        let guard = settings.0.lock().unwrap_or_else(PoisonError::into_inner);
        let store = guard.as_ref().ok_or_else(unavailable)?;
        let service = &worker.state::<VaultRootServiceState>().0;
        // Again, under the locks: the backup folder may have been changed
        // since the preview, and it is read here from the settings this call
        // holds, so it cannot change before the write.
        let proposed = service
            .control()
            .ok()
            .and_then(|control| match control.active {
                Some(AuthorityOperation::Prepared { prepared })
                    if prepared.prepared_intent_id == prepared_intent_id =>
                {
                    CanonicalDirectoryPath::new(prepared.proposed_root).ok()
                }
                _ => None,
            });
        if let Some(proposed) = proposed {
            let destination = store
                .read()
                .ok()
                .and_then(|document| document.operational.backup_destination)
                .and_then(|destination| destination.revalidate().ok());
            if proposed.overlaps(protected_root().path())
                || destination.is_some_and(|destination| proposed.overlaps(destination.as_path()))
            {
                return Err(in_use_error(&task_correlation));
            }
        }
        service
            .approve(
                &ApproveVaultRootChange {
                    prepared_intent_id: &prepared_intent_id,
                    payload_sha256: &payload_sha256,
                    typed_code: &code,
                    idempotency_id: &client_request_id,
                    receipt_id: &receipt_id,
                    settings: store,
                    ledger: &ledger,
                },
                now(),
            )
            .map_err(|failure| change_error(&failure, &task_correlation))
    })
    .await
    .map_err(|_| {
        error(
            "VAULT_CHANGE_FAILED",
            "desktop.vault_change_failed",
            &correlation,
            true,
        )
    })??;
    app.state::<VaultRootSession>().lock().chosen = None;
    Ok(VaultRootResultDto {
        outcome: match outcome {
            VaultRootChangeOutcome::Changed => "changed",
            VaultRootChangeOutcome::NotChanged => "not_changed",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{mint_code, typed_code, CODE_ALPHABET, CODE_LENGTH};

    #[test]
    fn the_host_reads_the_code_only_after_one_of_the_fixed_phrases() {
        assert_eq!(typed_code("更換 Vault K7QX2M").as_deref(), Some("K7QX2M"));
        assert_eq!(
            typed_code("  change   vault  k7qx2m ").as_deref(),
            Some("K7QX2M")
        );
        assert_eq!(
            typed_code("CAMBIAR VAULT K7QX2M").as_deref(),
            Some("K7QX2M")
        );
        // The code alone, another phrase, or nothing at all is not a
        // confirmation.
        assert_eq!(typed_code("K7QX2M"), None);
        assert_eq!(typed_code("DELETE VAULT K7QX2M"), None);
        assert_eq!(typed_code(""), None);
    }

    #[test]
    fn a_code_has_six_characters_none_of_them_easy_to_misread() {
        let code = mint_code().unwrap_or_default();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)));
        assert!(!code.contains(['I', 'L', 'O', '0', '1']));
    }
}
