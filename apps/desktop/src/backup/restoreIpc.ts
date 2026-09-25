import { invoke } from "@tauri-apps/api/core";

/**
 * The reviewed restore commands (S7-B1; DG3 restore amendment). The picked
 * file stays in the host behind an opaque token; the webview learns its own
 * name only (ADR 0011). The passphrase typed here opens that one file and is
 * never stored.
 */

export interface ChosenBackupDto {
  readonly chosen: boolean;
  readonly token: string | null;
  readonly fileName: string | null;
}

export interface CheckedBackupDto {
  /** RFC 3339, UTC. */
  readonly createdAt: string;
  readonly schemaVersion: number;
  readonly recordCount: number;
  readonly needsUpgrade: boolean;
}

export interface RestorePreviewDto {
  readonly preparedIntentId: string;
  readonly payloadSha256: string;
  readonly archiveCreatedAt: string;
  readonly archiveSchemaVersion: number;
  readonly archiveRecordCount: number;
  /** `null` when PMC cannot open the current Ledger: unavailable, not zero. */
  readonly currentRecordCount: number | null;
  readonly currentLastChangeAtMillis: number | null;
  /** The recovery evidence's kind (restore-unopened amendment §1). */
  readonly recoveryKind: "operational_backup" | "preservation_copy";
  /** Its file name. */
  readonly recoveryName: string;
  readonly recoveryVerifiedAtMillis: number;
  /** `YYYY-MM-DD`, the date the person types. */
  readonly confirmationDate: string;
  readonly needsUpgrade: boolean;
  readonly expiresAtMillis: number;
}

export type RestoreOutcome =
  | "restored"
  | "failed_before_replacement"
  | "recovery_put_back"
  | "recovery_failed"
  /** Approved, then stopped without an outcome; the next start puts it back. */
  | "interrupted";

export type LedgerAvailability =
  | "ready"
  | "replacing"
  | "restore_recovery_required"
  | "upgrade_required"
  | "unsupported_old"
  | "newer_version"
  | "open_failed"
  // A new profile that has not chosen how to begin (item ⑨).
  | "first_run";

export interface RestoreResultDto {
  readonly outcome: RestoreOutcome;
  /** Nothing was replaced because the current Ledger's files changed after
   * the preview (restore-unopened amendment §5). */
  readonly sourceChanged: boolean;
  readonly ledger: LedgerAvailability;
}

export interface SystemHealthDto {
  readonly ledger: LedgerAvailability;
  /** The recovery backup's file name, when a restore could not be put back. */
  readonly recoveryBackup: string | null;
  /** The settings document as startup found it (item ⑨, §2). */
  readonly settings: "ok" | "set_aside" | "unavailable";
  /** Why chosen sample data did not open, or why the sample cannot be used. */
  readonly sample: "unresolved" | "missing" | "foreign" | null;
  /** Folders a finished sample operation could not remove yet. */
  readonly sampleCleanupPending: number;
}

export interface RestoreActions {
  /** `title` and `filterName` are the file dialog's labels in the person's language. */
  readonly chooseRestoreArchive: (title: string, filterName: string) => Promise<ChosenBackupDto>;
  /** The recovery backup a failed restore left, from the backup registry. */
  readonly chooseRecoveryArchive: () => Promise<ChosenBackupDto>;
  readonly discardRestoreSelection: () => Promise<void>;
  readonly checkRestoreArchive: (token: string, passphrase: string) => Promise<CheckedBackupDto>;
  readonly prepareRestore: (token: string) => Promise<RestorePreviewDto>;
  readonly rejectPreparedRestore: (preparedIntentId: string) => Promise<void>;
  readonly approveAndExecuteRestore: (
    preparedIntentId: string,
    payloadSha256: string,
    typedDate: string,
    clientRequestId: string,
  ) => Promise<RestoreResultDto>;
}

export const tauriRestoreActions: RestoreActions = {
  chooseRestoreArchive: (title, filterName) =>
    invoke<ChosenBackupDto>("choose_restore_archive", { title, filterName }),
  chooseRecoveryArchive: () => invoke<ChosenBackupDto>("choose_recovery_archive"),
  discardRestoreSelection: () => invoke<null>("discard_restore_selection").then(() => undefined),
  checkRestoreArchive: (token, passphrase) =>
    invoke<CheckedBackupDto>("check_restore_archive", { token, passphrase }),
  prepareRestore: (token) => invoke<RestorePreviewDto>("prepare_restore_from_archive", { token }),
  rejectPreparedRestore: (preparedIntentId) =>
    invoke<null>("reject_prepared_restore", { preparedIntentId }).then(() => undefined),
  approveAndExecuteRestore: (preparedIntentId, payloadSha256, typedDate, clientRequestId) =>
    invoke<RestoreResultDto>("approve_and_execute_restore", {
      preparedIntentId,
      payloadSha256,
      typedDate,
      clientRequestId,
    }),
};

export interface SystemHealthActions {
  readonly loadSystemHealth: () => Promise<SystemHealthDto>;
  readonly quit: () => Promise<void>;
}

export const tauriSystemHealthActions: SystemHealthActions = {
  loadSystemHealth: () => invoke<SystemHealthDto>("get_system_health"),
  quit: () => invoke<null>("quit_pmc").then(() => undefined),
};
