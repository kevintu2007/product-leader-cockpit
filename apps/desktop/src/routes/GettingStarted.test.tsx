import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { BackupStatusDto } from "../backup/backupIpc";
import type { VaultStatusDto } from "./cockpitContract";
import { GettingStarted } from "./GettingStarted";

function backup(overrides: Partial<BackupStatusDto> = {}): BackupStatusDto {
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

function vault(configured: boolean): () => Promise<VaultStatusDto> {
  return () =>
    Promise.resolve({
      configured,
      available: configured,
      reason: configured ? null : "notConfigured",
      folderName: configured ? "Vault" : null,
      correlationId: "c-1",
    });
}

const READY = backup({
  state: "current",
  destination: "available",
  folderName: "Backups",
  passphrase: "remembered",
  lastVerifiedAtMillis: 1_790_000_000_000,
});

function step(name: string): HTMLElement {
  const item = screen.getByText(name).closest("li");
  if (item === null) throw new Error(`no step ${name}`);
  return item;
}

describe("Getting started", () => {
  it("starts with the backup folder and offers only that step", async () => {
    const onOpenSettings = vi.fn();
    render(
      <GettingStarted
        backupReadFailed={false}
        backup={backup()}
        loadVaultStatus={vault(false)}
        onOpenSettings={onOpenSettings}
        onOpenPortfolio={vi.fn()}
        productCount={0}
      />,
    );
    expect(await screen.findByRole("heading", { name: "開始使用" })).toBeVisible();
    expect(within(step("選擇備份資料夾")).getByText("下一步")).toBeVisible();
    for (const later of [
      "設定復原密語",
      "完成第一份備份",
      "選擇 Product Vault 資料夾",
      "新增第一個 Product",
    ]) {
      expect(within(step(later)).getByText("等上一步完成")).toBeVisible();
    }
    expect(screen.getAllByRole("button")).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "開啟設定" }));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    // DG1: no progress bar or count on the Cockpit.
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.queryByText(/\d\s*[/／]\s*5/)).toBeNull();
  });

  it("reads each tick from the real state: a session-only passphrase is gone after a restart", async () => {
    render(
      <GettingStarted
        backupReadFailed={false}
        backup={{ ...READY, passphrase: "not_set" }}
        loadVaultStatus={vault(true)}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={vi.fn()}
        productCount={2}
      />,
    );
    await screen.findByRole("heading", { name: "開始使用" });
    expect(within(step("選擇備份資料夾")).getByText("已完成")).toBeVisible();
    expect(within(step("設定復原密語")).getByText("下一步")).toBeVisible();
    // Later steps keep their own states rather than all "waiting".
    expect(within(step("完成第一份備份")).getByText("已完成")).toBeVisible();
    expect(within(step("新增第一個 Product")).getByText("已完成")).toBeVisible();
  });

  it("claims nothing about a Vault it cannot read, and offers nothing after it", async () => {
    render(
      <GettingStarted
        backupReadFailed={false}
        backup={READY}
        loadVaultStatus={() => Promise.reject(new Error("unavailable"))}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={vi.fn()}
        productCount={0}
      />,
    );
    await screen.findByRole("heading", { name: "開始使用" });
    expect(within(step("選擇 Product Vault 資料夾")).getByText("PMC 還無法判斷")).toBeVisible();
    expect(within(step("新增第一個 Product")).getByText("等上一步完成")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("sends the last step to Portfolio", async () => {
    const onOpenPortfolio = vi.fn();
    render(
      <GettingStarted
        backupReadFailed={false}
        backup={READY}
        loadVaultStatus={vault(true)}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={onOpenPortfolio}
        productCount={0}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "開啟 Portfolio" }));
    expect(onOpenPortfolio).toHaveBeenCalledTimes(1);
  });

  it("is gone once all five are done", async () => {
    const { container } = render(
      <GettingStarted
        backupReadFailed={false}
        backup={READY}
        loadVaultStatus={vault(true)}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={vi.fn()}
        productCount={1}
      />,
    );
    await vi.waitFor(() => {
      expect(container).toBeEmptyDOMElement();
    });
  });

  it("shows nothing until the statuses are read, so a finished setup never flashes", async () => {
    const { container } = render(
      <GettingStarted
        backupReadFailed={false}
        backup={undefined}
        loadVaultStatus={vault(false)}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={vi.fn()}
        productCount={0}
      />,
    );
    await Promise.resolve();
    expect(container).toBeEmptyDOMElement();
  });
  it("says it cannot tell when the backup status cannot be read", async () => {
    render(
      <GettingStarted
        backup={undefined}
        backupReadFailed
        loadVaultStatus={vault(false)}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={vi.fn()}
        productCount={0}
      />,
    );
    await screen.findByRole("heading", { name: "開始使用" });
    expect(within(step("選擇備份資料夾")).getByText("PMC 還無法判斷")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });
  it("does not trust an older backup status once the newest read has failed", async () => {
    render(
      <GettingStarted
        backup={READY}
        backupReadFailed
        loadVaultStatus={vault(true)}
        onOpenSettings={vi.fn()}
        onOpenPortfolio={vi.fn()}
        productCount={0}
      />,
    );
    await screen.findByRole("heading", { name: "開始使用" });
    expect(within(step("選擇備份資料夾")).getByText("PMC 還無法判斷")).toBeVisible();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
