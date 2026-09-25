import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/**
 * The host checks the fixed phrase of the Vault-folder confirmation itself
 * (item ⑦, §3.3). Its list must be exactly the phrases the six catalogs
 * show, or a person typing what the sheet asks for would be refused — or a
 * phrase no sheet shows would be accepted.
 */
const LANGUAGES = ["en", "zh-TW", "zh-CN", "ja", "ko", "es"] as const;

function catalogPhrase(language: string): string {
  const text = readFileSync(`apps/desktop/src/i18n/catalogs/pages.${language}.ts`, "utf8");
  const match = /"vaultRoot\.confirm\.phrase":\s*"([^"]+)"/.exec(text);
  if (match?.[1] === undefined) {
    throw new Error(`no phrase in ${language}`);
  }
  return match[1];
}

function hostPhrases(): string[] {
  const source = readFileSync("apps/desktop/src-tauri/src/vault_root.rs", "utf8");
  const block = /const CONFIRMATION_PHRASES: \[&str; \d+\] = \[([\s\S]*?)\];/.exec(source)?.[1];
  if (block === undefined) {
    throw new Error("no CONFIRMATION_PHRASES in vault_root.rs");
  }
  return [...block.matchAll(/"([^"]+)"/g)].map((found) => found[1] ?? "");
}

describe("the Vault confirmation phrases", () => {
  it("are the same in the host and in every catalog", () => {
    expect(hostPhrases().sort()).toEqual(LANGUAGES.map(catalogPhrase).sort());
  });
});
