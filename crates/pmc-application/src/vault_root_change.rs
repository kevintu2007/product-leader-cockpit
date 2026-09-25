//! Changing which folder the Live workspace reads Evidence from (item ⑦;
//! the accepted DG3 Vault-root amendment §3; DG0: "Change Product Vault /
//! Product Ledger authority path — H2b").
//!
//! The settings document is the committed pointer. Everything that must
//! survive a crash *around* that one write — the Prepared Intent, what it
//! bound, the approval receipt, the phase and the previous folder — lives in
//! the authority control record beside it, because a settings write cannot
//! be atomic with the backup that is its recovery evidence or with the audit
//! events that record it.
//!
//! What the preview binds, and re-checks at approval:
//!
//! - the settings revision it read, so a folder chosen elsewhere is not
//!   overwritten blind;
//! - the Ledger revision it read, so an Evidence created, pinned, relocated
//!   or superseded since invalidates it;
//! - every Evidence reference by id, version and pinned fingerprint, each
//!   proved to resolve to the same content under the proposed folder.
//!
//! A reference with no pinned fingerprint cannot be proved and blocks the
//! change. Its last observation is not used in its place: an observation is
//! what was seen once, not the reference's identity, and treating it as
//! identity would quietly promote a guess into an authority change.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::EvidenceVerification;
use pmc_knowledge::evidence::observe_evidence;
use pmc_knowledge::vault::VaultRoot;
use pmc_ledger::sqlite::{EvidenceVaultEntry, LedgerOpenError, SqliteProductLedger};
use pmc_platform::authority_control::{
    AuthorityControl, AuthorityControlError, AuthorityControlStore, AuthorityOperation,
    BoundEvidence, PreparedVaultRootChange, VaultRootChangeOutcome, VaultRootChangePhase,
    VaultRootChangeTerminal,
};
use pmc_platform::backup_registry::BackupRecord;
use pmc_platform::host_audit::{
    AuditOutcome, AuditWorkspace, HostAuditEvent, HostAuditLog, VAULT_ROOT_CHANGED,
    VAULT_ROOT_NOT_CHANGED, VAULT_ROOT_PREPARED, VAULT_ROOT_REJECTED,
};
use pmc_platform::settings::{
    CanonicalDirectoryPath, DeferredDirectoryPath, OperationalPatch, PatchOutcome,
    SettingsDocument, SettingsPatch, SettingsStore, ValuePatch,
};
use pmc_platform::workspace::WorkspaceKind;

use crate::restore_service::sha256_bytes;

#[derive(Debug)]
pub enum VaultRootChangeError {
    /// Another change of this workspace's Vault is already prepared or
    /// running.
    AnotherChangeActive,
    /// This workspace's Vault is not the person's to choose (Training).
    NotLiveWorkspace,
    /// The chosen folder cannot serve as a Vault root right now.
    ProposedRootUnusable,
    /// The chosen folder is the one already in use.
    ProposedRootUnchanged,
    /// The named backup does not hold the Ledger as it is now, or was made
    /// before this change began, so it is not this change's recovery
    /// evidence.
    RecoveryBackupStale,
    /// The folder was changed but the host audit log could not record it.
    /// The change happened; saying otherwise would be the larger lie (ADR
    /// 0012: nothing is reported done until its record is durable).
    ChangedButNotRecorded,
    /// The audit log could not record this step, so the step did not happen.
    NotRecorded,
    /// References with no pinned fingerprint: the change cannot be proved
    /// safe until they are pinned.
    UnpinnedEvidence {
        count: u64,
    },
    /// References whose file under the proposed folder is missing or holds
    /// different content.
    UnresolvedEvidence {
        count: u64,
    },
    /// No such Prepared Intent, or it is not the one named.
    NotPrepared,
    /// The preview the person approved is not the one on record.
    PayloadMismatch,
    /// The typed confirmation does not match this preview's code.
    ConfirmationMismatch,
    Expired,
    /// The settings or the Ledger changed since the preview.
    Stale,
    Settings,
    Ledger(LedgerOpenError),
    Control(AuthorityControlError),
}

impl std::fmt::Display for VaultRootChangeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AnotherChangeActive => formatter.write_str("another Vault change is active"),
            Self::NotLiveWorkspace => formatter.write_str("only the Live Vault is configurable"),
            Self::ProposedRootUnusable => formatter.write_str("the chosen folder is not usable"),
            Self::ProposedRootUnchanged => formatter.write_str("that folder is already in use"),
            Self::RecoveryBackupStale => {
                formatter.write_str("the backup is not this change's recovery evidence")
            }
            Self::ChangedButNotRecorded => {
                formatter.write_str("the folder changed but the change could not be recorded")
            }
            Self::NotRecorded => formatter.write_str("the step could not be recorded"),
            Self::UnpinnedEvidence { count } => {
                write!(formatter, "{count} Evidence references have no fingerprint")
            }
            Self::UnresolvedEvidence { count } => {
                write!(formatter, "{count} Evidence references do not resolve")
            }
            Self::NotPrepared => formatter.write_str("no such prepared change"),
            Self::PayloadMismatch => formatter.write_str("the preview changed"),
            Self::ConfirmationMismatch => formatter.write_str("the confirmation does not match"),
            Self::Expired => formatter.write_str("the prepared change expired"),
            Self::Stale => formatter.write_str("the settings or the Ledger changed"),
            Self::Settings => formatter.write_str("the settings could not be read or written"),
            Self::Ledger(error) => write!(formatter, "the Ledger could not be read: {error}"),
            Self::Control(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for VaultRootChangeError {}

impl From<AuthorityControlError> for VaultRootChangeError {
    fn from(error: AuthorityControlError) -> Self {
        Self::Control(error)
    }
}

/// What the sheet may show: no path, no digest, no other record's content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultRootPreview {
    pub prepared_intent_id: String,
    pub payload_sha256: String,
    pub proposed_folder_name: String,
    pub previous_folder_name: Option<String>,
    /// Every Evidence reference the Ledger holds, and how many of them the
    /// proposed folder serves with the same content.
    pub evidence_count: u64,
    pub resolved_count: u64,
    pub recovery_archive_id: String,
    pub recovery_verified_at_millis: i64,
    pub confirmation_code: String,
    pub expires_at_millis: i64,
}

/// One Vault-root change, from the preview to the committed pointer.
pub struct VaultRootService {
    control: AuthorityControlStore,
    audit: HostAuditLog,
    workspace: AuditWorkspace,
    live: bool,
}

impl VaultRootService {
    #[must_use]
    pub fn in_directory(root: &Path, name: &str, kind: WorkspaceKind) -> Self {
        Self {
            control: AuthorityControlStore::new(
                root.join(format!("authority-control-{name}-v1.json")),
            ),
            audit: HostAuditLog::new(root.join("host-audit-v1.jsonl")),
            workspace: match kind {
                WorkspaceKind::Live => AuditWorkspace::Live,
                WorkspaceKind::Training => AuditWorkspace::Training,
            },
            live: kind == WorkspaceKind::Live,
        }
    }

    /// The record as it is, for a surface that must say whether a change is
    /// in flight.
    pub fn control(&self) -> Result<AuthorityControl, VaultRootChangeError> {
        Ok(self.control.load()?)
    }

    /// Step 1–2 (§3.1–§3.2): resolve every Evidence reference under the
    /// proposed folder, bind what was read, and record the Prepared Intent.
    /// Nothing is changed.
    ///
    /// `recovery` is the verified Operational Backup the host has just made
    /// or identified; it must hold the Ledger revision this preview reads,
    /// or it is not evidence for *this* change.
    pub fn prepare(
        &self,
        input: &PrepareVaultRootChange<'_>,
        now: UtcTimestamp,
    ) -> Result<VaultRootPreview, VaultRootChangeError> {
        if !self.live {
            return Err(VaultRootChangeError::NotLiveWorkspace);
        }
        let previous = input.settings.operational.live_vault_root.clone();
        if previous
            .as_ref()
            .is_some_and(|folder| folder.as_stored() == input.proposed_root.as_path())
        {
            return Err(VaultRootChangeError::ProposedRootUnchanged);
        }
        let ledger_revision = input
            .ledger
            .revision()
            .map_err(VaultRootChangeError::Ledger)?;
        // §3.2's recovery evidence is a backup that holds the current Ledger
        // *and* the current settings. The registry records the Ledger
        // revision a backup holds; the settings revision its `settings.json`
        // was read at comes from the run that made it. Both must be the ones
        // this preview reads — a setting saved between the backup's read and
        // this one would otherwise be replaced with no backup holding it —
        // and the backup must post-date the start of the change.
        if input.recovery.ledger_revision != ledger_revision
            || input.recovery_settings_revision != input.settings.revision
            || input.recovery.verified_at_millis < input.recovery_not_before_millis
        {
            return Err(VaultRootChangeError::RecoveryBackupStale);
        }
        let entries = input
            .ledger
            .list_evidence_vault_entries()
            .map_err(VaultRootChangeError::Ledger)?;
        let unpinned = entries
            .iter()
            .filter(|entry| entry.fingerprint.is_none())
            .count();
        if unpinned > 0 {
            return Err(VaultRootChangeError::UnpinnedEvidence {
                count: unpinned as u64,
            });
        }
        let root = VaultRoot::validate(input.proposed_root.as_path())
            .map_err(|_| VaultRootChangeError::ProposedRootUnusable)?;
        let mut bound = Vec::with_capacity(entries.len());
        let mut unresolved = 0_u64;
        for entry in &entries {
            if resolves(&root, entry, now) {
                bound.push(bind(entry));
            } else {
                unresolved += 1;
            }
        }
        if unresolved > 0 {
            return Err(VaultRootChangeError::UnresolvedEvidence { count: unresolved });
        }

        let proposed_folder_name = folder_name(input.proposed_root.as_path());
        let previous_folder_name = previous
            .as_ref()
            .and_then(DeferredDirectoryPath::folder_name);
        let payload_sha256 = payload_digest(
            input.prepared_intent_id,
            &proposed_folder_name,
            previous_folder_name.as_deref(),
            input.settings.revision,
            ledger_revision,
            &bound,
            &input.recovery.archive_id,
        );
        let prepared = PreparedVaultRootChange {
            prepared_intent_id: input.prepared_intent_id.to_owned(),
            payload_sha256: payload_sha256.clone(),
            proposed_root: input.proposed_root.as_path().to_path_buf(),
            proposed_folder_name: proposed_folder_name.clone(),
            previous_root: previous
                .as_ref()
                .map(|folder| folder.as_stored().to_path_buf()),
            previous_folder_name: previous_folder_name.clone(),
            settings_revision: input.settings.revision,
            ledger_revision,
            bound_evidence: bound.clone(),
            recovery_archive_id: input.recovery.archive_id.clone(),
            recovery_verified_at_millis: input.recovery.verified_at_millis,
            confirmation_code: input.confirmation_code.to_owned(),
            prepared_at_millis: now.unix_millis(),
            expires_at_millis: now.unix_millis() + input.valid_for_millis,
        };
        self.control.update(|control| match control.active {
            Some(_) => Err(VaultRootChangeError::AnotherChangeActive),
            None => {
                control.active = Some(AuthorityOperation::Prepared {
                    prepared: prepared.clone(),
                });
                Ok(())
            }
        })??;
        self.record(
            VAULT_ROOT_PREPARED,
            AuditOutcome::Succeeded,
            vec![
                (
                    "prepared_intent_id".to_owned(),
                    input.prepared_intent_id.to_owned(),
                ),
                ("evidence_count".to_owned(), entries.len().to_string()),
                (
                    "recovery_archive_id".to_owned(),
                    input.recovery.archive_id.clone(),
                ),
            ],
            now,
        )
        .map_err(|()| {
            // The preview was not recorded, so it does not stand: take the
            // intent back out, or it would block every later preview while
            // no screen knows its id to reject it.
            let intent = input.prepared_intent_id;
            let _ = self.control.update(|control| -> Result<(), ()> {
                if matches!(
                    &control.active,
                    Some(AuthorityOperation::Prepared { prepared })
                        if prepared.prepared_intent_id == intent
                ) {
                    control.active = None;
                }
                Ok(())
            });
            VaultRootChangeError::NotRecorded
        })?;
        Ok(VaultRootPreview {
            prepared_intent_id: prepared.prepared_intent_id,
            payload_sha256,
            proposed_folder_name,
            previous_folder_name,
            evidence_count: entries.len() as u64,
            resolved_count: bound.len() as u64,
            recovery_archive_id: prepared.recovery_archive_id,
            recovery_verified_at_millis: prepared.recovery_verified_at_millis,
            confirmation_code: prepared.confirmation_code,
            expires_at_millis: prepared.expires_at_millis,
        })
    }

    /// H2b reject (§3.6): the Prepared Intent is discarded, nothing changes,
    /// and the choice is recorded.
    pub fn reject(
        &self,
        prepared_intent_id: &str,
        now: UtcTimestamp,
    ) -> Result<(), VaultRootChangeError> {
        self.control.update(|control| match &control.active {
            Some(AuthorityOperation::Prepared { prepared })
                if prepared.prepared_intent_id == prepared_intent_id =>
            {
                control.active = None;
                Ok(())
            }
            _ => Err(VaultRootChangeError::NotPrepared),
        })??;
        self.record(
            VAULT_ROOT_REJECTED,
            AuditOutcome::Refused,
            vec![(
                "prepared_intent_id".to_owned(),
                prepared_intent_id.to_owned(),
            )],
            now,
        )
        .map_err(|()| VaultRootChangeError::NotRecorded)
    }

    /// Steps 3–5 (§3.3–§3.5): the typed confirmation and the acknowledged
    /// preview are checked, the approval is recorded, everything the preview
    /// bound is proved again, and only then does the setting move.
    ///
    /// The caller holds the Ledger's write lock and the backup gate's
    /// exclusion for the whole call, so nothing can write between the
    /// re-check and the commit.
    pub fn approve(
        &self,
        input: &ApproveVaultRootChange<'_>,
        now: UtcTimestamp,
    ) -> Result<VaultRootChangeOutcome, VaultRootChangeError> {
        // Claim it first: exactly one approval may proceed, and a repeated
        // request gets the outcome the first one reached.
        let prepared = self.claim(input, now)?;
        let prepared = match prepared {
            Claimed::Already(outcome) => return Ok(outcome),
            Claimed::Now(prepared) => prepared,
        };
        let outcome = self.commit(&prepared, input, now);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                // Nothing moved: the setting is written once, last.
                self.finish(
                    &prepared,
                    input.idempotency_id,
                    input.receipt_id,
                    VaultRootChangeOutcome::NotChanged,
                    now,
                )?;
                return Err(error);
            }
        };
        self.finish(
            &prepared,
            input.idempotency_id,
            input.receipt_id,
            outcome,
            now,
        )?;
        Ok(outcome)
    }

    /// At startup, before anything reads the Vault: a change still only
    /// prepared is discarded, and one that was approved is resolved by
    /// reading the settings — never by assuming which side of the write the
    /// crash fell on.
    pub fn reconcile(
        &self,
        settings: &SettingsStore,
        now: UtcTimestamp,
    ) -> Result<Option<VaultRootChangeOutcome>, VaultRootChangeError> {
        let control = self.control.load()?;
        let Some(active) = control.active else {
            return Ok(None);
        };
        match active {
            AuthorityOperation::Prepared { .. } => {
                self.control
                    .update(|control| -> Result<(), ()> {
                        control.active = None;
                        Ok(())
                    })?
                    .ok();
                Ok(None)
            }
            AuthorityOperation::Executing {
                prepared,
                idempotency_id,
                receipt_id,
                ..
            } => {
                let committed = settings
                    .read()
                    .ok()
                    .and_then(|document| document.operational.live_vault_root)
                    .is_some_and(|folder| folder.as_stored() == prepared.proposed_root);
                let outcome = if committed {
                    VaultRootChangeOutcome::Changed
                } else {
                    VaultRootChangeOutcome::NotChanged
                };
                self.finish(&prepared, &idempotency_id, &receipt_id, outcome, now)?;
                Ok(Some(outcome))
            }
        }
    }

    fn claim(
        &self,
        input: &ApproveVaultRootChange<'_>,
        now: UtcTimestamp,
    ) -> Result<Claimed, VaultRootChangeError> {
        let millis = now.unix_millis();
        self.control.update(|control| {
            if let Some(finished) = control
                .finished
                .iter()
                .find(|finished| finished.idempotency_id == input.idempotency_id)
            {
                return Ok(Claimed::Already(finished.outcome));
            }
            match &control.active {
                Some(AuthorityOperation::Executing { prepared, .. })
                    if prepared.prepared_intent_id == input.prepared_intent_id =>
                {
                    // Approved once already and not finished: the caller
                    // must not start a second commit.
                    Err(VaultRootChangeError::AnotherChangeActive)
                }
                Some(AuthorityOperation::Prepared { prepared })
                    if prepared.prepared_intent_id == input.prepared_intent_id =>
                {
                    if prepared.payload_sha256 != input.payload_sha256 {
                        return Err(VaultRootChangeError::PayloadMismatch);
                    }
                    if prepared.confirmation_code != input.typed_code {
                        return Err(VaultRootChangeError::ConfirmationMismatch);
                    }
                    if millis > prepared.expires_at_millis {
                        return Err(VaultRootChangeError::Expired);
                    }
                    let prepared = prepared.clone();
                    control.active = Some(AuthorityOperation::Executing {
                        prepared: prepared.clone(),
                        idempotency_id: input.idempotency_id.to_owned(),
                        receipt_id: input.receipt_id.to_owned(),
                        approved_at_millis: millis,
                        phase: VaultRootChangePhase::Approved,
                    });
                    Ok(Claimed::Now(Box::new(prepared)))
                }
                _ => Err(VaultRootChangeError::NotPrepared),
            }
        })?
    }

    /// Prove the preview again, then write the setting.
    fn commit(
        &self,
        prepared: &PreparedVaultRootChange,
        input: &ApproveVaultRootChange<'_>,
        now: UtcTimestamp,
    ) -> Result<VaultRootChangeOutcome, VaultRootChangeError> {
        let document = input
            .settings
            .read()
            .map_err(|_| VaultRootChangeError::Settings)?;
        if document.revision != prepared.settings_revision {
            return Err(VaultRootChangeError::Stale);
        }
        {
            let ledger = input.ledger;
            let revision = ledger.revision().map_err(VaultRootChangeError::Ledger)?;
            if revision != prepared.ledger_revision {
                return Err(VaultRootChangeError::Stale);
            }
            // The whole set again, not a count: a reference could have been
            // replaced by another with the same total. The revision alone is
            // not trusted for this — the set is what the person approved.
            let entries = ledger
                .list_evidence_vault_entries()
                .map_err(VaultRootChangeError::Ledger)?;
            let bound = entries.iter().map(bind).collect::<Vec<_>>();
            if bound != prepared.bound_evidence {
                return Err(VaultRootChangeError::Stale);
            }
            let root = VaultRoot::validate(&prepared.proposed_root)
                .map_err(|_| VaultRootChangeError::ProposedRootUnusable)?;
            let unresolved = entries
                .iter()
                .filter(|entry| !resolves(&root, entry, now))
                .count();
            if unresolved > 0 {
                return Err(VaultRootChangeError::UnresolvedEvidence {
                    count: unresolved as u64,
                });
            }
        }
        self.control
            .update(|control| -> Result<(), ()> {
                if let Some(AuthorityOperation::Executing { phase, .. }) = &mut control.active {
                    *phase = VaultRootChangePhase::Committing;
                }
                Ok(())
            })?
            .ok();
        let folder = DeferredDirectoryPath::from(
            CanonicalDirectoryPath::new(prepared.proposed_root.clone())
                .map_err(|_| VaultRootChangeError::ProposedRootUnusable)?,
        );
        match input
            .settings
            .apply_durable(
                document.revision,
                SettingsPatch::Operational(OperationalPatch {
                    live_vault_root: ValuePatch::Set(folder),
                    ..OperationalPatch::default()
                }),
            )
            .map_err(|_| VaultRootChangeError::Settings)?
        {
            PatchOutcome::Committed { .. } => Ok(VaultRootChangeOutcome::Changed),
            PatchOutcome::Stale { .. } => Err(VaultRootChangeError::Stale),
        }
    }

    fn finish(
        &self,
        prepared: &PreparedVaultRootChange,
        idempotency_id: &str,
        receipt_id: &str,
        outcome: VaultRootChangeOutcome,
        now: UtcTimestamp,
    ) -> Result<(), VaultRootChangeError> {
        let terminal = VaultRootChangeTerminal {
            idempotency_id: idempotency_id.to_owned(),
            prepared_intent_id: prepared.prepared_intent_id.clone(),
            outcome,
            previous_folder_name: prepared.previous_folder_name.clone(),
            folder_name: prepared.proposed_folder_name.clone(),
            recovery_archive_id: prepared.recovery_archive_id.clone(),
            completed_at_millis: now.unix_millis(),
        };
        self.control
            .update(|control| -> Result<(), ()> {
                control.active = None;
                control.finished.push(terminal.clone());
                Ok(())
            })?
            .ok();
        let (code, audit_outcome) = match outcome {
            VaultRootChangeOutcome::Changed => (VAULT_ROOT_CHANGED, AuditOutcome::Succeeded),
            VaultRootChangeOutcome::NotChanged => (VAULT_ROOT_NOT_CHANGED, AuditOutcome::Failed),
        };
        self.record(
            code,
            audit_outcome,
            vec![
                (
                    "prepared_intent_id".to_owned(),
                    prepared.prepared_intent_id.clone(),
                ),
                ("receipt_id".to_owned(), receipt_id.to_owned()),
                (
                    "evidence_count".to_owned(),
                    prepared.bound_evidence.len().to_string(),
                ),
                (
                    "recovery_archive_id".to_owned(),
                    prepared.recovery_archive_id.clone(),
                ),
            ],
            now,
        )
        // The record of what happened is part of the action (ADR 0012). A
        // change that was made but could not be recorded is reported as
        // exactly that, not as a failure — it did happen.
        .map_err(|()| match outcome {
            VaultRootChangeOutcome::Changed => VaultRootChangeError::ChangedButNotRecorded,
            VaultRootChangeOutcome::NotChanged => VaultRootChangeError::NotRecorded,
        })
    }

    /// Append one event, or say it could not be appended. Nothing here is
    /// reported as done on an event that was not written (ADR 0012).
    fn record(
        &self,
        code: &str,
        outcome: AuditOutcome,
        facts: Vec<(String, String)>,
        now: UtcTimestamp,
    ) -> Result<(), ()> {
        // Two events of one code can share a millisecond; the counter keeps
        // their ids apart without naming anything about them.
        let sequence = AUDIT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let event_id = format!("{}-{code}-{sequence}", now.unix_millis());
        self.audit
            .append(&HostAuditEvent::new(
                event_id,
                now.unix_millis(),
                self.workspace,
                code,
                outcome,
                facts,
            ))
            .map_err(|_| ())
    }
}

/// Keeps two audit ids apart inside one millisecond.
static AUDIT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

enum Claimed {
    /// Boxed: the prepared intent is much larger than an outcome, and this
    /// value only carries it from the claim to the commit.
    Now(Box<PreparedVaultRootChange>),
    Already(VaultRootChangeOutcome),
}

/// What a preview needs. The host has already validated `proposed_root` and
/// made or identified `recovery`.
pub struct PrepareVaultRootChange<'a> {
    pub proposed_root: &'a CanonicalDirectoryPath,
    pub ledger: &'a SqliteProductLedger,
    pub settings: &'a SettingsDocument,
    pub recovery: &'a BackupRecord,
    /// When this change began. A backup verified before it is not evidence
    /// for it: it may hold older settings than the ones being replaced.
    pub recovery_not_before_millis: i64,
    /// The settings revision the recovery backup's `settings.json` was read
    /// at, from the run that made it. Must equal the revision this preview
    /// reads.
    pub recovery_settings_revision: u64,
    pub prepared_intent_id: &'a str,
    pub confirmation_code: &'a str,
    pub valid_for_millis: i64,
}

/// What an approval needs. The Ledger is required: everything the preview
/// bound is proved against it again before the setting moves, and an
/// approval that could not do that would be an approval of nothing.
pub struct ApproveVaultRootChange<'a> {
    pub prepared_intent_id: &'a str,
    pub payload_sha256: &'a str,
    pub typed_code: &'a str,
    pub idempotency_id: &'a str,
    pub receipt_id: &'a str,
    pub settings: &'a SettingsStore,
    pub ledger: &'a SqliteProductLedger,
}

/// Does the file under this root hold exactly what the reference pinned?
/// `observe_evidence` is the one reviewed path that resolves inside a root
/// and hashes; `Verified` is its answer for "the same content", and nothing
/// else counts — a missing file degrades, a changed one mismatches.
fn resolves(root: &VaultRoot, entry: &EvidenceVaultEntry, now: UtcTimestamp) -> bool {
    let Some(fingerprint) = entry.fingerprint.as_ref() else {
        return false;
    };
    matches!(
        observe_evidence(
            root,
            &entry.vault_path,
            Some(fingerprint.digest()),
            &EvidenceVerification::Unverified,
            now,
        ),
        Ok(EvidenceVerification::Verified { .. })
    )
}

fn bind(entry: &EvidenceVaultEntry) -> BoundEvidence {
    BoundEvidence {
        evidence_id: entry.id.as_str().to_owned(),
        version: entry.version.get(),
        fingerprint_algorithm: entry
            .fingerprint
            .as_ref()
            .map_or_else(String::new, |fingerprint| {
                fingerprint.algorithm().as_persisted().to_owned()
            }),
        fingerprint_digest: entry
            .fingerprint
            .as_ref()
            .map_or_else(String::new, |fingerprint| {
                fingerprint.digest().as_str().to_owned()
            }),
    }
}

/// The folder's own name — its last component — or the drive's letter and
/// colon for a drive root, which has none. Never the rest of the path.
fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || {
            path.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_owned()
        },
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The canonical payload the person's confirmation is bound to. Folder names
/// rather than paths, the two revisions, and every bound reference in order:
/// anything the preview claimed that later differs changes this digest.
fn payload_digest(
    prepared_intent_id: &str,
    proposed_folder_name: &str,
    previous_folder_name: Option<&str>,
    settings_revision: u64,
    ledger_revision: u64,
    bound: &[BoundEvidence],
    recovery_archive_id: &str,
) -> String {
    let mut payload = format!(
        "pmc-vault-root-change/v1\nintent={prepared_intent_id}\nproposed={proposed_folder_name}\nprevious={}\nsettings_revision={settings_revision}\nledger_revision={ledger_revision}\nrecovery={recovery_archive_id}\ncount={}\n",
        previous_folder_name.unwrap_or(""),
        bound.len(),
    );
    for entry in bound {
        payload.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            entry.evidence_id, entry.version, entry.fingerprint_algorithm, entry.fingerprint_digest
        ));
    }
    sha256_bytes(payload.as_bytes())
}
