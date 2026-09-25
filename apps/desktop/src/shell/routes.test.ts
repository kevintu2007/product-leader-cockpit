import { describe, expect, it } from "vitest";

import { DEFAULT_ROUTE_ID, documentTitleFor, findRoute, PRIMARY_ROUTES, ROUTES } from "./routes";

describe("PRIMARY_ROUTES", () => {
  it("contains exactly the seven stable primary destinations in the frozen order", () => {
    expect(PRIMARY_ROUTES.map((route) => route.id)).toEqual([
      "executive-cockpit",
      "portfolio",
      "work-queue",
      "reviews-and-reports",
      "product-vault",
      "people",
      "settings",
    ]);
  });

  it("excludes System Health, which is reachable but not a primary destination", () => {
    expect(PRIMARY_ROUTES.some((route) => route.id === "system-health")).toBe(false);
  });
});

describe("ROUTES", () => {
  it("includes System Health as a reachable system destination", () => {
    expect(ROUTES.some((route) => route.id === "system-health")).toBe(true);
  });

  it("gives every route a non-empty label", () => {
    for (const route of ROUTES) {
      expect(route.label.trim().length).toBeGreaterThan(0);
    }
  });
});

describe("findRoute", () => {
  it("returns the matching route definition", () => {
    expect(findRoute("portfolio").label).toBe("Portfolio");
  });
});

describe("documentTitleFor", () => {
  it("begins with the route name and ends with Product Mission Control", () => {
    const title = documentTitleFor("executive-cockpit");
    expect(title.startsWith("Executive Cockpit")).toBe(true);
    expect(title.endsWith("Product Mission Control")).toBe(true);
  });

  it("produces a distinct title for every route", () => {
    const titles = ROUTES.map((route) => documentTitleFor(route.id));
    expect(new Set(titles).size).toBe(titles.length);
  });
});

describe("DEFAULT_ROUTE_ID", () => {
  it("is Executive Cockpit, the accepted default home", () => {
    expect(DEFAULT_ROUTE_ID).toBe("executive-cockpit");
  });
});
