import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import {
  loadDisplayLocale,
  preferenceFrom,
  resolvePreference,
  saveDisplayLocale,
  storedPreferenceForBoot,
} from "./i18n/displayLocale";
import { LocaleRoot } from "./i18n/LocaleRoot";
import "./design-system/tokens.css";
import "./overlays/overlays.css";
import "./shell/shell.css";
import "./routes/routes.css";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("PLATFORM_DESKTOP_ROOT_MISSING");
}
const container = root;

/** Renders once the stored language is known, so the first paint is already
 * in it. When it can't be read in time, the app follows the system this
 * session. Called exactly once: `storedPreferenceForBoot` settles once. */
function start(stored: unknown): void {
  const preference = preferenceFrom(stored);
  const systemTags = navigator.languages;
  document.documentElement.lang = resolvePreference(preference, systemTags);
  createRoot(container).render(
    <StrictMode>
      <LocaleRoot initial={preference} systemTags={systemTags} save={saveDisplayLocale}>
        <App />
      </LocaleRoot>
    </StrictMode>,
  );
}

void storedPreferenceForBoot(loadDisplayLocale, 2000).then(start);
