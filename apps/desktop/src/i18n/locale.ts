/**
 * The six UI languages and how a system language tag maps onto them.
 *
 * English is the fallback: a tag that names none of the six gets English
 * rather than a guess at the nearest relative.
 */
export const SUPPORTED_LOCALES = ["en", "zh-TW", "zh-CN", "ja", "ko", "es"] as const;

export type Locale = (typeof SUPPORTED_LOCALES)[number];

export const FALLBACK_LOCALE: Locale = "en";

export function isLocale(value: string): value is Locale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/**
 * The supported locale a BCP 47 tag names, or `null` when it names none.
 *
 * Chinese is split by script, not by country alone: Traditional (`Hant`, or a
 * region that writes it -- Taiwan, Hong Kong, Macau) is `zh-TW`, Simplified
 * (`Hans`, mainland China, Singapore) is `zh-CN`. A bare `zh` carries no script,
 * and takes CLDR's likely script for it, Simplified.
 */
export function localeForTag(tag: string): Locale | null {
  const subtags = tag.trim().replace(/_/g, "-").toLowerCase().split("-");
  const [language, ...rest] = subtags;
  switch (language) {
    case "en":
      return "en";
    case "ja":
      return "ja";
    case "ko":
      return "ko";
    case "es":
      return "es";
    case "zh": {
      if (rest.includes("hant")) {
        return "zh-TW";
      }
      if (rest.includes("hans")) {
        return "zh-CN";
      }
      if (rest.some((subtag) => subtag === "tw" || subtag === "hk" || subtag === "mo")) {
        return "zh-TW";
      }
      return "zh-CN";
    }
    default:
      return null;
  }
}

/** The first of the system's preferred languages PMC speaks, else English. */
export function localeForSystem(tags: readonly string[]): Locale {
  for (const tag of tags) {
    const locale = localeForTag(tag);
    if (locale !== null) {
      return locale;
    }
  }
  return FALLBACK_LOCALE;
}
