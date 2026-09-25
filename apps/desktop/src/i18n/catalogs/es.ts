import type { Catalog } from "../messages";
import { INSPECTOR_ES } from "./inspector.es";
import { LABELS_ES } from "./labels.es";
import { LENS_ES } from "./lens.es";
import { PAGES_ES } from "./pages.es";
import { REVIEW_ES } from "./review.es";
import { ROUTES_ES } from "./routes.es";
import { SAFE_ERRORS_ES } from "./safeErrors.es";
import { SHELL_ES } from "./shell.es";
import { WORK_QUEUE_ES } from "./workQueue.es";
import { RESTORE_ES } from "./restore.es";
import { ENTRY_ES } from "./entry.es";

/**
 * The Spanish catalog. Same keys and placeholders as English;
 * `catalogs.test.ts` fails the build otherwise. Spanish has two plural
 * forms, and they differ as English's do.
 *
 * Drafted with AI assistance and not yet reviewed by a native speaker;
 * corrections are welcome.
 */
export const ES = {
  "common.yes": "Sí",
  "common.no": "No",
  "common.idSeparator": ", ",

  "error.unknown": "Se produjo un error inesperado.",
  "error.retryHint": "Puedes volver a intentarlo.",
  "error.noRetryHint": "Volver a intentarlo no cambiará el resultado.",
  "error.withHint": "{message} {hint}",

  ...SAFE_ERRORS_ES,
  ...LABELS_ES,
  ...SHELL_ES,
  ...REVIEW_ES,
  ...LENS_ES,
  ...ROUTES_ES,
  ...INSPECTOR_ES,
  ...PAGES_ES,
  ...WORK_QUEUE_ES,

  // Operational Restore and System Health: see restore.es.ts.
  ...RESTORE_ES,

  // Record entry, the Portfolio family: see entry.es.ts.
  ...ENTRY_ES,
} as const satisfies Catalog;
