import { createContext, useContext } from "react";

import type { LocalePreference } from "./displayLocale";
import type { Locale } from "./locale";

/** The language setting, for the one screen that changes it. */
export interface LocaleSetting {
  /** What is stored: a language, or following the system. */
  readonly preference: LocalePreference;
  /** The language the system asks for, which following it means. */
  readonly systemLocale: Locale;
  /** Store a new preference, then switch to it. Rejects with the host's
   * safe error when it could not be stored; nothing changes then. */
  readonly choose: (preference: LocalePreference) => Promise<void>;
}

/** `null` outside `LocaleRoot`: a route rendered alone has no setting to
 * change, and shows none. */
export const LocaleSettingContext = createContext<LocaleSetting | null>(null);

export function useLocaleSetting(): LocaleSetting | null {
  return useContext(LocaleSettingContext);
}
