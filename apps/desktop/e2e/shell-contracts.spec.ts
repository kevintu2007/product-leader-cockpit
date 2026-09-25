import { expect, test } from "@playwright/test";

/**
 * Tier 1: the frozen DG3 presentation contracts, asserted against the real
 * assembled application in a real browser.
 *
 * Every expectation here comes from a contract that is already frozen --
 * the DG3 Route Contract table, the DG3 Accessibility and Interaction
 * Contract, and the design system's Navigation pattern -- so these tests do
 * not churn as feature slices land. That is the whole reason Tier 1 can
 * start before the feature surface settles.
 *
 * What these prove that the component tests cannot: the component tests
 * mount pieces in jsdom. They never assemble the shell, never run a real
 * layout engine, and never set a real document title. Reflow, focus order
 * across the whole shell, and the title contract are only observable here.
 */

/** The seven primary destinations in their frozen DG3 order. */
const PRIMARY_DESTINATIONS = [
  "Executive Cockpit",
  "Portfolio",
  "Work Queue",
  "Reviews & Reports",
  "Product Vault",
  "People",
  "Settings",
] as const;

/** DG3 uses an en dash between the route name and the product name. */
const TITLE_SEPARATOR = "–";

test.beforeEach(async ({ page }) => {
  await page.goto("/");
});

test("the seven primary destinations render in their frozen order", async ({ page }) => {
  const items = page.getByRole("navigation", { name: "Primary" }).getByRole("listitem");
  await expect(items).toHaveText([...PRIMARY_DESTINATIONS]);
});

test("System Health is reachable without displacing the seven primary destinations", async ({
  page,
}) => {
  // DG3 Information Architecture: System Health is a system destination, so
  // it must be reachable but must not sit inside the primary seven.
  const primaryList = page.getByRole("navigation", { name: "Primary" }).getByRole("listitem");
  await expect(primaryList).toHaveCount(PRIMARY_DESTINATIONS.length);
  await expect(page.getByRole("button", { name: "System Health" })).toBeVisible();
});

test("every route sets its own distinct document title", async ({ page }) => {
  // DG3 Accessibility and Interaction Contract: "Every route sets a
  // distinct document title beginning with the current route name and
  // Product Mission Control, so Windows task switching and assistive
  // technology can distinguish open product surfaces."
  const seen = new Set<string>();
  for (const destination of [...PRIMARY_DESTINATIONS, "System Health"]) {
    await page.getByRole("button", { name: destination, exact: true }).click();
    const expected = `${destination} ${TITLE_SEPARATOR} Product Mission Control`;
    await expect(page).toHaveTitle(expected);
    expect(seen.has(expected), `${destination} must not reuse another route's title`).toBe(false);
    seen.add(expected);
  }
});

test("the active destination is announced, not only coloured", async ({ page }) => {
  // Design system Navigation pattern: "Show selected... states
  // distinctly... colour alone is insufficient." aria-current is what
  // carries that for assistive technology.
  const cockpit = page.getByRole("button", { name: "Executive Cockpit", exact: true });
  await expect(cockpit).toHaveAttribute("aria-current", "page");

  await page.getByRole("button", { name: "Portfolio", exact: true }).click();

  await expect(page.getByRole("button", { name: "Portfolio", exact: true })).toHaveAttribute(
    "aria-current",
    "page",
  );
  await expect(cockpit).not.toHaveAttribute("aria-current", "page");
  // Exactly one destination may claim to be current.
  await expect(page.locator('[aria-current="page"]')).toHaveCount(1);
});

test("every destination is reachable and operable by keyboard alone", async ({ page }) => {
  // Native <button> elements are used precisely so Tab/Enter/Space work
  // without extra wiring. This proves that end to end rather than trusting
  // the element choice.
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Executive Cockpit", exact: true })).toBeFocused();

  // Tab must walk the destinations in the same frozen order they render in.
  for (const destination of PRIMARY_DESTINATIONS.slice(1)) {
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: destination, exact: true })).toBeFocused();
  }

  await page.keyboard.press("Enter");
  await expect(page).toHaveTitle(`Settings ${TITLE_SEPARATOR} Product Mission Control`);
});

test("Space activates a focused destination as well as Enter", async ({ page }) => {
  await page.getByRole("button", { name: "Portfolio", exact: true }).focus();
  await page.keyboard.press("Space");
  await expect(page).toHaveTitle(`Portfolio ${TITLE_SEPARATOR} Product Mission Control`);
});

test("the shell reflows at 1366x768 without a horizontal scrollbar", async ({ page }) => {
  // The DG3 reflow floor. A component test in jsdom cannot see this at all,
  // because jsdom has no layout engine.
  for (const destination of PRIMARY_DESTINATIONS) {
    await page.getByRole("button", { name: destination, exact: true }).click();
    const overflows = await page.evaluate(
      () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
    );
    expect(overflows, `${destination} must not overflow horizontally at 1366x768`).toBe(false);
  }
});

test("the content region is labelled by the route heading", async ({ page }) => {
  const main = page.getByRole("main");
  await expect(main).toHaveAccessibleName("Executive Cockpit");
  await expect(main.getByRole("heading", { level: 1 })).toHaveText("Executive Cockpit");
});

test("unimplemented routes say so in the product's own language", async ({ page }) => {
  // Executive Cockpit and Portfolio both have owning slices now, so this
  // navigates to one that does not. Pinning the string keeps a silent
  // regression to an English fallback visible, and pinning the fallback itself
  // keeps an unimplemented route from rendering an empty panel that looks like
  // a working screen with no data in it.
  await page.getByRole("button", { name: "Reviews & Reports", exact: true }).click();

  await expect(
    page.getByText("此畫面尚未實作，等待所屬垂直切片通過驗證後才會開放。"),
  ).toBeVisible();
});

test("the Executive Cockpit route renders its own content, not the placeholder", async ({
  page,
}) => {
  // Outside Tauri the IPC call cannot succeed, so the route settles into its
  // error state. That is the point: it proves the route is genuinely wired
  // and reached, and that a failed query reports failure instead of showing
  // stale or invented data.
  await expect(page.getByRole("alert")).toContainText("沒有顯示任何舊資料");
  await expect(page.getByText("此畫面尚未實作，等待所屬垂直切片通過驗證後才會開放。")).toHaveCount(
    0,
  );
});

/** The five semantic app-scale steps DG3 freezes, independent of Windows
 * display scaling. */
const APP_SCALE_STEPS = [90, 100, 110, 120, 130] as const;

test("every shell surface honours all five app-scale steps", async ({ page }) => {
  // DG3 requires the five steps to be honoured. A surface that ignores the
  // scale stays small exactly when a reader has asked for larger text, and
  // jsdom cannot see this at all because it computes no styles.
  // Only surfaces the default app actually renders. The policy strip returns
  // null with no states, so it is covered by its own component test rather
  // than skipped silently here -- a loop that `continue`s past a missing
  // element passes without asserting anything, which is worse than no test.
  const surfaces = [
    { name: "route title", selector: ".pmc-route-title" },
    { name: "navigation", selector: ".pmc-nav-item" },
  ];

  for (const { name, selector } of surfaces) {
    await expect(page.locator(selector).first()).toBeVisible();
    const sizes: number[] = [];
    for (const step of APP_SCALE_STEPS) {
      const size = await page.evaluate(
        ({ selector: target, step: value }) => {
          document.documentElement.style.setProperty("--pmc-app-scale", String(value / 100));
          const element = document.querySelector(target);
          if (!element) throw new Error(`missing ${target}`);
          return Number.parseFloat(window.getComputedStyle(element).fontSize);
        },
        { selector, step },
      );
      sizes.push(size);
    }
    await page.evaluate(() => {
      document.documentElement.style.removeProperty("--pmc-app-scale");
    });

    expect(sizes.length, `${name} must render at every step`).toBe(APP_SCALE_STEPS.length);
    // Walked with an explicit predecessor rather than by index, so the
    // comparison cannot silently compare against an absent element.
    let previous: number | undefined;
    for (const size of sizes) {
      if (previous !== undefined) {
        expect(size, `${name} must grow at every step`).toBeGreaterThan(previous);
      }
      previous = size;
    }
  }
});

test("the shell renders in both themes without borrowing the other's colours", async ({ page }) => {
  // A token defined only inside a media block never applies in the
  // un-stamped state, which is how a page ends up rendering one theme's text
  // on the other theme's ground.
  const readColours = () =>
    page.evaluate(() => {
      const shell = document.querySelector(".pmc-shell");
      if (!shell) return null;
      const style = window.getComputedStyle(shell);
      return { background: style.backgroundColor, color: style.color };
    });

  await page.emulateMedia({ colorScheme: "light" });
  const light = await readColours();
  await page.emulateMedia({ colorScheme: "dark" });
  const dark = await readColours();

  expect(light).not.toBeNull();
  expect(dark).not.toBeNull();
  // Every colour must resolve to something, in both themes.
  for (const resolved of [light, dark]) {
    expect(resolved?.background).not.toBe("");
    expect(resolved?.color).not.toBe("");
    expect(resolved?.background).not.toBe("rgba(0, 0, 0, 0)");
  }
  // And the two themes must actually differ, or one of them is not defined.
  expect(dark?.background).not.toBe(light?.background);
});

test("an explicit theme choice beats the operating system preference", async ({ page }) => {
  // The three-state contract: an explicit `data-theme` must win in both
  // directions, not only when it agrees with the OS.
  await page.emulateMedia({ colorScheme: "dark" });
  const osDark = await page.evaluate(() => window.getComputedStyle(document.body).backgroundColor);

  const forcedLight = await page.evaluate(() => {
    document.documentElement.setAttribute("data-theme", "light");
    return window.getComputedStyle(document.body).backgroundColor;
  });

  expect(forcedLight).not.toBe(osDark);
});
