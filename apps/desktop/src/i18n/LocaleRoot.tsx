import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { resolvePreference, type LocalePreference } from "./displayLocale";
import { I18nProvider } from "./I18nProvider";
import { localeForSystem } from "./locale";
import { LocaleSettingContext, type LocaleSetting } from "./localeSetting";

/**
 * Holds the language preference for the whole app: resolves it, supplies
 * the matching catalog, keeps `<html lang>` in step, and lets Settings
 * change it. A change is stored first and shown only once the host has
 * kept it, so the screen never shows a language that won't come back.
 */
export function LocaleRoot({
  initial,
  systemTags,
  save,
  children,
}: {
  readonly initial: LocalePreference;
  readonly systemTags: readonly string[];
  readonly save: (preference: LocalePreference) => Promise<unknown>;
  readonly children: ReactNode;
}): ReactNode {
  const [preference, setPreference] = useState<LocalePreference>(initial);
  const locale = resolvePreference(preference, systemTags);
  const systemLocale = localeForSystem(systemTags);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  const choose = useCallback(
    async (next: LocalePreference) => {
      await save(next);
      setPreference(next);
    },
    [save],
  );

  const setting = useMemo<LocaleSetting>(
    () => ({ preference, systemLocale, choose }),
    [preference, systemLocale, choose],
  );

  return (
    <LocaleSettingContext.Provider value={setting}>
      <I18nProvider locale={locale}>{children}</I18nProvider>
    </LocaleSettingContext.Provider>
  );
}
