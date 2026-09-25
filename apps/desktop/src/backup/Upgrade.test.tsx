import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { BackupActions, BackupStatusDto } from "./backupIpc";
import type { BackupsSectionProps } from "./BackupsSection";
import type { RestoreActions } from "./restoreIpc";
import type { UpgradeActions, UpgradeGateDto, UpgradeResultDto } from "./upgradeIpc";
import { UpgradeGate } from "./UpgradeGate";

const GATE: UpgradeGateDto = {
  state: "upgrade",
  appVersion: "0.2.0",
  fromSchema: 46,
  toSchema: 47,
  recordCount: 139,
  live: true,
};

function status(overrides: Partial<BackupStatusDto> = {}): BackupStatusDto {
  return {
    state: "current",
    destination: "available",
    folderName: "Backups",
    passphrase: "session",
    lastVerifiedAtMillis: 1_790_000_000_000,
    nextDueAtMillis: 1_790_086_400_000,
    orphanCount: 0,
    lastRun: null,
    ...overrides,
  };
}

function backups(overrides: Partial<BackupStatusDto> = {}): BackupsSectionProps {
  const actions: BackupActions = {
    loadBackupStatus: vi.fn(() => Promise.resolve(status(overrides))),
    runBackupNow: vi.fn(() => Promise.resolve(status(overrides))),
    chooseBackupDestination: vi.fn(() =>
      Promise.resolve({ configured: true, available: true, chosen: true }),
    ),
    generateRecoveryPassphrase: vi.fn(() => Promise.resolve("words")),
    setBackupPassphrase: vi.fn(() => Promise.resolve({ available: true, remembered: false })),
  };
  return { actions, status: status(overrides), onStatus: vi.fn(), refresh: vi.fn() };
}

function upgradeActions(result: UpgradeResultDto): UpgradeActions {
  return {
    loadUpgradeGate: vi.fn(() => Promise.resolve(GATE)),
    runUpgrade: vi.fn(() => Promise.resolve(result)),
    quit: vi.fn(() => Promise.resolve()),
  };
}

describe("The upgrade gate", () => {
  it("shows both formats and the record count, and upgrades", async () => {
    const actions = upgradeActions({
      outcome: "upgraded",
      backupVerifiedAtMillis: 1_790_000_100_000,
      ledger: "ready",
    });
    const onUpgraded = vi.fn();
    render(
      <UpgradeGate
        gate={GATE}
        actions={actions}
        backups={backups()}
        onUpgraded={onUpgraded}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    expect(screen.getByRole("heading", { name: "升級這個工作區" })).toBeVisible();
    expect(screen.getByText(/PMC 0\.2\.0 用較新的格式/)).toBeVisible();
    expect(screen.getByText("schema 46")).toBeVisible();
    expect(screen.getByText("schema 47")).toBeVisible();
    expect(screen.getByText("139")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "升級" }));
    await screen.findByText(/工作區已升級。.* 的備份保存了升級前的狀態。/);
    fireEvent.click(screen.getByRole("button", { name: "繼續" }));
    expect(onUpgraded).toHaveBeenCalled();
  });

  it("keeps Upgrade disabled until the backup folder and passphrase are set", () => {
    render(
      <UpgradeGate
        gate={GATE}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups({ passphrase: "not_set" })}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "升級" })).toBeDisabled();
    expect(screen.getByText(/請先備份：設定備份資料夾與密語/)).toBeVisible();
    expect(screen.getByRole("button", { name: "設定密語…" })).toBeVisible();
    // Only the setup rows: no "Back up now" on the gate.
    expect(screen.queryByRole("button", { name: "立即備份" })).toBeNull();
  });

  it("offers Try again when the upgrade rolled back", async () => {
    const actions = upgradeActions({
      outcome: "rolled_back",
      backupVerifiedAtMillis: 1_790_000_100_000,
      ledger: "upgrade_required",
    });
    render(
      <UpgradeGate
        gate={GATE}
        actions={actions}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "升級" }));
    await screen.findByText("升級沒有完成，沒有任何變更。");
    fireEvent.click(screen.getByRole("button", { name: "再試一次" }));
    expect(actions.runUpgrade).toHaveBeenCalledTimes(2);
  });

  it("sends an outcome it cannot tell to System Health", async () => {
    const onOpenSystemHealth = vi.fn();
    render(
      <UpgradeGate
        gate={GATE}
        actions={upgradeActions({
          outcome: "outcome_unknown",
          backupVerifiedAtMillis: 1_790_000_100_000,
          ledger: "open_failed",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={onOpenSystemHealth}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "升級" }));
    await screen.findByText(/PMC 無法判斷升級是否完成/);
    fireEvent.click(screen.getByRole("button", { name: "開啟 System Health" }));
    expect(onOpenSystemHealth).toHaveBeenCalled();
  });

  it("offers only Quit for a Ledger made by a newer PMC", () => {
    render(
      <UpgradeGate
        gate={{ ...GATE, state: "newer_version", fromSchema: null, toSchema: null }}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    expect(screen.getByText(/由較新版本的 PMC 建立/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "升級" })).toBeNull();
    expect(screen.getByRole("button", { name: "結束 PMC" })).toBeVisible();
  });
  it("never offers an upgrade for Training, and starts one upgrade on a double click", async () => {
    const { unmount } = render(
      <UpgradeGate
        gate={{ ...GATE, state: "training_reset", live: false }}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    expect(screen.getByText(/請用範例工作區重設/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "升級" })).toBeNull();
    unmount();

    const actions = upgradeActions({
      outcome: "upgraded",
      backupVerifiedAtMillis: 1_790_000_100_000,
      ledger: "ready",
    });
    render(
      <UpgradeGate
        gate={GATE}
        actions={actions}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    const button = screen.getByRole("button", { name: "升級" });
    fireEvent.click(button);
    fireEvent.click(button);
    await screen.findByText(/工作區已升級/);
    expect(actions.runUpgrade).toHaveBeenCalledTimes(1);
  });
});

function restoreActions(): RestoreActions {
  return {
    chooseRestoreArchive: vi.fn(() =>
      Promise.resolve({ chosen: false, token: null, fileName: null }),
    ),
    chooseRecoveryArchive: vi.fn(() =>
      Promise.resolve({ chosen: false, token: null, fileName: null }),
    ),
    discardRestoreSelection: vi.fn(() => Promise.resolve()),
    checkRestoreArchive: vi.fn(() => Promise.reject(new Error("not used"))),
    prepareRestore: vi.fn(() => Promise.reject(new Error("not used"))),
    rejectPreparedRestore: vi.fn(() => Promise.resolve()),
    approveAndExecuteRestore: vi.fn(() => Promise.reject(new Error("not used"))),
  };
}

describe("Restore from the upgrade gate (restore-unopened amendment, first cut)", () => {
  it("sits beside Upgrade, closed until backups are set, and opens the restore sheet", () => {
    const restore = { actions: restoreActions(), onFinished: vi.fn() };
    const { unmount } = render(
      <UpgradeGate
        gate={GATE}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups({ destination: "not_set", folderName: null })}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        restore={restore}
      />,
    );
    // The recovery backup needs the same folder and passphrase as the upgrade.
    expect(screen.getByRole("button", { name: "從備份還原…" })).toBeDisabled();
    unmount();

    render(
      <UpgradeGate
        gate={GATE}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        restore={restore}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "從備份還原…" }));
    expect(screen.getByRole("dialog")).toBeVisible();
  });

  it("is offered again after an upgrade rolled back", async () => {
    render(
      <UpgradeGate
        gate={GATE}
        actions={upgradeActions({
          outcome: "rolled_back",
          backupVerifiedAtMillis: 1_790_000_100_000,
          ledger: "upgrade_required",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        restore={{ actions: restoreActions(), onFinished: vi.fn() }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "升級" }));
    await screen.findByText("升級沒有完成，沒有任何變更。");
    fireEvent.click(screen.getByRole("button", { name: "從備份還原…" }));
    expect(screen.getByRole("dialog")).toBeVisible();
  });

  it("is not offered without a restore (the Training workspace) or for a newer Ledger", () => {
    const { unmount } = render(
      <UpgradeGate
        gate={GATE}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "從備份還原…" })).toBeNull();
    unmount();
    render(
      <UpgradeGate
        gate={{ ...GATE, state: "newer_version", fromSchema: null, toSchema: null }}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        restore={{ actions: restoreActions(), onFinished: vi.fn() }}
      />,
    );
    // No downgrade (upgrade-gate amendment §2 stands).
    expect(screen.queryByRole("button", { name: "從備份還原…" })).toBeNull();
  });
});

describe("Restore on an older unsupported Ledger (restore-unopened amendment, cut B)", () => {
  it("offers restore beside Quit, closed until backups are set, and opens the sheet", () => {
    const blocked = {
      ...GATE,
      state: "unsupported_old" as const,
      fromSchema: null,
      toSchema: null,
    };
    const restore = { actions: restoreActions(), onFinished: vi.fn() };
    const { unmount } = render(
      <UpgradeGate
        gate={blocked}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups({ passphrase: "not_set" })}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        restore={restore}
      />,
    );
    expect(screen.getByRole("button", { name: "從備份還原…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "設定密語…" })).toBeVisible();
    unmount();
    render(
      <UpgradeGate
        gate={blocked}
        actions={upgradeActions({
          outcome: "upgraded",
          backupVerifiedAtMillis: 0,
          ledger: "ready",
        })}
        backups={backups()}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        restore={restore}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "從備份還原…" }));
    expect(screen.getByRole("dialog")).toBeVisible();
    expect(screen.getByRole("button", { name: "結束 PMC" })).toBeVisible();
  });
});
