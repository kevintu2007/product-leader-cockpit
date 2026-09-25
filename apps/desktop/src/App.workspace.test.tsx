import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { WorkspaceStatusDto } from "./workspace/workspaceIpc";

const calls: string[] = [];
let status: WorkspaceStatusDto;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string) => {
    calls.push(command);
    if (command === "get_workspace_status") {
      return Promise.resolve(status);
    }
    // Everything else is refused, as outside the desktop host.
    return Promise.reject(new Error(`not in this test: ${command}`));
  }),
}));

const { App } = await import("./App");

function workspace(overrides: Partial<WorkspaceStatusDto>): WorkspaceStatusDto {
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

describe("App and the chosen workspace", () => {
  beforeEach(() => {
    calls.length = 0;
  });

  it("asks nothing of any workspace before the first-run choice", async () => {
    status = workspace({ open: null, selected: null, firstRun: true });
    render(<App />);
    expect(await screen.findByRole("heading", { name: "選擇開始的方式" })).toBeVisible();
    // Give any stray effect a chance to run.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(calls).toEqual(["get_workspace_status"]);
    expect(screen.queryByRole("navigation", { name: "主要導覽" })).toBeNull();
  });

  it("labels the sample workspace in the top bar", async () => {
    status = workspace({ open: "training", selected: "training" });
    render(<App />);
    expect(await screen.findByRole("navigation", { name: "主要導覽" })).toBeVisible();
    expect(screen.getByRole("button", { name: "範例工作區" })).toBeVisible();
  });

  it("shows no sample marker in your own workspace", async () => {
    status = workspace({});
    render(<App />);
    expect(await screen.findByRole("navigation", { name: "主要導覽" })).toBeVisible();
    await waitFor(() => {
      expect(calls).toContain("get_workspace_status");
    });
    expect(screen.queryByText("範例工作區")).toBeNull();
  });
});
