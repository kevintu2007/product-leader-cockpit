import { describe, expect, it } from "vitest";

import { CATALOGS, DEFAULT_UI_LOCALE } from "./catalogs";
import { EN } from "./catalogs/en";
import { SUPPORTED_LOCALES } from "./locale";

function placeholders(template: string): string[] {
  return [...template.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map((match) => match[1] ?? "").sort();
}

/** Every catalog that exists, read as plain text by key. */
const present: [string, Readonly<Record<string, string>>][] = Object.entries(CATALOGS);

describe("message catalogs", () => {
  it("only exist for supported languages", () => {
    for (const [locale] of present) {
      expect(SUPPORTED_LOCALES).toContain(locale);
    }
  });

  it("exist for every supported language", () => {
    // A language offered without its catalog would silently show English.
    expect(Object.keys(CATALOGS).sort()).toEqual([...SUPPORTED_LOCALES].sort());
  });

  it.each(present)("%s has exactly the English keys", (_locale, catalog) => {
    // A missing key would fall back to nothing; an extra one is a message no
    // screen asks for and nobody reviews.
    expect(Object.keys(catalog).sort()).toEqual(Object.keys(EN).sort());
  });

  it.each(present)(
    "%s fills the same placeholders as English, message by message",
    (_locale, catalog) => {
      for (const [key, english] of Object.entries(EN)) {
        expect({ key, slots: placeholders(catalog[key] ?? "") }).toEqual({
          key,
          slots: placeholders(english),
        });
      }
    },
  );

  it.each(present)("%s has no empty message", (_locale, catalog) => {
    for (const [key, text] of Object.entries(catalog)) {
      expect({ key, empty: text.trim() === "" }).toEqual({ key, empty: false });
    }
  });

  it("gives every plural message the `other` form every language needs", () => {
    const bases = new Set(
      Object.keys(EN)
        .filter((key) => /\.(zero|one|two|few|many)$/.test(key))
        .map((key) => key.replace(/\.[a-z]+$/, "")),
    );
    for (const base of bases) {
      expect(Object.keys(EN)).toContain(`${base}.other`);
    }
  });

  it("gives every plural form of one message the same placeholders", () => {
    // `plural` types its parameters from the `other` form; a form that used
    // a different slot would print it unfilled.
    const english: Readonly<Record<string, string>> = EN;
    for (const [key, text] of Object.entries(EN)) {
      const form = /^(.*)\.(zero|one|two|few|many)$/.exec(key);
      if (form !== null) {
        expect({ key, slots: placeholders(text) }).toEqual({
          key,
          slots: placeholders(english[`${form[1] ?? ""}.other`] ?? ""),
        });
      }
    }
  });

  it("has a catalog for the language the UI shows by default", () => {
    expect(CATALOGS[DEFAULT_UI_LOCALE]).toBeDefined();
  });
});
