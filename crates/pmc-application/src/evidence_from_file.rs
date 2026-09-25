//! Evidence from a file (item ⑦-3; DG3 Vault-root and Evidence-from-file
//! amendment §4, accepted 2026-09-23): the person picks a file inside the
//! Live Vault, sees what PMC observed, and creates an Evidence reference for
//! it — pinned and verified from one observation taken at create time.
//!
//! The host holds the picked file behind an opaque token and never names a
//! path (ADR 0011); everything that touches a path lives here and in
//! `pmc-platform`. Three steps:
//!
//! 1. [`observe_chosen_file`] — the file is inside the Vault root (component
//!    containment, no link or reparse point, a regular file), named by its
//!    Vault-relative path as Windows stores it, and fingerprinted. That
//!    observation is what the sheet shows.
//! 2. [`preview_chosen_file`] — whether a reference already names this file
//!    (then the sheet offers that one instead of a second) and which
//!    references hold the same content elsewhere (a warning). Guidance only.
//! 3. [`create_evidence_from_file`] — under the same Vault root the file was
//!    chosen under: reserve the id (a retry finds the reference its first
//!    attempt created and returns it); observe the file again, and if the
//!    bytes changed, stop and hand back the new observation for the person
//!    to confirm; then create through the Ledger's reservation-checked
//!    writer, which re-checks "one reference per file" under its write lock.
//!
//! "The same file" is the caller's `same_name` comparison — Windows' own,
//! from `pmc-platform`, in the desktop app.
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use pmc_domain::audit::AuditEventIdSource;
use pmc_domain::classification::DataClassification;
use pmc_domain::evidence::{
    EvidenceFingerprint, EvidenceReferenceRecord, FingerprintAlgorithm, OperationContext,
    VaultRelativePath,
};
use pmc_domain::identity::EvidenceReferenceId;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::IntegrityDigest;
use pmc_ledger::sqlite::{
    EvidenceMatch, LedgerOpenError, ReservationRequest, ReservedEntityKind, ReservedId,
    SqliteProductLedger,
};
use pmc_platform::filesystem::compute_sha256_fingerprint;
use pmc_platform::paths::{resolve_contained_path, vault_relative_file};

use crate::desktop_runtime::OpaqueIdSource;
use crate::evidence_writes::{DesktopVault, VaultUnavailable};
use crate::record_entry::{reserve_record_id, RecordEntryError};

/// The reservation every Evidence-from-a-file create names (schema v48).
pub const CREATE_EVIDENCE_REFERENCE: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::EvidenceReference,
    operation: "create_evidence_reference",
};

/// One reading of a file's bytes: what they hash to, and when.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservation {
    pub fingerprint: EvidenceFingerprint,
    pub observed_at: UtcTimestamp,
}

/// A file the person chose, as the host holds it behind a token. Only
/// [`observe_chosen_file`] (and a changed-bytes refusal) make one, so its
/// path is always one that passed the Vault checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChosenEvidenceFile {
    /// The canonical Vault root it was chosen under.
    root: PathBuf,
    vault_path: VaultRelativePath,
    observation: FileObservation,
}

impl ChosenEvidenceFile {
    /// The file's own name — the only part of its path the sheet shows.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.vault_path
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or_else(|| self.vault_path.as_str())
    }

    #[must_use]
    pub const fn observation(&self) -> &FileObservation {
        &self.observation
    }

    #[must_use]
    pub const fn vault_path(&self) -> &VaultRelativePath {
        &self.vault_path
    }
}

/// Why no reference was created. Nothing was written unless the variant
/// says so.
#[derive(Debug)]
pub enum EvidenceFromFileError {
    /// No usable Vault right now.
    Vault(VaultUnavailable),
    /// The file is not a regular file inside the Vault, or is reached
    /// through a link (§5 "the chosen file is outside the Vault").
    OutsideVault,
    /// The file could not be read (§5).
    Unreadable,
    /// The Vault folder is not the one the file was chosen under: choose it
    /// again rather than reading an old choice under a new folder.
    VaultChanged,
    /// The bytes changed since they were shown (§4.2). Carries the new
    /// observation, for the person to confirm before creating.
    FileChanged(ChosenEvidenceFile),
    /// A reference already names this file (§4.5): a branch of the sheet,
    /// not an error — offer this one instead.
    AlreadyReferenced(EvidenceMatch),
    /// The Ledger read failed.
    Read(LedgerOpenError),
    /// The reservation or the create was refused or failed.
    Entry(RecordEntryError),
}

impl From<VaultUnavailable> for EvidenceFromFileError {
    fn from(reason: VaultUnavailable) -> Self {
        Self::Vault(reason)
    }
}

impl From<RecordEntryError> for EvidenceFromFileError {
    fn from(error: RecordEntryError) -> Self {
        Self::Entry(error)
    }
}

fn observe(path: &Path, now: UtcTimestamp) -> Result<FileObservation, EvidenceFromFileError> {
    let digest = compute_sha256_fingerprint(path).map_err(|_| EvidenceFromFileError::Unreadable)?;
    let digest = IntegrityDigest::parse(digest).map_err(|_| EvidenceFromFileError::Unreadable)?;
    Ok(FileObservation {
        fingerprint: EvidenceFingerprint::new(FingerprintAlgorithm::Sha256, digest),
        observed_at: now,
    })
}

/// Step 1: the file the host's picker returned, checked against the Vault
/// as it is now and observed.
pub fn observe_chosen_file(
    vault: &DesktopVault,
    picked: &Path,
    now: UtcTimestamp,
) -> Result<ChosenEvidenceFile, EvidenceFromFileError> {
    let root = vault.root()?;
    let relative = vault_relative_file(root.as_path(), picked)
        .map_err(|_| EvidenceFromFileError::OutsideVault)?;
    let vault_path =
        VaultRelativePath::parse(relative).map_err(|_| EvidenceFromFileError::OutsideVault)?;
    let resolved = resolve_contained_path(root.as_path(), Path::new(vault_path.as_str()))
        .map_err(|_| EvidenceFromFileError::OutsideVault)?;
    Ok(ChosenEvidenceFile {
        root: root.as_path().to_path_buf(),
        observation: observe(&resolved, now)?,
        vault_path,
    })
}

/// What the sheet shows beside the observation (§4.5). Ids and versions
/// only: never another reference's path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChosenFilePreview {
    /// The reference that already names this file, if any.
    pub existing: Option<EvidenceMatch>,
    /// References holding the same content under another path.
    pub same_content: Vec<EvidenceMatch>,
}

/// Step 2: guidance for the sheet. The create checks again; this read
/// authorizes nothing.
pub fn preview_chosen_file(
    ledger: &SqliteProductLedger,
    chosen: &ChosenEvidenceFile,
    same_name: fn(&str, &str) -> bool,
) -> Result<ChosenFilePreview, EvidenceFromFileError> {
    let existing = ledger
        .find_evidence_at_vault_path(&chosen.vault_path, same_name)
        .map_err(EvidenceFromFileError::Read)?
        .into_iter()
        .next();
    let same_content = ledger
        .find_evidence_with_fingerprint(&chosen.observation.fingerprint)
        .map_err(EvidenceFromFileError::Read)?
        .into_iter()
        .filter(|found| existing.as_ref().is_none_or(|here| here.id != found.id))
        .collect();
    Ok(ChosenFilePreview {
        existing,
        same_content,
    })
}

/// Step 3: create the reference (H1-User). A retry of the same request
/// returns what the first attempt created — whatever the file holds now —
/// and nothing is created twice.
#[allow(clippy::too_many_arguments)]
pub fn create_evidence_from_file(
    ledger: &mut SqliteProductLedger,
    vault: &DesktopVault,
    chosen: &ChosenEvidenceFile,
    classification: DataClassification,
    context: OperationContext,
    ids: &mut OpaqueIdSource,
    now: UtcTimestamp,
    same_name: fn(&str, &str) -> bool,
) -> Result<EvidenceReferenceRecord, EvidenceFromFileError> {
    let root = vault.root()?;
    if root.as_path() != chosen.root {
        return Err(EvidenceFromFileError::VaultChanged);
    }
    // The id first: a retry must find what its first attempt created before
    // it reads the file again. A new observation carries a new instant, and
    // the Ledger refuses a replay whose command differs from the original.
    let reserved: ReservedId<EvidenceReferenceId> = reserve_record_id(
        ledger,
        &context.idempotency_id,
        CREATE_EVIDENCE_REFERENCE,
        now,
        || ids.next_evidence_reference_id(),
    )?;
    if reserved.replayed() {
        let created = ledger
            .get_evidence_reference(reserved.id(), context.correlation_id.clone())
            .map_err(|error| EvidenceFromFileError::Entry(RecordEntryError::Domain(error)))?;
        if let Some(record) = created {
            // Only the reserved create can use this id; it named this file.
            if record.vault_path != chosen.vault_path {
                return Err(EvidenceFromFileError::Entry(
                    RecordEntryError::RequestReused,
                ));
            }
            return Ok(record);
        }
    }
    let resolved = resolve_contained_path(root.as_path(), Path::new(chosen.vault_path.as_str()))
        .map_err(|_| EvidenceFromFileError::OutsideVault)?;
    // The observation the reference records is this one, taken now.
    let observation = observe(&resolved, now)?;
    if observation.fingerprint != chosen.observation.fingerprint {
        return Err(EvidenceFromFileError::FileChanged(ChosenEvidenceFile {
            root: chosen.root.clone(),
            vault_path: chosen.vault_path.clone(),
            observation,
        }));
    }
    // Guidance before the write; the writer checks again under its lock.
    if let Some(existing) = ledger
        .find_evidence_at_vault_path(&chosen.vault_path, same_name)
        .map_err(EvidenceFromFileError::Read)?
        .into_iter()
        .next()
    {
        return Err(EvidenceFromFileError::AlreadyReferenced(existing));
    }
    let audit_event_id = ids
        .next_audit_event_id()
        .map_err(|error| EvidenceFromFileError::Entry(RecordEntryError::Id(error)))?;
    ledger
        .create_evidence_reference_from_reservation(
            &reserved,
            chosen.vault_path.clone(),
            observation.fingerprint,
            observation.observed_at,
            classification,
            context,
            audit_event_id,
            now,
            same_name,
        )
        .map(|outcome| outcome.record)
        .map_err(|error| EvidenceFromFileError::Entry(RecordEntryError::from(error)))
}
