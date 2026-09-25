/** The five semantic app-scale steps DG3 freezes, independent of Windows
 * display scaling (100/125/150%). */
export const APP_TEXT_SCALE_STEPS = [90, 100, 110, 120, 130] as const;
export type AppTextScaleStep = (typeof APP_TEXT_SCALE_STEPS)[number];

const STORAGE_KEY = "pmc-app-text-scale";

function isStep(value: number): value is AppTextScaleStep {
  return (APP_TEXT_SCALE_STEPS as readonly number[]).includes(value);
}

/**
 * The scale this person chose, or 100 when nothing usable is stored. A
 * presentation setting only (S10): it lives in the webview's own storage and
 * never reaches the Ledger.
 */
export function readStoredTextScale(): AppTextScaleStep {
  try {
    const stored = Number(window.localStorage.getItem(STORAGE_KEY));
    return isStep(stored) ? stored : 100;
  } catch {
    return 100;
  }
}

/** Applies a step to the whole app and remembers it. */
export function applyTextScale(step: AppTextScaleStep): void {
  document.documentElement.style.setProperty("--pmc-app-scale", String(step / 100));
  try {
    window.localStorage.setItem(STORAGE_KEY, String(step));
  } catch {
    // Storage can be unavailable; the scale still applies for this session.
  }
}
