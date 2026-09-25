//! The Ledger upgrade gate in the host (DG3 upgrade-gate amendment, accepted
//! 2026-09-22; ADR 0004).
//!
//! A Live Ledger in an older supported format is not opened: the webview
//! shows the upgrade screen instead of the shell. `run_upgrade` makes a new
//! Operational Backup of exactly that Ledger — a backup run like any other,
//! so writes and other backups wait and it counts as the day's backup — and
//! only on its receipt upgrades the file in one transaction
//! (`pmc_application::backup_service` holds the files and the upgrade; this
//! module holds no path). The outcome is what the Ledger crate read back,
//! never assumed.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use pmc_application::backup_service::UpgradeRun;
use pmc_application::desktop_runtime::SystemClock;
use pmc_application::operational_backup::BackupError;
use pmc_application::restore_service::ReopenedLedger;
use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;
use pmc_platform::backup_registry::new_archive_id;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::backup_gate::BackupGate;
use crate::backup_run::{settings_export, stored_destination, BackupServiceState};
use crate::backup_secret::BackupPassphraseState;
use crate::display_settings::SettingsState;
use crate::ledger_state::{LedgerClosed, LedgerState, UnavailableReason};
use crate::restore_run::RestoreSession;
use crate::runtime::host_correlation;
use crate::safe_error::SafeErrorDto;

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeGateDto {
    /// `none` (the Ledger is open, or unavailable for another reason) |
    /// `upgrade` | `training_reset` (an older Training workspace: it is
    /// reset, never upgraded) | `unsupported_old` | `newer_version`.
    pub state: &'static str,
    /// This PMC's version, for "PMC {version} stores your records…".
    pub app_version: &'static str,
    pub from_schema: Option<u32>,
    pub to_schema: Option<u32>,
    pub record_count: Option<u64>,
    /// Upgrading applies to the Live workspace only; Training is reset.
    pub live: bool,
}

/// H0: whether the upgrade gate replaces the shell, and its facts (§2–§3).
#[tauri::command]
pub fn get_upgrade_gate(app: AppHandle) -> Result<UpgradeGateDto, SafeErrorDto> {
    let live = app.state::<RestoreSession>().live();
    let none = UpgradeGateDto {
        state: "none",
        app_version: env!("CARGO_PKG_VERSION"),
        from_schema: None,
        to_schema: None,
        record_count: None,
        live,
    };
    match app.state::<LedgerState>().state() {
        Err(LedgerClosed::Unavailable(UnavailableReason::UpgradeRequired)) => {
            match app.state::<BackupServiceState>().0.upgrade_plan() {
                Some(_) if !live => Ok(UpgradeGateDto {
                    state: "training_reset",
                    ..none
                }),
                Some(plan) => Ok(UpgradeGateDto {
                    state: "upgrade",
                    from_schema: Some(plan.from_schema),
                    to_schema: Some(plan.to_schema),
                    record_count: Some(plan.record_count),
                    ..none
                }),
                // No longer an upgradeable Ledger: System Health says why.
                None => Ok(none),
            }
        }
        Err(LedgerClosed::Unavailable(UnavailableReason::UnsupportedOld)) => Ok(UpgradeGateDto {
            state: "unsupported_old",
            ..none
        }),
        Err(LedgerClosed::Unavailable(UnavailableReason::NewerVersion)) => Ok(UpgradeGateDto {
            state: "newer_version",
            ..none
        }),
        _ => Ok(none),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeResultDto {
    /// `upgraded` | `rolled_back` | `upgraded_unreadable` | `outcome_unknown`.
    pub outcome: &'static str,
    /// When the pre-upgrade backup that holds the Ledger as it was verified.
    pub backup_verified_at_millis: i64,
    /// `ready`, or why the Ledger is still not open.
    pub ledger: &'static str,
}

enum Failed {
    Running,
    Backup(BackupError),
}

/// H1-User "Upgrade" (§4): the pre-upgrade backup, then the upgrade. `Err`
/// means nothing was upgraded; every `Ok` names the backup that holds the
/// Ledger as it was.
#[tauri::command]
pub async fn run_upgrade(app: AppHandle) -> Result<UpgradeResultDto, SafeErrorDto> {
    let correlation = host_correlation();
    if !app.state::<RestoreSession>().live() {
        return Err(error(
            "UPGRADE_LIVE_ONLY",
            "desktop.upgrade_live_only",
            &correlation,
            false,
        ));
    }
    let not_required = || {
        error(
            "UPGRADE_NOT_REQUIRED",
            "desktop.upgrade_not_required",
            &correlation,
            false,
        )
    };
    if app.state::<LedgerState>().state()
        != Err(LedgerClosed::Unavailable(
            UnavailableReason::UpgradeRequired,
        ))
    {
        return Err(not_required());
    }
    let plan = app
        .state::<BackupServiceState>()
        .0
        .upgrade_plan()
        .ok_or_else(not_required)?;
    let settings = app.state::<SettingsState>();
    let destination = stored_destination(&settings)
        .ok_or_else(|| {
            error(
                "BACKUP_DESTINATION_NOT_SET",
                "desktop.backup_destination_not_set",
                &correlation,
                false,
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
            error(
                "BACKUP_PASSPHRASE_REQUIRED",
                "desktop.backup_passphrase_required",
                &correlation,
                false,
            )
        })?;
    let settings_json =
        settings_export(&settings).ok_or_else(crate::display_settings::unavailable)?;
    let archive_id = new_archive_id().ok_or_else(|| {
        error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        )
    })?;

    let expected = plan.source.clone();
    let worker = app.clone();
    let ran = tauri::async_runtime::spawn_blocking(move || {
        let gate = worker.state::<BackupGate>();
        let service = worker.state::<BackupServiceState>();
        let ledgers = worker.state::<LedgerState>();
        // A backup run: other backups and restores wait. The Ledger is
        // closed, so no write can be in flight.
        let (ticket, exclusive) = gate.begin_run().map_err(|_| Failed::Running)?;
        drop(exclusive);
        match service.0.back_up_and_upgrade(
            &plan,
            &settings_json,
            &destination,
            &passphrase,
            &archive_id,
            &now,
        ) {
            Err(failure) => {
                ticket.finish(Err("desktop.backup_failed"));
                Err(Failed::Backup(failure))
            }
            Ok((record, run)) => {
                ticket.finish(Ok(record.verified_at_millis));
                match run {
                    UpgradeRun::Upgraded => match service.0.reopen_ledger() {
                        ReopenedLedger::Ready(ledger) => ledgers.install(ledger),
                        // Upgraded, but it does not open: say exactly that.
                        _ => {
                            ledgers.set_unavailable(UnavailableReason::OpenFailed);
                            return Ok((
                                record.verified_at_millis,
                                UpgradeRun::UpgradedButUnreadable,
                            ));
                        }
                    },
                    // Nothing changed: the gate stays.
                    UpgradeRun::RolledBack => {}
                    UpgradeRun::UpgradedButUnreadable | UpgradeRun::OutcomeUnknown => {
                        ledgers.set_unavailable(UnavailableReason::OpenFailed);
                    }
                }
                Ok((record.verified_at_millis, run))
            }
        }
    })
    .await
    .map_err(|_| {
        // The worker stopped without a result: tell only what the file
        // shows now.
        if app
            .state::<BackupServiceState>()
            .0
            .still_the_source(&expected)
        {
            error(
                "UPGRADE_FAILED_UNCHANGED",
                "desktop.upgrade_failed_unchanged",
                &correlation,
                true,
            )
        } else {
            app.state::<LedgerState>()
                .set_unavailable(UnavailableReason::OpenFailed);
            error(
                "UPGRADE_OUTCOME_UNKNOWN",
                "desktop.upgrade_outcome_unknown",
                &correlation,
                false,
            )
        }
    })?;
    let (verified_at, run) = ran.map_err(|failure| match failure {
        Failed::Running => error(
            "BACKUP_RUNNING",
            "desktop.backup_running",
            &correlation,
            true,
        ),
        // The Ledger changed after the screen read it: nothing was backed up
        // or upgraded.
        Failed::Backup(BackupError::SourceChanged) => error(
            "UPGRADE_FAILED_UNCHANGED",
            "desktop.upgrade_failed_unchanged",
            &correlation,
            true,
        ),
        Failed::Backup(_) => error(
            "UPGRADE_BACKUP_FAILED",
            "desktop.upgrade_backup_failed",
            &correlation,
            true,
        ),
    })?;
    let ledger = app
        .state::<LedgerState>()
        .state()
        .map_or_else(LedgerClosed::as_str, |()| "ready");
    Ok(UpgradeResultDto {
        outcome: match run {
            UpgradeRun::Upgraded => "upgraded",
            UpgradeRun::RolledBack => "rolled_back",
            UpgradeRun::UpgradedButUnreadable => "upgraded_unreadable",
            UpgradeRun::OutcomeUnknown => "outcome_unknown",
        },
        backup_verified_at_millis: verified_at,
        ledger,
    })
}
