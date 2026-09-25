//! Operational Restore in the host (S7-B1, H2b; DG3 restore amendment,
//! accepted 2026-09-22; ADR 0011, ADR 0012).
//!
//! The sheet's steps map to one command each: choose the file (the host's
//! picker; the webview gets an opaque token and the file's own name), check
//! it, prepare (the recovery backup and the exact preview), then approve and
//! execute, or reject. The picked file and the checked archive stay here;
//! no path, passphrase or record content goes back over IPC.
//!
//! Replacing the Ledger takes the backup gate's restore hold first — writes
//! are refused as "a restore is in progress" and no backup can start — then
//! the approval, then closes the Ledger under the gate's exclusive hold, so
//! no write is in flight when the file moves. The Ledger is reopened, or
//! marked unavailable for System Health, before the hold is released.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use pmc_application::desktop_runtime::SystemClock;
use pmc_application::restore_service::{
    CheckedArchive, ChosenArchive, ExecutedRestore, RecoveryBackupInput, RecoveryKind,
    ReopenedLedger, RestoreError,
};
use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;
use pmc_platform::backup_archive::Passphrase;
use pmc_platform::backup_registry::new_archive_id;
use pmc_platform::preservation::PreservationError;
use pmc_platform::restore_control::{RestoreControlError, RestoreOutcome, UnopenedReason};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::backup_gate::BackupGate;
use crate::backup_run::BackupServiceState;
use crate::backup_secret::BackupPassphraseState;
use crate::display_settings::SettingsState;
use crate::ledger_state::{LedgerClosed, LedgerState, UnavailableReason};
use crate::native_dialogs::pick_backup_file;
use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

/// How long a prepared preview may be approved (the H2b Prepared Intent).
const PREPARED_FOR_MILLIS: i64 = 15 * 60 * 1000;

#[derive(Default)]
struct Session {
    chosen: Option<(String, ChosenArchive)>,
    checked: Option<(String, CheckedArchive)>,
    /// A check or a preparation is running; a second is refused.
    busy: bool,
}

/// The file the person picked and what the check found, for this session.
pub struct RestoreSession {
    live: bool,
    inner: Mutex<Session>,
}

impl RestoreSession {
    pub fn new(live: bool) -> Self {
        Self {
            live,
            inner: Mutex::new(Session::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Session> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// This is the Live workspace.
    pub const fn live(&self) -> bool {
        self.live
    }

    fn forget(&self) {
        let mut inner = self.lock();
        inner.chosen = None;
        inner.checked = None;
    }
}

/// Marks the session busy; clears it however the step ends.
struct Busy<'a>(&'a RestoreSession);

impl<'a> Busy<'a> {
    fn claim(
        session: &'a RestoreSession,
        correlation: &CorrelationId,
    ) -> Result<Self, SafeErrorDto> {
        let mut inner = session.lock();
        if inner.busy {
            return Err(error(
                "RESTORE_RUNNING",
                "desktop.restore_running",
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

/// DG3 restore amendment §5: one key per reason.
fn restore_error(failure: &RestoreError, correlation: &CorrelationId) -> SafeErrorDto {
    let (code, key, retryable) = match failure {
        RestoreError::Unreadable => (
            "RESTORE_FILE_UNREADABLE",
            "desktop.restore_file_unreadable",
            true,
        ),
        RestoreError::WrongPassphrase => (
            "RESTORE_WRONG_PASSPHRASE",
            "desktop.restore_wrong_passphrase",
            false,
        ),
        RestoreError::Damaged => ("RESTORE_DAMAGED", "desktop.restore_damaged", false),
        RestoreError::NewerVersion => (
            "RESTORE_NEWER_VERSION",
            "desktop.restore_newer_version",
            false,
        ),
        RestoreError::UnsupportedOld => (
            "RESTORE_UNSUPPORTED_OLD",
            "desktop.restore_unsupported_old",
            false,
        ),
        RestoreError::RecoveryBackupFailed(_) => (
            "RESTORE_RECOVERY_BACKUP_FAILED",
            "desktop.restore_recovery_backup_failed",
            true,
        ),
        RestoreError::AnotherRestoreActive => ("RESTORE_RUNNING", "desktop.restore_running", true),
        // Only a Ledger waiting for its upgrade can be restored while closed
        // (restore-unopened amendment, first cut).
        RestoreError::CurrentNotInspectable => {
            ("LEDGER_UNAVAILABLE", "desktop.ledger_unavailable", false)
        }
        // Restore-unopened amendment §5, each with its own message.
        RestoreError::PreservationFailed(PreservationError::Locked) => (
            "RESTORE_LEDGER_LOCKED",
            "desktop.restore_ledger_locked",
            true,
        ),
        RestoreError::PreservationFailed(_) => (
            "RESTORE_PRESERVATION_FAILED",
            "desktop.restore_preservation_failed",
            true,
        ),
        RestoreError::Control(RestoreControlError::Unreadable) => (
            "RESTORE_STATE_UNREADABLE",
            "desktop.restore_state_unreadable",
            false,
        ),
        RestoreError::NotPrepared | RestoreError::DigestMismatch => (
            "RESTORE_PREVIEW_STALE",
            "desktop.restore_preview_stale",
            false,
        ),
        RestoreError::ConfirmationMismatch => (
            "VALIDATION_INVALID_FIELD",
            "desktop.restore_confirmation_mismatch",
            false,
        ),
        RestoreError::Control(_) | RestoreError::WorkArea(_) => (
            "RESTORE_FAILED_UNCHANGED",
            "desktop.restore_failed_unchanged",
            true,
        ),
    };
    error(code, key, correlation, retryable)
}

fn live_only(session: &RestoreSession, correlation: &CorrelationId) -> Result<(), SafeErrorDto> {
    if session.live {
        Ok(())
    } else {
        Err(error(
            "RESTORE_LIVE_ONLY",
            "desktop.restore_live_only",
            correlation,
            false,
        ))
    }
}

fn stale(correlation: &CorrelationId) -> SafeErrorDto {
    error(
        "RESTORE_PREVIEW_STALE",
        "desktop.restore_preview_stale",
        correlation,
        false,
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenBackupDto {
    /// False when the person cancelled the picker.
    pub chosen: bool,
    /// Opaque; names the picked file in the next step.
    pub token: Option<String>,
    /// The file's own name only.
    pub file_name: Option<String>,
}

/// H1-User: open the host's file picker (DG3 restore amendment §3.1).
/// `title` and `filter_name` are the dialog's labels in the person's
/// language.
#[tauri::command]
pub async fn choose_restore_archive(
    window: tauri::Window,
    app: AppHandle,
    title: String,
    filter_name: String,
) -> Result<ChosenBackupDto, SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<RestoreSession>();
    live_only(&session, &correlation)?;
    let title = title.chars().take(120).collect::<String>();
    let filter_name = filter_name.chars().take(60).collect::<String>();
    // No lock is held while the dialog is open.
    let Some(picked) = pick_backup_file(window, title, filter_name).await else {
        return Ok(ChosenBackupDto {
            chosen: false,
            token: None,
            file_name: None,
        });
    };
    let chosen = ChosenArchive::from_picked(picked).ok_or_else(|| {
        error(
            "RESTORE_FILE_UNREADABLE",
            "desktop.restore_file_unreadable",
            &correlation,
            true,
        )
    })?;
    let token = new_archive_id().ok_or_else(|| {
        error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        )
    })?;
    let file_name = chosen.file_name();
    let mut inner = session.lock();
    inner.chosen = Some((token.clone(), chosen));
    inner.checked = None;
    Ok(ChosenBackupDto {
        chosen: true,
        token: Some(token),
        file_name: Some(file_name),
    })
}

/// H1-User (restore-unopened amendment §2): the recovery backup a failed
/// restore left, offered first on System Health — no picker; the host finds
/// it in the backup registry, and it must still be listed and unaltered.
#[tauri::command]
pub fn choose_recovery_archive(app: AppHandle) -> Result<ChosenBackupDto, SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<RestoreSession>();
    live_only(&session, &correlation)?;
    let chosen = app
        .state::<BackupServiceState>()
        .0
        .recovery_archive()
        .ok_or_else(|| {
            error(
                "RESTORE_RECOVERY_BACKUP_MISSING",
                "desktop.restore_recovery_backup_missing",
                &correlation,
                false,
            )
        })?;
    let token = new_archive_id().ok_or_else(|| {
        error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        )
    })?;
    let file_name = chosen.file_name();
    let mut inner = session.lock();
    inner.chosen = Some((token.clone(), chosen));
    inner.checked = None;
    Ok(ChosenBackupDto {
        chosen: true,
        token: Some(token),
        file_name: Some(file_name),
    })
}

/// H0: forget the picked file and any check of it (the sheet closed or went
/// back). A check still running finishes and is discarded.
#[tauri::command]
pub fn discard_restore_selection(app: AppHandle) -> Result<(), SafeErrorDto> {
    app.state::<RestoreSession>().forget();
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedBackupDto {
    /// RFC 3339, UTC.
    pub created_at: String,
    pub schema_version: u32,
    pub record_count: u64,
    /// Older than this PMC: after the restore, the upgrade screen.
    pub needs_upgrade: bool,
}

/// H0 (nothing is replaced): decrypt the picked file and check every part
/// of it (§3.3). The passphrase opens this file only and is never stored.
#[tauri::command]
pub async fn check_restore_archive(
    app: AppHandle,
    token: String,
    passphrase: String,
) -> Result<CheckedBackupDto, SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<RestoreSession>();
    live_only(&session, &correlation)?;
    let _busy = Busy::claim(&session, &correlation)?;
    let chosen = match &session.lock().chosen {
        Some((current, chosen)) if *current == token => chosen.clone(),
        _ => return Err(stale(&correlation)),
    };
    let worker = app.clone();
    let checked = tauri::async_runtime::spawn_blocking(move || {
        worker
            .state::<BackupServiceState>()
            .0
            .check_chosen_archive(&chosen, &Passphrase::new(passphrase))
    })
    .await
    .map_err(|_| {
        error(
            "RESTORE_FAILED_UNCHANGED",
            "desktop.restore_failed_unchanged",
            &correlation,
            true,
        )
    })?
    .map_err(|failure| restore_error(&failure, &correlation))?;
    let dto = CheckedBackupDto {
        created_at: checked.created_at.clone(),
        schema_version: checked.schema_version,
        record_count: checked.record_count,
        needs_upgrade: checked.needs_upgrade,
    };
    let mut inner = session.lock();
    // Discarded when the person went back or picked another file meanwhile.
    if !matches!(&inner.chosen, Some((current, _)) if *current == token) {
        return Err(stale(&correlation));
    }
    inner.checked = Some((token, checked));
    Ok(dto)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreviewDto {
    pub prepared_intent_id: String,
    /// What the approval acknowledges; any change invalidates it.
    pub payload_sha256: String,
    pub archive_created_at: String,
    pub archive_schema_version: u32,
    pub archive_record_count: u64,
    /// `None` when the current Ledger cannot be inspected.
    pub current_record_count: Option<u64>,
    pub current_last_change_at_millis: Option<i64>,
    /// `operational_backup` | `preservation_copy` (restore-unopened §1).
    pub recovery_kind: &'static str,
    /// The recovery evidence's file name, which the sheet names it by.
    pub recovery_name: String,
    pub recovery_verified_at_millis: i64,
    /// The date to type, `YYYY-MM-DD`.
    pub confirmation_date: String,
    pub needs_upgrade: bool,
    pub expires_at_millis: i64,
}

/// H1-User: back up the current workspace, then build the exact preview
/// (§3.4). The recovery backup uses the backup folder and passphrase set in
/// Backups, like any other backup, and counts as one.
#[tauri::command]
pub async fn prepare_restore_from_archive(
    app: AppHandle,
    token: String,
) -> Result<RestorePreviewDto, SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<RestoreSession>();
    live_only(&session, &correlation)?;
    let _busy = Busy::claim(&session, &correlation)?;
    let checked = match &session.lock().checked {
        Some((current, checked)) if *current == token => checked.clone(),
        _ => return Err(stale(&correlation)),
    };
    let unset = |key: &'static str, code: &'static str| error(code, key, &correlation, false);
    let settings_state = app.state::<SettingsState>();
    let destination = {
        let guard = settings_state
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard
            .as_ref()
            .and_then(|store| store.read().ok())
            .and_then(|document| document.operational.backup_destination)
    }
    .ok_or_else(|| {
        unset(
            "desktop.backup_destination_not_set",
            "BACKUP_DESTINATION_NOT_SET",
        )
    })?
    .revalidate()
    .map_err(|_| {
        error(
            "BACKUP_DESTINATION_UNAVAILABLE",
            "desktop.backup_destination_unavailable",
            &correlation,
            true,
        )
    })?;
    let passphrase = app
        .state::<BackupPassphraseState>()
        .current()
        .ok_or_else(|| {
            unset(
                "desktop.backup_passphrase_required",
                "BACKUP_PASSPHRASE_REQUIRED",
            )
        })?;
    let archive_id = new_archive_id();
    let intent_id = new_archive_id();
    let (Some(archive_id), Some(intent_id)) = (archive_id, intent_id) else {
        return Err(error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        ));
    };

    let worker = app.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let gate = worker.state::<BackupGate>();
        let ledgers = worker.state::<LedgerState>();
        let service = worker.state::<BackupServiceState>();
        let settings = worker.state::<SettingsState>();
        // The recovery backup is a backup run: writes are refused as
        // "backing up" and the Ledger is held until the preview is bound.
        let Ok((ticket, exclusive)) = gate.begin_run() else {
            return Err(Prepared::Running);
        };
        let document = settings
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|store| store.read().ok())
            .ok_or(Prepared::Settings)?;
        let recovery = RecoveryBackupInput {
            destination: &destination,
            passphrase: &passphrase,
            archive_id: &archive_id,
        };
        // Which path is the Ledger's own state's to say, never the webview's
        // (restore-unopened amendment §4): open, or waiting for its upgrade.
        let outcome = match ledgers.read_for_snapshot(&exclusive) {
            Ok(ledger) => service.0.prepare_restore(
                &checked,
                &ledger,
                &document,
                &recovery,
                &intent_id,
                &now,
                PREPARED_FOR_MILLIS,
            ),
            // Closed and waiting for its upgrade: nothing can write to it,
            // and the backup run holds off every other backup and restore.
            Err(LedgerClosed::Unavailable(UnavailableReason::UpgradeRequired)) => {
                service.0.prepare_restore_closed(
                    &checked,
                    &document,
                    &recovery,
                    &intent_id,
                    &now,
                    PREPARED_FOR_MILLIS,
                )
            }
            // Not inspectable: a preservation copy of its exact files
            // (restore-unopened amendment §1, cut B).
            Err(LedgerClosed::Unavailable(reason)) => match unopened_reason(reason) {
                Some(unopened) => service.0.prepare_restore_preserved(
                    &checked,
                    &document,
                    unopened,
                    &recovery,
                    &intent_id,
                    &now,
                    PREPARED_FOR_MILLIS,
                ),
                None => return Err(Prepared::Closed(LedgerClosed::Unavailable(reason))),
            },
            Err(closed) => return Err(Prepared::Closed(closed)),
        };
        drop(exclusive);
        match outcome {
            Ok(preview) => {
                ticket.finish(Ok(preview.recovery_verified_at_millis));
                Ok(preview)
            }
            Err(failure) => {
                ticket.finish(Err(match failure {
                    RestoreError::RecoveryBackupFailed(_) => "desktop.backup_failed",
                    _ => "desktop.restore_failed_unchanged",
                }));
                Err(Prepared::Failed(failure))
            }
        }
    })
    .await
    .map_err(|_| {
        error(
            "RESTORE_FAILED_UNCHANGED",
            "desktop.restore_failed_unchanged",
            &correlation,
            true,
        )
    })?;
    let preview = prepared.map_err(|failure| match failure {
        Prepared::Running => error(
            "BACKUP_RUNNING",
            "desktop.backup_running",
            &correlation,
            true,
        ),
        Prepared::Closed(closed) => closed.to_safe_error(&correlation),
        Prepared::Settings => crate::display_settings::unavailable(),

        Prepared::Failed(failure) => restore_error(&failure, &correlation),
    })?;
    Ok(RestorePreviewDto {
        prepared_intent_id: preview.prepared_intent_id,
        payload_sha256: preview.payload_sha256,
        archive_created_at: preview.archive_created_at,
        archive_schema_version: preview.archive_schema_version,
        archive_record_count: preview.archive_record_count,
        current_record_count: preview.current_record_count,
        current_last_change_at_millis: preview.current_last_change_at_millis,
        recovery_kind: match preview.recovery_kind {
            RecoveryKind::OperationalBackup => "operational_backup",
            RecoveryKind::PreservationCopy => "preservation_copy",
        },
        recovery_name: preview.recovery_name,
        recovery_verified_at_millis: preview.recovery_verified_at_millis,
        confirmation_date: preview.confirmation_date,
        needs_upgrade: preview.needs_upgrade,
        expires_at_millis: preview.expires_at_millis,
    })
}

enum Prepared {
    Running,
    Closed(LedgerClosed),
    Settings,
    Failed(RestoreError),
}

/// H2b reject: discard the preview; nothing changes and the choice is
/// recorded.
#[tauri::command]
pub fn reject_prepared_restore(
    app: AppHandle,
    prepared_intent_id: String,
) -> Result<(), SafeErrorDto> {
    let correlation = host_correlation();
    let session = app.state::<RestoreSession>();
    live_only(&session, &correlation)?;
    let result = app
        .state::<BackupServiceState>()
        .0
        .reject_restore(&prepared_intent_id, now());
    session.forget();
    result.map_err(|failure| restore_error(&failure, &correlation))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResultDto {
    /// `restored` | `failed_before_replacement` | `recovery_put_back` |
    /// `recovery_failed` | `interrupted` (approved, then stopped without an
    /// outcome; the next start puts it back).
    pub outcome: &'static str,
    /// Nothing was replaced because the current Ledger's files changed after
    /// the preview (restore-unopened amendment §5).
    pub source_changed: bool,
    /// `ready`, or why the Ledger is not open (the upgrade screen or System
    /// Health follows).
    pub ledger: &'static str,
}

const fn outcome_name(outcome: RestoreOutcome) -> &'static str {
    match outcome {
        RestoreOutcome::Restored => "restored",
        RestoreOutcome::FailedBeforeReplacement => "failed_before_replacement",
        RestoreOutcome::RecoveryPutBack => "recovery_put_back",
        RestoreOutcome::RecoveryFailed => "recovery_failed",
    }
}

fn ledger_name(ledgers: &LedgerState) -> &'static str {
    ledgers
        .state()
        .map_or_else(LedgerClosed::as_str, |()| "ready")
}

/// Which unopened state a preservation restore may replace (restore-unopened
/// amendment §2): never a newer Ledger (no downgrade).
const fn unopened_reason(reason: UnavailableReason) -> Option<UnopenedReason> {
    match reason {
        UnavailableReason::OpenFailed => Some(UnopenedReason::OpenFailed),
        UnavailableReason::RestoreRecoveryRequired => Some(UnopenedReason::RestoreRecoveryRequired),
        UnavailableReason::UnsupportedOld => Some(UnopenedReason::UnsupportedOld),
        // Nothing to restore over before the first-run choice.
        UnavailableReason::UpgradeRequired
        | UnavailableReason::NewerVersion
        | UnavailableReason::FirstRun => None,
    }
}

/// Why the Ledger is not open after a restore that left a file PMC cannot
/// open: when nothing was replaced, or the files were put back exactly, it is
/// the Ledger it was, so the state it was in; otherwise a generic failure.
/// (A failed recovery is decided before this, by the restore record.)
const fn unopened_after(
    outcome: Option<RestoreOutcome>,
    was: Option<UnavailableReason>,
) -> UnavailableReason {
    match (outcome, was) {
        (
            Some(RestoreOutcome::FailedBeforeReplacement | RestoreOutcome::RecoveryPutBack),
            Some(
                reason @ (UnavailableReason::UpgradeRequired
                | UnavailableReason::UnsupportedOld
                | UnavailableReason::OpenFailed),
            ),
        ) => reason,
        _ => UnavailableReason::OpenFailed,
    }
}

/// Open the Ledger again after a restore ended, or say why it cannot be.
/// `was` is why the Ledger was not open before, when it was not: files put
/// back exactly (or never moved) are that Ledger again, so it is that state
/// again, not a generic failure.
fn reopen(
    ledgers: &LedgerState,
    service: &BackupServiceState,
    outcome: Option<RestoreOutcome>,
    was: Option<UnavailableReason>,
) {
    if outcome == Some(RestoreOutcome::RecoveryFailed) || service.0.restore_needs_recovery() {
        ledgers.set_unavailable(UnavailableReason::RestoreRecoveryRequired);
        return;
    }
    match service.0.reopen_ledger() {
        ReopenedLedger::Ready(ledger) => ledgers.install(ledger),
        ReopenedLedger::UpgradeRequired => {
            ledgers.set_unavailable(UnavailableReason::UpgradeRequired);
        }
        ReopenedLedger::Failed => ledgers.set_unavailable(unopened_after(outcome, was)),
    }
}

/// H2b approve and execute (§3.5–3.7): the typed date and the acknowledged
/// preview are checked, the approval is recorded, then the Ledger is closed,
/// replaced, reopened and checked. No cancel from the approval on.
#[tauri::command]
pub async fn approve_and_execute_restore(
    app: AppHandle,
    prepared_intent_id: String,
    payload_sha256: String,
    typed_date: String,
    client_request_id: String,
) -> Result<RestoreResultDto, SafeErrorDto> {
    let correlation = host_correlation();
    live_only(&app.state::<RestoreSession>(), &correlation)?;
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
    let executed = tauri::async_runtime::spawn_blocking(move || {
        let gate = worker.state::<BackupGate>();
        let ledgers = worker.state::<LedgerState>();
        let service = worker.state::<BackupServiceState>();
        let settings = worker.state::<SettingsState>();
        // Before the approval: once it is recorded the restore must run, so
        // everything that could refuse it is settled first.
        let (ticket, exclusive) = gate.begin_restore().map_err(Executed::Gate)?;
        // Open, or closed waiting for its upgrade (the gate's restore); any
        // other closed state is refused before the approval is recorded.
        let closed_reason = match ledgers.state() {
            Ok(()) => None,
            Err(LedgerClosed::Unavailable(reason))
                if reason == UnavailableReason::UpgradeRequired
                    || unopened_reason(reason).is_some() =>
            {
                Some(reason)
            }
            Err(closed) => return Err(Executed::Closed(closed)),
        };
        let approved = service
            .0
            .approve_restore(
                &prepared_intent_id,
                &payload_sha256,
                &typed_date,
                &client_request_id,
                &receipt_id,
                now(),
            )
            .map_err(Executed::Failed)?;
        if let Some(outcome) = approved {
            // A repeated request: the restore already ran.
            return Ok(Some(ExecutedRestore::from(outcome)));
        }
        // No write is in flight: the exclusive hold waited for them. A Ledger
        // waiting for its upgrade was never open; it is being replaced now.
        if let Some(reason) = closed_reason {
            let _ = ledgers.begin_replacing(reason, &exclusive);
        } else {
            let _ = ledgers.close_for_restore(&exclusive);
        }
        drop(exclusive);
        let guard = settings.0.lock().unwrap_or_else(PoisonError::into_inner);
        let outcome = match guard.as_ref() {
            Some(store) => service.0.execute_restore_explained(store, now()),
            None => Err(RestoreError::WorkArea(std::io::Error::from(
                std::io::ErrorKind::NotFound,
            ))),
        };
        drop(guard);
        reopen(
            &ledgers,
            &service,
            outcome.as_ref().ok().map(|executed| executed.outcome),
            closed_reason,
        );
        drop(ticket);
        // Approved, then stopped without an outcome: the next start puts it
        // back (never forward). Not an error to retry: the approval is used.
        Ok(outcome.ok())
    })
    .await
    .map_err(|_| {
        error(
            "RESTORE_FAILED_UNCHANGED",
            "desktop.restore_failed_unchanged",
            &correlation,
            true,
        )
    })?;
    app.state::<RestoreSession>().forget();
    let outcome = executed.map_err(|failure| match failure {
        Executed::Gate(refusal) => refusal.to_safe_error(&correlation),
        Executed::Closed(closed) => closed.to_safe_error(&correlation),
        Executed::Failed(failure) => restore_error(&failure, &correlation),
    })?;
    Ok(RestoreResultDto {
        outcome: outcome.map_or("interrupted", |executed| outcome_name(executed.outcome)),
        source_changed: outcome.is_some_and(|executed| executed.source_changed),
        ledger: ledger_name(&app.state::<LedgerState>()),
    })
}

enum Executed {
    Gate(crate::backup_gate::GateRefusal),
    Closed(LedgerClosed),
    Failed(RestoreError),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemHealthDto {
    /// `ready` | `replacing` | `restore_recovery_required` |
    /// `upgrade_required` | `open_failed`.
    pub ledger: &'static str,
    /// The recovery backup's file name, when a restore could not be put back.
    pub recovery_backup: Option<String>,
    /// `ok` | `set_aside` (could not be read; set aside and a fresh one
    /// written) | `unavailable` (could not be opened at all) — the settings
    /// document as startup found it (sample-workspace amendment §2).
    pub settings: &'static str,
    /// Why chosen sample data did not open, or why the sample cannot be
    /// used: `unresolved` | `missing` | `foreign`; `None` when neither.
    pub sample: Option<&'static str>,
    /// Folders a finished sample operation could not remove yet (§8).
    pub sample_cleanup_pending: usize,
}

/// H0: whether the Ledger is open, and if not, why (S11 System Health);
/// and what startup found about the settings and the sample.
#[tauri::command]
pub fn get_system_health(app: AppHandle) -> Result<SystemHealthDto, SafeErrorDto> {
    let ledger = ledger_name(&app.state::<LedgerState>());
    let recovery_backup = if ledger == "restore_recovery_required" {
        app.state::<BackupServiceState>()
            .0
            .recovery_archive_needed()
    } else {
        None
    };
    let status = crate::workspace_startup::status_dto(
        &app.state::<crate::workspace_startup::WorkspaceStatus>(),
    );
    Ok(SystemHealthDto {
        ledger,
        recovery_backup,
        settings: if status.settings_unavailable {
            "unavailable"
        } else if status.settings_set_aside {
            "set_aside"
        } else {
            "ok"
        },
        sample: (status.sample_fell_back || status.sample == "unresolved")
            .then_some(status.sample)
            .filter(|sample| *sample != "available"),
        // Read now, not as startup found it: a removal may have finished.
        sample_cleanup_pending: app
            .state::<crate::sample_workspace::SampleWorkspaceState>()
            .cleanup_pending(),
    })
}

/// H0: close PMC — System Health's "Quit", for when the Ledger cannot be
/// used.
#[tauri::command]
pub fn quit_pmc(app: AppHandle) -> Result<(), SafeErrorDto> {
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preservation_restore_is_offered_for_every_unopened_state_but_newer() {
        assert_eq!(
            unopened_reason(UnavailableReason::OpenFailed),
            Some(UnopenedReason::OpenFailed)
        );
        assert_eq!(
            unopened_reason(UnavailableReason::RestoreRecoveryRequired),
            Some(UnopenedReason::RestoreRecoveryRequired)
        );
        assert_eq!(
            unopened_reason(UnavailableReason::UnsupportedOld),
            Some(UnopenedReason::UnsupportedOld)
        );
        // An inspectable Ledger takes the backup path; a newer one, none.
        assert_eq!(unopened_reason(UnavailableReason::UpgradeRequired), None);
        assert_eq!(unopened_reason(UnavailableReason::NewerVersion), None);
    }

    #[test]
    fn a_ledger_nothing_replaced_or_put_back_is_the_ledger_it_was() {
        for was in [
            UnavailableReason::UpgradeRequired,
            UnavailableReason::UnsupportedOld,
            UnavailableReason::OpenFailed,
        ] {
            for outcome in [
                RestoreOutcome::FailedBeforeReplacement,
                RestoreOutcome::RecoveryPutBack,
            ] {
                assert_eq!(unopened_after(Some(outcome), Some(was)), was, "{outcome:?}");
            }
            // Replaced, or no outcome: what is there now is not that Ledger.
            assert_eq!(
                unopened_after(Some(RestoreOutcome::Restored), Some(was)),
                UnavailableReason::OpenFailed
            );
            assert_eq!(
                unopened_after(None, Some(was)),
                UnavailableReason::OpenFailed
            );
        }
        // An open Ledger that did not reopen is a failure to open.
        assert_eq!(
            unopened_after(Some(RestoreOutcome::RecoveryPutBack), None),
            UnavailableReason::OpenFailed
        );
    }
}
