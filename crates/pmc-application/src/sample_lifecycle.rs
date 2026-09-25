//! Preparing, resetting and deleting the sample workspace (item ⑨; the
//! accepted sample-workspace amendment §7 and §8).
//!
//! The whole sample folder is one unit: its Ledger with the files SQLite
//! keeps beside it, its synthetic Vault and its manifest. It is never
//! changed in place. A fresh sample is built and verified in a sibling
//! staging folder, the current folder is renamed aside, the staging folder
//! is renamed into place, and only then is the old one removed. A delete
//! renames the folder aside before removing it. Every step is recorded
//! first in the operation record outside the sample folder
//! ([`pmc_platform::sample_control`]), so a crash between any two steps is
//! finished or rolled back at the next start by [`SampleWorkspace::reconcile`]
//! — decided from what is on disk, never assumed.
//!
//! Settings, Live, the instance lock and the host audit log are never
//! touched; the audit log records each reset and delete and survives them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use pmc_domain::time::UtcTimestamp;
use pmc_platform::host_audit::{
    AuditOutcome, AuditWorkspace, HostAuditEvent, HostAuditLog, SAMPLE_DELETED,
    SAMPLE_DELETE_PREPARED, SAMPLE_DELETE_REJECTED, SAMPLE_NOT_DELETED, SAMPLE_RESET,
    SAMPLE_RESET_ROLLED_BACK,
};
use pmc_platform::sample_control::{
    DeletePhase, PreparedSampleDelete, ResetPhase, SampleControl, SampleControlError,
    SampleControlStore, SampleDeleteBinding, SampleOperation, SampleOutcome, SampleTerminal,
};
use pmc_platform::settings::ProtectedSettingsRoot;
use pmc_platform::workspace::{WorkspaceError, WorkspaceIdentity, WorkspaceKind};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::sample_workspace::{
    inspect_folder, inventory_sha256, read_manifest, remove_tree, write_seed_at, SampleState,
    SeedError, SeedManifest, SeedReport, LEDGER_FILE_NAME, MANIFEST_FILE_NAME,
    SUPPORTED_SCHEMA_VERSION,
};
use pmc_ledger::sqlite::CURRENT_SCHEMA_VERSION;

const CONTROL_FILE_NAME: &str = "sample-control-v1.json";
const AUDIT_FILE_NAME: &str = "host-audit-v1.jsonl";
const STAGING: &str = "staging";
const RETIRED: &str = "retired";
const DELETED: &str = "deleted";
const MAX_OPERATION_ID_LENGTH: usize = 64;
/// What a delete preview's payload names as its operation and its effect.
const DELETE_OPERATION: &str = "delete_sample_workspace";
const DELETE_EFFECT: &str = "irreversible_live_unaffected";

/// What preparing the sample data did (§7).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedSample {
    /// A complete, current sample was already there and is used as it is.
    Reused(SeedManifest),
    /// The sample was missing, incomplete or from an older seed, and was
    /// built beside it and swapped in.
    Built(SeedReport),
}

#[derive(Debug)]
pub enum SampleError {
    Seed(SeedError),
    Control(SampleControlError),
    /// §8: reset and delete are offered only from Live, and the sample is
    /// never prepared while it is the workspace that is open.
    SampleIsOpen,
    /// Another reset or delete is recorded and not finished.
    OperationInProgress,
    /// An operation id or intent id that is not a plain token.
    InvalidIdentifier,
    /// There is no sample folder to delete.
    NothingToDelete,
    /// No delete preview with this id is waiting.
    NotPrepared,
    Expired,
    /// The acknowledged payload is not this preview's.
    PayloadMismatch,
    /// Something the delete preview bound changed; nothing was deleted.
    Changed,
    /// The step happened (or, for a preview, was taken back out) but its
    /// audit event could not be written (ADR 0012).
    NotRecorded,
}

impl std::fmt::Display for SampleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Seed(error) => write!(formatter, "sample data: {error}"),
            Self::Control(error) => write!(formatter, "sample operation record: {error}"),
            Self::SampleIsOpen => formatter.write_str("the sample workspace is the one open"),
            Self::OperationInProgress => {
                formatter.write_str("another sample reset or delete is not finished")
            }
            Self::InvalidIdentifier => formatter.write_str("not a valid operation identifier"),
            Self::NothingToDelete => formatter.write_str("there is no sample data to delete"),
            Self::NotPrepared => formatter.write_str("no delete preview with this id is waiting"),
            Self::Expired => formatter.write_str("the delete preview has expired"),
            Self::PayloadMismatch => formatter.write_str("the acknowledged preview differs"),
            Self::Changed => formatter.write_str("the sample changed since the preview"),
            Self::NotRecorded => formatter.write_str("the audit event could not be written"),
        }
    }
}

impl std::error::Error for SampleError {}

impl From<SeedError> for SampleError {
    fn from(error: SeedError) -> Self {
        Self::Seed(error)
    }
}

impl From<SampleControlError> for SampleError {
    fn from(error: SampleControlError) -> Self {
        Self::Control(error)
    }
}

impl From<WorkspaceError> for SampleError {
    fn from(error: WorkspaceError) -> Self {
        Self::Seed(SeedError::Workspace(error))
    }
}

impl From<io::Error> for SampleError {
    fn from(error: io::Error) -> Self {
        Self::Seed(SeedError::Io(error))
    }
}

/// The exact facts a delete asks the person to approve (§8).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SampleDeletePreview {
    pub prepared_intent_id: String,
    pub payload_sha256: String,
    pub seed_id: String,
    pub seed_version: u32,
    pub has_ledger: bool,
    pub has_vault: bool,
    pub has_generated_files: bool,
    pub settings_revision: u64,
    /// Always `delete_sample_workspace`.
    pub operation: String,
    /// Which sample: a digest of its location, never the path.
    pub workspace_identity_sha256: String,
    /// Every directory and file bound, as one digest.
    pub inventory_sha256: String,
    /// Always `irreversible_live_unaffected`.
    pub effect: String,
    pub expires_at_millis: i64,
}

/// An approval of a delete preview. The host has already checked the typed
/// phrase; the service checks everything the preview bound and issues the
/// approval receipt itself.
#[derive(Clone, Copy, Debug)]
pub struct ApproveSampleDelete<'a> {
    pub prepared_intent_id: &'a str,
    pub payload_sha256: &'a str,
    pub settings_revision: u64,
    pub idempotency_id: &'a str,
}

/// The sample workspace of one profile, and the one place its folder is
/// replaced or removed.
pub struct SampleWorkspace {
    identity: WorkspaceIdentity,
    control: SampleControlStore,
    audit: HostAuditLog,
}

impl SampleWorkspace {
    /// The profile's sample. Only Training can be one: the seed permit is
    /// minted by Training's policy (Live refuses) and accepted against this
    /// exact identity before anything else can happen.
    pub fn new(protected_root: &ProtectedSettingsRoot) -> Result<Self, SampleError> {
        let identity = WorkspaceIdentity::resolve(protected_root, WorkspaceKind::Training)?;
        let permit = identity.synthetic_seed_policy().authorize()?;
        identity.accept_synthetic_seed_permit(permit)?;
        Ok(Self {
            identity,
            control: SampleControlStore::new(protected_root.path().join(CONTROL_FILE_NAME)),
            audit: HostAuditLog::new(protected_root.path().join(AUDIT_FILE_NAME)),
        })
    }

    /// The operation record as it is.
    pub fn control(&self) -> Result<SampleControl, SampleError> {
        Ok(self.control.load()?)
    }

    /// What the sample folder holds now, touching nothing.
    pub fn inspect(&self) -> Result<SampleState, SampleError> {
        Ok(inspect_folder(
            self.canonical(),
            &self.vault_in(self.canonical()),
        )?)
    }

    /// §7: reuse a complete current sample; build a missing, incomplete or
    /// older one beside it and swap it in; refuse anything PMC did not write
    /// with nothing deleted. `open` is the workspace open now — `None`
    /// before any is (first run). Never while the sample is open.
    pub fn prepare(
        &self,
        open: Option<WorkspaceKind>,
        operation_id: &str,
        now: UtcTimestamp,
    ) -> Result<PreparedSample, SampleError> {
        if open == Some(WorkspaceKind::Training) {
            return Err(SampleError::SampleIsOpen);
        }
        schema_supported()?;
        if self.control.load()?.active.is_some() {
            return Err(SampleError::OperationInProgress);
        }
        match self.inspect()? {
            SampleState::Current(manifest) => Ok(PreparedSample::Reused(manifest)),
            SampleState::Foreign(what) => Err(SampleError::Seed(SeedError::ForeignContents(what))),
            SampleState::Absent | SampleState::Outdated(_) => {
                self.replace(operation_id, now).map(PreparedSample::Built)
            }
        }
    }

    /// §8 H1: the sample returns to its starting state. From Live only.
    pub fn reset(
        &self,
        open: WorkspaceKind,
        operation_id: &str,
        now: UtcTimestamp,
    ) -> Result<SeedReport, SampleError> {
        if open != WorkspaceKind::Live {
            return Err(SampleError::SampleIsOpen);
        }
        schema_supported()?;
        if let SampleState::Foreign(what) = self.inspect()? {
            return Err(SampleError::Seed(SeedError::ForeignContents(what)));
        }
        self.replace(operation_id, now)
    }

    /// Build a fresh sample beside the current one and swap it in.
    fn replace(&self, operation_id: &str, now: UtcTimestamp) -> Result<SeedReport, SampleError> {
        check_token(operation_id)?;
        let canonical = self.canonical().to_path_buf();
        // A repeated request after a reset finished answers with the sample
        // it left, never a second reset; an id already used for anything
        // else is refused.
        if let Some(terminal) = self.control.load()?.terminal_of(operation_id) {
            return if terminal.outcome == SampleOutcome::Reset
                && terminal.prepared_intent_id.is_none()
            {
                current_report(&canonical)
            } else {
                Err(SampleError::InvalidIdentifier)
            };
        }
        self.control.update(|control| match control.active {
            Some(_) => Err(SampleError::OperationInProgress),
            None => {
                control.active = Some(SampleOperation::Reset {
                    operation_id: operation_id.to_owned(),
                    phase: ResetPhase::Building,
                    started_at_millis: now.unix_millis(),
                    built_manifest_sha256: None,
                });
                Ok(())
            }
        })??;

        let staging = self.sibling(STAGING, operation_id);
        let retired = self.sibling(RETIRED, operation_id);
        // A folder already there under this name is not ours: refuse, and
        // leave it exactly as it was.
        if let Err(error) = create_staging(&staging) {
            self.finish(
                operation_id,
                None,
                None,
                SampleOutcome::ResetRolledBack,
                now,
            )?;
            return Err(error);
        }
        let manifest = match self.build_staging(&staging) {
            Ok(manifest) => manifest,
            Err(error) => {
                self.cleanup(&staging);
                self.finish(
                    operation_id,
                    None,
                    None,
                    SampleOutcome::ResetRolledBack,
                    now,
                )?;
                return Err(error);
            }
        };
        self.record_built(operation_id, &manifest)?;
        let had_sample = canonical.exists();
        if had_sample {
            // Looked at again right before it moves: something PMC did not
            // write that appeared while the fresh sample was being built is
            // never renamed away with the old one.
            if let SampleState::Foreign(what) = self.inspect()? {
                self.cleanup(&staging);
                self.finish(
                    operation_id,
                    None,
                    None,
                    SampleOutcome::ResetRolledBack,
                    now,
                )?;
                return Err(SampleError::Seed(SeedError::ForeignContents(what)));
            }
            if let Err(error) = fs::rename(&canonical, &retired) {
                self.cleanup(&staging);
                self.finish(
                    operation_id,
                    None,
                    None,
                    SampleOutcome::ResetRolledBack,
                    now,
                )?;
                return Err(error.into());
            }
        }
        self.set_reset_phase(operation_id, ResetPhase::SwappedAside)?;
        let installed = fs::rename(&staging, &canonical)
            .map_err(SampleError::from)
            .and_then(|()| match self.inspect()? {
                // Renamed into place *and* verified there (§8).
                SampleState::Current(installed) if installed == manifest => Ok(()),
                _ => Err(SampleError::Seed(SeedError::Manifest(
                    "the reset sample did not verify in place".to_owned(),
                ))),
            });
        if let Err(error) = installed {
            // Put the previous sample back. If that cannot be done now, the
            // operation stays recorded and the next start finishes it — the
            // old sample is never treated as leftover.
            self.roll_back_install(&canonical, &staging, &retired, true, had_sample)?;
            self.finish(
                operation_id,
                None,
                None,
                SampleOutcome::ResetRolledBack,
                now,
            )?;
            return Err(error);
        }
        self.finish(operation_id, None, None, SampleOutcome::Reset, now)?;
        if had_sample {
            self.cleanup_retired(&retired);
        }
        Ok(SeedReport {
            training_root: canonical,
            manifest,
        })
    }

    /// Undo a reset whose fresh sample is not (or not verifiably) in place:
    /// the fresh copy goes back to staging, the previous sample back into
    /// place, and the staging copy is removed. An error leaves the operation
    /// active for the next start.
    ///
    /// `swapped`: the second rename may have happened (the phase reached
    /// `SwappedAside`). Only then can the folder in place be the fresh copy;
    /// before it, the folder in place is the previous sample and is never
    /// moved.
    fn roll_back_install(
        &self,
        canonical: &Path,
        staging: &Path,
        retired: &Path,
        swapped: bool,
        had_sample: bool,
    ) -> Result<(), SampleError> {
        if swapped && canonical.exists() && !staging.exists() && (retired.exists() || !had_sample) {
            fs::rename(canonical, staging)?;
        }
        if !canonical.exists() && retired.exists() {
            fs::rename(retired, canonical)?;
        }
        self.cleanup(staging);
        Ok(())
    }

    /// A fresh sample in `staging`, verified exactly as the sample itself
    /// is before it may replace it.
    fn build_staging(&self, staging: &Path) -> Result<SeedManifest, SampleError> {
        let report = write_seed_at(staging, &self.vault_in(staging))?;
        match inspect_folder(staging, &self.vault_in(staging))? {
            SampleState::Current(manifest) if manifest == report.manifest => Ok(manifest),
            _ => Err(SampleError::Seed(SeedError::Manifest(
                "the sample built beside the current one did not verify".to_owned(),
            ))),
        }
    }

    /// §8 H2b, step 1: bind what exists now and record the Prepared Intent.
    /// Nothing is changed. From Live only.
    pub fn prepare_delete(
        &self,
        open: WorkspaceKind,
        prepared_intent_id: &str,
        settings_revision: u64,
        valid_for_millis: i64,
        now: UtcTimestamp,
    ) -> Result<SampleDeletePreview, SampleError> {
        if open != WorkspaceKind::Live {
            return Err(SampleError::SampleIsOpen);
        }
        check_token(prepared_intent_id)?;
        let binding = self.binding(settings_revision)?;
        let expires_at_millis = now.unix_millis().saturating_add(valid_for_millis);
        let prepared = PreparedSampleDelete {
            prepared_intent_id: prepared_intent_id.to_owned(),
            payload_sha256: payload_digest(prepared_intent_id, &binding, expires_at_millis)?,
            binding,
            prepared_at_millis: now.unix_millis(),
            expires_at_millis,
        };
        self.control.update(|control| match control.active {
            Some(_) => Err(SampleError::OperationInProgress),
            None => {
                control.active = Some(SampleOperation::DeletePrepared {
                    prepared: prepared.clone(),
                });
                Ok(())
            }
        })??;
        if self
            .record(
                SAMPLE_DELETE_PREPARED,
                AuditOutcome::Succeeded,
                vec![
                    (
                        "prepared_intent_id".to_owned(),
                        prepared_intent_id.to_owned(),
                    ),
                    (
                        "seed_version".to_owned(),
                        prepared.binding.seed_version.to_string(),
                    ),
                ],
                now,
            )
            .is_err()
        {
            // Not recorded, so it does not stand: take it back out, or it
            // would block every later operation while no screen holds it.
            self.discard(prepared_intent_id)?;
            return Err(SampleError::NotRecorded);
        }
        Ok(SampleDeletePreview {
            prepared_intent_id: prepared.prepared_intent_id,
            payload_sha256: prepared.payload_sha256,
            seed_id: prepared.binding.seed_id,
            seed_version: prepared.binding.seed_version,
            has_ledger: prepared.binding.has_ledger,
            has_vault: prepared.binding.has_vault,
            has_generated_files: prepared.binding.has_generated_files,
            settings_revision: prepared.binding.settings_revision,
            operation: prepared.binding.operation,
            workspace_identity_sha256: prepared.binding.workspace_identity_sha256,
            inventory_sha256: prepared.binding.inventory_sha256,
            effect: prepared.binding.effect,
            expires_at_millis: prepared.expires_at_millis,
        })
    }

    /// §8 H2b reject: the preview is discarded, nothing changes, and the
    /// choice is recorded.
    pub fn reject_delete(
        &self,
        prepared_intent_id: &str,
        now: UtcTimestamp,
    ) -> Result<(), SampleError> {
        self.control.update(|control| match &control.active {
            Some(SampleOperation::DeletePrepared { prepared })
                if prepared.prepared_intent_id == prepared_intent_id =>
            {
                control.active = None;
                Ok(())
            }
            _ => Err(SampleError::NotPrepared),
        })??;
        self.record_refusal(prepared_intent_id, "rejected", now)
    }

    /// §8 H2b approve: every bound fact is derived again and compared whole;
    /// the approval receipt is issued from this preview and this request and
    /// recorded as the intent is consumed; only then is the folder renamed
    /// aside and removed. A repeated approval of the same preview answers
    /// with the outcome the first one reached.
    pub fn approve_delete(
        &self,
        open: WorkspaceKind,
        input: &ApproveSampleDelete<'_>,
        now: UtcTimestamp,
    ) -> Result<SampleOutcome, SampleError> {
        if open != WorkspaceKind::Live {
            return Err(SampleError::SampleIsOpen);
        }
        check_token(input.idempotency_id)?;
        let control = self.control.load()?;
        if let Some(terminal) = control.terminal_of(input.idempotency_id) {
            // Only the same preview's approval gets the recorded answer.
            if terminal.prepared_intent_id.as_deref() != Some(input.prepared_intent_id) {
                return Err(SampleError::InvalidIdentifier);
            }
            let repeated = receipt_digest(
                input.prepared_intent_id,
                input.payload_sha256,
                input.idempotency_id,
            );
            return if terminal.receipt_id.as_deref() == Some(repeated.as_str()) {
                Ok(terminal.outcome)
            } else {
                Err(SampleError::PayloadMismatch)
            };
        }
        let prepared = match control.active {
            Some(SampleOperation::DeletePrepared { prepared })
                if prepared.prepared_intent_id == input.prepared_intent_id =>
            {
                prepared
            }
            Some(SampleOperation::Deleting { .. }) => return Err(SampleError::OperationInProgress),
            _ => return Err(SampleError::NotPrepared),
        };
        if now.unix_millis() > prepared.expires_at_millis {
            self.discard(input.prepared_intent_id)?;
            self.record_refusal(input.prepared_intent_id, "expired", now)?;
            return Err(SampleError::Expired);
        }
        if prepared.payload_sha256 != input.payload_sha256 {
            return Err(SampleError::PayloadMismatch);
        }
        // Re-derived whole, never a single flag: any change to the files, the
        // seed, the settings or which sample this is refuses.
        let now_bound = self.binding(input.settings_revision);
        if !matches!(&now_bound, Ok(binding) if *binding == prepared.binding) {
            self.discard(input.prepared_intent_id)?;
            self.record_refusal(input.prepared_intent_id, "changed", now)?;
            return Err(SampleError::Changed);
        }
        let receipt_id = receipt_digest(
            input.prepared_intent_id,
            &prepared.payload_sha256,
            input.idempotency_id,
        );
        // Consuming the intent and issuing the receipt are one write: the
        // same preview can never be approved twice.
        self.control.update(|control| match &control.active {
            Some(SampleOperation::DeletePrepared { prepared: waiting })
                if waiting.prepared_intent_id == input.prepared_intent_id =>
            {
                control.active = Some(SampleOperation::Deleting {
                    prepared: waiting.clone(),
                    idempotency_id: input.idempotency_id.to_owned(),
                    receipt_id: receipt_id.clone(),
                    phase: DeletePhase::Approved,
                });
                Ok(())
            }
            _ => Err(SampleError::NotPrepared),
        })??;

        let tombstone = self.sibling(DELETED, input.idempotency_id);
        if let Err(error) = fs::rename(self.canonical(), &tombstone) {
            self.finish(
                input.idempotency_id,
                Some(input.prepared_intent_id),
                Some(&receipt_id),
                SampleOutcome::NotDeleted,
                now,
            )?;
            return Err(error.into());
        }
        self.control.update(|control| -> Result<(), SampleError> {
            if let Some(SampleOperation::Deleting { phase, .. }) = &mut control.active {
                *phase = DeletePhase::RenamedAside;
            }
            Ok(())
        })??;
        self.finish(
            input.idempotency_id,
            Some(input.prepared_intent_id),
            Some(&receipt_id),
            SampleOutcome::Deleted,
            now,
        )?;
        // Gone from where PMC looks the moment the rename landed; a removal
        // that cannot finish now is kept for the next start and reported.
        self.cleanup(&tombstone);
        Ok(SampleOutcome::Deleted)
    }

    /// At startup, before the workspace is chosen: finish or roll back an
    /// interrupted reset or delete from what is on disk, discard a delete
    /// that was only prepared, and remove the folders earlier operations
    /// recorded as theirs to remove. An `Err` leaves the operation recorded;
    /// the host keeps the sample unavailable and says why.
    pub fn reconcile(&self, now: UtcTimestamp) -> Result<Option<SampleOutcome>, SampleError> {
        let control = self.control.load()?;
        let outcome = match control.active {
            None => None,
            Some(SampleOperation::DeletePrepared { prepared }) => {
                self.discard(&prepared.prepared_intent_id)?;
                self.record_refusal(&prepared.prepared_intent_id, "discarded_at_startup", now)?;
                None
            }
            Some(SampleOperation::Reset {
                operation_id,
                phase,
                built_manifest_sha256,
                ..
            }) => {
                let canonical = self.canonical().to_path_buf();
                let staging = self.sibling(STAGING, &operation_id);
                let retired = self.sibling(RETIRED, &operation_id);
                // The fresh sample is in place exactly when the last phase was
                // reached, the staging folder is gone, and the sample there
                // verifies. Anything short of that rolls back to the sample
                // that was there before (or to none, if there was none).
                let installed = phase == ResetPhase::SwappedAside
                    && !staging.exists()
                    && match self.inspect()? {
                        SampleState::Current(manifest) => {
                            built_manifest_sha256.is_some()
                                && built_manifest_sha256 == Some(manifest_digest(&manifest)?)
                        }
                        _ => false,
                    };
                let outcome = if installed {
                    SampleOutcome::Reset
                } else {
                    let had_sample = retired.exists();
                    self.roll_back_install(
                        &canonical,
                        &staging,
                        &retired,
                        phase == ResetPhase::SwappedAside,
                        had_sample,
                    )?;
                    SampleOutcome::ResetRolledBack
                };
                self.finish(&operation_id, None, None, outcome, now)?;
                if installed {
                    self.cleanup_retired(&retired);
                }
                Some(outcome)
            }
            Some(SampleOperation::Deleting {
                prepared,
                idempotency_id,
                receipt_id,
                ..
            }) => {
                let tombstone = self.sibling(DELETED, &idempotency_id);
                let renamed = tombstone.exists() || !self.canonical().exists();
                let outcome = if renamed {
                    SampleOutcome::Deleted
                } else {
                    // Approved, but the rename never happened.
                    SampleOutcome::NotDeleted
                };
                self.finish(
                    &idempotency_id,
                    Some(&prepared.prepared_intent_id),
                    Some(&receipt_id),
                    outcome,
                    now,
                )?;
                if renamed {
                    self.cleanup(&tombstone);
                }
                Some(outcome)
            }
        };
        self.sweep();
        Ok(outcome)
    }

    /// Folders an earlier operation recorded as its own to remove and could
    /// not remove yet. Nothing else is ever touched here.
    #[must_use]
    pub fn pending_cleanup(&self) -> Vec<String> {
        self.control
            .load()
            .map(|control| control.pending_cleanup)
            .unwrap_or_default()
    }

    /// Remove exactly the folders recorded in `pending_cleanup`; one that
    /// still cannot be removed stays recorded.
    fn sweep(&self) {
        let Some(parent) = self.canonical().parent().map(Path::to_path_buf) else {
            return;
        };
        for name in self.pending_cleanup() {
            if remove_tree(&parent.join(&name)).is_ok() {
                let _ = self.control.update(|control| -> Result<(), SampleError> {
                    control.pending_cleanup.retain(|pending| *pending != name);
                    Ok(())
                });
            }
        }
    }

    /// Remove a sibling folder this service created; if it cannot be removed
    /// now, record it for the next start rather than forget it.
    fn cleanup(&self, folder: &Path) {
        if remove_tree(folder).is_err() && folder.exists() {
            if let Some(name) = folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            {
                let _ = self.control.update(|control| -> Result<(), SampleError> {
                    if !control.pending_cleanup.contains(&name) {
                        control.pending_cleanup.push(name);
                    }
                    Ok(())
                });
            }
        }
    }

    /// The previous sample, once the fresh one is in place: removed if it is
    /// still only PMC's; something PMC did not write that got into it is
    /// kept where it is.
    fn cleanup_retired(&self, retired: &Path) {
        if matches!(
            inspect_folder(retired, &self.vault_in(retired)),
            Ok(SampleState::Foreign(_))
        ) {
            return;
        }
        self.cleanup(retired);
    }

    fn binding(&self, settings_revision: u64) -> Result<SampleDeleteBinding, SampleError> {
        let root = self.canonical();
        match self.inspect()? {
            SampleState::Absent => return Err(SampleError::NothingToDelete),
            SampleState::Foreign(what) => {
                return Err(SampleError::Seed(SeedError::ForeignContents(what)))
            }
            SampleState::Current(_) | SampleState::Outdated(_) => {}
        }
        let manifest = read_manifest(root).ok().flatten();
        Ok(SampleDeleteBinding {
            operation: DELETE_OPERATION.to_owned(),
            workspace_identity_sha256: hex(&Sha256::digest(root.to_string_lossy().as_bytes())),
            effect: DELETE_EFFECT.to_owned(),
            seed_id: manifest
                .as_ref()
                .map(|manifest| manifest.seed_id.clone())
                .unwrap_or_default(),
            seed_version: manifest
                .as_ref()
                .map_or(0, |manifest| manifest.seed_version),
            inventory_sha256: inventory_sha256(root)?,
            has_ledger: root.join(LEDGER_FILE_NAME).is_file(),
            has_vault: self.vault_in(root).is_dir(),
            has_generated_files: root.join(MANIFEST_FILE_NAME).is_file(),
            settings_revision,
        })
    }

    /// `Built`, with the manifest of the sample that was verified.
    fn record_built(&self, operation_id: &str, manifest: &SeedManifest) -> Result<(), SampleError> {
        let digest = manifest_digest(manifest)?;
        self.control.update(|control| -> Result<(), SampleError> {
            if let Some(SampleOperation::Reset {
                operation_id: active,
                phase,
                built_manifest_sha256,
                ..
            }) = &mut control.active
            {
                if active == operation_id {
                    *phase = ResetPhase::Built;
                    *built_manifest_sha256 = Some(digest);
                }
            }
            Ok(())
        })??;
        Ok(())
    }

    fn set_reset_phase(&self, operation_id: &str, next: ResetPhase) -> Result<(), SampleError> {
        self.control.update(|control| -> Result<(), SampleError> {
            if let Some(SampleOperation::Reset {
                operation_id: active,
                phase,
                ..
            }) = &mut control.active
            {
                if active == operation_id {
                    *phase = next;
                }
            }
            Ok(())
        })??;
        Ok(())
    }

    fn discard(&self, prepared_intent_id: &str) -> Result<(), SampleError> {
        self.control.update(|control| -> Result<(), SampleError> {
            if matches!(&control.active, Some(SampleOperation::DeletePrepared { prepared })
                if prepared.prepared_intent_id == prepared_intent_id)
            {
                control.active = None;
            }
            Ok(())
        })??;
        Ok(())
    }

    fn finish(
        &self,
        operation_id: &str,
        prepared_intent_id: Option<&str>,
        receipt_id: Option<&str>,
        outcome: SampleOutcome,
        now: UtcTimestamp,
    ) -> Result<(), SampleError> {
        self.control.update(|control| -> Result<(), SampleError> {
            control.finish(SampleTerminal {
                operation_id: operation_id.to_owned(),
                prepared_intent_id: prepared_intent_id.map(ToOwned::to_owned),
                receipt_id: receipt_id.map(ToOwned::to_owned),
                outcome,
                completed_at_millis: now.unix_millis(),
            });
            Ok(())
        })??;
        let (code, audit_outcome) = match outcome {
            SampleOutcome::Reset => (SAMPLE_RESET, AuditOutcome::Succeeded),
            SampleOutcome::ResetRolledBack => (SAMPLE_RESET_ROLLED_BACK, AuditOutcome::Failed),
            SampleOutcome::Deleted => (SAMPLE_DELETED, AuditOutcome::Succeeded),
            SampleOutcome::NotDeleted => (SAMPLE_NOT_DELETED, AuditOutcome::Failed),
        };
        let mut facts = vec![("operation_id".to_owned(), operation_id.to_owned())];
        if let Some(intent) = prepared_intent_id {
            facts.push(("prepared_intent_id".to_owned(), intent.to_owned()));
        }
        self.record(code, audit_outcome, facts, now)
    }

    /// A delete preview that ended without deleting: rejected, expired, a
    /// bound fact that changed, or discarded at startup (§8).
    fn record_refusal(
        &self,
        prepared_intent_id: &str,
        reason: &str,
        now: UtcTimestamp,
    ) -> Result<(), SampleError> {
        self.record(
            SAMPLE_DELETE_REJECTED,
            AuditOutcome::Refused,
            vec![
                (
                    "prepared_intent_id".to_owned(),
                    prepared_intent_id.to_owned(),
                ),
                ("reason".to_owned(), reason.to_owned()),
            ],
            now,
        )
    }

    fn record(
        &self,
        code: &str,
        outcome: AuditOutcome,
        facts: Vec<(String, String)>,
        now: UtcTimestamp,
    ) -> Result<(), SampleError> {
        let event_id = format!(
            "sample-{}-{}",
            now.unix_millis(),
            hex(&Sha256::digest(format!("{code}{facts:?}").as_bytes()))[..12].to_owned()
        );
        self.audit
            .append(&HostAuditEvent::new(
                event_id,
                now.unix_millis(),
                AuditWorkspace::Training,
                code,
                outcome,
                facts,
            ))
            .map_err(|_| SampleError::NotRecorded)
    }

    fn canonical(&self) -> &Path {
        self.identity.root().as_path()
    }

    /// The synthetic Vault inside a sample folder, named as the workspace
    /// identity names it — never a second literal.
    fn vault_in(&self, folder: &Path) -> PathBuf {
        let name = self
            .identity
            .synthetic_vault_root()
            .and_then(|vault| vault.file_name().map(ToOwned::to_owned))
            .unwrap_or_default();
        folder.join(name)
    }

    /// `workspaces/training.<kind>-<token>`: a sibling of the sample, in the
    /// same application-derived directory.
    fn sibling(&self, kind: &str, token: &str) -> PathBuf {
        let base = self
            .canonical()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.canonical()
            .with_file_name(format!("{base}.{kind}-{token}"))
    }
}

/// Create the staging folder, and only a new one: `create_dir`, never
/// `create_dir_all`, so a folder or link already under this name is refused
/// and left alone.
fn create_staging(staging: &Path) -> Result<(), SampleError> {
    if let Some(parent) = staging.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(staging)?;
    Ok(())
}

/// A plain token naming one operation, derived from a caller's request id:
/// a domain-separated digest, so a retry of the same request names the same
/// operation and two kinds of operation never collide. For the host, which
/// names operations itself rather than taking a name from the webview.
#[must_use]
pub fn request_token(domain: &str, client_request_id: &str) -> String {
    hex(&Sha256::digest(
        format!("{domain}\n{client_request_id}").as_bytes(),
    ))[..32]
        .to_owned()
}

fn schema_supported() -> Result<(), SampleError> {
    if CURRENT_SCHEMA_VERSION == SUPPORTED_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(SampleError::Seed(SeedError::SchemaMismatch {
            supported: SUPPORTED_SCHEMA_VERSION,
            current: CURRENT_SCHEMA_VERSION,
        }))
    }
}

/// Operation and intent ids become folder names: plain tokens only.
fn check_token(token: &str) -> Result<(), SampleError> {
    let plain = !token.is_empty()
        && token.len() <= MAX_OPERATION_ID_LENGTH
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if plain {
        Ok(())
    } else {
        Err(SampleError::InvalidIdentifier)
    }
}

fn current_report(canonical: &Path) -> Result<SeedReport, SampleError> {
    let manifest = read_manifest(canonical)?.ok_or(SampleError::Seed(SeedError::Manifest(
        "the reset sample has no manifest".to_owned(),
    )))?;
    Ok(SeedReport {
        training_root: canonical.to_path_buf(),
        manifest,
    })
}

#[derive(Serialize)]
struct DeletePayload<'a> {
    format: &'static str,
    prepared_intent_id: &'a str,
    binding: &'a SampleDeleteBinding,
    expires_at_millis: i64,
}

fn payload_digest(
    prepared_intent_id: &str,
    binding: &SampleDeleteBinding,
    expires_at_millis: i64,
) -> Result<String, SampleError> {
    let bytes = serde_json::to_vec(&DeletePayload {
        format: "pmc-sample-delete/v1",
        prepared_intent_id,
        binding,
        expires_at_millis,
    })
    .map_err(|error| SampleError::Seed(SeedError::Manifest(error.to_string())))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

/// The approval receipt, issued by this service from the preview it
/// consumes and the request that consumed it.
fn receipt_digest(prepared_intent_id: &str, payload_sha256: &str, idempotency_id: &str) -> String {
    hex(&Sha256::digest(
        format!("pmc-sample-delete-receipt/v1\n{prepared_intent_id}\n{payload_sha256}\n{idempotency_id}")
            .as_bytes(),
    ))
}

fn manifest_digest(manifest: &SeedManifest) -> Result<String, SampleError> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| SampleError::Seed(SeedError::Manifest(error.to_string())))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
