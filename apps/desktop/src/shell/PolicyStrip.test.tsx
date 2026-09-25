import { readFileSync } from "node:fs";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PolicyStrip } from "./PolicyStrip";

describe("PolicyStrip", () => {
  it("renders nothing when no policy state applies", () => {
    const { container } = render(<PolicyStrip states={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders a live status region announcing each applicable state", () => {
    render(
      <PolicyStrip
        states={[
          { kind: "degraded", message: "Product Vault 目前無法連線" },
          { kind: "backup-due", message: "上次備份已超過 7 天" },
        ]}
      />,
    );

    const region = screen.getByRole("status");
    expect(region).toHaveAttribute("aria-live", "polite");
    expect(screen.getByText("Product Vault 目前無法連線")).toBeVisible();
    expect(screen.getByText("上次備份已超過 7 天")).toBeVisible();
  });

  it("labels each state kind in text, not color alone", () => {
    render(<PolicyStrip states={[{ kind: "out-of-sync", message: "投影檔案尚未重建" }]} />);
    expect(screen.getByText("未同步")).toBeVisible();
  });

  it("supports all five documented policy state kinds", () => {
    render(
      <PolicyStrip
        states={[
          { kind: "degraded", message: "a" },
          { kind: "evidence-verification-pending", message: "b" },
          { kind: "out-of-sync", message: "c" },
          { kind: "cancelling", message: "d" },
          { kind: "backup-due", message: "e" },
        ]}
      />,
    );
    expect(screen.getByText("降級模式")).toBeVisible();
    expect(screen.getByText("證據驗證待處理")).toBeVisible();
    expect(screen.getByText("未同步")).toBeVisible();
    expect(screen.getByText("取消中")).toBeVisible();
    expect(screen.getByText("備份已到期")).toBeVisible();
  });
});

describe("app scale", () => {
  it("every shell font size honours the app-scale multiplier", () => {
    // The policy strip renders null with no states, so the browser-level
    // scale test cannot reach it. This guards the whole stylesheet instead of
    // one selector: a surface that declares a bare `font-size` stays small
    // exactly when a reader has asked for larger text, which is how the
    // policy strip -- the persistent cross-route governance state -- was
    // found unscaled.
    //
    // jsdom does not resolve `calc()` against custom properties, so this
    // asserts the declaration rather than a computed size. The defect being
    // guarded against is the multiplier being absent from the rule, which is
    // exactly what this catches.
    const css = readFileSync("apps/desktop/src/shell/shell.css", "utf8");
    const declarations = css.match(/font-size:[^;]+;/g) ?? [];

    expect(declarations.length).toBeGreaterThan(0);
    for (const declaration of declarations) {
      expect(declaration, "every shell font size must scale").toContain("--pmc-app-scale");
    }
  });
});
