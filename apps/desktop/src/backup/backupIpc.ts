import { invoke } from "@tauri-apps/api/core";

/**
 * The reviewed backup commands (S7-A; DG3 backup-setup amendment). No path
 * crosses this boundary in either direction (ADR 0011): the host names the
 * backup folder by its own last component only, and never returns a
 * passphrase it holds.
 */

export type BackupState =
  "not_required" | "checking" | "backing_up" | "restoring" | "due" | "current";
export type BackupDestination = "not_set" | "unavailable" | "available";
export type BackupPassphrase = "not_set" | "session" | "remembered";

export interface BackupLastRunDto {
  readonly runId: number;
  readonly succeeded: boolean;
  readonly failureKey: string | null;
}

export interface BackupStatusDto {
  readonly state: BackupState;
  readonly destination: BackupDestination;
  readonly folderName: string | null;
  readonly passphrase: BackupPassphrase;
  readonly lastVerifiedAtMillis: number | null;
  readonly nextDueAtMillis: number | null;
  readonly orphanCount: number;
  readonly lastRun: BackupLastRunDto | null;
}

export interface BackupDestinationDto {
  readonly configured: boolean;
  readonly available: boolean;
  readonly chosen: boolean;
}

export interface PassphraseStatusDto {
  readonly available: boolean;
  readonly remembered: boolean;
}

/** Everything Settings → Backups asks the host for. */
export interface BackupActions {
  readonly loadBackupStatus: () => Promise<BackupStatusDto>;
  readonly runBackupNow: () => Promise<BackupStatusDto>;
  /** `title` is the folder dialog's title in the person's language. */
  readonly chooseBackupDestination: (title: string) => Promise<BackupDestinationDto>;
  readonly generateRecoveryPassphrase: () => Promise<string>;
  readonly setBackupPassphrase: (
    passphrase: string,
    remember: boolean,
  ) => Promise<PassphraseStatusDto>;
}

export const tauriBackupActions: BackupActions = {
  loadBackupStatus: () => invoke<BackupStatusDto>("get_backup_status"),
  runBackupNow: () => invoke<BackupStatusDto>("run_backup_now"),
  chooseBackupDestination: (title) =>
    invoke<BackupDestinationDto>("choose_backup_destination", { title }),
  generateRecoveryPassphrase: () => invoke<string>("generate_recovery_passphrase"),
  setBackupPassphrase: (passphrase, remember) =>
    invoke<PassphraseStatusDto>("set_backup_passphrase", { passphrase, remember }),
};
