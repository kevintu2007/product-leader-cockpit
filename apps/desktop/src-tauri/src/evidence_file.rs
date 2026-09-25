//! Evidence from a file (item ⑦-3; DG3 Vault-root and Evidence-from-file
//! amendment §4, accepted 2026-09-23): the host's file picker, opened in the
//! Vault folder; the chosen file held behind an opaque, expiring token bound
//! to this workspace's settings as they were when it was chosen; and the
//! create, which observes the file again and pins what it reads.
//!
//! The webview supplies the dialog title, the token, the chosen
//! classification and one `clientRequestId` per opened sheet. It learns the
//! file's own name, when it was observed, and existing references by id and
//! version only — never a path (ADR 0011).

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use pmc_application::evidence_from_file::{
    create_evidence_from_file as create_from_file, observe_chosen_file, preview_chosen_file,
    ChosenEvidenceFile, EvidenceFromFileError,
};
use pmc_application::evidence_writes::VaultUnavailable;
use pmc_domain::classification::DataClassification;
use pmc_domain::evidence::OperationContext;
use pmc_domain::identity::{CorrelationId, IdempotencyId};
use pmc_domain::work_management::EvidenceVerification;
use pmc_ledger::sqlite::EvidenceMatch;
use pmc_platform::backup_registry::new_archive_id;
use pmc_platform::windows_names::same_windows_name;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::backup_gate::BackupGate;
use crate::display_settings::SettingsState;
use crate::ledger_state::{LedgerState, VaultState};
use crate::native_dialogs::pick_evidence_file;
use crate::runtime::{host_correlation, HostRuntime};
use crate::safe_error::SafeErrorDto;

/// How long a chosen file stays chosen: long enough to pick a
/// classification, short enough that an old choice is not acted on later.
const CHOICE_VALID_FOR_MILLIS: i64 = 30 * 60 * 1000;

struct Held {
    token: String,
    chosen: ChosenEvidenceFile,
    /// The settings revision when it was chosen: a Vault change since makes
    /// the choice stale (the root is compared too, in the application).
    settings_revision: Option<u64>,
    expires_at_millis: i64,
}

/// The one file this session has chosen, if any.
#[derive(Default)]
pub struct EvidenceFileSession(Mutex<Option<Held>>);

impl EvidenceFileSession {
    fn lock(&self) -> MutexGuard<'_, Option<Held>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn error(
    code: &'static str,
    key: &'static str,
    correlation: &CorrelationId,
    retryable: bool,
) -> SafeErrorDto {
    SafeErrorDto::host(code, key, correlation, retryable)
}

fn stale(correlation: &CorrelationId) -> SafeErrorDto {
    error(
        "EVIDENCE_FILE_CHOICE_STALE",
        "desktop.evidence_file_choice_stale",
        correlation,
        false,
    )
}

fn settings_revision(settings: &SettingsState) -> Option<u64> {
    settings
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|store| store.read().ok())
        .map(|document| document.revision)
}

/// DG3 Vault-root amendment §5: one key per reason.
fn file_error(failure: EvidenceFromFileError, correlation: &CorrelationId) -> SafeErrorDto {
    match failure {
        EvidenceFromFileError::Vault(VaultUnavailable::NotConfigured) => error(
            "VAULT_NOT_CONFIGURED",
            "desktop.vault_not_configured",
            correlation,
            false,
        ),
        EvidenceFromFileError::Vault(VaultUnavailable::InvalidRoot) => error(
            "VAULT_ROOT_UNAVAILABLE",
            "desktop.vault_root_unavailable",
            correlation,
            true,
        ),
        EvidenceFromFileError::Vault(VaultUnavailable::ChangeUnresolved) => error(
            "VAULT_CHANGE_UNRESOLVED",
            "desktop.vault_change_unresolved",
            correlation,
            false,
        ),
        EvidenceFromFileError::OutsideVault => error(
            "EVIDENCE_FILE_OUTSIDE_VAULT",
            "desktop.evidence_file_outside_vault",
            correlation,
            false,
        ),
        EvidenceFromFileError::Unreadable => error(
            "EVIDENCE_FILE_UNREADABLE",
            "desktop.evidence_file_unreadable",
            correlation,
            true,
        ),
        EvidenceFromFileError::VaultChanged
        | EvidenceFromFileError::FileChanged(_)
        | EvidenceFromFileError::AlreadyReferenced(_) => stale(correlation),
        EvidenceFromFileError::Read(open) => SafeErrorDto::from_open(open, correlation),
        EvidenceFromFileError::Entry(entry) => {
            crate::entry_commands::entry_error(entry, correlation)
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceMatchDto {
    pub evidence_id: String,
    pub version: u64,
}

fn match_dto(found: &EvidenceMatch) -> EvidenceMatchDto {
    EvidenceMatchDto {
        evidence_id: found.id.as_str().to_owned(),
        version: found.version.get(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChosenEvidenceFileDto {
    /// False when the person cancelled the picker.
    pub chosen: bool,
    pub token: Option<String>,
    /// The file's own name only.
    pub file_name: Option<String>,
    pub observed_at_millis: Option<i64>,
    /// A reference that already names this file: offer it instead (§4.5).
    pub existing: Option<EvidenceMatchDto>,
    /// References holding the same content under another path: a warning.
    pub same_content: Vec<EvidenceMatchDto>,
}

/// H1-User: the host's file picker, opened in the Vault folder (§4.1). The
/// file is checked against the Vault as it is now, observed, and held
/// behind a token; the sheet learns its name and what already references it.
#[tauri::command]
pub async fn choose_evidence_file(
    window: tauri::Window,
    app: AppHandle,
    title: String,
) -> Result<ChosenEvidenceFileDto, SafeErrorDto> {
    let correlation = host_correlation();
    let settings = app.state::<SettingsState>();
    let vault = app.state::<VaultState>().current(&settings);
    let start = vault
        .root()
        .map_err(|reason| file_error(EvidenceFromFileError::Vault(reason), &correlation))?
        .as_path()
        .to_path_buf();
    let revision = settings_revision(&settings);
    let title = title.chars().take(120).collect::<String>();
    // No lock is held while the dialog is open.
    let Some(picked) = pick_evidence_file(window, title, start).await else {
        return Ok(ChosenEvidenceFileDto {
            chosen: false,
            token: None,
            file_name: None,
            observed_at_millis: None,
            existing: None,
            same_content: Vec::new(),
        });
    };
    let now = app.state::<HostRuntime>().now();
    let chosen = observe_chosen_file(&vault, &picked, now)
        .map_err(|failure| file_error(failure, &correlation))?;
    let ledgers = app.state::<LedgerState>();
    let preview = {
        let ledger = ledgers
            .read()
            .map_err(|closed| closed.to_safe_error(&correlation))?;
        preview_chosen_file(&ledger, &chosen, same_windows_name)
            .map_err(|failure| file_error(failure, &correlation))?
    };
    let token = new_archive_id().ok_or_else(|| {
        error(
            "PLATFORM_INTERNAL",
            "desktop.random_unavailable",
            &correlation,
            true,
        )
    })?;
    let dto = ChosenEvidenceFileDto {
        chosen: true,
        token: Some(token.clone()),
        file_name: Some(chosen.file_name().to_owned()),
        observed_at_millis: Some(chosen.observation().observed_at.unix_millis()),
        existing: preview.existing.as_ref().map(match_dto),
        same_content: preview.same_content.iter().map(match_dto).collect(),
    };
    *app.state::<EvidenceFileSession>().lock() = Some(Held {
        token,
        chosen,
        settings_revision: revision,
        expires_at_millis: now.unix_millis().saturating_add(CHOICE_VALID_FOR_MILLIS),
    });
    Ok(dto)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceFromFileResultDto {
    /// `created` | `file_changed` (observe again; confirm the new
    /// observation) | `already_referenced` (offer that one instead).
    pub outcome: &'static str,
    /// The created reference, or the one that already names the file.
    pub evidence: Option<EvidenceMatchDto>,
    /// The observation the reference records, or the new one to confirm.
    pub observed_at_millis: Option<i64>,
}

/// H1-User (§4.2–§4.7): create the reference for the chosen file — observed
/// again now, fingerprint pinned and verified from that observation,
/// `UserEntered`, with the classification the person chose (never
/// Unclassified). A retry of the same request returns what was created.
#[tauri::command]
pub fn create_evidence_from_file(
    app: AppHandle,
    token: String,
    classification: String,
    client_request_id: String,
) -> Result<EvidenceFromFileResultDto, SafeErrorDto> {
    let correlation = host_correlation();
    let runtime = app.state::<HostRuntime>();
    let now = runtime.now();
    let session = app.state::<EvidenceFileSession>();
    let held = match &*session.lock() {
        Some(held) if held.token == token && now.unix_millis() <= held.expires_at_millis => {
            (held.chosen.clone(), held.settings_revision)
        }
        _ => return Err(stale(&correlation)),
    };
    let classification = DataClassification::from_persisted(&classification)
        .ok()
        .filter(|chosen| *chosen != DataClassification::Unclassified)
        .ok_or_else(|| {
            error(
                "VALIDATION_INVALID_FIELD",
                "desktop.entry_classification_required",
                &correlation,
                false,
            )
        })?;
    let idempotency = IdempotencyId::parse(client_request_id).map_err(|_| {
        error(
            "VALIDATION_INVALID_FIELD",
            "desktop.invalid_argument",
            &correlation,
            false,
        )
    })?;
    let settings = app.state::<SettingsState>();
    // The Vault setting changed since the file was chosen: choose it again.
    if settings_revision(&settings) != held.1 {
        return Err(stale(&correlation));
    }
    let vault = app.state::<VaultState>().current(&settings);
    let gate = app.state::<BackupGate>();
    let ledgers = app.state::<LedgerState>();
    let mut ledger = ledgers
        .write(&gate, now)
        .map_err(|refusal| refusal.to_safe_error(&correlation))?;
    let mut ids = runtime.ids();
    let created = create_from_file(
        &mut ledger,
        &vault,
        &held.0,
        classification,
        OperationContext {
            idempotency_id: idempotency,
            correlation_id: correlation.clone(),
        },
        &mut ids,
        now,
        same_windows_name,
    );
    drop(ledger);
    match created {
        // The choice stays held until the sheet lets go or it expires: a
        // retry after a lost reply must reach the reservation's replay and
        // learn the id this create made.
        Ok(record) => {
            Ok(EvidenceFromFileResultDto {
                outcome: "created",
                evidence: Some(EvidenceMatchDto {
                    evidence_id: record.id.as_str().to_owned(),
                    version: record.version.get(),
                }),
                // The one observation it was created from (a retry answers
                // the first attempt's).
                observed_at_millis: match record.verification {
                    EvidenceVerification::Verified { verified_at, .. } => {
                        Some(verified_at.unix_millis())
                    }
                    _ => None,
                },
            })
        }
        // The bytes changed since they were shown: the sheet shows the new
        // observation, and the same token now holds it (§4.2).
        Err(EvidenceFromFileError::FileChanged(refreshed)) => {
            let observed = refreshed.observation().observed_at.unix_millis();
            if let Some(current) = session.lock().as_mut() {
                if current.token == token {
                    current.chosen = refreshed;
                }
            }
            Ok(EvidenceFromFileResultDto {
                outcome: "file_changed",
                evidence: None,
                observed_at_millis: Some(observed),
            })
        }
        Err(EvidenceFromFileError::AlreadyReferenced(existing)) => Ok(EvidenceFromFileResultDto {
            outcome: "already_referenced",
            evidence: Some(match_dto(&existing)),
            observed_at_millis: None,
        }),
        Err(EvidenceFromFileError::VaultChanged) => {
            *session.lock() = None;
            Err(stale(&correlation))
        }
        Err(failure) => Err(file_error(failure, &correlation)),
    }
}

/// H0: forget the chosen file (the sheet closed).
#[tauri::command]
pub fn discard_evidence_file_choice(app: AppHandle) -> Result<(), SafeErrorDto> {
    *app.state::<EvidenceFileSession>().lock() = None;
    Ok(())
}
