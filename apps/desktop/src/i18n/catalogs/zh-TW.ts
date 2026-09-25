import type { Catalog } from "../messages";
import { INSPECTOR_ZH_TW } from "./inspector.zh-TW";
import { LABELS_ZH_TW } from "./labels.zh-TW";
import { LENS_ZH_TW } from "./lens.zh-TW";
import { PAGES_ZH_TW } from "./pages.zh-TW";
import { REVIEW_ZH_TW } from "./review.zh-TW";
import { ROUTES_ZH_TW } from "./routes.zh-TW";
import { SHELL_ZH_TW } from "./shell.zh-TW";
import { WORK_QUEUE_ZH_TW } from "./workQueue.zh-TW";
import { RESTORE_ZH_TW } from "./restore.zh-TW";
import { ENTRY_ZH_TW } from "./entry.zh-TW";
import { SAFE_ERRORS_ZH_TW } from "./safeErrors.zh-TW";

/**
 * The Traditional Chinese catalog. Same keys and placeholders as English;
 * `catalogs.test.ts` fails the build otherwise.
 *
 * Chinese has one plural form, so a `.one` message reads the same as its
 * `.other`; both are kept so every catalog has the same key set.
 */
export const ZH_TW = {
  "common.yes": "是",
  "common.no": "否",
  "common.idSeparator": "、",

  "error.unknown": "發生了一個未預期的錯誤。",
  "error.retryHint": "可以重試。",
  "error.noRetryHint": "重試不會改變結果。",
  "error.withHint": "{message}{hint}",

  ...SAFE_ERRORS_ZH_TW,
  ...LABELS_ZH_TW,
  ...SHELL_ZH_TW,
  ...REVIEW_ZH_TW,
  ...LENS_ZH_TW,
  ...ROUTES_ZH_TW,
  ...INSPECTOR_ZH_TW,
  ...PAGES_ZH_TW,

  ...WORK_QUEUE_ZH_TW,

  // Operational Restore and System Health: see restore.zh-TW.ts.

  ...RESTORE_ZH_TW,

  // Record entry, the Portfolio family: see entry.zh-TW.ts.
  ...ENTRY_ZH_TW,
} as const satisfies Catalog;
