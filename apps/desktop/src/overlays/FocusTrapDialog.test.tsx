import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FocusTrapDialog } from "./FocusTrapDialog";

function Sample({ onEscape }: { onEscape: () => void }) {
  return (
    <FocusTrapDialog titleId="t" onEscape={onEscape}>
      <h2 id="t">Sample dialog</h2>
      <button type="button">First</button>
      <button type="button">Second</button>
      <button type="button">Last</button>
    </FocusTrapDialog>
  );
}

describe("FocusTrapDialog", () => {
  it("renders as an accessible modal dialog labelled by titleId", () => {
    render(<Sample onEscape={vi.fn()} />);
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveAccessibleName("Sample dialog");
  });

  it("moves focus to the first focusable element on mount", () => {
    render(<Sample onEscape={vi.fn()} />);
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
  });

  it("wraps Tab from the last focusable element back to the first", async () => {
    const user = userEvent.setup();
    render(<Sample onEscape={vi.fn()} />);

    screen.getByRole("button", { name: "Last" }).focus();
    await user.tab();

    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
  });

  it("wraps Shift+Tab from the first focusable element back to the last", async () => {
    const user = userEvent.setup();
    render(<Sample onEscape={vi.fn()} />);

    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
    await user.tab({ shift: true });

    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
  });

  it("calls onEscape when Escape is pressed", async () => {
    const user = userEvent.setup();
    const onEscape = vi.fn();
    render(<Sample onEscape={onEscape} />);

    await user.keyboard("{Escape}");

    expect(onEscape).toHaveBeenCalledOnce();
  });

  it("a dialog inside another owns Escape and Tab: the outer one is left alone", async () => {
    const user = userEvent.setup();
    const outerEscape = vi.fn();
    const innerEscape = vi.fn();
    render(
      <FocusTrapDialog titleId="outer" onEscape={outerEscape}>
        <h2 id="outer">Outer</h2>
        <button type="button">Outer first</button>
        <FocusTrapDialog titleId="inner" onEscape={innerEscape}>
          <h2 id="inner">Inner</h2>
          <button type="button">Inner first</button>
          <button type="button">Inner last</button>
        </FocusTrapDialog>
        <button type="button">Outer last</button>
      </FocusTrapDialog>,
    );

    screen.getByRole("button", { name: "Inner last" }).focus();
    await user.tab();
    expect(screen.getByRole("button", { name: "Inner first" })).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(innerEscape).toHaveBeenCalledOnce();
    expect(outerEscape).not.toHaveBeenCalled();
  });

  it("returns focus to the opener when unmounted", () => {
    const opener = document.createElement("button");
    opener.textContent = "Opener";
    document.body.appendChild(opener);
    opener.focus();
    expect(opener).toHaveFocus();

    const { unmount } = render(<Sample onEscape={vi.fn()} />);
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();

    unmount();
    expect(opener).toHaveFocus();
    opener.remove();
  });
});
