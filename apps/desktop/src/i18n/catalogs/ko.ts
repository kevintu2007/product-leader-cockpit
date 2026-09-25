import type { Catalog } from "../messages";
import { INSPECTOR_KO } from "./inspector.ko";
import { LABELS_KO } from "./labels.ko";
import { LENS_KO } from "./lens.ko";
import { PAGES_KO } from "./pages.ko";
import { REVIEW_KO } from "./review.ko";
import { ROUTES_KO } from "./routes.ko";
import { SAFE_ERRORS_KO } from "./safeErrors.ko";
import { SHELL_KO } from "./shell.ko";
import { WORK_QUEUE_KO } from "./workQueue.ko";
import { RESTORE_KO } from "./restore.ko";
import { ENTRY_KO } from "./entry.ko";

/**
 * The Korean catalog. Same keys and placeholders as English;
 * `catalogs.test.ts` fails the build otherwise. Korean has one plural
 * form, so each `.one` reads the same as its `.other`.
 *
 * Drafted with AI assistance and not yet reviewed by a native speaker;
 * corrections are welcome.
 */
export const KO = {
  "common.yes": "예",
  "common.no": "아니요",
  "common.idSeparator": ", ",

  "error.unknown": "예기치 않은 오류가 발생했습니다.",
  "error.retryHint": "다시 시도할 수 있습니다.",
  "error.noRetryHint": "다시 시도해도 결과는 같습니다.",
  "error.withHint": "{message} {hint}",

  ...SAFE_ERRORS_KO,
  ...LABELS_KO,
  ...SHELL_KO,
  ...REVIEW_KO,
  ...LENS_KO,
  ...ROUTES_KO,
  ...INSPECTOR_KO,
  ...PAGES_KO,
  ...WORK_QUEUE_KO,

  // Operational Restore and System Health: see restore.ko.ts.
  ...RESTORE_KO,

  // Record entry, the Portfolio family: see entry.ko.ts.
  ...ENTRY_KO,
} as const satisfies Catalog;
