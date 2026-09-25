import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

import "@testing-library/jest-dom/vitest";

// vitest.config.ts does not set `test.globals: true`, so
// @testing-library/react's automatic per-test cleanup (which relies on a
// global `afterEach`) never registers on its own. Without this, every
// `render()` in a multi-test file accumulates in the jsdom document and
// later queries in the same file see duplicate elements.
afterEach(() => {
  cleanup();
});
