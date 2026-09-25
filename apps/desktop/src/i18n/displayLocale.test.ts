import { describe, expect, it, vi } from "vitest";

import {
  FOLLOW_SYSTEM,
  LANGUAGE_NAMES,
  preferenceFrom,
  resolvePreference,
  storedPreferenceForBoot,
} from "./displayLocale";
import { SUPPORTED_LOCALES } from "./locale";

describe("the language preference", () => {
  it("follows the system when told to, and falls back to English there", () => {
    expect(resolvePreference(FOLLOW_SYSTEM, ["ko-KR", "en-US"])).toBe("ko");
    expect(resolvePreference(FOLLOW_SYSTEM, ["zh-Hant-HK"])).toBe("zh-TW");
    expect(resolvePreference(FOLLOW_SYSTEM, ["fr-FR", "de"])).toBe("en");
    expect(resolvePreference(FOLLOW_SYSTEM, [])).toBe("en");
  });

  it("uses a stored language as it is, whatever the system says", () => {
    expect(resolvePreference("zh-TW", ["en-US"])).toBe("zh-TW");
    expect(resolvePreference("es", ["ja-JP"])).toBe("es");
  });

  it("reads anything it doesn't know, or nothing at all, as following the system", () => {
    expect(preferenceFrom("ja")).toBe("ja");
    expect(preferenceFrom("und")).toBe(FOLLOW_SYSTEM);
    expect(preferenceFrom("fr")).toBe(FOLLOW_SYSTEM);
    expect(preferenceFrom(null)).toBe(FOLLOW_SYSTEM);
    expect(preferenceFrom(42)).toBe(FOLLOW_SYSTEM);
  });

  it("starts with the stored preference when the host answers in time", async () => {
    await expect(storedPreferenceForBoot(() => Promise.resolve("ja"), 50)).resolves.toBe("ja");
  });

  it("starts following the system when the read fails or never answers", async () => {
    await expect(
      storedPreferenceForBoot(() => Promise.reject(new Error("down")), 50),
    ).resolves.toBeNull();
    vi.useFakeTimers();
    try {
      const boot = storedPreferenceForBoot(() => new Promise<string>(() => undefined), 50);
      vi.advanceTimersByTime(50);
      await expect(boot).resolves.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("names every language in that language", () => {
    expect(Object.keys(LANGUAGE_NAMES).sort()).toEqual([...SUPPORTED_LOCALES].sort());
    expect(LANGUAGE_NAMES["zh-TW"]).toBe("繁體中文");
    expect(LANGUAGE_NAMES.ko).toBe("한국어");
  });
});
