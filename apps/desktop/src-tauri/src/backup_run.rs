//! Running Operational Backups in the host (S7-A; ADR 0010, ADR 0012; DG3
//! backup-setup amendment §4).
//!
//! One run at a time. A run holds the backup gate exclusively only while it
//! marks itself running and takes the Ledger snapshot; encryption and the
//! full verification happen on a blocking worker with no lock held, while the
//! gate refuses writes as "backing up". A run counts only once the archive
//! verified end to end, was renamed into place, was audited and is in the
//! registry. Every file the run touches is resolved by
//! `pmc_application::backup_service`; the host holds no path (ADR 0011).
//!
//! At startup the work area is emptied, the registry's archives are checked
//! byte for byte, and — when a backup is due, the folder is reachable and the
//! passphrase is remembered — one backup runs without asking.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use pmc_application::backup_service::BackupService;
use pmc_application::desktop_runtime::SystemClock;
use pmc_application::operational_backup::BackupError;
use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;
use pmc_platform::backup_registry::{new_archive_id, BackupRecord};
use pmc_platform::host_audit::{AuditOutcome, AuditWorkspace, BACKUP_COMPLETED, BACKUP_FAILED};
use pmc_platform::settings::DeferredDirectoryPath;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::backup_gate::{BackupGate, GateState, RunTicket};
use crate::backup_secret::{BackupPassphraseState, PassphraseSource};
use crate::display_settings::SettingsState;
use crate::ledger_state::LedgerState;
use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

/// This workspace's backup files, managed as Tauri state.
pub struct BackupServiceState(pub BackupService);

/// The failure keys a run can end with (DG3 amendment §6), each with its
/// stable error code.
pub(crate) fn run_error(key: &'static str, correlation: &CorrelationId) -> SafeErrorDto {
    let (code, retryable) = match key {
        "desktop.backup_destination_not_set" => ("BACKUP_DESTINATION_NOT_SET", false),
        "desktop.backup_destination_unavailable" => ("BACKUP_DESTINATION_UNAVAILABLE", true),
        "desktop.backup_passphrase_required" => ("BACKUP_PASSPHRASE_REQUIRED", false),
        "desktop.backup_running" => ("BACKUP_RUNNING", true),
        "desktop.backup_verification_failed" => ("BACKUP_VERIFICATION_FAILED", true),
        "desktop.random_unavailable" => ("PLATFORM_INTERNAL", true),
        _ => ("BACKUP_FAILED", true),
    };
    SafeErrorDto::host(code, key, correlation, retryable)
}

fn now() -> UtcTimestamp {
    SystemClock.read()
}

pub(crate) fn stored_destination(settings: &SettingsState) -> Option<DeferredDirectoryPath> {
    let guard = settings
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.as_ref()?.read().ok()?.operational.backup_destination
}

pub(crate) fn settings_export(settings: &SettingsState) -> Option<Vec<u8>> {
    settings_export_at(settings).map(|(_, bytes)| bytes)
}

/// The settings a backup carries and the revision they were read at, from
/// one read of the document — so a caller can prove which settings the
/// backup holds (item ⑦: a Vault change's recovery evidence must hold the
/// settings it replaces, and the registry records no settings revision).
pub(crate) fn settings_export_at(settings: &SettingsState) -> Option<(u64, Vec<u8>)> {
    let guard = settings
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let document = guard.as_ref()?.read().ok()?;
    Some((document.revision, document.backup_export().ok()?))
}

/// A verified, registered backup and the settings revision its
/// `settings.json` was read at.
pub struct CompletedBackup {
    pub record: BackupRecord,
    pub settings_revision: u64,
}

fn fact(name: &str, value: impl ToString) -> (String, String) {
    (name.to_owned(), value.to_string())
}

/// One backup, start to finish: the record it verified and registered and
/// the settings revision it carries, or the failure key. Both are what a
/// Vault change binds as its recovery evidence (item ⑦).
pub async fn run_backup(app: &AppHandle) -> Result<CompletedBackup, &'static str> {
    // The sample workspace is never backed up (sample-workspace amendment
    // §6): refused before the gate or the Ledger is touched. The service
    // refuses it again on its own.
    if app.state::<BackupServiceState>().0.workspace() != AuditWorkspace::Live {
        return Err("desktop.sample_backup_refused");
    }
    // Nothing is backed up before the first-run choice: no workspace is
    // open (§2), even for a direct IPC call.
    if app
        .state::<crate::workspace_startup::WorkspaceStatus>()
        .open()
        .is_none()
    {
        return Err("desktop.workspace_not_chosen");
    }
    let settings = app.state::<SettingsState>();
    let destination = stored_destination(&settings)
        .ok_or("desktop.backup_destination_not_set")?
        .revalidate()
        .map_err(|_| "desktop.backup_destination_unavailable")?;
    let passphrase = app
        .state::<BackupPassphraseState>()
        .current()
        .ok_or("desktop.backup_passphrase_required")?;
    let (settings_revision, settings_json) =
        settings_export_at(&settings).ok_or("desktop.backup_failed")?;
    let archive_id = new_archive_id().ok_or("desktop.random_unavailable")?;

    let gate = app.state::<BackupGate>();
    let service = app.state::<BackupServiceState>();
    let (snapshot, ticket) = {
        let (ticket, exclusive) = gate.begin_run().map_err(|_| "desktop.backup_running")?;
        let snapshot = match app
            .state::<LedgerState>()
            .inner()
            .read_for_snapshot(&exclusive)
        {
            Ok(ledger) => service.0.snapshot(&ledger),
            // Closed for a restore, or unavailable: nothing to back up.
            Err(_) => Err(BackupError::Snapshot),
        };
        (snapshot, ticket)
    };
    let id_fact = fact("archive_id", &archive_id);
    let Ok(snapshot) = snapshot else {
        return Err(finish(
            app,
            ticket,
            "desktop.backup_failed",
            vec![fact("category", "snapshot"), id_fact],
        ));
    };
    let count_fact = fact("record_count", snapshot.inventory.record_count);

    let worker = app.clone();
    let published = tauri::async_runtime::spawn_blocking(move || {
        let service = worker.state::<BackupServiceState>();
        service.0.publish(
            &snapshot,
            &settings_json,
            &destination,
            &passphrase,
            &archive_id,
            &now,
            &|extracted, manifest| {
                worker
                    .state::<LedgerState>()
                    .read()
                    .is_ok_and(|ledger| ledger.verify_snapshot(extracted, manifest).is_ok())
            },
        )
    })
    .await;

    let failed = |category: &str| {
        vec![
            fact("category", category),
            id_fact.clone(),
            count_fact.clone(),
        ]
    };
    let record = match published {
        Ok(Ok(record)) => record,
        Ok(Err(BackupError::VerificationFailed)) => {
            return Err(finish(
                app,
                ticket,
                "desktop.backup_verification_failed",
                failed("verification"),
            ))
        }
        Ok(Err(BackupError::Residue(_))) => {
            return Err(finish(
                app,
                ticket,
                "desktop.backup_failed",
                failed("residue"),
            ))
        }
        Ok(Err(_)) | Err(_) => {
            return Err(finish(
                app,
                ticket,
                "desktop.backup_failed",
                failed("write"),
            ))
        }
    };

    // Verified and in place: audit it, register it, then count it. A backup
    // whose audit cannot be written is not reported as done (ADR 0012 §4);
    // its archive stays as an orphan, never trusted and never deleted.
    let audited = service.0.audit(
        BACKUP_COMPLETED,
        AuditOutcome::Succeeded,
        vec![
            fact("archive_id", &record.archive_id),
            fact("record_count", record.authority_record_count),
        ],
        now(),
    );
    if !audited {
        return Err(finish(
            app,
            ticket,
            "desktop.backup_failed",
            failed("audit"),
        ));
    }
    if service.0.register(&record).is_err() {
        return Err(finish(
            app,
            ticket,
            "desktop.backup_failed",
            failed("registry"),
        ));
    }
    ticket.finish(Ok(record.verified_at_millis));
    Ok(CompletedBackup {
        record,
        settings_revision,
    })
}

fn finish(
    app: &AppHandle,
    ticket: RunTicket<'_>,
    key: &'static str,
    facts: Vec<(String, String)>,
) -> &'static str {
    // The run is already failing; an unwritable audit changes nothing more.
    let _ = app.state::<BackupServiceState>().0.audit(
        BACKUP_FAILED,
        AuditOutcome::Failed,
        facts,
        now(),
    );
    ticket.finish(Err(key));
    key
}

/// Startup: check the recorded archives, open the gate accordingly, and back
/// up at once when one is due and can run unattended.
pub async fn start(app: AppHandle) {
    let destination = stored_destination(&app.state::<SettingsState>())
        .and_then(|folder| folder.revalidate().ok());
    // Hashing every recorded archive is blocking file I/O.
    let checker = app.clone();
    let folder = destination.clone();
    let Ok(found) = tauri::async_runtime::spawn_blocking(move || {
        checker
            .state::<BackupServiceState>()
            .0
            .reconcile(folder.as_ref())
    })
    .await
    else {
        // The check itself failed: the gate stays closed ("checking").
        return;
    };
    let gate = app.state::<BackupGate>();
    gate.reconciled(found.valid_verified_at, found.orphan_count);
    let due = gate.view(now()).state == GateState::Due;
    let remembered = app.state::<BackupPassphraseState>().source() == PassphraseSource::Remembered;
    // A Ledger that is not open (the upgrade gate, System Health) has
    // nothing to back up; the upgrade makes its own backup.
    let open = app.state::<LedgerState>().state().is_ok();
    if due && destination.is_some() && remembered && open {
        let _ = run_backup(&app).await;
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRunDto {
    pub run_id: u64,
    pub succeeded: bool,
    pub failure_key: Option<&'static str>,
}

/// Everything the Backups section and the policy strip show. No path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatusDto {
    /// `not_required` | `checking` | `backing_up` | `due` | `current`.
    pub state: &'static str,
    /// `not_set` | `unavailable` | `available`.
    pub destination: &'static str,
    /// The folder's own name only (accepted 2026-09-21).
    pub folder_name: Option<String>,
    /// `not_set` | `session` | `remembered`.
    pub passphrase: &'static str,
    pub last_verified_at_millis: Option<i64>,
    pub next_due_at_millis: Option<i64>,
    pub orphan_count: usize,
    pub last_run: Option<LastRunDto>,
}

fn status(app: &AppHandle) -> BackupStatusDto {
    let view = app.state::<BackupGate>().view(now());
    let stored = stored_destination(&app.state::<SettingsState>());
    let destination = match &stored {
        None => "not_set",
        Some(folder) if folder.revalidate().is_ok() => "available",
        Some(_) => "unavailable",
    };
    BackupStatusDto {
        state: view.state.as_str(),
        destination,
        folder_name: stored.as_ref().and_then(DeferredDirectoryPath::folder_name),
        passphrase: app.state::<BackupPassphraseState>().source().as_str(),
        last_verified_at_millis: view.last_verified_at_millis,
        next_due_at_millis: view.next_due_at_millis,
        orphan_count: view.orphan_count,
        last_run: view.last_run.map(|run| LastRunDto {
            run_id: run.run_id,
            succeeded: run.failure_key.is_none(),
            failure_key: run.failure_key,
        }),
    }
}

/// H0: the backup state for Settings → Backups and the policy strip.
#[tauri::command]
pub fn get_backup_status(app: AppHandle) -> Result<BackupStatusDto, SafeErrorDto> {
    Ok(status(&app))
}

/// H1-User: "Back up now". Resolves when the run has ended — verified and
/// registered, or failed with its reason.
#[tauri::command]
pub async fn run_backup_now(app: AppHandle) -> Result<BackupStatusDto, SafeErrorDto> {
    let correlation = host_correlation();
    run_backup(&app)
        .await
        .map_err(|key| run_error(key, &correlation))?;
    Ok(status(&app))
}
