//! The file-backed side of running Operational Backups for one workspace:
//! where the registry, the private work area and the host audit log live,
//! and the steps that touch them (S7-A; ADR 0010, ADR 0012).
//!
//! The desktop host orchestrates runs and the write gate but holds no path
//! itself (ADR 0011's host rule); every path is resolved here, under the
//! protected app-data root.

use std::path::{Path, PathBuf};

use pmc_domain::time::UtcTimestamp;
use pmc_ledger::sqlite::{
    inspect_ledger, inspect_standalone_snapshot, LedgerCompatibility, LedgerOpenError,
    LedgerSnapshotManifest, LedgerUpgradeError, SqliteProductLedger, UpgradeOutcome, UpgradeSource,
    VerifiedPreUpgradeBackup,
};
use pmc_platform::backup_archive::Passphrase;
use pmc_platform::backup_registry::{
    check_record, new_archive_id, orphan_archives, BackupRecord, RecordStatus, RegistryError,
    RegistryStore,
};
use pmc_platform::host_audit::{
    AuditOutcome, AuditWorkspace, HostAuditEvent, HostAuditLog, BACKUP_COMPLETED,
    BOOTSTRAP_EMPTY_AUTHORITY,
};
use pmc_platform::restore_control::RestoreControlStore;
use pmc_platform::settings::{CanonicalDirectoryPath, ProtectedSettingsRoot};
use pmc_platform::workspace::{WorkspaceIdentity, WorkspaceKind};

use crate::operational_backup::{
    clear_work_area, publish_backup, snapshot_ledger, snapshot_ledger_file, BackupError,
    LedgerSnapshot, PublishInput,
};

/// The Ledger's file name inside a workspace directory.
pub const LEDGER_FILE_NAME: &str = "product-ledger.sqlite3";

const REGISTRY_SAVE_ATTEMPTS: u32 = 3;

/// What the startup check found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reconciliation {
    /// Verification times of the records whose archives are still byte for
    /// byte what was verified.
    pub valid_verified_at: Vec<i64>,
    /// Archives in the destination the registry does not know.
    pub orphan_count: usize,
}

/// Proof that a backup of exactly one Ledger source was made for an upgrade
/// and verified end to end. Only [`BackupService::back_up_before_upgrade`]
/// makes one; nothing else can.
#[derive(Debug)]
pub struct PreUpgradeBackupReceipt {
    source: UpgradeSource,
    archive_id: String,
    verified_at_millis: i64,
}

impl PreUpgradeBackupReceipt {
    /// The source the verified archive holds.
    #[must_use]
    pub const fn source(&self) -> &UpgradeSource {
        &self.source
    }

    #[must_use]
    pub fn archive_id(&self) -> &str {
        &self.archive_id
    }

    #[must_use]
    pub const fn verified_at_millis(&self) -> i64 {
        self.verified_at_millis
    }
}

impl VerifiedPreUpgradeBackup for PreUpgradeBackupReceipt {
    fn verified_source(&self) -> &UpgradeSource {
        &self.source
    }
}

/// Why [`BackupService::upgrade`] did not upgrade.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpgradeFailure {
    /// The workspace's Ledger file could not be resolved.
    WorkspaceUnavailable,
    /// The receipt is for another source than the one passed.
    ReceiptForAnotherSource,
    /// The pre-upgrade archive is gone or no longer the verified bytes.
    BackupNoLongerValid,
    /// Upgraded, but the projections of a restore that preceded it could not
    /// be marked out of sync (S7 plan §7).
    ProjectionsNotMarked,
    /// The Ledger crate's own refusal or outcome.
    Ledger(LedgerUpgradeError),
}

/// The upgrade screen's facts, and the source its backup must hold.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradePlan {
    pub source: UpgradeSource,
    pub from_schema: u32,
    pub to_schema: u32,
    pub record_count: u64,
}

/// Why a Ledger did not open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnopenedLedger {
    UpgradeRequired,
    UnsupportedOld,
    NewerVersion,
    Other,
}

/// How an upgrade that was attempted ended (DG3 upgrade-gate amendment §4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeRun {
    Upgraded,
    /// Nothing changed.
    RolledBack,
    UpgradedButUnreadable,
    OutcomeUnknown,
}
/// One workspace's backup files, all under the protected root.
pub struct BackupService {
    pub(crate) registry: RegistryStore,
    work: PathBuf,
    audit: HostAuditLog,
    workspace: AuditWorkspace,
    /// The workspace's Ledger file; `None` when the workspace could not be
    /// resolved (then nothing here can back it up or inspect it).
    pub(crate) ledger: Option<PathBuf>,
    /// A restore's private work area and its control record (8e).
    pub(crate) restore_work: PathBuf,
    pub(crate) restore_control: RestoreControlStore,
}

impl BackupService {
    #[must_use]
    pub fn new(root: &ProtectedSettingsRoot, kind: WorkspaceKind) -> Self {
        let (name, workspace) = match kind {
            WorkspaceKind::Live => ("live", AuditWorkspace::Live),
            WorkspaceKind::Training => ("training", AuditWorkspace::Training),
        };
        let ledger = WorkspaceIdentity::resolve(root, kind)
            .ok()
            .map(|identity| identity.root().as_path().join(LEDGER_FILE_NAME));
        Self::in_directory(root.path(), name, workspace, ledger)
    }

    /// The same layout under any directory (tests; the host always uses
    /// [`BackupService::new`]).
    #[must_use]
    pub fn in_directory(
        root: &Path,
        name: &str,
        workspace: AuditWorkspace,
        ledger: Option<PathBuf>,
    ) -> Self {
        Self {
            registry: RegistryStore::new(root.join(format!("backup-registry-{name}-v1.json"))),
            work: root.join(format!("backup-work-{name}")),
            audit: HostAuditLog::new(root.join("host-audit-v1.jsonl")),
            workspace,
            ledger,
            restore_work: root.join(format!("restore-work-{name}")),
            restore_control: RestoreControlStore::new(
                root.join(format!("restore-control-{name}-v1.json")),
            ),
        }
    }

    /// Classify this workspace's Ledger without opening it (8d): current,
    /// upgradeable, or why neither.
    pub fn inspect(&self) -> Result<LedgerCompatibility, LedgerOpenError> {
        let ledger = self
            .ledger
            .as_ref()
            .ok_or(LedgerOpenError::StorageUnavailable)?;
        inspect_ledger(ledger)
    }

    /// The backup an upgrade requires (DG3 upgrade-gate amendment §4; product
    /// owner, 2026-09-22): a new Operational Backup of the closed Ledger file,
    /// made now, holding exactly `expected`, verified end to end, audited and
    /// registered like any other — and a receipt bound to that source, which
    /// `upgrade_in_place` requires. A Ledger that is not `expected` any more
    /// is refused before anything is published.
    #[allow(clippy::too_many_arguments)]
    pub fn back_up_before_upgrade(
        &self,
        expected: &UpgradeSource,
        settings_json: &[u8],
        destination: &CanonicalDirectoryPath,
        passphrase: &Passphrase,
        archive_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
    ) -> Result<(BackupRecord, PreUpgradeBackupReceipt), BackupError> {
        let record = self.back_up_closed_ledger(
            expected,
            settings_json,
            destination,
            passphrase,
            archive_id,
            clock,
            "pre_upgrade",
        )?;
        let receipt = PreUpgradeBackupReceipt {
            source: expected.clone(),
            archive_id: record.archive_id.clone(),
            verified_at_millis: record.verified_at_millis,
        };
        Ok((record, receipt))
    }

    /// A new Operational Backup of the closed Ledger file holding exactly
    /// `expected`, verified end to end, audited with `purpose` and
    /// registered: the upgrade's required backup, and the recovery backup of
    /// a restore on the upgrade gate (DG3 restore-unopened amendment, first
    /// cut). Only [`Self::back_up_before_upgrade`] turns one into an upgrade
    /// receipt.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn back_up_closed_ledger(
        &self,
        expected: &UpgradeSource,
        settings_json: &[u8],
        destination: &CanonicalDirectoryPath,
        passphrase: &Passphrase,
        archive_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
        purpose: &str,
    ) -> Result<BackupRecord, BackupError> {
        // Live only, as [`Self::snapshot`].
        if self.workspace != AuditWorkspace::Live {
            return Err(BackupError::Snapshot);
        }
        let ledger = self.ledger.as_ref().ok_or(BackupError::Snapshot)?;
        let (snapshot, inspection) = snapshot_ledger_file(ledger, &self.work)?;
        if UpgradeSource::from(&inspection) != *expected {
            clear_work_area(&self.work).map_err(BackupError::Residue)?;
            return Err(BackupError::SourceChanged);
        }
        let record = self.publish(
            &snapshot,
            settings_json,
            destination,
            passphrase,
            archive_id,
            clock,
            &|extracted, manifest| inspect_standalone_snapshot(extracted, manifest).is_ok(),
        )?;
        // The archive must be the source, field by field.
        if record.ledger_schema_version != expected.schema_version
            || record.ledger_revision != expected.ledger_revision
            || record.authority_inventory_sha256 != expected.inventory_sha256
        {
            return Err(BackupError::VerificationFailed);
        }
        let audited = self.audit(
            BACKUP_COMPLETED,
            AuditOutcome::Succeeded,
            vec![
                ("archive_id".to_owned(), record.archive_id.clone()),
                (
                    "record_count".to_owned(),
                    record.authority_record_count.to_string(),
                ),
                ("purpose".to_owned(), purpose.to_owned()),
            ],
            clock(),
        );
        if !audited {
            return Err(BackupError::NotRecorded);
        }
        self.register(&record)
            .map_err(|_| BackupError::NotRecorded)?;
        Ok(record)
    }

    /// The only way to run the in-place upgrade (a policy check keeps it so):
    /// with a receipt [`BackupService::back_up_before_upgrade`] made for
    /// exactly `expected`, whose archive is still in place byte for byte.
    /// The receipt is consumed. The Ledger crate then re-inspects the file
    /// and refuses a source that changed in any way since the backup.
    pub fn upgrade(
        &self,
        expected: &UpgradeSource,
        receipt: PreUpgradeBackupReceipt,
    ) -> Result<UpgradeOutcome, UpgradeFailure> {
        let ledger = self
            .ledger
            .as_ref()
            .ok_or(UpgradeFailure::WorkspaceUnavailable)?;
        if receipt.source != *expected {
            return Err(UpgradeFailure::ReceiptForAnotherSource);
        }
        let registry = self
            .registry
            .load()
            .map_err(|_| UpgradeFailure::BackupNoLongerValid)?;
        let still_valid = registry.records.iter().any(|record| {
            record.archive_id == receipt.archive_id && check_record(record) == RecordStatus::Valid
        });
        if !still_valid {
            return Err(UpgradeFailure::BackupNoLongerValid);
        }
        let outcome = pmc_ledger::sqlite::upgrade_in_place(ledger, expected, &receipt)
            .map_err(UpgradeFailure::Ledger)?;
        self.finish_pending_restore_projections()?;
        Ok(outcome)
    }

    /// What the upgrade screen shows (DG3 upgrade-gate amendment §3), when
    /// this workspace's Ledger is an older supported format; `None`
    /// otherwise.
    #[must_use]
    pub fn upgrade_plan(&self) -> Option<UpgradePlan> {
        match self.inspect().ok()? {
            LedgerCompatibility::UpgradeableFrom(inspection) => Some(UpgradePlan {
                source: UpgradeSource::from(&inspection),
                from_schema: inspection.schema_version,
                to_schema: pmc_ledger::sqlite::CURRENT_SCHEMA_VERSION,
                record_count: inspection.inventory.record_count,
            }),
            _ => None,
        }
    }

    /// Whether the Ledger file is still exactly `source`, inspected afresh.
    #[must_use]
    pub fn still_the_source(&self, source: &UpgradeSource) -> bool {
        self.upgrade_plan()
            .is_some_and(|plan| plan.source == *source)
    }

    /// Why a Ledger that did not open did not (the upgrade gate's §2).
    #[must_use]
    pub fn classify_unopened(&self) -> UnopenedLedger {
        match self.inspect() {
            Ok(LedgerCompatibility::UpgradeableFrom(_)) => UnopenedLedger::UpgradeRequired,
            Ok(LedgerCompatibility::UnsupportedOld { .. }) => UnopenedLedger::UnsupportedOld,
            Ok(LedgerCompatibility::Future { .. }) => UnopenedLedger::NewerVersion,
            _ => UnopenedLedger::Other,
        }
    }

    /// The whole upgrade of §4, with the Ledger closed: a new pre-upgrade
    /// backup of exactly `plan`'s source, then the upgrade on its receipt.
    /// `Err` means nothing was upgraded (the backup failed or the source
    /// changed); every `Ok` carries the backup that holds the Ledger as it
    /// was, and an outcome told apart by the Ledger crate's read-back.
    pub fn back_up_and_upgrade(
        &self,
        plan: &UpgradePlan,
        settings_json: &[u8],
        destination: &CanonicalDirectoryPath,
        passphrase: &Passphrase,
        archive_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
    ) -> Result<(BackupRecord, UpgradeRun), BackupError> {
        let (record, receipt) = self.back_up_before_upgrade(
            &plan.source,
            settings_json,
            destination,
            passphrase,
            archive_id,
            clock,
        )?;
        let run = match self.upgrade(&plan.source, receipt) {
            // Upgraded; a restore's projection flag not yet marked is
            // retried at every start.
            Ok(UpgradeOutcome::Upgraded { .. }) | Err(UpgradeFailure::ProjectionsNotMarked) => {
                UpgradeRun::Upgraded
            }
            Err(UpgradeFailure::Ledger(LedgerUpgradeError::UpgradedButUnreadable)) => {
                UpgradeRun::UpgradedButUnreadable
            }
            Err(UpgradeFailure::Ledger(LedgerUpgradeError::OutcomeUnknown)) => {
                UpgradeRun::OutcomeUnknown
            }
            // Rolled back or refused: "nothing changed" only if the Ledger
            // still is, field for field, the source that was backed up.
            Err(_) if self.still_the_source(&plan.source) => UpgradeRun::RolledBack,
            Err(_) => UpgradeRun::OutcomeUnknown,
        };
        Ok((record, run))
    }
    /// A restore from an older schema left its projections to be marked out
    /// of sync once that Ledger opens (S7 plan §7). Marks them if the current
    /// Ledger is the one the restore installed — matched by its authority
    /// inventory, which the upgrade preserves — and clears only that flag.
    /// Safe to call again: the host runs it after every upgrade and at
    /// startup, so a failure after an upgrade is retried.
    pub fn finish_pending_restore_projections(&self) -> Result<(), UpgradeFailure> {
        let ledger = self
            .ledger
            .as_ref()
            .ok_or(UpgradeFailure::WorkspaceUnavailable)?;
        let control = self
            .restore_control
            .load()
            .map_err(|_| UpgradeFailure::ProjectionsNotMarked)?;
        let Some(LedgerCompatibility::Current(inspection)) = inspect_ledger(ledger).ok() else {
            // Not openable yet (the upgrade gate is still ahead): later.
            return Ok(());
        };
        let current = inspection.inventory.sha256;
        if !control
            .finished
            .iter()
            .any(|finished| finished.projections_pending_for.as_deref() == Some(current.as_str()))
        {
            return Ok(());
        }
        let now = UtcTimestamp::from_unix_millis(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
                }),
        );
        SqliteProductLedger::open(ledger)
            .ok()
            .and_then(|mut opened| opened.mark_projections_out_of_sync_after_restore(now).ok())
            .ok_or(UpgradeFailure::ProjectionsNotMarked)?;
        self.restore_control
            .update(|control| -> Result<(), ()> {
                for finished in &mut control.finished {
                    if finished.projections_pending_for.as_deref() == Some(current.as_str()) {
                        finished.projections_pending_for = None;
                    }
                }
                Ok(())
            })
            .map_err(|_| UpgradeFailure::ProjectionsNotMarked)?
            .map_err(|()| UpgradeFailure::ProjectionsNotMarked)
    }

    #[must_use]
    pub const fn workspace(&self) -> AuditWorkspace {
        self.workspace
    }

    /// Step 1, under the caller's exclusive hold of the Ledger.
    ///
    /// Live only: the sample workspace holds synthetic data and is never
    /// backed up (the accepted sample-workspace amendment §6). Refused here,
    /// before anything is read, whatever the caller checked.
    pub fn snapshot(&self, ledger: &SqliteProductLedger) -> Result<LedgerSnapshot, BackupError> {
        if self.workspace != AuditWorkspace::Live {
            return Err(BackupError::Snapshot);
        }
        snapshot_ledger(ledger, &self.work)
    }

    /// Step 2, with no lock held: write, verify end to end and publish.
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &self,
        snapshot: &LedgerSnapshot,
        settings_json: &[u8],
        destination: &CanonicalDirectoryPath,
        passphrase: &Passphrase,
        archive_id: &str,
        clock: &dyn Fn() -> UtcTimestamp,
        verify_ledger: &dyn Fn(&Path, &LedgerSnapshotManifest) -> bool,
    ) -> Result<BackupRecord, BackupError> {
        publish_backup(PublishInput {
            snapshot,
            settings_json,
            destination,
            work: &self.work,
            passphrase,
            archive_id,
            clock,
            verify_ledger,
        })
    }

    /// Save one verified record, reloading on a concurrent change a bounded
    /// number of times. A record already present is not added twice.
    pub fn register(&self, record: &BackupRecord) -> Result<(), RegistryError> {
        let mut last = RegistryError::Conflict;
        for _ in 0..REGISTRY_SAVE_ATTEMPTS {
            let mut registry = self.registry.load()?;
            if !registry
                .records
                .iter()
                .any(|known| known.archive_id == record.archive_id)
            {
                registry.records.push(record.clone());
            }
            match self.registry.save(&registry) {
                Ok(()) => return Ok(()),
                Err(RegistryError::Conflict) => last = RegistryError::Conflict,
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    /// Append one host audit event; `false` when it could not be written and
    /// flushed (ADR 0012 §4).
    pub fn audit(
        &self,
        code: &str,
        outcome: AuditOutcome,
        facts: Vec<(String, String)>,
        now: UtcTimestamp,
    ) -> bool {
        let Some(event_id) = new_archive_id() else {
            return false;
        };
        let event = HostAuditEvent::new(
            format!("host-{event_id}"),
            now.unix_millis(),
            self.workspace,
            code,
            outcome,
            facts,
        );
        self.audit.append(&event).is_ok()
    }

    /// The named bootstrap of S7 §6 (ADR 0012 §3): recorded once per Live
    /// workspace, `succeeded` only for a pristine Ledger. It exempts nothing
    /// but the creation of the empty Ledger; the first operating write still
    /// waits for a verified backup.
    pub fn record_bootstrap(&self, ledger: &SqliteProductLedger, now: UtcTimestamp) {
        if self.workspace != AuditWorkspace::Live {
            return;
        }
        match self
            .audit
            .has_event(AuditWorkspace::Live, BOOTSTRAP_EMPTY_AUTHORITY)
        {
            Ok(false) => {}
            // Already recorded, or the log cannot be read: never write twice.
            Ok(true) | Err(_) => return,
        }
        let pristine = ledger.is_pristine_authority().unwrap_or(false);
        // Nothing waits on this event; if it cannot be written, the next
        // launch finds it missing and tries again.
        let _ = self.audit(
            BOOTSTRAP_EMPTY_AUTHORITY,
            if pristine {
                AuditOutcome::Succeeded
            } else {
                AuditOutcome::Refused
            },
            Vec::new(),
            now,
        );
    }

    /// Startup: empty the work area (a crash may have left plaintext), then
    /// check every recorded archive by size and full hash. Rechecking never
    /// refreshes a verification time.
    #[must_use]
    pub fn reconcile(&self, destination: Option<&CanonicalDirectoryPath>) -> Reconciliation {
        let _ = clear_work_area(&self.work);
        let registry = self.registry.load().unwrap_or_default();
        let valid_verified_at = registry
            .records
            .iter()
            .filter(|record| check_record(record) == RecordStatus::Valid)
            .map(|record| record.verified_at_millis)
            .collect();
        let orphan_count = destination.map_or(0, |folder| {
            orphan_archives(folder.as_path(), &registry).len()
        });
        Reconciliation {
            valid_verified_at,
            orphan_count,
        }
    }
}
