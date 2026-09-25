import { createContext, useContext } from "react";

import { DEFAULT_UI_LOCALE, translatorFor } from "./catalogs";
import type { Translator } from "./messages";

/** The active language. Without a provider a component speaks the default
 * UI language, so a route rendered alone in a test reads exactly as it does
 * in the app today. */
export const I18nContext = createContext<Translator>(translatorFor(DEFAULT_UI_LOCALE));

/** The active language's translator: `t(key, params)`, `t.plural(...)`. */
export function useT(): Translator {
  return useContext(I18nContext);
}
