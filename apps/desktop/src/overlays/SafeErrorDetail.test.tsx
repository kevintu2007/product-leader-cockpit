import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { SafeErrorDetail } from "./SafeErrorDetail";

describe("SafeErrorDetail", () => {
  it("renders the safe message and correlation id as an announced alert", () => {
    render(
      <SafeErrorDetail
        message="Product Vault 暫時無法連線，請稍後再試。"
        correlationId="corr-abc-123"
        retryable={false}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Product Vault 暫時無法連線，請稍後再試。");
    expect(alert).toHaveTextContent("corr-abc-123");
  });

  it("copies only the correlation id to the clipboard", async () => {
    const user = userEvent.setup();
    // userEvent.setup() installs its own navigator.clipboard stub (for its
    // own user.copy()/user.paste() support), so the mock this assertion
    // needs must be installed *after* setup(), not before -- otherwise
    // userEvent's own stub silently overwrites it.
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    render(
      <SafeErrorDetail message="安全錯誤訊息" correlationId="corr-xyz-789" retryable={false} />,
    );

    await user.click(screen.getByRole("button", { name: "複製" }));

    expect(writeText).toHaveBeenCalledWith("corr-xyz-789");
    expect(screen.getByRole("button", { name: "已複製" })).toBeVisible();
  });

  it("shows a retry action only when retryable and a handler is supplied", () => {
    const { rerender } = render(
      <SafeErrorDetail message="安全錯誤訊息" correlationId="corr-1" retryable={false} />,
    );
    expect(screen.queryByRole("button", { name: "重試" })).not.toBeInTheDocument();

    const onRetry = vi.fn();
    rerender(
      <SafeErrorDetail message="安全錯誤訊息" correlationId="corr-1" retryable onRetry={onRetry} />,
    );
    expect(screen.getByRole("button", { name: "重試" })).toBeVisible();
  });

  it("calls the retry handler when the retry action is activated", async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    render(
      <SafeErrorDetail message="安全錯誤訊息" correlationId="corr-1" retryable onRetry={onRetry} />,
    );

    await user.click(screen.getByRole("button", { name: "重試" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("renders every legal next action and calls its handler when selected", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(
      <SafeErrorDetail
        message="安全錯誤訊息"
        correlationId="corr-1"
        retryable={false}
        nextActions={[{ label: "檢查網路連線後重試", onSelect }]}
      />,
    );

    const action = screen.getByRole("button", { name: "檢查網路連線後重試" });
    expect(action).toBeVisible();
    await user.click(action);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("never renders anything beyond the safe fields supplied", () => {
    render(<SafeErrorDetail message="安全錯誤訊息" correlationId="corr-1" retryable={false} />);
    // No stack trace, file path, or raw message key ever appears -- there
    // is simply no prop through which the caller could pass one.
    expect(screen.queryByText(/\.rs:|\.ts:|C:\\|privateDetailRef/)).not.toBeInTheDocument();
  });
});
