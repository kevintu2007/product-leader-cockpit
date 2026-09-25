/**
 * English copy for Operational Restore (DG3 restore amendment §3–§5) and
 * S11 System Health, with the host error keys the restore commands raise.
 */
export const RESTORE_EN = {
  "restore.open": "Restore from a backup…",
  "restore.title": "Restore from a backup",
  "restore.cancel": "Cancel",
  "restore.choose.lede":
    "Choose an Operational Backup made on this computer or another. Nothing changes until you confirm the last step.",
  "restore.choose.button": "Choose backup file…",
  "restore.choose.dialogTitle": "Choose the backup to restore",
  "restore.choose.filterName": "PMC backups",
  "restore.file": "Backup file: {name}",
  "restore.passphrase.label": "Passphrase for this backup",
  "restore.passphrase.check": "Check the backup",
  "restore.checking": "Checking the backup… this can take a minute.",
  "restore.backup": "Created {time}, from Ledger schema {schema}, {count} records.",
  "restore.recovery.running":
    "Before replacing anything, PMC backs up the current workspace. This can take a minute.",
  "restore.preview.lede":
    "This is exactly what the restore does. Any change before you confirm cancels it.",
  "restore.preview.backup": "Backup",
  "restore.preview.now": "Now",
  "restore.now": "{count} records, last change {time}.",
  "restore.now.noChange": "{count} records, no change yet.",
  "restore.preview.replaced": "Replaced",
  "restore.replaced":
    "The Product Ledger, and the settings in the backup: language, time zone, theme, retention, AI and log settings.",
  "restore.preview.kept": "Not replaced",
  "restore.kept":
    "The Product Vault, the backup folder and passphrase settings, and every other backup.",
  "restore.preview.recovery": "Recovery backup",
  "restore.recovery.done": "The current workspace was backed up and verified at {time}.",
  "restore.needsUpgrade":
    "This backup comes from an earlier version of PMC. After it is restored, PMC asks to upgrade it before you can use it.",
  "restore.confirm.label": "To confirm, type the backup's creation date: {date}",
  "restore.confirm.button": "Replace with this backup",
  "restore.reject": "Don't restore",
  "restore.replacing": "Replacing… do not close PMC.",
  "restore.result.restored":
    "Restored the backup from {time}. What was here before is saved as the backup from {recovery}.",
  "restore.result.unchanged": "The restore failed. Nothing was changed.",
  "restore.result.putBack":
    "The restore failed after replacing began. PMC put the previous workspace back.",
  "restore.result.recoveryFailed":
    "The restore failed, and PMC could not put the previous workspace back. System Health names the recovery backup to restore.",
  "restore.result.interrupted":
    "The restore stopped before it finished. Nothing more will change now; when PMC starts again it puts the previous workspace back.",
  "restore.continue": "Continue",
  "restore.openSystemHealth": "Open System Health",

  "policyStrip.restoring": "Restoring",
  "backup.strip.restoring": "A restore is in progress. New records and changes wait until it ends.",

  "health.headline": "Whether PMC can read and keep your records",
  "health.loading": "Reading the system status.",
  "health.unavailable": "The system status can't be read right now. {message}",
  "health.ledger.label": "Product Ledger",
  "health.ledger.ready": "Open and readable.",
  "health.ledger.replacing": "A restore is replacing it now.",
  "health.ledger.restore_recovery_required":
    "Not opened. A restore stopped part way, so the file in its place may be neither the old Ledger nor the backup.",
  "health.ledger.upgrade_required":
    "Not opened yet. It comes from an earlier version of PMC and must be upgraded first.",
  "health.ledger.open_failed": "It could not be opened.",
  "health.recoveryBackup":
    "Restore the recovery backup {name} from the backup folder to get back to where you were.",
  "health.quit": "Quit PMC",

  "safeError.desktop.restore_running": "A restore is in progress. Try again when it ends.",
  "safeError.desktop.ledger_unavailable": "The Product Ledger is not open. System Health says why.",
  "safeError.desktop.restore_file_unreadable": "This backup file can't be read.",
  "safeError.desktop.restore_wrong_passphrase": "This passphrase does not open this backup.",
  "safeError.desktop.restore_damaged": "This backup is damaged: a check of its contents failed.",
  "safeError.desktop.restore_newer_version":
    "This backup was made by a newer version of PMC. Update PMC to restore it.",
  "safeError.desktop.restore_unsupported_old":
    "This backup comes from a version of PMC too old to restore.",
  "safeError.desktop.restore_recovery_backup_failed":
    "The backup of the current workspace failed, so nothing was replaced.",
  "safeError.desktop.restore_preview_stale":
    "This preview is no longer current. Choose the backup again.",
  "safeError.desktop.restore_confirmation_mismatch":
    "The date typed is not the backup's creation date.",
  "safeError.desktop.restore_failed_unchanged": "The restore failed. Nothing was changed.",
  "safeError.desktop.restore_live_only": "Restore applies to the Live workspace only.",

  "upgrade.title": "Upgrade this workspace",
  "upgrade.body":
    "PMC {version} stores your records in a newer format. Upgrading takes a moment. PMC backs up your workspace first; if anything fails, nothing changes.",
  "upgrade.fact.current": "Current format",
  "upgrade.fact.new": "New format",
  "upgrade.schema": "schema {schema}",
  "upgrade.fact.records": "Records",
  "upgrade.fact.lastBackup": "Last verified backup",
  "upgrade.fact.noBackup": "None",
  "upgrade.backupFirst": "Backup first: set the backup folder and passphrase, then upgrade.",
  "upgrade.run": "Upgrade",
  "upgrade.backingUp": "Backing up your workspace…",
  "upgrade.upgrading": "Upgrading…",
  "upgrade.result.upgraded":
    "Your workspace is upgraded. The backup from {time} holds it as it was before.",
  "upgrade.result.rolledBack": "The upgrade did not complete. Nothing was changed.",
  "upgrade.result.unreadable":
    "Your workspace was upgraded, but PMC could not open it. The backup from {time} holds it as it was before.",
  "upgrade.result.unknown":
    "PMC cannot tell whether the upgrade completed. The backup from {time} holds your workspace as it was before.",
  "upgrade.retry": "Try again",
  "upgrade.blocked.title": "This workspace can't be opened",
  "upgrade.newer": "This workspace was made by a newer version of PMC. Update PMC to open it.",
  "upgrade.unsupportedOld":
    "This workspace was made by a development version of PMC and cannot be upgraded.",
  "health.ledger.unsupported_old":
    "Not opened. It was made by a development version of PMC and cannot be upgraded.",
  "health.ledger.newer_version": "Not opened. It was made by a newer version of PMC.",
  "safeError.desktop.upgrade_live_only": "Upgrading applies to the Live workspace only.",
  "safeError.desktop.upgrade_not_required": "This workspace does not need an upgrade.",
  "safeError.desktop.upgrade_backup_failed":
    "The backup before the upgrade failed, so the upgrade did not start. Check the backup folder and try again.",
  "safeError.desktop.upgrade_failed_unchanged":
    "The upgrade did not complete. Nothing was changed.",
  "upgrade.trainingReset":
    "This Training workspace comes from an earlier version of PMC. It is not upgraded: reset it with the sample workspace.",
  "safeError.desktop.upgrade_outcome_unknown":
    "PMC cannot tell whether the upgrade completed. The backup made just before it holds your workspace as it was.",
  "restore.now.unavailable":
    "PMC cannot open the current Product Ledger, so its record count and last change are unavailable.",
  "restore.recovery.preserved":
    "PMC preserved and rechecked an exact copy of the current Ledger files as “{name}”. The copy matched byte for byte. PMC could not verify it as a usable Product Ledger, so it is not an Operational Backup and does not count as your last verified backup.",
  "restore.preserve.running":
    "Before anything is replaced, PMC keeps an exact copy of the current Ledger files and checks it. This can take a minute.",
  "restore.result.restoredPreserved":
    "Restored the backup from {time}. The files that were here before are kept as “{name}”.",
  "restore.result.putBackPreserved":
    "The restore failed after replacement began. PMC put the previous Ledger files back exactly as they were. The Product Ledger still cannot be opened. You can try another backup or quit PMC.",
  "health.restore.backupFirst":
    "Before restoring, PMC keeps a copy of the current Ledger files in the backup folder. Set the backup folder and the recovery passphrase first.",
  "restore.choose.recovery": "Use the recovery backup “{name}”",
  "restore.choose.other": "Choose another backup file…",
  "restore.result.sourceChanged":
    "The current Ledger files changed after the preview, so nothing was replaced. Start the restore again to see what would be replaced now.",
} as const;
