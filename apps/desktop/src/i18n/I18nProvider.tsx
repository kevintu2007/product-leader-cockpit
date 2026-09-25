import { useMemo, type ReactNode } from "react";

import { translatorFor } from "./catalogs";
import type { Locale } from "./locale";
import { I18nContext } from "./useT";

/** Supplies the active language to everything beneath it. */
export function I18nProvider({
  locale,
  children,
}: {
  readonly locale: Locale;
  readonly children: ReactNode;
}): ReactNode {
  const translator = useMemo(() => translatorFor(locale), [locale]);
  return <I18nContext.Provider value={translator}>{children}</I18nContext.Provider>;
}
