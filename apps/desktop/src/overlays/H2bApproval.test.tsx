import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { H2bApproval, type H2bApprovalProps } from "./H2bApproval";

const localRecoveryEvidence: H2bApprovalProps["evidence"] = {
  kind: "local-recovery",
  verifiedAt: "2026-08-24 09:12 Asia/Taipei",
  scope: "完整 Product Ledger",
  compatibility: "schema v18 相容",
};

function renderApproval(overrides: Partial<H2bApprovalProps> = {}) {
  const onApprove = vi.fn();
  const onReject = vi.fn();
  render(
    <H2bApproval
      title="核准並執行：Restore from Verified Archive"
      summary="核准後將以此備份還原 Product Ledger，此操作無法復原。"
      classification="restricted"
      fields={[{ label: "備份時間", value: "2026-08-23 02:00" }]}
      evidence={localRecoveryEvidence}
      confirmationPhrase="RESTORE"
      onApprove={onApprove}
      onReject={onReject}
      {...overrides}
    />,
  );
  return { onApprove, onReject };
}

describe("H2bApproval", () => {
  it("renders the title, summary, classification, and preview fields", () => {
    renderApproval();
    expect(
      screen.getByRole("heading", { name: "核准並執行：Restore from Verified Archive" }),
    ).toBeVisible();
    expect(screen.getByText("Restricted")).toBeVisible();
    expect(screen.getByText("備份時間")).toBeVisible();
    expect(screen.getByText("2026-08-23 02:00")).toBeVisible();
  });

  it("keeps Approve disabled until the typed confirmation exactly matches", async () => {
    const user = userEvent.setup();
    renderApproval();

    const approve = screen.getByRole("button", { name: "核准" });
    const input = screen.getByLabelText("請輸入「RESTORE」以核准");
    expect(approve).toBeDisabled();

    await user.type(input, "restore");
    expect(approve).toBeDisabled();

    await user.clear(input);
    await user.type(input, "RESTORE");
    expect(approve).toBeEnabled();
  });

  it("calls onApprove exactly once, only after the confirmation matches", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject } = renderApproval();

    await user.type(screen.getByLabelText("請輸入「RESTORE」以核准"), "RESTORE");
    await user.click(screen.getByRole("button", { name: "核准" }));

    expect(onApprove).toHaveBeenCalledOnce();
    expect(onReject).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "核准" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("已核准");
  });

  it("allows Reject at any time, even with no confirmation typed, and only once", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject } = renderApproval();

    await user.click(screen.getByRole("button", { name: "拒絕" }));

    expect(onReject).toHaveBeenCalledOnce();
    expect(onApprove).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("已拒絕");
  });

  it("treats Escape as the safe reject path", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject } = renderApproval();

    await user.keyboard("{Escape}");

    expect(onReject).toHaveBeenCalledOnce();
    expect(onApprove).not.toHaveBeenCalled();
  });

  it("renders local recovery evidence with verification time, scope, and compatibility", () => {
    renderApproval();
    expect(screen.getByText("2026-08-24 09:12 Asia/Taipei")).toBeVisible();
    expect(screen.getByText("完整 Product Ledger")).toBeVisible();
    expect(screen.getByText("schema v18 相容")).toBeVisible();
  });

  it("renders external egress evidence with the fixed irreversible-transmission statement", () => {
    renderApproval({
      evidence: {
        kind: "external-egress",
        provider: "Anthropic",
        account: "workspace-1",
        purpose: "Weekly Review Work Packet",
        exactPayloadSummary: "本季 Fact Pack 摘要（不含 Restricted 內容）",
      },
    });

    expect(screen.getByText("Anthropic")).toBeVisible();
    expect(screen.getByText("workspace-1")).toBeVisible();
    expect(screen.getByText("Weekly Review Work Packet")).toBeVisible();
    expect(screen.getByText("本季 Fact Pack 摘要（不含 Restricted 內容）")).toBeVisible();
    expect(
      screen.getByText(
        "此操作一經核准即無法收回：上述內容將實際傳送至外部服務，且無法追回或撤銷。",
      ),
    ).toBeVisible();
  });
});
