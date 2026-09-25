import type { MessageKey } from "../i18n/messages";

/**
 * The Executive Lens marking modes. Kept apart from the components so the
 * component module exports components only.
 */
export type LensMode = "timing" | "observability" | "evidence";

export const LENS_MODES = [
  { mode: "timing", label: "lens.mode.timing" },
  { mode: "observability", label: "lens.mode.observability" },
  { mode: "evidence", label: "lens.mode.evidence" },
] as const satisfies readonly { readonly mode: LensMode; readonly label: MessageKey }[];

/** What each mode marks. Modes change emphasis and wording, never positions. */
export const LENS_MODE_COPY = {
  timing: "lens.modeCopy.timing",
  observability: "lens.modeCopy.observability",
  evidence: "lens.modeCopy.evidence",
} as const satisfies Record<LensMode, MessageKey>;
