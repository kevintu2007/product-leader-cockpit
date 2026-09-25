import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { BackupsSectionProps } from "../backup/BackupsSection";
import type { BackupActions, BackupStatusDto } from "../backup/backupIpc";
import type { VaultRootActions, VaultRootPreviewDto } from "./vaultRootIpc";
import { VaultRootSheet } from "./VaultRootSheet";

function backupStatus(overrides: Partial<BackupStatusDto> = {}): BackupStatusDto {
  return {
    state: "not_required",
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

function backups(status: BackupStatusDto): BackupsSectionProps {
  const backupActions: BackupActions = {
    loadBackupStatus: vi.fn(() => Promise.resolve(status)),
    runBackupNow: vi.fn(() => Promise.resolve(status)),
    chooseBackupDestination: vi.fn(() =>
      Promise.resolve({ configured: true, available: true, chosen: true }),
    ),
    generateRecoveryPassphrase: vi.fn(() => Promise.resolve("synthetic words")),
    setBackupPassphrase: vi.fn(() => Promise.resolve({ available: true, remembered: false })),
  };
  return { actions: backupActions, status, onStatus: vi.fn(), refresh: vi.fn() };
}

function preview(overrides: Partial<VaultRootPreviewDto> = {}): VaultRootPreviewDto {
  return {
    preparedIntentId: "intent-1",
    payloadSha256: "a".repeat(64),
    proposedFolderName: "Product Vault",
    previousFolderName: null,
    evidenceCount: 2,
    resolvedCount: 2,
    recoveryVerifiedAtMillis: 1_700_000_000_000,
    confirmationCode: "K7QX2M",
    expiresAtMillis: 1_700_000_900_000,
    ...overrides,
  };
}

function actions(overrides: Partial<VaultRootActions> = {}): VaultRootActions {
  return {
    chooseVaultFolder: vi.fn(() =>
      Promise.resolve({ chosen: true, token: "token-1", folderName: "Product Vault" }),
    ),
    prepareVaultRootChange: vi.fn(() => Promise.resolve(preview())),
    rejectVaultRootChange: vi.fn(() => Promise.resolve()),
    approveVaultRootChange: vi.fn(() => Promise.resolve({ outcome: "changed" as const })),
    ...overrides,
  };
}

async function toPreview(fake: VaultRootActions) {
  render(<VaultRootSheet actions={fake} onClose={() => undefined} onFinished={() => undefined} />);
  await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
  await screen.findByText("在新資料夾下，2 筆參照中有 2 筆內容相同。");
}

describe("VaultRootSheet (item ⑦, H2b)", () => {
  it("shows the exact preview, and the recovery backup, before anything can change", async () => {
    const fake = actions();
    await toPreview(fake);
    expect(fake.prepareVaultRootChange).toHaveBeenCalledWith("token-1");
    expect(screen.getByText("Product Vault")).toBeInTheDocument();
    // No Vault was set before: the preview says so rather than naming one.
    expect(screen.getByText("尚未設定")).toBeInTheDocument();
    expect(screen.getByText(/已於 .* 備份並驗證/)).toBeInTheDocument();
    expect(fake.approveVaultRootChange).not.toHaveBeenCalled();
  });

  it("keeps the change closed until the phrase and this preview's code are both typed", async () => {
    const fake = actions();
    await toPreview(fake);
    const input = screen.getByLabelText("請輸入「更換 Vault K7QX2M」以確認");
    const confirm = screen.getByRole("button", { name: "使用這個資料夾" });
    expect(confirm).toBeDisabled();

    // The code alone is not the confirmation.
    fireEvent.change(input, { target: { value: "K7QX2M" } });
    expect(confirm).toBeDisabled();
    // Another preview's code is not either.
    fireEvent.change(input, { target: { value: "更換 Vault AAAAAA" } });
    expect(confirm).toBeDisabled();

    // Case and extra spaces do not matter; the words do.
    fireEvent.change(input, { target: { value: "  更換   vault k7qx2m " } });
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);
    await waitFor(() => {
      expect(fake.approveVaultRootChange).toHaveBeenCalledWith(
        "intent-1",
        "a".repeat(64),
        // The whole confirmation, as typed: the host checks it too.
        "  更換   vault k7qx2m ",
        expect.any(String),
      );
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "這個工作區現在從「Product Vault」讀取 Evidence。",
    );
  });

  it("says why a folder cannot be used, and changes nothing", async () => {
    const fake = actions({
      prepareVaultRootChange: vi.fn(() =>
        Promise.reject(
          Object.assign(new Error("host refused"), {
            errorCode: "VAULT_EVIDENCE_UNPINNED",
            messageKey: "desktop.vault_evidence_unpinned",
            messageParams: [{ key: "count", value: { type: "unsigned", value: 3 } }],
            correlationId: "host-1",
            retryable: false,
            extensions: [],
          }),
        ),
      ),
    });
    render(
      <VaultRootSheet actions={fake} onClose={() => undefined} onFinished={() => undefined} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    expect(await screen.findByText(/有 3 筆 Evidence 參照沒有釘選指紋/)).toBeInTheDocument();
    // Back at the start, with nothing to confirm.
    expect(screen.queryByRole("button", { name: "使用這個資料夾" })).toBeNull();
    expect(fake.approveVaultRootChange).not.toHaveBeenCalled();
  });

  it("rejecting records the choice and closes without changing anything", async () => {
    const fake = actions();
    const onClose = vi.fn();
    render(<VaultRootSheet actions={fake} onClose={onClose} onFinished={() => undefined} />);
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    await userEvent.click(await screen.findByRole("button", { name: "不要變更" }));
    await waitFor(() => {
      expect(onClose).toHaveBeenCalled();
    });
    expect(fake.rejectVaultRootChange).toHaveBeenCalledWith("intent-1");
    expect(fake.approveVaultRootChange).not.toHaveBeenCalled();
  });

  it("withdraws a preview it no longer shows, so it cannot block the next change", async () => {
    // Review finding (⑦-2b): leaving the page with a preview on
    // screen, or before a slow prepare returns, must not leave a Prepared
    // intent behind that no screen can reject.
    const fake = actions();
    const { unmount } = render(
      <VaultRootSheet actions={fake} onClose={() => undefined} onFinished={() => undefined} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    await screen.findByRole("button", { name: "使用這個資料夾" });
    unmount();
    expect(fake.rejectVaultRootChange).toHaveBeenCalledWith("intent-1");

    let finishPrepare: ((value: VaultRootPreviewDto) => void) | undefined;
    const slow = actions({
      prepareVaultRootChange: vi.fn(
        () =>
          new Promise<VaultRootPreviewDto>((resolve) => {
            finishPrepare = resolve;
          }),
      ),
    });
    const second = render(
      <VaultRootSheet actions={slow} onClose={() => undefined} onFinished={() => undefined} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    await waitFor(() => {
      expect(slow.prepareVaultRootChange).toHaveBeenCalled();
    });
    second.unmount();
    finishPrepare?.(preview({ preparedIntentId: "intent-late" }));
    await waitFor(() => {
      expect(slow.rejectVaultRootChange).toHaveBeenCalledWith("intent-late");
    });
  });

  it("without backups set up, offers the setup here and keeps the folder choice closed", async () => {
    // Product owner, 2026-09-23: the change backs up the workspace first, so
    // a person who has not set up backups is shown how, not only refused.
    const fake = actions();
    const onClose = vi.fn();
    const notSet = backups(backupStatus());
    const { rerender } = render(
      <VaultRootSheet
        actions={fake}
        onClose={onClose}
        onFinished={() => undefined}
        backups={notSet}
      />,
    );
    expect(
      screen.getByText(
        "更換 Vault 之前，PMC 會先備份這個工作區，萬一需要時可以還原。請先設定備份資料夾與復原密語。",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "選擇 Vault 資料夾…" })).toBeDisabled();
    // The folder and passphrase rows, and only those.
    expect(screen.getByText("備份資料夾")).toBeInTheDocument();
    expect(screen.getByText("復原密語")).toBeInTheDocument();
    expect(screen.queryByText("最近一次備份")).toBeNull();

    // The passphrase dialog opens on top; Escape there closes it, not the sheet.
    await userEvent.click(screen.getByRole("button", { name: "設定密語…" }));
    expect(await screen.findByRole("dialog", { name: "設定復原密語" })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "設定復原密語" })).toBeNull();
    });
    expect(onClose).not.toHaveBeenCalled();

    // A folder alone is not enough; a folder and a passphrase are.
    rerender(
      <VaultRootSheet
        actions={fake}
        onClose={onClose}
        onFinished={() => undefined}
        backups={backups(backupStatus({ destination: "available", folderName: "Backups" }))}
      />,
    );
    expect(screen.getByRole("button", { name: "選擇 Vault 資料夾…" })).toBeDisabled();
    rerender(
      <VaultRootSheet
        actions={fake}
        onClose={onClose}
        onFinished={() => undefined}
        backups={backups(
          backupStatus({ destination: "available", folderName: "Backups", passphrase: "session" }),
        )}
      />,
    );
    expect(screen.getByRole("button", { name: "選擇 Vault 資料夾…" })).toBeEnabled();
    expect(screen.queryByText("備份資料夾")).toBeNull();
    // The rows that had focus are gone; focus is on the choice they held back.
    expect(screen.getByRole("button", { name: "選擇 Vault 資料夾…" })).toHaveFocus();
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    await screen.findByText("在新資料夾下，2 筆參照中有 2 筆內容相同。");
  });

  it("a backup folder that is set but unreachable still holds the choice back", () => {
    render(
      <VaultRootSheet
        actions={actions()}
        onClose={() => undefined}
        onFinished={() => undefined}
        backups={backups(
          backupStatus({
            destination: "unavailable",
            folderName: "Backups",
            passphrase: "session",
          }),
        )}
      />,
    );
    expect(screen.getByRole("button", { name: "選擇 Vault 資料夾…" })).toBeDisabled();
    expect(screen.getByText("備份資料夾")).toBeInTheDocument();
  });

  it("a cancelled picker leaves the sheet where it was", async () => {
    const fake = actions({
      chooseVaultFolder: vi.fn(() =>
        Promise.resolve({ chosen: false, token: null, folderName: null }),
      ),
    });
    render(
      <VaultRootSheet actions={fake} onClose={() => undefined} onFinished={() => undefined} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "選擇 Vault 資料夾…" }));
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "選擇 Vault 資料夾…" })).toBeEnabled();
    });
    expect(fake.prepareVaultRootChange).not.toHaveBeenCalled();
  });
});
