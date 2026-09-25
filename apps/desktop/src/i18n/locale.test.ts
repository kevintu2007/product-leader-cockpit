import { describe, expect, it } from "vitest";

import { localeForSystem, localeForTag } from "./locale";

describe("system language to UI language", () => {
  it.each([
    ["en-US", "en"],
    ["en-GB", "en"],
    ["ja-JP", "ja"],
    ["ko-KR", "ko"],
    ["es-419", "es"],
    ["es-ES", "es"],
    ["zh-TW", "zh-TW"],
    ["zh-HK", "zh-TW"],
    ["zh-MO", "zh-TW"],
    ["zh-Hant", "zh-TW"],
    ["zh-Hant-CN", "zh-TW"],
    ["zh-CN", "zh-CN"],
    ["zh-SG", "zh-CN"],
    ["zh-Hans", "zh-CN"],
    ["zh-Hans-TW", "zh-CN"],
    ["zh", "zh-CN"],
    ["zh_TW", "zh-TW"],
  ])("%s speaks %s", (tag, locale) => {
    expect(localeForTag(tag)).toBe(locale);
  });

  it("does not guess a relative for a language it does not speak", () => {
    expect(localeForTag("fr-FR")).toBeNull();
    expect(localeForTag("")).toBeNull();
  });

  it("takes the first preferred language it speaks, else English", () => {
    expect(localeForSystem(["fr-FR", "ja-JP", "en-US"])).toBe("ja");
    expect(localeForSystem(["fr-FR", "de-DE"])).toBe("en");
    expect(localeForSystem([])).toBe("en");
  });
});
