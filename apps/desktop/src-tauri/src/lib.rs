#![forbid(unsafe_code)]

mod backup_gate;
mod backup_run;
mod backup_secret;
mod backup_setup;
mod commands;
mod display_settings;
mod entry_commands;
mod evidence_file;
mod ledger_state;
mod native_dialogs;
mod restore_run;
mod runtime;
mod safe_error;
mod sample_workspace;
mod single_instance;
mod upgrade_run;
mod vault_root;
mod workspace_startup;
mod write_commands;

use pmc_platform::workspace::WorkspaceKind;
use tauri::Manager;

use display_settings::SettingsState;
use ledger_state::{LedgerState, UnavailableReason, VaultState};
use pmc_application::backup_service::UnopenedLedger;

/// The one launch switch a developer build accepts: `--pmc-workspace=training`
/// opens the Training workspace the `pmc-seed` tool fills, instead of Live.
///
/// Debug builds only. A release build ignores the argument entirely and
/// always opens Live, so no shipped binary can be pointed at synthetic data
/// by a launch flag. The default is Live in every build.
pub fn workspace_from_args(arguments: impl IntoIterator<Item = String>) -> WorkspaceKind {
    let requested = arguments
        .into_iter()
        .any(|argument| argument == "--pmc-workspace=training");
    if cfg!(debug_assertions) && requested {
        WorkspaceKind::Training
    } else {
        WorkspaceKind::Live
    }
}

/// The window title's sample marker (§5): "Product Mission Control — Sample
/// data", in the stored language. Set once, when the sample opens.
fn mark_sample_window(app: &tauri::App, settings: Option<&pmc_platform::settings::SettingsStore>) {
    let locale = settings
        .and_then(|store| store.read().ok())
        .map(|document| document.display.locale)
        .unwrap_or_default();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&format!(
            "Product Mission Control — {}",
            workspace_startup::sample_title_suffix(&locale)
        ));
    }
}

/// Starts the minimal Windows-first desktop host. Decides which workspace
/// this run opens (the accepted sample-workspace amendment §2), opens its
/// real Product Ledger once and holds it as managed state for the IPC
/// command surface in `commands`.
///
/// `requested` is Live, or Training only through the debug-build developer
/// seam ([`workspace_from_args`]); a person's choice is read from settings.
pub fn run(requested: WorkspaceKind) {
    let protected_root = ledger_state::protected_root();
    // One PMC per profile (item ⑩): a second launch is told so and stops
    // here, before a window, the Ledger or the settings document.
    let Some(instance) = single_instance::claim(&protected_root) else {
        return;
    };
    tauri::Builder::default()
        .setup(move |app| {
            // Held for the whole run; released by the OS when the process ends.
            // Held for the whole run, a restart included: released by the OS
            // when the process ends.
            app.manage(instance);
            app.manage(sample_workspace::RestartPending::default());
            // Decided before the Ledger opens, because opening it creates the
            // Live workspace this looks for.
            let used_before = ledger_state::live_profile_exists(&protected_root);
            let at_startup = display_settings::open(&protected_root, used_before);
            // The sample's own interrupted reset or delete is finished first:
            // whether sample data can open depends on it (§8).
            let sample = sample_workspace::reconcile_at_startup(
                &protected_root,
                runtime::HostRuntime::new().now(),
            );
            let startup = workspace_startup::decide(
                at_startup.selection,
                requested == WorkspaceKind::Training,
                sample.availability,
            );
            let status = workspace_startup::WorkspaceStatus {
                startup,
                selection: at_startup.selection,
                sample: sample.availability,
                settings_set_aside: at_startup.set_aside,
                settings_unavailable: at_startup.store.is_none(),
                sample_cleanup_pending: sample.cleanup_pending,
            };
            let first_run = startup == workspace_startup::Startup::FirstRun;
            // During the first-run choice nothing is opened; the managed
            // state below is Live's, with its Ledger closed.
            let workspace = status.open().unwrap_or(WorkspaceKind::Live);
            if workspace == WorkspaceKind::Training {
                mark_sample_window(app, at_startup.store.as_ref());
            }
            app.manage(status);
            app.manage(sample.state);
            let settings = at_startup.store;
            let backups =
                pmc_application::backup_service::BackupService::new(&protected_root, workspace);
            // Before the Ledger is opened (S7-B1): a restore a crash
            // interrupted is put back, never finished; then projections a
            // restore left to mark are marked, if that Ledger opens now. Both
            // are retried at every launch until they succeed. Not during the
            // first-run choice: no workspace is open yet (§2).
            if !first_run {
                if let Some(store) = settings.as_ref() {
                    let _ = backups.reconcile_restore(store, runtime::HostRuntime::new().now());
                }
                let _ = backups.finish_pending_restore_projections();
            }
            // A Vault change a crash interrupted (item ⑦): a preview is
            // discarded, and an approval is resolved by reading the settings
            // — never by assuming which side of the write the crash fell on.
            // Before anything reads the Vault; retried at every launch.
            let vault_changes = pmc_application::vault_root_change::VaultRootService::in_directory(
                protected_root.path(),
                match workspace {
                    WorkspaceKind::Live => "live",
                    WorkspaceKind::Training => "training",
                },
                workspace,
            );
            // One that cannot be resolved closes the Vault for this run: the
            // settings may name a folder the change never finished
            // committing, so no Evidence is read from any folder until a
            // later start resolves it (fail closed, never assumed).
            let vault_change_unresolved = match settings.as_ref() {
                // No Vault is read before the first-run choice.
                _ if first_run => false,
                Some(store) => vault_changes
                    .reconcile(store, runtime::HostRuntime::new().now())
                    .is_err(),
                None => vault_changes
                    .control()
                    .map_or(true, |control| control.active.is_some()),
            };
            app.manage(vault_root::VaultRootServiceState(vault_changes));
            app.manage(vault_root::VaultRootSession::new(
                workspace == WorkspaceKind::Live,
            ));
            app.manage(SettingsState(std::sync::Mutex::new(settings)));
            // A Ledger a restore may have left half replaced is never opened:
            // System Health says so and names the recovery backup.
            let (ledger, vault) = if first_run {
                // Nothing is opened, and nothing is created, until the choice.
                (
                    LedgerState::unavailable(UnavailableReason::FirstRun),
                    ledger_state::resolve_vault(&protected_root, workspace),
                )
            } else if backups.restore_needs_recovery() {
                (
                    LedgerState::unavailable(
                        ledger_state::UnavailableReason::RestoreRecoveryRequired,
                    ),
                    ledger_state::resolve_vault(&protected_root, workspace),
                )
            } else {
                let (opened, vault) = ledger_state::open_workspace(&protected_root, workspace);
                let ledger = match opened {
                    Ok(ledger) => {
                        let ledger = LedgerState::new(ledger);
                        // The named S7 bootstrap: creating the empty Live
                        // Ledger, once.
                        if let Ok(open) = ledger.read() {
                            backups.record_bootstrap(&open, runtime::HostRuntime::new().now());
                        }
                        ledger
                    }
                    // Not opened: an older format goes to the upgrade gate,
                    // anything else to System Health (DG3 upgrade-gate §2).
                    Err(_) => LedgerState::unavailable(match backups.classify_unopened() {
                        UnopenedLedger::UpgradeRequired => UnavailableReason::UpgradeRequired,
                        UnopenedLedger::UnsupportedOld => UnavailableReason::UnsupportedOld,
                        UnopenedLedger::NewerVersion => UnavailableReason::NewerVersion,
                        UnopenedLedger::Other => UnavailableReason::OpenFailed,
                    }),
                };
                (ledger, vault)
            };
            app.manage(ledger);
            app.manage(VaultState::new(workspace, vault, vault_change_unresolved));
            app.manage(evidence_file::EvidenceFileSession::default());
            app.manage(runtime::HostRuntime::new());
            app.manage(backup_secret::BackupPassphraseState::new());
            // Live writes stay closed until the startup check below finishes.
            app.manage(backup_gate::BackupGate::new(
                workspace == WorkspaceKind::Live,
            ));
            app.manage(backup_run::BackupServiceState(backups));
            // Restore is Live's, and not before the first-run choice.
            app.manage(restore_run::RestoreSession::new(
                workspace == WorkspaceKind::Live && !first_run,
            ));
            // No backup work before the first-run choice (§2).
            if !first_run {
                tauri::async_runtime::spawn(backup_run::start(app.handle().clone()));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_ledger_status,
            workspace_startup::get_workspace_status,
            sample_workspace::choose_first_workspace,
            sample_workspace::switch_workspace,
            sample_workspace::reset_sample_data,
            sample_workspace::prepare_sample_delete,
            sample_workspace::reject_sample_delete,
            sample_workspace::approve_sample_delete,
            display_settings::get_display_locale,
            display_settings::set_display_locale,
            display_settings::get_display_timezone,
            backup_setup::get_backup_destination,
            backup_setup::choose_backup_destination,
            backup_secret::generate_recovery_passphrase,
            backup_secret::set_backup_passphrase,
            backup_secret::get_backup_passphrase_status,
            backup_run::get_backup_status,
            backup_run::run_backup_now,
            restore_run::choose_restore_archive,
            restore_run::choose_recovery_archive,
            evidence_file::choose_evidence_file,
            evidence_file::create_evidence_from_file,
            evidence_file::discard_evidence_file_choice,
            restore_run::discard_restore_selection,
            restore_run::check_restore_archive,
            restore_run::prepare_restore_from_archive,
            restore_run::reject_prepared_restore,
            restore_run::approve_and_execute_restore,
            restore_run::get_system_health,
            restore_run::quit_pmc,
            vault_root::choose_vault_folder,
            vault_root::prepare_vault_root_change,
            vault_root::reject_vault_root_change,
            vault_root::approve_vault_root_change,
            upgrade_run::get_upgrade_gate,
            upgrade_run::run_upgrade,
            entry_commands::list_entry_records,
            entry_commands::get_entry_record,
            entry_commands::create_portfolio_record,
            entry_commands::update_portfolio_record,
            entry_commands::create_product_record,
            entry_commands::update_product_record,
            entry_commands::create_roadmap_record,
            entry_commands::update_roadmap_record,
            entry_commands::create_kpi_definition_record,
            entry_commands::update_kpi_definition_record,
            entry_commands::create_kpi_observation_record,
            entry_commands::update_kpi_observation_record,
            entry_commands::link_portfolio_product_record,
            entry_commands::link_product_roadmap_record,
            entry_commands::link_product_kpi_record,
            entry_commands::create_initiative_record,
            entry_commands::update_initiative_record,
            entry_commands::create_project_record,
            entry_commands::update_project_record,
            entry_commands::create_milestone_record,
            entry_commands::update_milestone_record,
            entry_commands::link_initiative_project_record,
            entry_commands::link_project_product_record,
            entry_commands::create_stakeholder_record,
            entry_commands::update_stakeholder_record,
            entry_commands::link_stakeholder_subject_record,
            entry_commands::create_action_request_draft_record,
            entry_commands::submit_action_request_record,
            entry_commands::create_decision_request_draft_record,
            entry_commands::submit_decision_request_record,
            entry_commands::create_issue_record,
            entry_commands::create_risk_record,
            entry_commands::update_risk_response_record,
            commands::get_executive_cockpit,
            commands::get_portfolio_overview,
            commands::get_people_directory,
            commands::get_work_queue,
            commands::get_product_detail,
            write_commands::prepare_accept_action_request,
            write_commands::approve_and_execute_accept_action_request,
            write_commands::reject_prepared_accept_action_request,
            write_commands::decline_action_request,
            write_commands::withdraw_action_request,
            write_commands::start_action,
            write_commands::get_action_completion_context,
            write_commands::link_action_completion_evidence,
            write_commands::prepare_complete_action,
            write_commands::prepare_cancel_action,
            write_commands::prepare_reopen_action,
            write_commands::approve_and_execute_complete_action,
            write_commands::approve_and_execute_cancel_action,
            write_commands::approve_and_execute_reopen_action,
            write_commands::reject_prepared_action_intent,
            write_commands::get_evidence_references,
            write_commands::withdraw_decision_request,
            write_commands::prepare_resolve_decision_request,
            write_commands::approve_and_execute_resolve_decision_request,
            write_commands::reject_prepared_decision_intent,
            write_commands::pin_evidence_fingerprint,
            write_commands::reobserve_evidence_verification,
            write_commands::get_vault_status,
            write_commands::link_evidence_to_product,
            write_commands::prepare_record_risk_occurrence,
            write_commands::prepare_close_risk,
            write_commands::approve_and_execute_record_risk_occurrence,
            write_commands::approve_and_execute_close_risk,
            write_commands::reject_prepared_risk_intent,
            write_commands::prepare_resolve_issue,
            write_commands::prepare_close_issue,
            write_commands::prepare_reopen_issue,
            write_commands::approve_and_execute_issue_transition,
            write_commands::reject_prepared_issue_intent
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("PLATFORM_DESKTOP_START_FAILED: {error}"));
}
