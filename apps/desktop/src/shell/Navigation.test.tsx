import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Navigation } from "./Navigation";
import { PRIMARY_ROUTES } from "./routes";

describe("Navigation", () => {
  it("renders all seven primary destinations in their frozen order", () => {
    render(<Navigation activeRouteId="executive-cockpit" onNavigate={vi.fn()} />);

    const items = screen.getAllByRole("button").map((button) => button.textContent);
    expect(items.slice(0, PRIMARY_ROUTES.length)).toEqual(
      PRIMARY_ROUTES.map((route) => route.label),
    );
  });

  it("also exposes System Health as a reachable, separate destination", () => {
    render(<Navigation activeRouteId="executive-cockpit" onNavigate={vi.fn()} />);
    expect(screen.getByRole("button", { name: "System Health" })).toBeVisible();
  });

  it("marks only the active destination with aria-current", () => {
    render(<Navigation activeRouteId="portfolio" onNavigate={vi.fn()} />);

    expect(screen.getByRole("button", { name: "Portfolio" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("button", { name: "Executive Cockpit" })).not.toHaveAttribute(
      "aria-current",
    );
  });

  it("calls onNavigate with the destination id when a keyboard user activates it", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    render(<Navigation activeRouteId="executive-cockpit" onNavigate={onNavigate} />);

    await user.tab();
    expect(screen.getByRole("button", { name: "Executive Cockpit" })).toHaveFocus();
    await user.keyboard("{Enter}");

    expect(onNavigate).toHaveBeenCalledWith("executive-cockpit");
  });

  it("names an attention badge in words and shows no badge for a destination without a count", () => {
    render(
      <Navigation
        activeRouteId="executive-cockpit"
        onNavigate={vi.fn()}
        counts={{ "work-queue": 4, portfolio: 0 }}
      />,
    );

    // The number travels in the accessible name, not only in the pill.
    expect(screen.getByRole("button", { name: "Work Queue，4 件需要注意" })).toBeInTheDocument();
    // Zero is not a fact worth a badge; the destination keeps its plain name.
    expect(screen.getByRole("button", { name: "Portfolio" })).toBeInTheDocument();
    expect(screen.queryByText("0")).toBeNull();
  });

  it("keeps every destination name in the DOM while the rail is collapsed", () => {
    render(<Navigation activeRouteId="executive-cockpit" onNavigate={vi.fn()} />);

    // The compact rail hides names visually until hover or focus; they must
    // still be there for assistive technology and for the expanded rail.
    for (const route of PRIMARY_ROUTES) {
      expect(screen.getByText(route.label)).toBeInTheDocument();
    }
  });

  it("reaches System Health purely via keyboard tabbing", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    render(<Navigation activeRouteId="executive-cockpit" onNavigate={onNavigate} />);

    for (let index = 0; index <= PRIMARY_ROUTES.length; index += 1) {
      await user.tab();
    }
    expect(screen.getByRole("button", { name: "System Health" })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onNavigate).toHaveBeenCalledWith("system-health");
  });
});
