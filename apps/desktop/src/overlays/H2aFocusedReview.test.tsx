import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { PreparedIntentDto } from "../routes/cockpitContract";
import { H2aFocusedReview, type H2aDecisionOutcome } from "./H2aFocusedReview";

const DIGEST = "d".repeat(64);
const PREPARED_AT = 1_700_000_000_000;

function prepared(overrides: Partial<PreparedIntentDto> = {}): PreparedIntentDto {
  return {
    preparedIntentId: "prepared-1",
    contractVersion: 1,
    intentType: "accept_action_request",
    operation: {
      kind: "acceptActionRequest",
      requestId: "AR-1042",
      requestVersion: 3,
      actionId: "action-9",
      actionClassification: "internal",
      actionSubject: "Ship the synthetic launch",
      commitmentDetails: "Synthetic commitment details",
      intendedOwner: "owner-1",
      intendedDueAtMillis: PREPARED_AT + 86_400_000,
    },
    targets: [{ kind: "action_request", id: "AR-1042", expectedVersion: 3 }],
    declaredEffects: [
      { kind: "accept_action_request", ids: ["AR-1042"] },
      { kind: "create_action", ids: ["action-9"] },
      { kind: "link_action_request_to_action", ids: ["AR-1042", "action-9"] },
    ],
    payloadDigest: DIGEST,
    resolvedClassification: "internal",
    classificationSources: [
      { role: "primary_target", id: null, classification: "internal" },
      { role: "created_action", id: null, classification: "internal" },
    ],
    support: null,
    policyResult: "allowed",
    authority: "head_of_products",
    preparedAtMillis: PREPARED_AT,
    expiresAtMillis: PREPARED_AT + 300_000,
    cancellationPolicy: "not_cancellable_after_submit",
    correlationId: "host-1",
    ...overrides,
  };
}

function renderReview(
  options: {
    readonly onApprove?: () => Promise<H2aDecisionOutcome>;
    readonly onReject?: () => Promise<H2aDecisionOutcome>;
    readonly now?: () => number;
    readonly prepared?: PreparedIntentDto;
  } = {},
) {
  const onApprove =
    options.onApprove ??
    vi.fn(() => Promise.resolve<H2aDecisionOutcome>({ kind: "approved", summary: "OK" }));
  const onReject =
    options.onReject ??
    vi.fn(() =>
      Promise.resolve<H2aDecisionOutcome>({ kind: "rejected", expiredAtRejection: false }),
    );
  const onPrepareAgain = vi.fn();
  const onDone = vi.fn();
  const onDismiss = vi.fn();
  render(
    <H2aFocusedReview
      title="核准並執行：接受 Action Request"
      summary="核准後將建立一筆 Action 並連結此 Request。"
      approveLabel="核准並接受"
      prepared={options.prepared ?? prepared()}
      onApprove={onApprove}
      onReject={onReject}
      onPrepareAgain={onPrepareAgain}
      onDone={onDone}
      onDismiss={onDismiss}
      now={options.now ?? (() => PREPARED_AT + 1_000)}
    />,
  );
  return { onApprove, onReject, onPrepareAgain, onDone, onDismiss };
}

describe("H2aFocusedReview", () => {
  it("renders the whole typed preview, the full digest and the absolute expiry", () => {
    renderReview();
    expect(screen.getByRole("heading", { name: "核准並執行：接受 Action Request" })).toBeVisible();
    // The badge, and the Action's own classification row, both in full.
    expect(screen.getAllByText("Internal")).toHaveLength(2);
    expect(screen.getByText("AR-1042（版本 3）")).toBeVisible();
    expect(screen.getByText("action-9")).toBeVisible();
    expect(screen.getByText("Ship the synthetic launch")).toBeVisible();
    expect(screen.getByText(/Action Request AR-1042（版本 3）/)).toBeVisible();
    expect(screen.getByText(/建立 Action（action-9）/)).toBeVisible();
    expect(screen.getByText(/這筆紀錄本身：Internal/)).toBeVisible();
    expect(
      screen.getByText(/政策允許、由 Head of Products 核准、核准送出後不能撤回/),
    ).toBeVisible();
    // No host identifier reaches the person.
    expect(screen.queryByText(/primary_target|create_action|head_of_products/)).toBeNull();
    expect(screen.getByText("此操作不依賴 Evidence 或 Judgment。")).toBeVisible();
    // The digest is shown whole, never truncated: it is what the person
    // acknowledges and what the host binds the approval to.
    expect(screen.getByText(DIGEST)).toBeVisible();
    expect(screen.getByText(/2023-11-14 22:18:20 UTC/)).toBeVisible();
    expect(screen.getByRole("timer")).toHaveTextContent("剩餘 4 分 59 秒");
  });

  it("has no typed confirmation input, unlike an H2b approval", () => {
    renderReview();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("approves once, reports the outcome, and then offers only close", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject, onDone } = renderReview();

    await user.click(screen.getByRole("button", { name: "核准並接受" }));

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("已核准並執行。OK");
    });
    expect(onApprove).toHaveBeenCalledOnce();
    expect(onReject).not.toHaveBeenCalled();
    // The outcome lives in the sticky footer, ahead of the buttons, so it is
    // on screen wherever the sheet is scrolled.
    const status = screen.getByRole("status");
    const approveButton = screen.getByRole("button", { name: "核准並接受" });
    expect(status.closest(".pmc-h2a-actions")).toBe(approveButton.closest(".pmc-h2a-actions"));
    expect(
      status.compareDocumentPosition(approveButton) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "核准並接受" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "拒絕" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "關閉" }));
    expect(onDone).toHaveBeenCalledOnce();
  });

  it("closes without deciding from outside the sheet, 先不決定 or Escape, sending nothing", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject, onDismiss } = renderReview();
    const dialog = screen.getByRole("dialog");

    // A click inside the sheet is not a close.
    await user.click(dialog);
    expect(onDismiss).not.toHaveBeenCalled();

    const backdrop = dialog.parentElement;
    if (backdrop === null) {
      throw new Error("the review sheet renders inside its backdrop");
    }
    await user.click(backdrop);
    expect(onDismiss).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "先不決定" }));
    expect(onDismiss).toHaveBeenCalledTimes(2);

    await user.keyboard("{Escape}");
    expect(onDismiss).toHaveBeenCalledTimes(3);
    expect(onReject).not.toHaveBeenCalled();
    expect(onApprove).not.toHaveBeenCalled();
  });

  it("records a rejection durably through the caller", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject } = renderReview();

    await user.click(screen.getByRole("button", { name: "拒絕" }));

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("已拒絕。");
    });
    expect(onReject).toHaveBeenCalledOnce();
    expect(onApprove).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "核准並接受" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "拒絕" })).toBeDisabled();
    // A second Escape after the decision does not reject again.
    await user.keyboard("{Escape}");
    expect(onReject).toHaveBeenCalledOnce();
  });

  it("keeps a failed approval recoverable: retry re-sends the same decision", async () => {
    const user = userEvent.setup();
    const onApprove = vi
      .fn<() => Promise<H2aDecisionOutcome>>()
      .mockResolvedValueOnce({
        kind: "failed",
        error: { message: "Product Ledger 目前忙碌中。", correlationId: "host-7", retryable: true },
      })
      .mockResolvedValueOnce({ kind: "approved", summary: "OK" });
    renderReview({ onApprove });

    await user.click(screen.getByRole("button", { name: "核准並接受" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("核准未完成。");
    expect(screen.getByText("host-7")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "拒絕" })).toBeEnabled();

    await user.click(screen.getByRole("button", { name: "重試" }));

    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("已核准並執行。OK");
    });
    expect(onApprove).toHaveBeenCalledTimes(2);
  });

  it("offers to prepare again when the host says the preview expired or changed", async () => {
    const user = userEvent.setup();
    const onApprove = vi.fn(() =>
      Promise.resolve<H2aDecisionOutcome>({
        kind: "failed",
        error: {
          message: "預覽已逾期或內容已變更。",
          correlationId: "host-8",
          retryable: false,
          errorCode: "SECURITY_PREVIEW_EXPIRED_OR_CHANGED",
        },
      }),
    );
    const { onPrepareAgain } = renderReview({ onApprove });

    await user.click(screen.getByRole("button", { name: "核准並接受" }));
    await user.click(await screen.findByRole("button", { name: "重新準備" }));

    expect(onPrepareAgain).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "重試" })).not.toBeInTheDocument();
  });

  it("cannot approve an expired preview but can still reject it or prepare again", async () => {
    const user = userEvent.setup();
    const { onApprove, onReject } = renderReview({ now: () => PREPARED_AT + 300_001 });

    expect(screen.getByRole("timer")).toHaveTextContent("已逾期");
    expect(screen.getByRole("button", { name: "核准並接受" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "重新準備" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "核准並接受" }));
    expect(onApprove).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "拒絕" }));
    expect(onReject).toHaveBeenCalledOnce();
  });

  it("says when a rejection landed on an already-expired preview", async () => {
    const user = userEvent.setup();
    renderReview({
      onReject: vi.fn(() =>
        Promise.resolve<H2aDecisionOutcome>({ kind: "rejected", expiredAtRejection: true }),
      ),
    });
    await user.click(screen.getByRole("button", { name: "拒絕" }));
    await waitFor(() => {
      expect(screen.getByRole("status")).toHaveTextContent("已拒絕（此預覽在拒絕時已逾期）。");
    });
  });

  it("renders a Cancel preview in the operation's own terms", () => {
    renderReview({
      prepared: prepared({
        intentType: "cancel_action",
        operation: {
          kind: "cancelAction",
          actionId: "action-9",
          actionVersion: 3,
          reason: "Scope moved to another initiative.",
          evidenceClassifications: [{ evidenceId: "evidence-1", classification: "confidential" }],
        },
      }),
    });
    expect(screen.getByText("action-9（版本 3）")).toBeVisible();
    expect(screen.getByText("Scope moved to another initiative.")).toBeVisible();
    expect(screen.getByText("evidence-1：Confidential")).toBeVisible();
  });
});
