//! The host's one door to the sample workspace service (item ⑨; the
//! accepted sample-workspace amendment). The policy check lets only this
//! module name it.
//!
//! The workspace open this run is derived here from what startup decided —
//! never taken from the webview — and every change of workspace takes effect
//! by restarting, only after the settings write has committed (§3, §4).

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::PoisonError;

use pmc_application::sample_lifecycle::{
    request_token, ApproveSampleDelete, SampleDeletePreview, SampleError, SampleWorkspace,
};
use pmc_application::sample_workspace::SeedError;
use pmc_domain::time::UtcTimestamp;
use pmc_platform::backup_registry::new_archive_id;
use pmc_platform::instance_lock::InstanceLock;
use pmc_platform::sample_control::SampleOutcome;
use pmc_platform::settings::{PatchOutcome, ProtectedSettingsRoot, SelectedWorkspace};
use pmc_platform::workspace::WorkspaceKind;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::display_settings::SettingsState;
use crate::runtime::{host_correlation, HostRuntime};
use crate::safe_error::SafeErrorDto;
use crate::workspace_startup::{SampleAvailability, Startup, WorkspaceStatus};

/// How long a delete preview may be approved (§8).
const DELETE_VALID_FOR_MILLIS: i64 = 15 * 60 * 1000;

/// The phrase a person types to delete the sample data (§8), in each UI
/// language; `sampleWorkspace.delete.phrase` in the renderer's catalogs
/// holds the same six (a renderer test keeps them equal).
pub const DELETE_PHRASES: [&str; 6] = [
    "DELETE SAMPLE DATA",
    "刪除範例資料",
    "删除示例数据",
    "サンプルデータを削除",
    "샘플 데이터 삭제",
    "BORRAR DATOS DE EJEMPLO",
];

/// The profile's sample, held for the run; `None` when it could not even
/// be resolved (then nothing about it is offered).
pub struct SampleWorkspaceState(pub Option<SampleWorkspace>);

impl SampleWorkspaceState {
    /// Folders a finished sample operation could not remove yet, as the
    /// operation record says now (System Health, §8).
    #[must_use]
    pub fn cleanup_pending(&self) -> usize {
        self.0
            .as_ref()
            .map_or(0, |sample| sample.pending_cleanup().len())
    }
}

/// What startup found about the sample, before any workspace is chosen.
pub struct SampleAtStartup {
    pub state: SampleWorkspaceState,
    pub availability: SampleAvailability,
    /// Folders a finished operation could not remove yet.
    pub cleanup_pending: usize,
}

/// Finish or roll back an interrupted reset or delete (§8), then read what
/// the sample folder holds. Runs under the profile's instance lock, which
/// the host holds for the whole run, and before anything opens a workspace.
/// A reconciliation that cannot finish leaves its operation recorded and
/// the sample unavailable for this run.
pub fn reconcile_at_startup(root: &ProtectedSettingsRoot, now: UtcTimestamp) -> SampleAtStartup {
    let sample = SampleWorkspace::new(root).ok();
    let reconciled = sample
        .as_ref()
        .is_some_and(|sample| sample.reconcile(now).is_ok());
    let state = sample.as_ref().and_then(|sample| sample.inspect().ok());
    let cleanup_pending = sample
        .as_ref()
        .map_or(0, |sample| sample.pending_cleanup().len());
    SampleAtStartup {
        state: SampleWorkspaceState(sample),
        availability: SampleAvailability::from_state(reconciled, state.as_ref()),
        cleanup_pending,
    }
}

fn error(code: &'static str, key: &'static str, retryable: bool) -> SafeErrorDto {
    SafeErrorDto::host(code, key, &host_correlation(), retryable)
}

/// What a command was doing when the service failed: the same failure means
/// different things to the person in each (§9, one key per reason).
#[derive(Clone, Copy)]
enum Doing {
    Preparing,
    Resetting,
    Deleting,
    Other,
}

/// One key per reason (§9).
fn sample_error(failure: &SampleError, doing: Doing) -> SafeErrorDto {
    match failure {
        SampleError::Seed(SeedError::ForeignContents(_)) => {
            error("SAMPLE_FOREIGN", "desktop.sample_foreign", false)
        }
        SampleError::SampleIsOpen => error("SAMPLE_IS_OPEN", "desktop.sample_is_open", false),
        SampleError::OperationInProgress => error(
            "SAMPLE_OPERATION_IN_PROGRESS",
            "desktop.sample_operation_in_progress",
            true,
        ),
        SampleError::InvalidIdentifier => error(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            false,
        ),
        SampleError::NothingToDelete => error(
            "SAMPLE_NOTHING_TO_DELETE",
            "desktop.sample_nothing_to_delete",
            false,
        ),
        SampleError::NotPrepared => error(
            "SAMPLE_DELETE_NOT_PREPARED",
            "desktop.sample_delete_not_prepared",
            false,
        ),
        SampleError::Expired => error(
            "SAMPLE_DELETE_EXPIRED",
            "desktop.sample_delete_expired",
            false,
        ),
        SampleError::PayloadMismatch | SampleError::Changed => error(
            "SAMPLE_DELETE_CHANGED",
            "desktop.sample_delete_changed",
            false,
        ),
        SampleError::NotRecorded => {
            error("SAMPLE_NOT_RECORDED", "desktop.sample_not_recorded", true)
        }
        SampleError::Seed(_) | SampleError::Control(_) => match doing {
            Doing::Resetting => error("SAMPLE_RESET_FAILED", "desktop.sample_reset_failed", true),
            Doing::Deleting => error("SAMPLE_DELETE_FAILED", "desktop.sample_delete_failed", true),
            Doing::Preparing => error("SAMPLE_FAILED", "desktop.sample_failed", true),
            Doing::Other => error("SAMPLE_UNAVAILABLE", "desktop.sample_unavailable", true),
        },
    }
}

fn open_workspace(app: &AppHandle) -> Option<WorkspaceKind> {
    app.state::<WorkspaceStatus>().open()
}

/// Operations and approvals are named by the host, never by the webview:
/// a domain-separated digest of the request id, so a retry of the same
/// request is the same operation and no two kinds can collide (a plain
/// token, which the service also requires).
fn host_id(domain: &str, client_request_id: &str) -> Result<String, SafeErrorDto> {
    if client_request_id.is_empty() || client_request_id.len() > 128 {
        return Err(error(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            false,
        ));
    }
    Ok(request_token(domain, client_request_id))
}

/// A fresh random token for an intent id.
fn new_token() -> Result<String, SafeErrorDto> {
    new_archive_id().ok_or_else(|| error("PLATFORM_INTERNAL", "desktop.random_unavailable", true))
}

/// The settings revision as it is now, under the settings lock.
fn settings_revision(app: &AppHandle) -> Result<u64, SafeErrorDto> {
    let settings = app.state::<SettingsState>();
    let guard = settings.0.lock().unwrap_or_else(PoisonError::into_inner);
    let store = guard
        .as_ref()
        .ok_or_else(crate::display_settings::unavailable)?;
    Ok(store
        .read()
        .map_err(|_| crate::display_settings::unavailable())?
        .revision)
}

/// Run a command's file work on a worker thread, never on the async
/// runtime's own threads.
async fn blocking<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&AppHandle) -> Result<T, SafeErrorDto> + Send + 'static,
) -> Result<T, SafeErrorDto> {
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || work(&worker))
        .await
        .map_err(|_| error("SAMPLE_FAILED", "desktop.sample_failed", true))?
}

fn with_service<T>(
    app: &AppHandle,
    work: impl FnOnce(&SampleWorkspace) -> Result<T, SafeErrorDto>,
) -> Result<T, SafeErrorDto> {
    let state = app.state::<SampleWorkspaceState>();
    let Some(service) = state.0.as_ref() else {
        return Err(error("SAMPLE_FAILED", "desktop.sample_failed", false));
    };
    work(service)
}

/// A workspace change was saved and PMC is restarting: nothing else may
/// choose or switch in the moments before it exits.
#[derive(Default)]
pub struct RestartPending(AtomicBool);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeDto {
    /// `restarting`: the choice is saved and PMC is restarting into it.
    pub outcome: &'static str,
}

/// Save which workspace opens next, then restart into it — only once the
/// write has committed (§3, §4). A failed write restarts nothing. With
/// `first_run`, a choice already recorded (by an earlier call of this run)
/// is never overwritten.
fn save_and_restart(
    app: &AppHandle,
    target: SelectedWorkspace,
    first_run: bool,
) -> Result<WorkspaceChangeDto, SafeErrorDto> {
    let not_saved = || {
        error(
            "WORKSPACE_CHOICE_NOT_SAVED",
            "desktop.workspace_choice_not_saved",
            true,
        )
    };
    let pending = app.state::<RestartPending>();
    if pending.0.load(Ordering::SeqCst) {
        return Err(error(
            "WORKSPACE_NOT_FIRST_RUN",
            "desktop.workspace_not_first_run",
            false,
        ));
    }
    {
        let settings = app.state::<SettingsState>();
        let guard = settings.0.lock().unwrap_or_else(PoisonError::into_inner);
        let store = guard.as_ref().ok_or_else(not_saved)?;
        let document = store.read().map_err(|_| not_saved())?;
        if first_run && document.operational.selected_workspace.is_some() {
            return Err(error(
                "WORKSPACE_NOT_FIRST_RUN",
                "desktop.workspace_not_first_run",
                false,
            ));
        }
        match store.select_workspace(document.revision, target) {
            Ok(PatchOutcome::Committed { .. }) => {}
            _ => return Err(not_saved()),
        }
        pending.0.store(true, Ordering::SeqCst);
    }
    // Tauri starts the new process before this one exits. This process keeps
    // the profile's hold until it exits; the marker tells the new one to wait
    // for it rather than report another PMC running.
    let now = app.state::<HostRuntime>().now().unix_millis();
    let _ = InstanceLock::mark_restart(&crate::ledger_state::protected_root(), now);
    app.request_restart();
    Ok(WorkspaceChangeDto {
        outcome: "restarting",
    })
}

/// Prepare the sample (§7): building it runs the whole scenario.
fn prepare_sample_data(app: &AppHandle, open: Option<WorkspaceKind>) -> Result<(), SafeErrorDto> {
    let operation_id = new_token()?;
    let now = app.state::<HostRuntime>().now();
    with_service(app, |service| {
        service
            .prepare(open, &operation_id, now)
            .map(|_| ())
            .map_err(|failure| sample_error(&failure, Doing::Preparing))
    })
}

fn parse_choice(choice: &str) -> Result<SelectedWorkspace, SafeErrorDto> {
    match choice {
        "live" => Ok(SelectedWorkspace::Live),
        "training" => Ok(SelectedWorkspace::Training),
        _ => Err(error(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            false,
        )),
    }
}

/// H1-User, first run only (§3): start with an empty workspace, or prepare
/// the sample data and start with it. Saved, then PMC restarts.
#[tauri::command]
pub async fn choose_first_workspace(
    app: AppHandle,
    choice: String,
) -> Result<WorkspaceChangeDto, SafeErrorDto> {
    let target = parse_choice(&choice)?;
    if app.state::<WorkspaceStatus>().startup != Startup::FirstRun {
        return Err(error(
            "WORKSPACE_NOT_FIRST_RUN",
            "desktop.workspace_not_first_run",
            false,
        ));
    }
    blocking(&app, move |app| {
        if target == SelectedWorkspace::Training {
            // A failure leaves the choice unmade: the screen offers it again.
            prepare_sample_data(app, None)?;
        }
        save_and_restart(app, target, true)
    })
    .await
}

/// H1-User (§4, Settings → Workspace): switch to the other workspace. The
/// sample is prepared first when switching to it; then saved and restarted.
#[tauri::command]
pub async fn switch_workspace(
    app: AppHandle,
    target: String,
) -> Result<WorkspaceChangeDto, SafeErrorDto> {
    let target = parse_choice(&target)?;
    let open = open_workspace(&app);
    let already = matches!(
        (open, target),
        (Some(WorkspaceKind::Live), SelectedWorkspace::Live)
            | (Some(WorkspaceKind::Training), SelectedWorkspace::Training)
    );
    if open.is_none() || already {
        return Err(error(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            false,
        ));
    }
    blocking(&app, move |app| {
        if target == SelectedWorkspace::Training {
            prepare_sample_data(app, open)?;
        }
        save_and_restart(app, target, false)
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleResetDto {
    pub seed_version: u32,
}

/// Reset and delete are offered only from Live (§8); the host derives it.
fn live_only(app: &AppHandle) -> Result<WorkspaceKind, SafeErrorDto> {
    match open_workspace(app) {
        Some(WorkspaceKind::Live) => Ok(WorkspaceKind::Live),
        _ => Err(sample_error(&SampleError::SampleIsOpen, Doing::Other)),
    }
}

/// H1-User (§8): the sample returns to its starting state. From Live only;
/// the service refuses otherwise too.
#[tauri::command]
pub async fn reset_sample_data(
    app: AppHandle,
    client_request_id: String,
) -> Result<SampleResetDto, SafeErrorDto> {
    let open = live_only(&app)?;
    let operation_id = host_id("pmc-sample-reset/v1", &client_request_id)?;
    let now = app.state::<HostRuntime>().now();
    blocking(&app, move |app| {
        with_service(app, |service| {
            service
                .reset(open, &operation_id, now)
                .map(|report| SampleResetDto {
                    seed_version: report.manifest.seed_version,
                })
                .map_err(|failure| sample_error(&failure, Doing::Resetting))
        })
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleDeletePreviewDto {
    pub prepared_intent_id: String,
    pub payload_sha256: String,
    pub seed_id: String,
    pub seed_version: u32,
    pub has_ledger: bool,
    pub has_vault: bool,
    pub has_generated_files: bool,
    pub settings_revision: u64,
    pub operation: String,
    pub inventory_sha256: String,
    pub effect: String,
    pub expires_at_millis: i64,
}

fn preview_dto(preview: SampleDeletePreview) -> SampleDeletePreviewDto {
    SampleDeletePreviewDto {
        prepared_intent_id: preview.prepared_intent_id,
        payload_sha256: preview.payload_sha256,
        seed_id: preview.seed_id,
        seed_version: preview.seed_version,
        has_ledger: preview.has_ledger,
        has_vault: preview.has_vault,
        has_generated_files: preview.has_generated_files,
        settings_revision: preview.settings_revision,
        operation: preview.operation,
        inventory_sha256: preview.inventory_sha256,
        effect: preview.effect,
        expires_at_millis: preview.expires_at_millis,
    }
}

/// H2b step 1 (§8): the exact preview. Nothing is changed. From Live only.
#[tauri::command]
pub async fn prepare_sample_delete(app: AppHandle) -> Result<SampleDeletePreviewDto, SafeErrorDto> {
    let open = live_only(&app)?;
    let intent = new_token()?;
    let now = app.state::<HostRuntime>().now();
    blocking(&app, move |app| {
        let revision = settings_revision(app)?;
        with_service(app, |service| {
            service
                .prepare_delete(open, &intent, revision, DELETE_VALID_FOR_MILLIS, now)
                .map(preview_dto)
                .map_err(|failure| sample_error(&failure, Doing::Other))
        })
    })
    .await
}

/// H2b reject (§8): the preview is discarded; nothing changes. From Live
/// only, like the preview it rejects.
#[tauri::command]
pub async fn reject_sample_delete(
    app: AppHandle,
    prepared_intent_id: String,
) -> Result<(), SafeErrorDto> {
    live_only(&app)?;
    let now = app.state::<HostRuntime>().now();
    blocking(&app, move |app| {
        with_service(app, |service| {
            service
                .reject_delete(&prepared_intent_id, now)
                .map_err(|failure| sample_error(&failure, Doing::Other))
        })
    })
    .await
}

/// The typed text, compared as the person means it: spaces collapsed, and
/// letters in either case.
fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// Whether the typed text is the delete phrase in one of the six languages.
#[must_use]
pub fn is_delete_phrase(typed: &str) -> bool {
    let typed = normalized(typed);
    DELETE_PHRASES
        .iter()
        .any(|phrase| normalized(phrase) == typed)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleDeletedDto {
    /// `deleted` | `not_deleted`.
    pub outcome: &'static str,
}

/// H2b approve (§8): the typed phrase is checked here — the confirmation is
/// part of the approval and the approval is the host's — then the service
/// proves every bound fact again before anything is removed.
#[tauri::command]
pub async fn approve_sample_delete(
    app: AppHandle,
    prepared_intent_id: String,
    payload_sha256: String,
    typed_confirmation: String,
    client_request_id: String,
) -> Result<SampleDeletedDto, SafeErrorDto> {
    if !is_delete_phrase(&typed_confirmation) {
        return Err(error(
            "SAMPLE_CONFIRMATION_MISMATCH",
            "desktop.sample_confirmation_mismatch",
            false,
        ));
    }
    let open = live_only(&app)?;
    let idempotency_id = host_id("pmc-sample-delete-approve/v1", &client_request_id)?;
    let now = app.state::<HostRuntime>().now();
    blocking(&app, move |app| {
        let revision = settings_revision(app)?;
        with_service(app, |service| {
            service
                .approve_delete(
                    open,
                    &ApproveSampleDelete {
                        prepared_intent_id: &prepared_intent_id,
                        payload_sha256: &payload_sha256,
                        settings_revision: revision,
                        idempotency_id: &idempotency_id,
                    },
                    now,
                )
                .map(|outcome| SampleDeletedDto {
                    outcome: match outcome {
                        SampleOutcome::Deleted => "deleted",
                        _ => "not_deleted",
                    },
                })
                .map_err(|failure| sample_error(&failure, Doing::Deleting))
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delete_phrase_is_accepted_in_every_language_and_nothing_else() {
        for phrase in DELETE_PHRASES {
            assert!(is_delete_phrase(phrase), "{phrase}");
        }
        assert!(is_delete_phrase("  delete   sample data "));
        assert!(!is_delete_phrase("DELETE"));
        assert!(!is_delete_phrase("DELETE SAMPLE DATA NOW"));
        assert!(!is_delete_phrase(""));
    }

    #[test]
    fn operation_ids_are_the_hosts_domain_separated_plain_tokens() {
        let reset = host_id("pmc-sample-reset/v1", "request-1").unwrap_or_default();
        let delete = host_id("pmc-sample-delete-approve/v1", "request-1").unwrap_or_default();
        assert_eq!(reset.len(), 32);
        assert!(reset.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(reset, delete);
        assert_eq!(
            host_id("pmc-sample-reset/v1", "request-1").unwrap_or_default(),
            reset
        );
        assert!(host_id("pmc-sample-reset/v1", "").is_err());
    }
}
