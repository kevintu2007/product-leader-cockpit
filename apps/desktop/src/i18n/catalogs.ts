import { EN } from "./catalogs/en";
import { ES } from "./catalogs/es";
import { JA } from "./catalogs/ja";
import { KO } from "./catalogs/ko";
import { ZH_CN } from "./catalogs/zh-CN";
import { ZH_TW } from "./catalogs/zh-TW";
import { FALLBACK_LOCALE, type Locale } from "./locale";
import { createTranslator, type Catalog, type Translator } from "./messages";

/**
 * One catalog per supported language; `catalogs.test.ts` fails the build if
 * one is missing. A locale without a catalog would fall back to English as a
 * whole rather than mix two languages on one screen.
 */
export const CATALOGS: Readonly<Partial<Record<Locale, Catalog>>> = {
  en: EN,
  "zh-TW": ZH_TW,
  "zh-CN": ZH_CN,
  ja: JA,
  ko: KO,
  es: ES,
};

/**
 * The language the UI shows until the language setting ships: the one every
 * screen was written in. Changing it is that slice's job, together with the
 * stored choice and the system default.
 */
export const DEFAULT_UI_LOCALE: Locale = "zh-TW";

export function translatorFor(locale: Locale): Translator {
  const catalog = CATALOGS[locale];
  return catalog === undefined
    ? createTranslator(FALLBACK_LOCALE, EN)
    : createTranslator(locale, catalog);
}
