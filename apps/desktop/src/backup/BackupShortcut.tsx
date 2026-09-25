import { createContext, useContext } from "react";

/**
 * Opens Settings → Backups from wherever a write was refused because a
 * backup is due or running (DG3 backup-setup amendment §5: the refusal
 * carries "a link to Settings → Backups"). `null` outside the app shell.
 */
export const BackupShortcutContext = createContext<(() => void) | null>(null);

export function useBackupShortcut(): (() => void) | null {
  return useContext(BackupShortcutContext);
}

/** The host's error codes for a write the backup gate refused. */
export function isBackupRefusal(errorCode: string | undefined): boolean {
  return errorCode === "BACKUP_DUE" || errorCode === "BACKUP_RUNNING";
}
