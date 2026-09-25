import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * The host checks the typed phrase of "Delete sample data" itself (the
 * accepted sample-workspace amendment §8). Its list must be exactly the
 * phrases the six catalogs show, or a person typing what the sheet asks for
 * would be refused — or a phrase no sheet shows would be accepted.
 */
const LANGUAGES = ["en", "zh-TW", "zh-CN", "ja", "ko", "es"] as const;

function catalogPhrase(language: string): string {
  const text = readFileSync(`apps/desktop/src/i18n/catalogs/pages.${language}.ts`, "utf8");
  const match = /"sampleWorkspace\.delete\.phrase":\s*"([^"]+)"/.exec(text);
  if (match?.[1] === undefined) {
    throw new Error(`no phrase in ${language}`);
  }
  return match[1];
}

function hostPhrases(): string[] {
  const source = readFileSync("apps/desktop/src-tauri/src/sample_workspace.rs", "utf8");
  const block = /pub const DELETE_PHRASES: \[&str; \d+\] = \[([\s\S]*?)\];/.exec(source)?.[1];
  if (block === undefined) {
    throw new Error("no DELETE_PHRASES in sample_workspace.rs");
  }
  return [...block.matchAll(/"([^"]+)"/g)].map((found) => found[1] ?? "");
}

describe("the sample delete phrases", () => {
  it("are the same in the host and in every catalog", () => {
    expect(hostPhrases().sort()).toEqual(LANGUAGES.map(catalogPhrase).sort());
  });
});
