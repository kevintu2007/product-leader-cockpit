import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SystemHealth } from "../routes/SystemHealth";
import type { BackupsSectionProps } from "./BackupsSection";
import type { BackupActions, BackupStatusDto } from "./backupIpc";
import type { RestoreActions, RestorePreviewDto, SystemHealthActions } from "./restoreIpc";
import { RestoreSheet } from "./RestoreSheet";

const PREVIEW: RestorePreviewDto = {
  preparedIntentId: "intent-1",
  payloadSha256: "a".repeat(64),
  archiveCreatedAt: "2026-09-20T02:00:00Z",
  archiveSchemaVersion: 46,
  archiveRecordCount: 139,
  currentRecordCount: 12,
  currentLastChangeAtMillis: 1_790_000_000_000,
  recoveryKind: "operational_backup",
  recoveryName: "pmc-operational-20260920T020000000Z-b1b1-v1.tar.zst.age",
  recoveryVerifiedAtMillis: 1_790_000_100_000,
  confirmationDate: "2026-09-20",
  needsUpgrade: false,
  expiresAtMillis: 1_790_000_900_000,
};

/** What System Health adds beside the Ledger: nothing wrong with the
 * settings or the sample. */
const HEALTH_OK = { settings: "ok", sample: null, sampleCleanupPending: 0 } as const;
function refusal(errorCode: string, messageKey: string) {
  return Object.assign(new Error("host refused"), {
    errorCode,
    messageKey,
    correlationId: "host-1",
    retryable: false,
    messageParams: [],
    extensions: [],
  });
}

function actions(overrides: Partial<RestoreActions> = {}): RestoreActions {
  return {
    chooseRestoreArchive: vi.fn(() =>
      Promise.resolve({
        chosen: true,
        token: "token-1",
        fileName: "pmc-operational-20260920-v1.tar.zst.age",
      }),
    ),
    chooseRecoveryArchive: vi.fn(() =>
      Promise.resolve({
        chosen: true,
        token: "token-recovery",
        fileName: "pmc-operational-20260919-recovery-v1.tar.zst.age",
      }),
    ),
    discardRestoreSelection: vi.fn(() => Promise.resolve()),
    checkRestoreArchive: vi.fn(() =>
      Promise.resolve({
        createdAt: PREVIEW.archiveCreatedAt,
        schemaVersion: 46,
        recordCount: 139,
        needsUpgrade: false,
      }),
    ),
    prepareRestore: vi.fn(() => Promise.resolve(PREVIEW)),
    rejectPreparedRestore: vi.fn(() => Promise.resolve()),
    approveAndExecuteRestore: vi.fn(() =>
      Promise.resolve({
        outcome: "restored" as const,
        sourceChanged: false,
        ledger: "ready" as const,
      }),
    ),
    ...overrides,
  };
}

async function reachPreview(host: RestoreActions) {
  fireEvent.click(screen.getByRole("button", { name: "選擇備份檔…" }));
  await screen.findByText("備份檔：pmc-operational-20260920-v1.tar.zst.age");
  fireEvent.change(screen.getByLabelText("這份備份的密語"), { target: { value: "secret words" } });
  fireEvent.click(screen.getByRole("button", { name: "檢查備份" }));
  await screen.findByText(/139 筆紀錄/);
  await waitFor(() => {
    expect(host.prepareRestore).toHaveBeenCalledWith("token-1");
  });
  await screen.findByLabelText(/請輸入備份的建立日期以確認：2026-09-20/);
}

describe("Restore from a backup", () => {
  it("replaces only after the backup's date is typed, then reports the result", async () => {
    const host = actions();
    const onFinished = vi.fn();
    render(<RestoreSheet actions={host} onClose={vi.fn()} onFinished={onFinished} />);
    await reachPreview(host);
    expect(host.checkRestoreArchive).toHaveBeenCalledWith("token-1", "secret words");
    expect(screen.getByText(/12 筆紀錄，最後變更於/)).toBeVisible();
    expect(screen.getByText(/Product Vault、備份資料夾與密語設定/)).toBeVisible();

    const replace = screen.getByRole("button", { name: "以這份備份取代" });
    expect(replace).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/請輸入備份的建立日期/), {
      target: { value: "2026-09-21" },
    });
    expect(replace).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/請輸入備份的建立日期/), {
      target: { value: "2026-09-20" },
    });
    expect(replace).toBeEnabled();
    fireEvent.click(replace);

    await screen.findByText(/已還原 .* 的備份。原本的內容已存成 .* 的備份。/);
    const [intent, digest, typed] = vi.mocked(host.approveAndExecuteRestore).mock.calls[0] ?? [];
    expect([intent, digest, typed]).toEqual(["intent-1", PREVIEW.payloadSha256, "2026-09-20"]);
    fireEvent.click(screen.getByRole("button", { name: "繼續" }));
    expect(onFinished).toHaveBeenCalledWith({
      outcome: "restored",
      sourceChanged: false,
      ledger: "ready",
    });
  });

  it("keeps the sheet on the passphrase when it does not open the backup", async () => {
    const host = actions({
      checkRestoreArchive: vi.fn(() =>
        Promise.reject(refusal("RESTORE_WRONG_PASSPHRASE", "desktop.restore_wrong_passphrase")),
      ),
    });
    render(<RestoreSheet actions={host} onClose={vi.fn()} onFinished={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "選擇備份檔…" }));
    await screen.findByLabelText("這份備份的密語");
    fireEvent.change(screen.getByLabelText("這份備份的密語"), { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "檢查備份" }));
    await screen.findByText(/這個密語打不開這份備份。/);
    expect(host.prepareRestore).not.toHaveBeenCalled();
    expect(screen.getByLabelText("這份備份的密語")).toBeEnabled();
  });

  it("records a rejection and changes nothing", async () => {
    const host = actions();
    const onClose = vi.fn();
    render(<RestoreSheet actions={host} onClose={onClose} onFinished={vi.fn()} />);
    await reachPreview(host);
    fireEvent.click(screen.getByRole("button", { name: "不還原" }));
    await waitFor(() => {
      expect(onClose).toHaveBeenCalled();
    });
    expect(host.rejectPreparedRestore).toHaveBeenCalledWith("intent-1");
    expect(host.approveAndExecuteRestore).not.toHaveBeenCalled();
  });

  it("says an unrecoverable restore needs System Health", async () => {
    const host = actions({
      approveAndExecuteRestore: vi.fn(() =>
        Promise.resolve({
          outcome: "recovery_failed" as const,
          sourceChanged: false,
          ledger: "restore_recovery_required" as const,
        }),
      ),
    });
    render(<RestoreSheet actions={host} onClose={vi.fn()} onFinished={vi.fn()} />);
    await reachPreview(host);
    fireEvent.change(screen.getByLabelText(/請輸入備份的建立日期/), {
      target: { value: "2026-09-20" },
    });
    fireEvent.click(screen.getByRole("button", { name: "以這份備份取代" }));
    await screen.findByText(/PMC 無法把原本的工作區放回去/);
    expect(screen.getByRole("button", { name: "開啟 System Health" })).toBeVisible();
  });
});

describe("Restore once it has started", () => {
  it("cannot be closed while the current workspace is being backed up", async () => {
    const host = actions({
      prepareRestore: vi.fn(() => new Promise<RestorePreviewDto>(() => undefined)),
    });
    const onClose = vi.fn();
    render(<RestoreSheet actions={host} onClose={onClose} onFinished={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "選擇備份檔…" }));
    await screen.findByLabelText("這份備份的密語");
    fireEvent.change(screen.getByLabelText("這份備份的密語"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "檢查備份" }));
    await screen.findByText(/取代任何東西之前，PMC 會先備份目前的工作區/);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    expect(host.discardRestoreSelection).not.toHaveBeenCalled();
  });

  it("says an approved restore that stopped is put back at the next start", async () => {
    const host = actions({
      approveAndExecuteRestore: vi.fn(() =>
        Promise.resolve({
          outcome: "interrupted" as const,
          sourceChanged: false,
          ledger: "restore_recovery_required" as const,
        }),
      ),
    });
    render(<RestoreSheet actions={host} onClose={vi.fn()} onFinished={vi.fn()} />);
    await reachPreview(host);
    fireEvent.change(screen.getByLabelText(/請輸入備份的建立日期/), {
      target: { value: "2026-09-20" },
    });
    fireEvent.click(screen.getByRole("button", { name: "以這份備份取代" }));
    await screen.findByText(/PMC 下次啟動時會把原本的工作區放回去/);
    expect(screen.getByRole("button", { name: "開啟 System Health" })).toBeVisible();
  });
});
describe("System Health", () => {
  function health(overrides: Partial<SystemHealthActions> = {}): SystemHealthActions {
    return {
      loadSystemHealth: vi.fn(() =>
        Promise.resolve({
          ledger: "restore_recovery_required" as const,
          recoveryBackup: "pmc-operational-20260922-abcd-v1.tar.zst.age",
          ...HEALTH_OK,
        }),
      ),
      quit: vi.fn(() => Promise.resolve()),
      ...overrides,
    };
  }

  it("names the recovery backup and offers Quit when the Ledger did not open", async () => {
    const host = health();
    render(<SystemHealth actions={host} />);
    await screen.findByText(/有一次還原中途停止/);
    expect(screen.getByText(/pmc-operational-20260922-abcd-v1\.tar\.zst\.age/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "結束 PMC" }));
    expect(host.quit).toHaveBeenCalled();
  });

  it("says the Ledger is open, with no Quit", async () => {
    render(
      <SystemHealth
        actions={health({
          loadSystemHealth: vi.fn(() =>
            Promise.resolve({
              ledger: "ready" as const,
              recoveryBackup: null,
              ...HEALTH_OK,
            }),
          ),
        })}
      />,
    );
    await screen.findByText("已開啟，可以讀取。");
    expect(screen.queryByRole("button", { name: "結束 PMC" })).toBeNull();
  });
});

const PRESERVED: RestorePreviewDto = {
  ...PREVIEW,
  currentRecordCount: null,
  currentLastChangeAtMillis: null,
  recoveryKind: "preservation_copy",
  recoveryName: "pmc-preservation-20260923T010203000Z-p1p1-v1.tar.zst.age",
};

function backups(overrides: Partial<BackupStatusDto> = {}): BackupsSectionProps {
  const status: BackupStatusDto = {
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
  const backupActions: BackupActions = {
    loadBackupStatus: vi.fn(() => Promise.resolve(status)),
    runBackupNow: vi.fn(() => Promise.resolve(status)),
    chooseBackupDestination: vi.fn(() =>
      Promise.resolve({ configured: true, available: true, chosen: true }),
    ),
    generateRecoveryPassphrase: vi.fn(() => Promise.resolve("words")),
    setBackupPassphrase: vi.fn(() => Promise.resolve({ available: true, remembered: false })),
  };
  return {
    actions: backupActions,
    status,
    onStatus: vi.fn(),
    refresh: vi.fn(),
    restore: {
      actions: actions({ prepareRestore: vi.fn(() => Promise.resolve(PRESERVED)) }),
      onFinished: vi.fn(),
    },
  };
}

describe("Restore when the Ledger cannot be opened (restore-unopened amendment)", () => {
  it("says the current Ledger's count is unavailable and names the preservation copy as such", async () => {
    const host = actions({ prepareRestore: vi.fn(() => Promise.resolve(PRESERVED)) });
    render(
      <RestoreSheet actions={host} onClose={vi.fn()} onFinished={vi.fn()} currentUnavailable />,
    );
    await reachPreview(host);
    expect(
      screen.getByText("PMC 無法開啟目前的 Product Ledger，所以無法得知它的紀錄數與最後變更時間。"),
    ).toBeVisible();
    // Never a count of zero, never "no change yet".
    expect(screen.queryByText(/0 筆紀錄/)).toBeNull();
    expect(screen.queryByText(/尚無變更/)).toBeNull();
    expect(screen.getByText(/不是 Operational Backup，也不算你最近一次驗證過的備份/)).toBeVisible();
    expect(
      screen.getByText(/pmc-preservation-20260923T010203000Z-p1p1-v1\.tar\.zst\.age/),
    ).toBeVisible();
  });

  it("reports a restored or put-back result in the preservation copy's words", async () => {
    for (const [outcome, text] of [
      ["restored", /原本在這裡的檔案保存為「pmc-preservation-/],
      ["recovery_put_back", /PMC 已把原本的 Ledger 檔案原樣放回。Product Ledger 仍然無法開啟/],
    ] as const) {
      const host = actions({
        prepareRestore: vi.fn(() => Promise.resolve(PRESERVED)),
        approveAndExecuteRestore: vi.fn(() =>
          Promise.resolve({ outcome, sourceChanged: false, ledger: "open_failed" as const }),
        ),
      });
      const { unmount } = render(
        <RestoreSheet actions={host} onClose={vi.fn()} onFinished={vi.fn()} currentUnavailable />,
      );
      await reachPreview(host);
      fireEvent.change(screen.getByLabelText(/請輸入備份的建立日期以確認/), {
        target: { value: "2026-09-20" },
      });
      fireEvent.click(screen.getByRole("button", { name: "以這份備份取代" }));
      expect(await screen.findByText(text)).toBeVisible();
      unmount();
    }
  });

  it("offers restore on System Health for a Ledger that does not open, once backups are set", async () => {
    const health: SystemHealthActions = {
      loadSystemHealth: vi.fn(() =>
        Promise.resolve({ ledger: "open_failed" as const, recoveryBackup: null, ...HEALTH_OK }),
      ),
      quit: vi.fn(() => Promise.resolve()),
    };
    const { unmount } = render(
      <SystemHealth actions={health} backups={backups({ passphrase: "not_set" })} />,
    );
    const button = await screen.findByRole("button", { name: "從備份還原…" });
    expect(button).toBeDisabled();
    expect(
      screen.getByText(/還原前，PMC 會先把目前的 Ledger 檔案保存一份到備份資料夾/),
    ).toBeVisible();
    unmount();

    render(<SystemHealth actions={health} backups={backups()} />);
    fireEvent.click(await screen.findByRole("button", { name: "從備份還原…" }));
    expect(screen.getByRole("dialog")).toBeVisible();
  });

  it("never offers restore for a newer Ledger, or without a Live workspace's restore", async () => {
    const newer: SystemHealthActions = {
      loadSystemHealth: vi.fn(() =>
        Promise.resolve({ ledger: "newer_version" as const, recoveryBackup: null, ...HEALTH_OK }),
      ),
      quit: vi.fn(() => Promise.resolve()),
    };
    const { unmount } = render(<SystemHealth actions={newer} backups={backups()} />);
    await screen.findByRole("button", { name: "結束 PMC" });
    expect(screen.queryByRole("button", { name: "從備份還原…" })).toBeNull();
    unmount();
    const failed: SystemHealthActions = {
      ...newer,
      loadSystemHealth: vi.fn(() =>
        Promise.resolve({ ledger: "open_failed" as const, recoveryBackup: null, ...HEALTH_OK }),
      ),
    };
    render(<SystemHealth actions={failed} backups={{ ...backups(), restore: undefined }} />);
    await screen.findByRole("button", { name: "結束 PMC" });
    expect(screen.queryByRole("button", { name: "從備份還原…" })).toBeNull();
  });
});

describe("After a failed recovery (restore-unopened amendment §2, §5)", () => {
  it("offers the recovery backup first, without a picker, and still allows another", async () => {
    const health: SystemHealthActions = {
      loadSystemHealth: vi.fn(() =>
        Promise.resolve({
          ledger: "restore_recovery_required" as const,
          recoveryBackup: "pmc-operational-20260919-recovery-v1.tar.zst.age",
          ...HEALTH_OK,
        }),
      ),
      quit: vi.fn(() => Promise.resolve()),
    };
    const setup = backups();
    render(<SystemHealth actions={health} backups={setup} />);
    fireEvent.click(await screen.findByRole("button", { name: "從備份還原…" }));
    const recovery = screen.getByRole("button", {
      name: "使用復原用備份「pmc-operational-20260919-recovery-v1.tar.zst.age」",
    });
    expect(screen.getByRole("button", { name: "選擇其他備份檔…" })).toBeEnabled();
    fireEvent.click(recovery);
    expect(
      await screen.findByText("備份檔：pmc-operational-20260919-recovery-v1.tar.zst.age"),
    ).toBeVisible();
    expect(setup.restore?.actions.chooseRecoveryArchive).toHaveBeenCalled();
    expect(setup.restore?.actions.chooseRestoreArchive).not.toHaveBeenCalled();
  });

  it("says when nothing was replaced because the files changed after the preview", async () => {
    const host = actions({
      approveAndExecuteRestore: vi.fn(() =>
        Promise.resolve({
          outcome: "failed_before_replacement" as const,
          sourceChanged: true,
          ledger: "open_failed" as const,
        }),
      ),
    });
    render(<RestoreSheet actions={host} onClose={vi.fn()} onFinished={vi.fn()} />);
    await reachPreview(host);
    fireEvent.change(screen.getByLabelText(/請輸入備份的建立日期以確認/), {
      target: { value: "2026-09-20" },
    });
    fireEvent.click(screen.getByRole("button", { name: "以這份備份取代" }));
    expect(await screen.findByText(/目前的 Ledger 檔案在預覽之後有變動/)).toBeVisible();
  });
});
