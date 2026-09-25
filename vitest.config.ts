import { defineConfig } from "vitest/config";

// Surfaces print times in the local zone. Tests pin that zone, so an expected
// "2023-11-14 22:13" means the same instant on every machine and in CI. It is
// set here, in the main process, because the zone is process-wide and a test
// worker thread cannot change it.
process.env.TZ = "UTC";

export default defineConfig({
  test: {
    environment: "jsdom",
    // Playwright owns apps/desktop/e2e. Without this, vitest's default
    // `**/*.spec.ts` glob would pick those files up and run them in jsdom,
    // where they cannot work.
    exclude: ["**/node_modules/**", "**/dist/**", "**/target/**", "apps/desktop/e2e/**"],
    maxWorkers: 1,
    pool: "threads",
    setupFiles: ["./apps/desktop/src/test/setup.ts"],
  },
});
