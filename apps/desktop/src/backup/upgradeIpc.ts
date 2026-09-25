import { invoke } from "@tauri-apps/api/core";

import type { LedgerAvailability } from "./restoreIpc";

/**
 * The reviewed upgrade-gate commands (DG3 upgrade-gate amendment). The
 * webview supplies nothing to the upgrade itself; it learns the formats, the
 * record count and the outcome — never a path.
 */

export type UpgradeGateState =
  "none" | "upgrade" | "training_reset" | "unsupported_old" | "newer_version";

export interface UpgradeGateDto {
  readonly state: UpgradeGateState;
  readonly appVersion: string;
  readonly fromSchema: number | null;
  readonly toSchema: number | null;
  readonly recordCount: number | null;
  readonly live: boolean;
}

export type UpgradeOutcome = "upgraded" | "rolled_back" | "upgraded_unreadable" | "outcome_unknown";

export interface UpgradeResultDto {
  readonly outcome: UpgradeOutcome;
  readonly backupVerifiedAtMillis: number;
  readonly ledger: LedgerAvailability;
}

export interface UpgradeActions {
  readonly loadUpgradeGate: () => Promise<UpgradeGateDto>;
  readonly runUpgrade: () => Promise<UpgradeResultDto>;
  readonly quit: () => Promise<void>;
}

export const tauriUpgradeActions: UpgradeActions = {
  loadUpgradeGate: () => invoke<UpgradeGateDto>("get_upgrade_gate"),
  runUpgrade: () => invoke<UpgradeResultDto>("run_upgrade"),
  quit: () => invoke<null>("quit_pmc").then(() => undefined),
};
