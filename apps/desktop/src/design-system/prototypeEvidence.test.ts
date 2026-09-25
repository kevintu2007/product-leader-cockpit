import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

/**
 * The production shell is required to *match the immutable
 * prototype evidence*, not merely to satisfy the DG3 contract text. The DG1
 * evidence at `docs/evidence/contrast-results.json` is that
 * record: it fixes the token version, the exact chart palette, and 89
 * measured contrast pairs.
 *
 * These tests compare the shipped design tokens against that file. Reading it
 * rather than restating its numbers is the point -- a restated expectation
 * drifts from its source silently, which is precisely what "immutable
 * evidence" exists to prevent.
 */

interface ContrastPair {
  readonly theme: string;
  readonly role: string;
  readonly foreground: string;
  readonly background: string;
  readonly minimum: number;
  readonly ratio: number;
  readonly pass: boolean;
}

interface ContrastEvidence {
  readonly tokenVersion: string;
  readonly requiredTokenCount: number;
  readonly missingTokens: readonly string[];
  readonly chartPalette: readonly { series: number; light: string; dark: string }[];
  readonly pairCount: number;
  readonly failures: readonly unknown[];
  readonly results: readonly ContrastPair[];
}

const evidence = JSON.parse(
  readFileSync("docs/evidence/contrast-results.json", "utf8"),
) as ContrastEvidence;

const tokens = readFileSync("apps/desktop/src/design-system/tokens.css", "utf8");

/** Every hex colour the stylesheet declares, upper-cased for comparison. */
function declaredColours(css: string): Set<string> {
  return new Set((css.match(/#[0-9a-fA-F]{6}\b/g) ?? []).map((hex) => hex.toUpperCase()));
}

describe("DG1 prototype evidence", () => {
  it("the evidence it is compared against recorded no failures of its own", () => {
    // If the evidence itself recorded failures, matching it would mean
    // matching a broken baseline.
    expect(evidence.failures).toHaveLength(0);
    expect(evidence.missingTokens).toHaveLength(0);
    expect(evidence.results).toHaveLength(evidence.pairCount);
  });

  it("every contrast pair in the evidence met its own minimum", () => {
    for (const pair of evidence.results) {
      expect(
        pair.ratio,
        `${pair.theme} ${pair.role} ${pair.foreground} on ${pair.background}`,
      ).toBeGreaterThanOrEqual(pair.minimum);
      expect(pair.pass).toBe(true);
    }
  });

  it("the production tokens carry the exact chart palette the evidence fixed", () => {
    // Eight series, each with a light and a dark value. A palette that drifted
    // would change what a reader sees without any contrast measurement having
    // been redone.
    const declared = declaredColours(tokens);
    const missing: string[] = [];
    for (const series of evidence.chartPalette) {
      for (const value of [series.light, series.dark]) {
        if (!declared.has(value.toUpperCase())) {
          missing.push(`series ${String(series.series)}: ${value}`);
        }
      }
    }

    expect(missing, "chart palette values absent from the shipped tokens").toEqual([]);
  });

  it("the production tokens carry every foreground and background the evidence measured", () => {
    // The contrast numbers only describe the shipped product if the shipped
    // product actually uses the colours that were measured.
    const declared = declaredColours(tokens);
    const measured = new Set<string>();
    for (const pair of evidence.results) {
      measured.add(pair.foreground.toUpperCase());
      measured.add(pair.background.toUpperCase());
    }

    const missing = [...measured].filter((colour) => !declared.has(colour)).sort();

    expect(missing, "measured colours absent from the shipped tokens").toEqual([]);
  });

  it("both themes are defined for the un-stamped default as well as the explicit choice", () => {
    // The three-state contract. A token defined only inside a media block
    // never applies when the root carries no `data-theme`, which is how a
    // page renders one theme's text on the other theme's ground.
    expect(tokens).toMatch(/^:root\s*\{/m);
    expect(tokens).toContain("@media (prefers-color-scheme: dark)");
    expect(tokens).toContain(':root:not([data-theme="light"])');
    expect(tokens).toContain(':root[data-theme="dark"]');
  });
});
