import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";

import { App } from "./App";

it("renders the shared production shell with Executive Cockpit as the default destination", async () => {
  render(<App />);

  // The shell appears once the host has said no upgrade gate replaces it.
  expect(await screen.findByRole("navigation", { name: "主要導覽" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Executive Cockpit" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  expect(screen.getByRole("heading", { name: "Executive Cockpit" })).toBeVisible();
});
