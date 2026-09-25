import { invoke } from "@tauri-apps/api/core";

/**
 * The reviewed Vault-folder commands (item ⑦, H2b; DG3 Vault-root
 * amendment §2–§3). The picked folder stays in the host behind an opaque
 * token; the webview learns its own name only (ADR 0011).
 */

export interface ChosenVaultFolderDto {
  readonly chosen: boolean;
  readonly token: string | null;
  readonly folderName: string | null;
}

export interface VaultRootPreviewDto {
  readonly preparedIntentId: string;
  readonly payloadSha256: string;
  readonly proposedFolderName: string;
  /** `null` when no Vault was set. */
  readonly previousFolderName: string | null;
  readonly evidenceCount: number;
  readonly resolvedCount: number;
  readonly recoveryVerifiedAtMillis: number;
  /** The code the person types after the fixed phrase. */
  readonly confirmationCode: string;
  readonly expiresAtMillis: number;
}

export interface VaultRootResultDto {
  readonly outcome: "changed" | "not_changed";
}

export interface VaultRootActions {
  /** `title` is the folder dialog's title in the person's language. */
  readonly chooseVaultFolder: (title: string) => Promise<ChosenVaultFolderDto>;
  readonly prepareVaultRootChange: (token: string) => Promise<VaultRootPreviewDto>;
  readonly rejectVaultRootChange: (preparedIntentId: string) => Promise<void>;
  readonly approveVaultRootChange: (
    preparedIntentId: string,
    payloadSha256: string,
    typedConfirmation: string,
    clientRequestId: string,
  ) => Promise<VaultRootResultDto>;
}

export const tauriVaultRootActions: VaultRootActions = {
  chooseVaultFolder: (title) => invoke<ChosenVaultFolderDto>("choose_vault_folder", { title }),
  prepareVaultRootChange: (token) =>
    invoke<VaultRootPreviewDto>("prepare_vault_root_change", { token }),
  rejectVaultRootChange: (preparedIntentId) =>
    invoke<null>("reject_vault_root_change", { preparedIntentId }).then(() => undefined),
  approveVaultRootChange: (preparedIntentId, payloadSha256, typedConfirmation, clientRequestId) =>
    invoke<VaultRootResultDto>("approve_vault_root_change", {
      preparedIntentId,
      payloadSha256,
      typedConfirmation,
      clientRequestId,
    }),
};
