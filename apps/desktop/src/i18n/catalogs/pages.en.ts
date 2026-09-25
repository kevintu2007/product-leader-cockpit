/** English copy for People (S09), Reviews & Reports, Product Vault and Settings. */
export const PAGES_EN = {
  // S09, People.
  "people.kind.person": "Person",
  "people.kind.organization": "Organization",
  "people.none": "None",
  "people.headline": "Who is responsible for what, and who is still waiting on an answer",
  "people.lede":
    "The Stakeholders and relationships here are yours to create and keep up; they're not user accounts. A request waiting on an answer isn't a commitment yet: an Action Request becomes an Action once it's accepted.",
  "people.empty":
    "There are no Stakeholders yet. Once you create some, this lists what they're responsible for and depend on.",
  "people.caption": "Sorted by identifier. Showing {from} to {to} of {total}.",
  "people.column.stakeholder": "Stakeholder",
  "people.column.kind": "Kind",
  "people.column.responsible": "Responsible for",
  "people.column.dependsOn": "Depends on",
  "people.column.waiting": "Requests waiting on an answer",
  "people.column.classification": "Classification",
  "people.previous": "Previous page",
  "people.next": "Next page",

  // Reviews & Reports.
  "reviews.route": "the data Review needs",
  "reviews.headline": "Before a Review, see what's still open",
  "reviews.lede":
    "This lists what the Work Queue has flagged, in the Executive Cockpit's order of attention. To act on it, go to the Work Queue.",
  "reviews.period": "Review period",
  "reviews.noPeriod": "No review period available: {reason}.",
  "reviews.notYet": "This version can't build a Fact Pack or approve a report yet.",
  "reviews.open": "Flagged in the Work Queue",
  "reviews.openNone": "The Work Queue hasn't flagged anything.",
  "reviews.count.one": "{count} in total.",
  "reviews.count.other": "{count} in total.",
  "reviews.goToQueue": "Act on it in the Work Queue",

  // Product Vault.
  "vault.reason.notConfigured": "This workspace has no Product Vault set.",
  "vault.reason.invalidRoot":
    "The Product Vault location doesn't exist, isn't a folder, or is a link.",
  "vault.reason.unknown": "The reason is unknown.",
  "vault.pausedBecause":
    "{reason} Pinning fingerprints and re-observing are paused; linking Evidence still works.",
  "vault.headline": "Evidence files and where they stand now",
  "vault.lede":
    "Files live in your own Product Vault; the Ledger records each piece of Evidence's use, fingerprint, verification and classification.",
  "vault.available": "Vault is available",
  "vault.unavailable": "Vault isn't available",
  "vault.availableHint": "You can pin fingerprints or re-observe from a Product's Evidence tab.",
  "vault.count": "{count} in total, sorted by identifier.",
  "vault.empty":
    "There's no Evidence in the Ledger yet. Link some from a Product's Evidence tab and it appears here.",
  "vault.column.evidence": "Evidence",
  "vault.column.role": "Use",
  "vault.column.verification": "Verification",
  "vault.column.fingerprint": "Fingerprint",
  "vault.column.classification": "Classification",
  "vault.column.version": "Version",
  "vault.roleUnset": "Not set",
  "vault.verificationOn": "{verification} ({date})",
  "vault.pinned": "Pinned",
  "vault.notPinned": "Not pinned",

  // S10, Settings.
  "settings.headline": "How things look, and where this workspace stands",
  "settings.lede":
    "Text size affects only this computer's screen and is separate from Windows display scaling. Switch the theme with the button at the top right.",
  "settings.textSize": "Text size",
  "settings.language": "Language",
  "settings.languageSystem": "System language ({language})",
  "settings.languageFailed": "The language wasn't saved. {message}",
  "settings.sampleBody":
    "Good morning. Start with what really moves results today. Demo Product Atlas has a milestone date that has passed; 2/2 KPIs observed.",
  "settings.sampleLabel": "Milestones: date passed, earliest 2026‑08‑15",
  "settings.dataSources": "Data sources",
  "settings.loading": "Reading the Ledger and Vault status.",
  "settings.unavailable": "The Ledger or Vault status can't be read right now. {message}",
  "settings.ledgerReadable":
    "Readable. Schema version {schema}; the Ledger is at version {revision}.",
  "settings.vaultNotSet": "Not set",
  "vaultRoot.open.choose": "Choose Vault folder…",
  "vaultRoot.open.change": "Change Vault folder…",
  "vault.reason.changeUnresolved":
    "A change of the Vault folder was interrupted and could not be finished. Restart PMC to finish it.",
  "safeError.desktop.vault_change_unresolved":
    "A change of the Vault folder was interrupted and could not be finished, so no Vault is used. Restart PMC to finish it.",
  "vaultRoot.title": "Product Vault folder",
  "vaultRoot.cancel": "Cancel",
  "vaultRoot.choose.lede":
    "Choose the folder this workspace reads Evidence from. Before anything changes, PMC backs up this workspace and checks every Evidence file under the folder.",
  "vaultRoot.backupFirst":
    "Before the Vault changes, PMC backs up this workspace so it can be restored. Set the backup folder and the recovery passphrase first.",
  "vaultRoot.choose.button": "Choose Vault folder…",
  "vaultRoot.choose.dialogTitle": "Choose the Product Vault folder",
  "vaultRoot.folder": "Folder: {name}",
  "vaultRoot.preparing":
    "Backing up this workspace, then checking every Evidence file under the new folder… this can take a minute.",
  "vaultRoot.preview.lede":
    "This is exactly what will change. Nothing in the Vault or the Ledger is moved.",
  "vaultRoot.preview.now": "Now",
  "vaultRoot.preview.new": "New",
  "vaultRoot.preview.evidence": "Evidence",
  "vaultRoot.preview.noEvidence":
    "No Evidence refers to the current Vault; nothing is re-resolved.",
  "vaultRoot.preview.resolved":
    "{resolved} of {count} references resolve to the same content under the new folder.",
  "vaultRoot.preview.recovery": "Recovery evidence",
  "vaultRoot.preview.recoveryVerified": "Backed up and verified at {time}",
  "vaultRoot.confirm.phrase": "CHANGE VAULT",
  "vaultRoot.confirm.label": "To confirm, type {phrase} {code}",
  "vaultRoot.confirm.button": "Use this folder",
  "vaultRoot.reject": "Don't change",
  "vaultRoot.changing": "Changing… do not close PMC.",
  "vaultRoot.result.changed": 'This workspace now reads Evidence from "{name}".',
  "vaultRoot.result.notChanged": "Nothing was changed.",
  "vaultRoot.done": "Done",
  "safeError.desktop.vault_change_active":
    "Another change of the Vault folder is already in progress. Finish or cancel it first.",
  "safeError.desktop.vault_not_live": "Only the Live workspace's Vault folder can be changed.",
  "safeError.desktop.vault_folder_unusable":
    "That folder can't be a Vault. Choose an existing folder you can read, and not a shortcut or link.",
  "safeError.desktop.vault_folder_unchanged": "That folder is already this workspace's Vault.",
  "safeError.desktop.vault_folder_in_use":
    "PMC already uses that folder. Choose one that is not PMC's own and does not hold or sit inside your backup folder.",
  "safeError.desktop.vault_recovery_backup_stale":
    "The backup made for this change no longer matches the workspace. Try again.",
  "safeError.desktop.vault_evidence_unpinned":
    "{count} Evidence references have no pinned fingerprint, so PMC cannot prove they are the same files in the new folder. Pin them first (Product → Evidence).",
  "safeError.desktop.vault_evidence_unresolved":
    "{count} Evidence references do not resolve to the same content under the new folder. Move or restore those files first.",
  "safeError.desktop.vault_preview_stale": "This preview is out of date. Choose the folder again.",
  "safeError.desktop.vault_confirmation_mismatch": "The code does not match this preview.",
  "safeError.desktop.vault_changed_but_not_recorded":
    "The folder was changed, but PMC could not record the change in its audit log.",
  "safeError.desktop.vault_change_not_recorded":
    "PMC could not record this step, so nothing was changed. Try again.",
  "safeError.desktop.vault_change_failed":
    "The Vault folder could not be changed. Nothing was changed. Try again.",
  "evidenceFile.open": "Add Evidence from a file…",
  "evidenceFile.title": "Evidence from a file",
  "evidenceFile.lede":
    "Choose a file inside the Vault folder. PMC records where it is and its fingerprint; the file stays where it is.",
  "evidenceFile.choose": "Choose file…",
  "evidenceFile.chooseAnother": "Choose another file…",
  "evidenceFile.dialogTitle": "Choose a file inside the Product Vault",
  "evidenceFile.file": "File: {name}",
  "evidenceFile.observed": "Observed {time}",
  "evidenceFile.changed": "The file changed since it was chosen.",
  "evidenceFile.existing": "Evidence {id} already refers to this file, so no new one is created.",
  "evidenceFile.existingLinked":
    "Evidence {id} already refers to this file and is already linked to this Product.",
  "evidenceFile.linkExisting": "Link it to this Product",
  "evidenceFile.sameContent": "Evidence {id} has the same content.",
  "evidenceFile.create": "Create",
  "evidenceFile.createAndLink": "Create and link to this Product",
  "evidenceFile.cancel": "Cancel",
  "evidenceFile.creating": "Creating…",
  "evidenceFile.linking": "Linking…",
  "evidenceFile.created": "Created Evidence {id}.",
  "evidenceFile.createdAndLinked": "Created Evidence {id} and linked it to {product}.",
  "evidenceFile.linkedExisting": "Linked Evidence {id} to {product}.",
  "evidenceFile.createdNotLinked": "Created Evidence {id}; it is not linked yet.",
  "evidenceFile.retryLink": "Link it now",
  "evidenceFile.done": "Done",
  "safeError.desktop.evidence_file_outside_vault": "Choose a file inside the Vault folder.",
  "safeError.desktop.evidence_file_unreadable": "The file could not be read.",
  "safeError.desktop.evidence_file_choice_stale":
    "This choice is out of date. Choose the file again.",
  "safeError.desktop.sample_backup_refused": "The sample workspace is never backed up.",
  "safeError.desktop.workspace_not_chosen": "Choose how to begin first.",
  "safeError.desktop.sample_foreign":
    "The sample folder holds something PMC did not write, so it was left as it is.",
  "safeError.desktop.sample_is_open":
    "Switch to your workspace first; the sample data cannot change while it is open.",
  "safeError.desktop.sample_operation_in_progress":
    "Another change to the sample data is not finished. Try again in a moment.",
  "safeError.desktop.sample_nothing_to_delete": "There is no sample data to delete.",
  "safeError.desktop.sample_delete_not_prepared":
    "This delete is not waiting any more. Start it again.",
  "safeError.desktop.sample_delete_expired":
    "This preview has expired. Nothing was deleted. Start again.",
  "safeError.desktop.sample_delete_changed":
    "The sample data changed since the preview. Nothing was deleted. Start again.",
  "safeError.desktop.sample_confirmation_mismatch":
    "The phrase does not match. Nothing was deleted.",
  "safeError.desktop.sample_not_recorded":
    "PMC could not record this step in its audit log. Try again.",
  "safeError.desktop.sample_failed":
    "The sample data could not be prepared. Your workspace is not affected. Try again.",
  "safeError.desktop.workspace_not_first_run": "This choice is only made once, on first run.",
  "safeError.desktop.workspace_choice_not_saved":
    "The choice could not be saved, so PMC did not restart. Try again.",
  "safeError.desktop.sample_reset_failed":
    "The sample data could not be reset. PMC puts it back as it was, now or at the next start.",
  "safeError.desktop.sample_delete_failed":
    "The sample data was not deleted, or not completely. PMC checks again at the next start.",
  "safeError.desktop.sample_unavailable":
    "The sample data cannot be read right now. Nothing was changed.",
  // "Choose how to begin" and Settings → Workspace (the accepted
  // sample-workspace amendment §3–§5, §8–§9).
  "firstRun.title": "Choose how to begin",
  "firstRun.lede": "Pick one to start. You can switch later in Settings → Workspace.",
  "firstRun.live.title": "Start with my workspace",
  "firstRun.live.body": "An empty workspace opens. Your records stay on this computer.",
  "firstRun.live.setup":
    "Then set up, in Settings: a backup folder, then a recovery passphrase, then the Vault folder. A backup cannot be restored without its passphrase.",
  "firstRun.sample.title": "Learn with sample data",
  "firstRun.sample.body":
    "Synthetic data for learning. It is separate from your work, and you can reset or delete it any time.",
  "firstRun.preparing": "Preparing the sample data… this can take a minute.",
  "firstRun.saving": "Saving your choice…",
  "firstRun.tryAgain": "Nothing is chosen yet. Choose again when you are ready.",
  "workspace.title": "Workspace",
  "workspace.openLive": "Your workspace is open.",
  "workspace.openSample": "Sample data is open. It is synthetic and separate from your work.",
  "workspace.sampleFellBack":
    "Sample data was chosen but could not be opened, so your workspace opened. System Health says why.",
  "workspace.switchToSample": "Switch to sample data",
  "workspace.switchToLive": "Switch to my workspace",
  "workspace.switchToSample.confirm":
    "PMC will restart and open the sample data. Your workspace is not changed.",
  "workspace.switchToLive.confirm": "PMC will restart and open your workspace.",
  "workspace.switchAndRestart": "Switch and restart",
  "workspace.restarting": "PMC is restarting…",
  "sampleWorkspace.badge": "Sample workspace",
  "sampleWorkspace.badge.open": "Open Settings → Workspace",
  "sampleWorkspace.manage": "Sample data",
  "sampleWorkspace.fromLiveOnly":
    "Reset and delete are offered from your workspace. Switch to it first.",
  "sampleWorkspace.cancel": "Cancel",
  "sampleWorkspace.close": "Close",
  "sampleWorkspace.done": "Done",
  "sampleWorkspace.reset.open": "Reset sample data…",
  "sampleWorkspace.reset": "Reset sample data",
  "sampleWorkspace.reset.confirm":
    "The sample data returns to its starting state. Your workspace is not affected.",
  "sampleWorkspace.resetting": "Resetting the sample data… this can take a minute.",
  "sampleWorkspace.reset.done": "The sample data is back to its starting state.",
  "sampleWorkspace.delete.open": "Delete sample data…",
  "sampleWorkspace.delete.title": "Delete sample data",
  "sampleWorkspace.delete.preparing": "Reading what the sample data holds…",
  "sampleWorkspace.delete.lede":
    "This is exactly what the delete removes. Any change before you confirm cancels it.",
  "sampleWorkspace.delete.what": "Sample data",
  "sampleWorkspace.delete.seed": "{seedId}, version {version}",
  "sampleWorkspace.delete.unknown":
    "This preview describes a change this screen cannot show, so it cannot be approved here. Nothing was deleted.",
  "sampleWorkspace.delete.parts": "What is removed",
  "sampleWorkspace.delete.part.ledger": "Its Product Ledger",
  "sampleWorkspace.delete.part.vault": "Its synthetic Vault",
  "sampleWorkspace.delete.part.generated": "Its generated files",
  "sampleWorkspace.delete.effect": "Effect",
  "sampleWorkspace.delete.effectValue":
    "Cannot be undone. Your workspace, settings and backups are not affected.",
  "sampleWorkspace.delete.expires": "Preview valid until",
  "sampleWorkspace.delete.settingsRevision": "Settings revision",
  "sampleWorkspace.delete.inventory": "Contents digest",
  "sampleWorkspace.delete.payload": "Preview digest",
  "sampleWorkspace.delete.confirmLabel": "Type {phrase} to confirm",
  "sampleWorkspace.delete.phrase": "DELETE SAMPLE DATA",
  "sampleWorkspace.delete.confirm": "Delete sample data",
  "sampleWorkspace.delete.reject": "Keep it",
  "sampleWorkspace.delete.deleting": "Deleting the sample data…",
  "sampleWorkspace.delete.deleted":
    "The sample data is deleted. Switching to sample data prepares it again.",
  "sampleWorkspace.delete.notDeleted": "The sample data was not deleted. Nothing changed.",
  "health.ledger.first_run": "Not opened yet: choose how to begin first.",
  "health.settings.setAside":
    "The settings could not be read, so PMC kept them aside and started with default settings.",
  "health.settings.unavailable": "The settings cannot be read or saved right now.",
  "health.sample.unresolved":
    "An unfinished change to the sample data could not be completed, so the sample data is unavailable.",
  "health.sample.missing": "Sample data was chosen but is not there, so your workspace opened.",
  "health.sample.foreign":
    "The sample folder holds something PMC did not write, so it was left as it is and the sample data is unavailable.",
  "health.sample.cleanupPending":
    "Leftover sample folders waiting to be removed: {count}. PMC removes them at a later start.",
  "gettingStarted.title": "Getting started",
  "gettingStarted.lede":
    "Set up your workspace in this order. Each step is checked from what PMC can see, and this list goes away when all are done.",
  "gettingStarted.backupFolder": "Choose a backup folder",
  "gettingStarted.backupFolder.why": "Backups go to a folder you choose, ideally on another drive.",
  "gettingStarted.passphrase": "Set a recovery passphrase",
  "gettingStarted.passphrase.why":
    "Backups are encrypted with it. A backup cannot be restored without it, and PMC cannot recover a lost passphrase.",
  "gettingStarted.firstBackup": "Make the first backup",
  "gettingStarted.firstBackup.why":
    "PMC accepts new records and changes only after a backup has verified.",
  "gettingStarted.vault": "Choose the Product Vault folder",
  "gettingStarted.vault.why":
    "The folder that holds your Evidence files. PMC records where each file is and its fingerprint; it never copies files.",
  "gettingStarted.firstProduct": "Add your first Product",
  "gettingStarted.firstProduct.why":
    "In Portfolio, select New Product…; the Cockpit places it on the Portfolio Lens.",
  "gettingStarted.status.done": "Done",
  "gettingStarted.status.next": "Next",
  "gettingStarted.status.waits": "Waits for the step above",
  "gettingStarted.status.unknown": "PMC cannot tell yet",
  "gettingStarted.openSettings": "Open Settings",
  "gettingStarted.openPortfolio": "Open Portfolio",
  "settings.vaultAvailable": "Available",
  "settings.vaultUnavailable": "Not available",
  "backup.headline": "Backups",
  "backup.loading": "Reading the backup status.",
  "backup.dueLede":
    "A backup is due. Until one verifies, this workspace does not accept new records or changes.",
  "backup.folder.label": "Backup folder",
  "backup.folder.notSet": "Not set",
  "backup.folder.set": "Set",
  "backup.folder.available": "Available",
  "backup.folder.unavailable": "Unavailable — plug in the drive or choose another folder",
  "backup.folder.choose": "Choose folder…",
  "backup.folder.dialogTitle": "Choose where PMC keeps backups",
  "backup.passphrase.label": "Recovery passphrase",
  "backup.passphrase.notSet": "Not set",
  "backup.passphrase.session": "Set for this session",
  "backup.passphrase.remembered": "Remembered on this Windows account",
  "backup.passphrase.setUp": "Set up passphrase…",
  "backup.last.label": "Last backup",
  "backup.last.none": "No backup yet",
  "backup.last.at": "Verified at {time}. Next one due at {next}.",
  "backup.orphans":
    "{count} backup file(s) in this folder are not in PMC's records and are not counted.",
  "backup.run": "Back up now",
  "backup.running": "Backing up… this can take a minute.",
  "backup.done": "Backed up and verified at {time}.",
  "backup.openBackups": "Open Backups",
  "backup.strip.due": "Back up first, then new records and changes are accepted again.",
  "backup.strip.running": "Backing up… new records and changes wait until it finishes.",
  "backup.passphrase.title": "Set up the recovery passphrase",
  "backup.passphrase.generatedLede":
    "This passphrase is shown once. Write it down or keep it in a password manager, then type it below.",
  "backup.passphrase.generating": "Making a passphrase.",
  "backup.passphrase.retype": "Type it exactly",
  "backup.passphrase.useOwn": "Use my own instead",
  "backup.passphrase.useGenerated": "Use a generated passphrase",
  "backup.passphrase.ownRule":
    "At least 20 characters or six words, and not made only of common passwords.",
  "backup.passphrase.own": "Passphrase",
  "backup.passphrase.ownAgain": "Passphrase again",
  "backup.passphrase.acknowledge":
    "PMC cannot recover this passphrase. Without it, these backups cannot be restored on this or another computer.",
  "backup.passphrase.remember": "Remember on this Windows account for automatic backups.",
  "backup.passphrase.confirm": "Use this passphrase",
  "backup.passphrase.cancel": "Cancel",
} as const;
