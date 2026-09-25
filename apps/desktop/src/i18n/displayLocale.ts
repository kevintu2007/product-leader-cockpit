import { invoke } from "@tauri-apps/api/core";

import { isLocale, localeForSystem, type Locale } from "./locale";

/**
 * The stored language preference: one of the six UI languages, or "follow
 * the system", which the host stores as BCP 47 `und` ("undetermined") and
 * this side resolves against the system's languages at every launch. `und`
 * is never a language to display in.
 */
export const FOLLOW_SYSTEM = "und";

export type LocalePreference = Locale | typeof FOLLOW_SYSTEM;

/** Each language's name in that language, so a person can find their own
 * whatever the screen is showing now. */
export const LANGUAGE_NAMES: Readonly<Record<Locale, string>> = {
  en: "English",
  "zh-TW": "繁體中文",
  "zh-CN": "简体中文",
  ja: "日本語",
  ko: "한국어",
  es: "Español",
};

/** A stored value as a preference; anything unknown follows the system. */
export function preferenceFrom(stored: unknown): LocalePreference {
  return typeof stored === "string" && isLocale(stored) ? stored : FOLLOW_SYSTEM;
}

/** The language to display for a preference, given the system's languages. */
export function resolvePreference(
  preference: LocalePreference,
  systemTags: readonly string[],
): Locale {
  return preference === FOLLOW_SYSTEM ? localeForSystem(systemTags) : preference;
}

/**
 * The stored preference for the first render, never waiting longer than
 * `timeoutMs`: a read that fails, or hasn't answered by then, gives `null`
 * (follow the system), so the window is never left blank.
 */
export function storedPreferenceForBoot(
  load: () => Promise<string>,
  timeoutMs: number,
): Promise<unknown> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      resolve(null);
    }, timeoutMs);
    load().then(
      (stored) => {
        clearTimeout(timer);
        resolve(stored);
      },
      () => {
        clearTimeout(timer);
        resolve(null);
      },
    );
  });
}

/** The stored preference, as the host holds it. */
export function loadDisplayLocale(): Promise<string> {
  return invoke<string>("get_display_locale");
}

/** Store a new preference; resolves with what the host stored. */
export function saveDisplayLocale(preference: LocalePreference): Promise<string> {
  return invoke<string>("set_display_locale", { locale: preference });
}
