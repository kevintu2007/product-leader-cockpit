import { defineConfig, devices } from "@playwright/test";

/**
 * Tier 1 end-to-end configuration (renderer level).
 *
 * This drives the assembled React application in a real browser through the
 * Vite dev server, with no Tauri host and no Ledger behind it. That
 * boundary is deliberate. Tier 1 exists to pin the presentation contracts
 * that are already frozen -- the DG3 route, title, keyboard and
 * accessibility contracts -- which component tests cannot prove because
 * they never assemble the real shell, and which will not churn as feature
 * slices land.
 *
 * Tier 2 (the real Windows application over WebDriver, via tauri-driver and
 * msedgedriver) is what release readiness needs for installer, upgrade,
 * crash, native integration and real IPC. It is deliberately NOT started here: those
 * behaviours depend on a feature surface that does not exist yet, so
 * scripting them now would mean rewriting them later.
 */
export default defineConfig({
  testDir: "./apps/desktop/e2e",
  // A frozen contract that renders differently on a retry is a real defect,
  // so failures are not retried into passing.
  retries: 0,
  fullyParallel: true,
  reporter: process.env.CI ? [["github"], ["list"]] : [["list"]],
  use: {
    baseURL: "http://127.0.0.1:1420",
    trace: "on-first-retry",
  },
  projects: [
    {
      // 1366x768 is the DG3 reflow floor the contract names explicitly, so
      // it is the default viewport rather than a special case tucked into
      // one test.
      name: "chromium-1366x768",
      use: { ...devices["Desktop Chrome"], viewport: { width: 1366, height: 768 } },
    },
  ],
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
