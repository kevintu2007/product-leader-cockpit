import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { AppShell } from "./AppShell";
import { documentTitleFor } from "./routes";

describe("AppShell", () => {
  it("defaults to Executive Cockpit as the active route", () => {
    render(<AppShell />);
    expect(screen.getByRole("button", { name: "Executive Cockpit" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("heading", { name: "Executive Cockpit" })).toBeVisible();
  });

  it("sets the document title to match the active route on mount", async () => {
    render(<AppShell />);
    await waitFor(() => {
      expect(document.title).toBe(documentTitleFor("executive-cockpit"));
    });
  });

  it("switches the active route, heading, and document title on navigation", async () => {
    const user = userEvent.setup();
    render(<AppShell />);

    await user.click(screen.getByRole("button", { name: "Portfolio" }));

    expect(screen.getByRole("button", { name: "Portfolio" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("heading", { name: "Portfolio" })).toBeVisible();
    await waitFor(() => {
      expect(document.title).toBe(documentTitleFor("portfolio"));
    });
  });

  it("shows a placeholder for route content that is not yet implemented", () => {
    render(<AppShell />);
    expect(screen.getByText("此畫面尚未實作，等待所屬垂直切片通過驗證後才會開放。")).toBeVisible();
  });

  it("renders supplied route content instead of the placeholder when provided", () => {
    render(<AppShell renderRoute={(routeId) => <p>Custom content for {routeId}</p>} />);
    expect(screen.getByText("Custom content for executive-cockpit")).toBeVisible();
  });

  it("names the active route as the page's single top-level heading", () => {
    render(<AppShell renderRoute={() => <h2>Route headline</h2>} />);

    const headings = screen.getAllByRole("heading", { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveTextContent("Executive Cockpit");
  });

  it("records an explicit theme choice that wins over the operating system", async () => {
    const user = userEvent.setup();
    delete document.documentElement.dataset.theme;
    window.localStorage.removeItem("pmc-theme");
    render(<AppShell />);

    await user.click(screen.getByRole("button", { name: "切換為深色主題" }));

    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(window.localStorage.getItem("pmc-theme")).toBe("dark");
    expect(screen.getByRole("button", { name: "切換為淺色主題" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "切換為淺色主題" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    window.localStorage.removeItem("pmc-theme");
    delete document.documentElement.dataset.theme;
  });

  it("lets route content navigate the shell", async () => {
    const user = userEvent.setup();
    render(
      <AppShell
        renderRoute={(routeId, navigate) =>
          routeId === "executive-cockpit" ? (
            <button
              type="button"
              onClick={() => {
                navigate("work-queue");
              }}
            >
              前往 Work Queue
            </button>
          ) : (
            <p>queue content</p>
          )
        }
      />,
    );

    await user.click(screen.getByRole("button", { name: "前往 Work Queue" }));

    expect(screen.getByText("queue content")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Work Queue");
  });

  it("surfaces supplied policy states in the persistent strip", () => {
    render(
      <AppShell policyStates={[{ kind: "degraded", message: "Product Vault 目前無法連線" }]} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Product Vault 目前無法連線");
  });
});

describe("route content fallback", () => {
  it("shows the placeholder when a slice supplies no content for a route", () => {
    // The prop's own contract says it falls back to a placeholder. Returning
    // undefined for an unimplemented route must therefore render the
    // placeholder, not an empty panel that looks like a working screen with
    // no data in it.
    render(<AppShell renderRoute={() => undefined} />);

    expect(
      screen.getByText("此畫面尚未實作，等待所屬垂直切片通過驗證後才會開放。"),
    ).toBeInTheDocument();
  });

  it("shows supplied content when a slice does have it", () => {
    render(<AppShell renderRoute={() => <p>real route content</p>} />);

    expect(screen.getByText("real route content")).toBeInTheDocument();
    expect(screen.queryByText("此畫面尚未實作，等待所屬垂直切片通過驗證後才會開放。")).toBeNull();
  });
});
