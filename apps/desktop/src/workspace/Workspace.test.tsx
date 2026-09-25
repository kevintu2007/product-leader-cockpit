import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { BackupsSectionProps } from "../backup/BackupsSection";
import { UpgradeGate } from "../backup/UpgradeGate";
import type { UpgradeGateDto } from "../backup/upgradeIpc";
import { FirstRunGate } from "./FirstRunGate";
import { SampleBadge } from "./SampleBadge";
import { SampleDeleteSheet } from "./SampleDeleteSheet";
import { WorkspaceSection } from "./WorkspaceSection";
import type {
  SampleDeletedDto,
  SampleDeletePreviewDto,
  WorkspaceActions,
  WorkspaceStatusDto,
} from "./workspaceIpc";

function refusal(errorCode: string, messageKey: string, retryable = true) {
  return Object.assign(new Error("host refused"), {
    errorCode,
    messageKey,
    correlationId: "host-1",
    retryable,
    messageParams: [],
    extensions: [],
  });
}

const PREVIEW: SampleDeletePreviewDto = {
  preparedIntentId: "intent-1",
  payloadSha256: "b".repeat(64),
  seedId: "pmc-training",
  seedVersion: 3,
  hasLedger: true,
  hasVault: true,
  hasGeneratedFiles: false,
  settingsRevision: 7,
  operation: "delete_sample_workspace",
  inventorySha256: "c".repeat(64),
  effect: "irreversible_live_unaffected",
  expiresAtMillis: 1_790_000_900_000,
};

function status(overrides: Partial<WorkspaceStatusDto> = {}): WorkspaceStatusDto {
  return {
    open: "live",
    selected: "live",
    firstRun: false,
    sample: "available",
    sampleFellBack: false,
    settingsSetAside: false,
    settingsUnavailable: false,
    sampleCleanupPending: 0,
    ...overrides,
  };
}

function actions(overrides: Partial<WorkspaceActions> = {}): WorkspaceActions {
  return {
    loadWorkspaceStatus: vi.fn(() => Promise.resolve(status())),
    chooseFirstWorkspace: vi.fn(() => Promise.resolve({ outcome: "restarting" as const })),
    switchWorkspace: vi.fn(() => Promise.resolve({ outcome: "restarting" as const })),
    resetSampleData: vi.fn(() => Promise.resolve({ seedVersion: 3 })),
    prepareSampleDelete: vi.fn(() => Promise.resolve(PREVIEW)),
    rejectSampleDelete: vi.fn(() => Promise.resolve()),
    approveSampleDelete: vi.fn(() => Promise.resolve({ outcome: "deleted" as const })),
    ...overrides,
  };
}

describe("Choose how to begin", () => {
  it("offers both choices equally, with nothing preselected", () => {
    render(<FirstRunGate actions={actions()} />);
    expect(screen.getByRole("heading", { name: "選擇開始的方式" })).toBeVisible();
    const live = screen.getByRole("button", { name: /從我的工作區開始/ });
    const sample = screen.getByRole("button", { name: /用範例資料學習/ });
    for (const choice of [live, sample]) {
      expect(choice).toBeEnabled();
      expect(choice).not.toHaveAttribute("aria-pressed");
    }
    // What to set up first, in the order the Vault change needs.
    expect(screen.getByText(/備份資料夾、復原密語、Vault 資料夾/)).toBeVisible();
  });

  it("makes one choice for a double click and says PMC is restarting", async () => {
    const workspace = actions();
    render(<FirstRunGate actions={workspace} />);
    const sample = screen.getByRole("button", { name: /用範例資料學習/ });
    fireEvent.click(sample);
    fireEvent.click(sample);
    expect(screen.getByText("正在準備範例資料…可能需要一分鐘。")).toBeVisible();
    await screen.findByText("PMC 正在重新啟動…");
    expect(workspace.chooseFirstWorkspace).toHaveBeenCalledTimes(1);
    expect(workspace.chooseFirstWorkspace).toHaveBeenCalledWith("training");
  });

  it("leaves the choice unmade when preparing fails, and can be chosen again", async () => {
    const choose = vi
      .fn<WorkspaceActions["chooseFirstWorkspace"]>()
      .mockRejectedValueOnce(refusal("SAMPLE_FAILED", "desktop.sample_failed"))
      .mockResolvedValueOnce({ outcome: "restarting" });
    render(<FirstRunGate actions={actions({ chooseFirstWorkspace: choose })} />);
    fireEvent.click(screen.getByRole("button", { name: /用範例資料學習/ }));
    await screen.findByText(/無法準備範例資料。你的工作區不受影響。/);
    expect(screen.getByText("還沒有做出選擇。準備好後再選一次。")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /從我的工作區開始/ }));
    await screen.findByText("PMC 正在重新啟動…");
    expect(choose).toHaveBeenLastCalledWith("live");
  });
});

describe("Settings → Workspace", () => {
  it("switches to the sample data only after saying PMC will restart", async () => {
    const workspace = actions();
    render(<WorkspaceSection status={status()} actions={workspace} onChanged={vi.fn()} />);
    expect(screen.getByText("目前開著你的工作區。")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "切換到範例資料" }));
    expect(workspace.switchWorkspace).not.toHaveBeenCalled();
    expect(screen.getByText(/PMC 會重新啟動並開啟範例資料/)).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "切換並重新啟動" }));
    await screen.findByText("PMC 正在重新啟動…");
    expect(workspace.switchWorkspace).toHaveBeenCalledWith("training");
  });

  it("says a failed save restarted nothing", async () => {
    const workspace = actions({
      switchWorkspace: vi.fn(() =>
        Promise.reject(refusal("WORKSPACE_CHOICE_NOT_SAVED", "desktop.workspace_choice_not_saved")),
      ),
    });
    render(<WorkspaceSection status={status()} actions={workspace} onChanged={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "切換到範例資料" }));
    fireEvent.click(screen.getByRole("button", { name: "切換並重新啟動" }));
    await screen.findByText(/無法儲存這個選擇，所以 PMC 沒有重新啟動/);
    expect(screen.getByRole("button", { name: "切換到範例資料" })).toBeEnabled();
  });

  it("offers no reset or delete while the sample data is open", () => {
    render(
      <WorkspaceSection
        status={status({ open: "training", selected: "training" })}
        actions={actions()}
        onChanged={vi.fn()}
      />,
    );
    expect(screen.getByText(/目前開著範例資料/)).toBeVisible();
    expect(screen.getByRole("button", { name: "切換到我的工作區" })).toBeVisible();
    expect(screen.getByText("重設與刪除要從你的工作區操作。請先切換過去。")).toBeVisible();
    expect(screen.queryByRole("button", { name: "重設範例資料…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "刪除範例資料…" })).toBeNull();
  });

  it("says why chosen sample data did not open", () => {
    render(
      <WorkspaceSection
        status={status({ selected: "training", sample: "foreign", sampleFellBack: true })}
        actions={actions()}
        onChanged={vi.fn()}
      />,
    );
    expect(screen.getByText(/你選了範例資料，但它無法開啟/)).toBeVisible();
    expect(screen.getByText(/範例資料夾裡有不是 PMC 寫入的東西/)).toBeVisible();
  });

  it("resets after one confirmation, retrying with the same request", async () => {
    const reset = vi
      .fn<WorkspaceActions["resetSampleData"]>()
      .mockRejectedValueOnce(refusal("SAMPLE_RESET_FAILED", "desktop.sample_reset_failed"))
      .mockResolvedValueOnce({ seedVersion: 3 });
    const onChanged = vi.fn();
    render(
      <WorkspaceSection
        status={status()}
        actions={actions({ resetSampleData: reset })}
        onChanged={onChanged}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "重設範例資料…" }));
    expect(screen.getByText("範例資料會回到初始狀態。你的工作區不受影響。")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "重設範例資料" }));
    await screen.findByText(/無法重設範例資料/);
    fireEvent.click(screen.getByRole("button", { name: "重設範例資料" }));
    await screen.findByText("範例資料已回到初始狀態。");
    expect(reset).toHaveBeenCalledTimes(2);
    expect(reset.mock.calls[1]?.[0]).toBe(reset.mock.calls[0]?.[0]);
    expect(onChanged).toHaveBeenCalledTimes(1);
  });
});

describe("Delete sample data", () => {
  it("shows exactly what goes, and deletes only after the typed phrase", async () => {
    const workspace = actions();
    const onFinished = vi.fn();
    render(<SampleDeleteSheet actions={workspace} onClose={vi.fn()} onFinished={onFinished} />);
    await screen.findByText("pmc-training，版本 3");
    expect(screen.getByText("它的 Product Ledger")).toBeVisible();
    expect(screen.getByText("它的合成 Vault")).toBeVisible();
    expect(screen.queryByText("它產生的檔案")).toBeNull();
    expect(screen.getByText("無法復原。你的工作區、設定與備份不受影響。")).toBeVisible();
    const confirm = screen.getByRole("button", { name: "刪除範例資料" });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText("輸入「刪除範例資料」以確認"), {
      target: { value: "刪除範例" },
    });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText("輸入「刪除範例資料」以確認"), {
      target: { value: " 刪除範例資料 " },
    });
    fireEvent.click(confirm);
    await screen.findByText("範例資料已刪除。切換到範例資料時會重新準備。");
    expect(workspace.approveSampleDelete).toHaveBeenCalledWith(
      "intent-1",
      PREVIEW.payloadSha256,
      " 刪除範例資料 ",
      expect.any(String),
    );
    expect(workspace.rejectSampleDelete).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "完成" }));
    expect(onFinished).toHaveBeenCalledWith({ outcome: "deleted" });
  });

  it("rejects the preview when the person keeps the data", async () => {
    const workspace = actions();
    const onClose = vi.fn();
    render(<SampleDeleteSheet actions={workspace} onClose={onClose} onFinished={vi.fn()} />);
    await screen.findByText("pmc-training，版本 3");
    fireEvent.click(screen.getByRole("button", { name: "保留" }));
    await waitFor(() => {
      expect(onClose).toHaveBeenCalled();
    });
    expect(workspace.rejectSampleDelete).toHaveBeenCalledWith("intent-1");
    expect(workspace.approveSampleDelete).not.toHaveBeenCalled();
  });

  it("withdraws a preview no screen shows any more", async () => {
    const workspace = actions();
    const { unmount } = render(
      <SampleDeleteSheet actions={workspace} onClose={vi.fn()} onFinished={vi.fn()} />,
    );
    await screen.findByText("pmc-training，版本 3");
    unmount();
    expect(workspace.rejectSampleDelete).toHaveBeenCalledWith("intent-1");
  });

  it("withdraws a preview that arrives after the sheet has gone", async () => {
    let arrive: (preview: SampleDeletePreviewDto) => void = () => undefined;
    const workspace = actions({
      prepareSampleDelete: vi.fn(
        () =>
          new Promise<SampleDeletePreviewDto>((resolve) => {
            arrive = resolve;
          }),
      ),
    });
    const { unmount } = render(
      <SampleDeleteSheet actions={workspace} onClose={vi.fn()} onFinished={vi.fn()} />,
    );
    unmount();
    await act(async () => {
      arrive(PREVIEW);
      await Promise.resolve();
    });
    expect(workspace.rejectSampleDelete).toHaveBeenCalledWith("intent-1");
  });

  it("keeps the preview and says why when the approval is refused", async () => {
    const workspace = actions({
      approveSampleDelete: vi.fn(() =>
        Promise.reject(refusal("SAMPLE_DELETE_CHANGED", "desktop.sample_delete_changed", false)),
      ),
    });
    render(<SampleDeleteSheet actions={workspace} onClose={vi.fn()} onFinished={vi.fn()} />);
    await screen.findByText("pmc-training，版本 3");
    fireEvent.change(screen.getByLabelText("輸入「刪除範例資料」以確認"), {
      target: { value: "刪除範例資料" },
    });
    fireEvent.click(screen.getByRole("button", { name: "刪除範例資料" }));
    await screen.findByText(/範例資料在預覽之後有變動，沒有刪除任何東西/);
    expect(screen.getByRole("button", { name: "保留" })).toBeEnabled();
  });

  it("shows the settings revision and both digests the approval is bound to", async () => {
    render(<SampleDeleteSheet actions={actions()} onClose={vi.fn()} onFinished={vi.fn()} />);
    await screen.findByText("pmc-training，版本 3");
    expect(screen.getByText("設定版本").nextElementSibling).toHaveTextContent("7");
    expect(screen.getByText("內容摘要").nextElementSibling).toHaveTextContent("c".repeat(64));
    expect(screen.getByText("預覽摘要").nextElementSibling).toHaveTextContent("b".repeat(64));
  });

  it("withdraws the preview when an approval is refused after the sheet has gone", async () => {
    let refuse: (reason: unknown) => void = () => undefined;
    const workspace = actions({
      approveSampleDelete: vi.fn(
        () =>
          new Promise<SampleDeletedDto>((_resolve, reject) => {
            refuse = reject;
          }),
      ),
    });
    const { unmount } = render(
      <SampleDeleteSheet actions={workspace} onClose={vi.fn()} onFinished={vi.fn()} />,
    );
    await screen.findByText("pmc-training，版本 3");
    fireEvent.change(screen.getByLabelText("輸入「刪除範例資料」以確認"), {
      target: { value: "刪除範例資料" },
    });
    fireEvent.click(screen.getByRole("button", { name: "刪除範例資料" }));
    unmount();
    // Leaving while the approval runs never rejects it.
    expect(workspace.rejectSampleDelete).not.toHaveBeenCalled();
    await act(async () => {
      refuse(refusal("SAMPLE_DELETE_EXPIRED", "desktop.sample_delete_expired", false));
      await Promise.resolve();
    });
    expect(workspace.rejectSampleDelete).toHaveBeenCalledWith("intent-1");
  });

  it("cannot approve a preview that binds a change the sheet cannot describe", async () => {
    const workspace = actions({
      prepareSampleDelete: vi.fn(() => Promise.resolve({ ...PREVIEW, effect: "something_else" })),
    });
    render(<SampleDeleteSheet actions={workspace} onClose={vi.fn()} onFinished={vi.fn()} />);
    await screen.findByText(/這份預覽描述的變更是這個畫面無法顯示的/);
    fireEvent.change(screen.getByLabelText("輸入「刪除範例資料」以確認"), {
      target: { value: "刪除範例資料" },
    });
    expect(screen.getByRole("button", { name: "刪除範例資料" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "保留" })).toBeEnabled();
  });
});

describe("The sample marker", () => {
  it("names the sample workspace in words and opens Settings", () => {
    const onOpen = vi.fn();
    render(<SampleBadge onOpen={onOpen} />);
    fireEvent.click(screen.getByRole("button", { name: "範例工作區" }));
    expect(onOpen).toHaveBeenCalled();
  });

  it("is a plain label where there is nothing to open", () => {
    render(<SampleBadge />);
    expect(screen.getByRole("note")).toHaveTextContent("範例工作區");
    expect(screen.queryByRole("button")).toBeNull();
  });
});

describe("The upgrade gate in the sample workspace", () => {
  const GATE: UpgradeGateDto = {
    state: "training_reset",
    appVersion: "0.2.0",
    fromSchema: 46,
    toSchema: 48,
    recordCount: 80,
    live: false,
  };
  const BACKUPS = {
    actions: {} as BackupsSectionProps["actions"],
    status: undefined,
    onStatus: vi.fn(),
    refresh: vi.fn(),
  } satisfies BackupsSectionProps;

  it("switches to your workspace instead of upgrading", async () => {
    const onSwitchToLive = vi.fn(() => Promise.resolve({ outcome: "restarting" as const }));
    render(
      <UpgradeGate
        gate={GATE}
        actions={{ loadUpgradeGate: vi.fn(), runUpgrade: vi.fn(), quit: vi.fn() }}
        backups={BACKUPS}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        onSwitchToLive={onSwitchToLive}
      />,
    );
    expect(screen.queryByRole("button", { name: "升級" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "切換到我的工作區" }));
    await screen.findByText("PMC 正在重新啟動…");
    expect(onSwitchToLive).toHaveBeenCalledTimes(1);
  });

  it("says why a switch failed and offers it again", async () => {
    const onSwitchToLive = vi.fn(() =>
      Promise.reject(refusal("WORKSPACE_CHOICE_NOT_SAVED", "desktop.workspace_choice_not_saved")),
    );
    render(
      <UpgradeGate
        gate={GATE}
        actions={{ loadUpgradeGate: vi.fn(), runUpgrade: vi.fn(), quit: vi.fn() }}
        backups={BACKUPS}
        onUpgraded={vi.fn()}
        onOpenSystemHealth={vi.fn()}
        onSwitchToLive={onSwitchToLive}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "切換到我的工作區" }));
    await screen.findByText(/無法儲存這個選擇，所以 PMC 沒有重新啟動/);
    expect(screen.getByRole("button", { name: "切換到我的工作區" })).toBeEnabled();
  });
});
