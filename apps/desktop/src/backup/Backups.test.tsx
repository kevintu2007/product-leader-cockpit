import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { PolicyStrip } from "../shell/PolicyStrip";
import { BackupShortcutContext } from "./BackupShortcut";
import { BackupsSection } from "./BackupsSection";
import type { BackupActions, BackupStatusDto } from "./backupIpc";
import { PassphraseDialog } from "./PassphraseDialog";

const WORDS = "abandon ability able about above absent absorb abstract absurd abuse";

function status(overrides: Partial<BackupStatusDto> = {}): BackupStatusDto {
  return {
    state: "due",
    destination: "not_set",
    folderName: null,
    passphrase: "not_set",
    lastVerifiedAtMillis: null,
    nextDueAtMillis: null,
    orphanCount: 0,
    lastRun: null,
    ...overrides,
  };
}

function actions(overrides: Partial<BackupActions> = {}): BackupActions {
  return {
    loadBackupStatus: vi.fn(() => Promise.resolve(status())),
    runBackupNow: vi.fn(() =>
      Promise.resolve(status({ state: "current", lastVerifiedAtMillis: 1_790_000_000_000 })),
    ),
    chooseBackupDestination: vi.fn(() =>
      Promise.resolve({ configured: true, available: true, chosen: true }),
    ),
    generateRecoveryPassphrase: vi.fn(() => Promise.resolve(WORDS)),
    setBackupPassphrase: vi.fn(() => Promise.resolve({ available: true, remembered: false })),
    ...overrides,
  };
}

describe("Settings → Backups", () => {
  it("says a backup is due and that nothing is set up yet", () => {
    render(
      <BackupsSection actions={actions()} status={status()} onStatus={vi.fn()} refresh={vi.fn()} />,
    );
    expect(screen.getByText(/備份已到期/)).toBeVisible();
    expect(screen.getAllByText("尚未設定")).toHaveLength(2);
    expect(screen.getByText("還沒有備份")).toBeVisible();
  });

  it("names the folder by its own name only, with its availability", () => {
    render(
      <BackupsSection
        actions={actions()}
        status={status({
          state: "current",
          destination: "unavailable",
          folderName: "PMC Backups",
          passphrase: "remembered",
        })}
        onStatus={vi.fn()}
        refresh={vi.fn()}
      />,
    );
    expect(screen.getByText("PMC Backups")).toBeVisible();
    expect(screen.getByText(/無法使用/)).toBeVisible();
    expect(screen.getByText("已記在這個 Windows 帳戶")).toBeVisible();
  });

  it("backs up now and reports the verified time", async () => {
    const onStatus = vi.fn();
    const host = actions();
    render(
      <BackupsSection
        actions={host}
        status={status({ destination: "available", passphrase: "session" })}
        onStatus={onStatus}
        refresh={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "立即備份" }));
    await waitFor(() => {
      expect(onStatus).toHaveBeenCalled();
    });
    expect(host.runBackupNow).toHaveBeenCalledTimes(1);
  });

  it("shows the host's reason when a backup cannot run", async () => {
    const host = actions({
      runBackupNow: vi.fn(() =>
        Promise.reject(
          Object.assign(new Error("host refused"), {
            errorCode: "BACKUP_PASSPHRASE_REQUIRED",
            messageKey: "desktop.backup_passphrase_required",
            correlationId: "host-1",
            retryable: false,
            messageParams: [],
            extensions: [],
          }),
        ),
      ),
    });
    render(
      <BackupsSection
        actions={host}
        status={status({ destination: "available" })}
        onStatus={vi.fn()}
        refresh={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "立即備份" }));
    // The message carries the host's retry hint after it.
    expect(await screen.findByText(/請先設定復原密語。/)).toBeVisible();
  });

  it("cannot start a backup while the startup check or a backup is running", () => {
    render(
      <BackupsSection
        actions={actions()}
        status={status({ state: "backing_up" })}
        onStatus={vi.fn()}
        refresh={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "立即備份" })).toBeDisabled();
    expect(screen.getByText("正在備份…可能需要一分鐘。")).toBeVisible();
  });
});

describe("the recovery passphrase dialog", () => {
  it("confirms only once the words are typed back exactly and the loss is acknowledged", async () => {
    const host = actions();
    const onDone = vi.fn();
    render(<PassphraseDialog actions={host} onDone={onDone} onClose={vi.fn()} />);
    expect(await screen.findByText(WORDS)).toBeVisible();
    const confirm = screen.getByRole("button", { name: "使用這組密語" });
    expect(confirm).toBeDisabled();

    fireEvent.change(screen.getByLabelText("完整輸入一次"), { target: { value: WORDS } });
    expect(confirm).toBeDisabled();
    fireEvent.click(screen.getByLabelText(/PMC 無法找回這組密語/));
    expect(confirm).toBeEnabled();

    fireEvent.click(confirm);
    await waitFor(() => {
      expect(onDone).toHaveBeenCalled();
    });
    // Remember is off unless ticked.
    expect(host.setBackupPassphrase).toHaveBeenCalledWith(WORDS, false);
  });

  it("takes the person's own passphrase when both entries match", async () => {
    const host = actions();
    render(<PassphraseDialog actions={host} onDone={vi.fn()} onClose={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "改用我自己的密語" }));
    const own = "river copper lantern meadow orbit violet";
    fireEvent.change(screen.getByLabelText("密語"), { target: { value: own } });
    fireEvent.change(screen.getByLabelText("再輸入一次密語"), { target: { value: own } });
    fireEvent.click(screen.getByLabelText(/PMC 無法找回這組密語/));
    fireEvent.click(screen.getByLabelText(/記在這個 Windows 帳戶/));
    fireEvent.click(screen.getByRole("button", { name: "使用這組密語" }));
    await waitFor(() => {
      expect(host.setBackupPassphrase).toHaveBeenCalledWith(own, true);
    });
  });

  it("never shows the same generated passphrase twice", async () => {
    const second = "zoo zone youth young yellow year wrong write wrist world";
    const generate = vi
      .fn<() => Promise<string>>()
      .mockResolvedValueOnce(WORDS)
      .mockResolvedValueOnce(second);
    render(
      <PassphraseDialog
        actions={actions({ generateRecoveryPassphrase: generate })}
        onDone={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    await screen.findByText(WORDS);
    fireEvent.click(screen.getByRole("button", { name: "改用我自己的密語" }));
    fireEvent.click(screen.getByRole("button", { name: "改用產生的密語" }));
    expect(await screen.findByText(second)).toBeVisible();
    expect(screen.queryByText(WORDS)).toBeNull();
  });

  it("closing it stores nothing", async () => {
    const host = actions();
    const onClose = vi.fn();
    render(<PassphraseDialog actions={host} onDone={vi.fn()} onClose={onClose} />);
    await screen.findByText(WORDS);
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onClose).toHaveBeenCalled();
    expect(host.setBackupPassphrase).not.toHaveBeenCalled();
  });
});

describe("a write refused by the backup gate", () => {
  it("offers Open Backups next to the reason", () => {
    const openBackups = vi.fn();
    render(
      <BackupShortcutContext.Provider value={openBackups}>
        <SafeErrorDetail
          message="備份已到期。請先完成備份，再試一次。"
          correlationId="host-2"
          retryable
          errorCode="BACKUP_DUE"
        />
      </BackupShortcutContext.Provider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "開啟備份設定" }));
    expect(openBackups).toHaveBeenCalled();
  });

  it("does not offer it for any other error", () => {
    render(
      <BackupShortcutContext.Provider value={vi.fn()}>
        <SafeErrorDetail message="x" correlationId="host-3" retryable errorCode="DOMAIN_CONFLICT" />
      </BackupShortcutContext.Provider>,
    );
    expect(screen.queryByRole("button", { name: "開啟備份設定" })).toBeNull();
  });

  it("the policy strip carries the same shortcut", () => {
    const openBackups = vi.fn();
    render(
      <PolicyStrip
        states={[
          {
            kind: "backup-due",
            message: "先完成備份",
            action: { label: "開啟備份設定", onSelect: openBackups },
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "開啟備份設定" }));
    expect(openBackups).toHaveBeenCalled();
  });
});
