import type { Catalog } from "../messages";
import { INSPECTOR_JA } from "./inspector.ja";
import { LABELS_JA } from "./labels.ja";
import { LENS_JA } from "./lens.ja";
import { PAGES_JA } from "./pages.ja";
import { REVIEW_JA } from "./review.ja";
import { ROUTES_JA } from "./routes.ja";
import { SAFE_ERRORS_JA } from "./safeErrors.ja";
import { SHELL_JA } from "./shell.ja";
import { WORK_QUEUE_JA } from "./workQueue.ja";
import { RESTORE_JA } from "./restore.ja";
import { ENTRY_JA } from "./entry.ja";

/**
 * The Japanese catalog. Same keys and placeholders as English;
 * `catalogs.test.ts` fails the build otherwise. Japanese has one plural
 * form, so each `.one` reads the same as its `.other`.
 *
 * Drafted with AI assistance and not yet reviewed by a native speaker;
 * corrections are welcome.
 */
export const JA = {
  "common.yes": "はい",
  "common.no": "いいえ",
  "common.idSeparator": "、",

  "error.unknown": "予期しないエラーが発生しました。",
  "error.retryHint": "再試行できます。",
  "error.noRetryHint": "再試行しても結果は変わりません。",
  "error.withHint": "{message}{hint}",

  ...SAFE_ERRORS_JA,
  ...LABELS_JA,
  ...SHELL_JA,
  ...REVIEW_JA,
  ...LENS_JA,
  ...ROUTES_JA,
  ...INSPECTOR_JA,
  ...PAGES_JA,
  ...WORK_QUEUE_JA,

  // Operational Restore and System Health: see restore.ja.ts.
  ...RESTORE_JA,

  // Record entry, the Portfolio family: see entry.ja.ts.
  ...ENTRY_JA,
} as const satisfies Catalog;
