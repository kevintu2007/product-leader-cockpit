import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { APP_TEXT_SCALE_STEPS } from "./appTextScale";
import { TextScalePreview } from "./TextScalePreview";

describe("TextScalePreview", () => {
  it("renders all five frozen app-scale steps", () => {
    render(<TextScalePreview value={100} onChange={vi.fn()} />);
    for (const step of APP_TEXT_SCALE_STEPS) {
      expect(screen.getByRole("button", { name: `${String(step)}%` })).toBeVisible();
    }
  });

  it("marks only the current value as pressed", () => {
    render(<TextScalePreview value={110} onChange={vi.fn()} />);
    expect(screen.getByRole("button", { name: "110%" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "100%" })).toHaveAttribute("aria-pressed", "false");
  });

  it("calls onChange with the selected step when a keyboard user activates it", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<TextScalePreview value={100} onChange={onChange} />);

    await user.tab();
    await user.tab();
    expect(screen.getByRole("button", { name: "100%" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "110%" })).toHaveFocus();
    await user.keyboard("{Enter}");

    expect(onChange).toHaveBeenCalledWith(110);
  });

  it("renders the default sample content when none is supplied", () => {
    render(<TextScalePreview value={100} onChange={vi.fn()} />);
    expect(screen.getByText("Executive Cockpit")).toBeVisible();
  });

  it("renders caller-supplied sample content instead of the default", () => {
    render(<TextScalePreview value={100} onChange={vi.fn()} sampleContent={<p>自訂預覽內容</p>} />);
    expect(screen.getByText("自訂預覽內容")).toBeVisible();
    expect(screen.queryByText("Executive Cockpit")).not.toBeInTheDocument();
  });

  it("applies the selected scale as a CSS custom property on the sample region", () => {
    render(<TextScalePreview value={130} onChange={vi.fn()} />);
    const sample = screen.getByText("Executive Cockpit").closest(".pmc-text-scale-sample");
    expect(sample).toHaveStyle({ "--pmc-app-scale": "1.3" });
  });
});
