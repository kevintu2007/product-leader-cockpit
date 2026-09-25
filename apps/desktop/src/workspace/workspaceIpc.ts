import { invoke } from "@tauri-apps/api/core";

/**
 * The reviewed sample-workspace commands (item ⑨; the accepted
 * sample-workspace amendment). The host decides which workspace is open;
 * the webview only asks. A change of workspace restarts PMC once the choice
 * is saved.
 */

export type WorkspaceName = "live" | "training";

export interface WorkspaceStatusDto {
  /** `null` while the first-run choice is shown. */
  readonly open: WorkspaceName | null;
  /** `null` when nothing is chosen yet. */
  readonly selected: WorkspaceName | null;
  readonly firstRun: boolean;
  /** What startup found; a reset or delete during this run does not change it. */
  readonly sample: "available" | "unresolved" | "missing" | "foreign";
  /** Sample data was chosen but could not be opened; Live opened. */
  readonly sampleFellBack: boolean;
  readonly settingsSetAside: boolean;
  readonly settingsUnavailable: boolean;
  /** At startup; System Health reads the current count. */
  readonly sampleCleanupPending: number;
}

export interface WorkspaceChangeDto {
  readonly outcome: "restarting";
}

export interface SampleResetDto {
  readonly seedVersion: number;
}

export interface SampleDeletePreviewDto {
  readonly preparedIntentId: string;
  readonly payloadSha256: string;
  readonly seedId: string;
  readonly seedVersion: number;
  readonly hasLedger: boolean;
  readonly hasVault: boolean;
  readonly hasGeneratedFiles: boolean;
  readonly settingsRevision: number;
  readonly operation: string;
  readonly inventorySha256: string;
  readonly effect: string;
  readonly expiresAtMillis: number;
}

export interface SampleDeletedDto {
  readonly outcome: "deleted" | "not_deleted";
}

export interface WorkspaceActions {
  readonly loadWorkspaceStatus: () => Promise<WorkspaceStatusDto>;
  readonly chooseFirstWorkspace: (choice: WorkspaceName) => Promise<WorkspaceChangeDto>;
  readonly switchWorkspace: (target: WorkspaceName) => Promise<WorkspaceChangeDto>;
  readonly resetSampleData: (clientRequestId: string) => Promise<SampleResetDto>;
  readonly prepareSampleDelete: () => Promise<SampleDeletePreviewDto>;
  readonly rejectSampleDelete: (preparedIntentId: string) => Promise<void>;
  readonly approveSampleDelete: (
    preparedIntentId: string,
    payloadSha256: string,
    typedConfirmation: string,
    clientRequestId: string,
  ) => Promise<SampleDeletedDto>;
}

export const tauriWorkspaceActions: WorkspaceActions = {
  loadWorkspaceStatus: () => invoke<WorkspaceStatusDto>("get_workspace_status"),
  chooseFirstWorkspace: (choice) =>
    invoke<WorkspaceChangeDto>("choose_first_workspace", { choice }),
  switchWorkspace: (target) => invoke<WorkspaceChangeDto>("switch_workspace", { target }),
  resetSampleData: (clientRequestId) =>
    invoke<SampleResetDto>("reset_sample_data", { clientRequestId }),
  prepareSampleDelete: () => invoke<SampleDeletePreviewDto>("prepare_sample_delete"),
  rejectSampleDelete: (preparedIntentId) =>
    invoke<null>("reject_sample_delete", { preparedIntentId }).then(() => undefined),
  approveSampleDelete: (preparedIntentId, payloadSha256, typedConfirmation, clientRequestId) =>
    invoke<SampleDeletedDto>("approve_sample_delete", {
      preparedIntentId,
      payloadSha256,
      typedConfirmation,
      clientRequestId,
    }),
};

export function newClientRequestId(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `sample-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}
