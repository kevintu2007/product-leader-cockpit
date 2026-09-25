import type { Catalog } from "../messages";
import { INSPECTOR_ZH_CN } from "./inspector.zh-CN";
import { LABELS_ZH_CN } from "./labels.zh-CN";
import { LENS_ZH_CN } from "./lens.zh-CN";
import { PAGES_ZH_CN } from "./pages.zh-CN";
import { REVIEW_ZH_CN } from "./review.zh-CN";
import { ROUTES_ZH_CN } from "./routes.zh-CN";
import { SHELL_ZH_CN } from "./shell.zh-CN";
import { WORK_QUEUE_ZH_CN } from "./workQueue.zh-CN";
import { RESTORE_ZH_CN } from "./restore.zh-CN";
import { ENTRY_ZH_CN } from "./entry.zh-CN";
import { SAFE_ERRORS_ZH_CN } from "./safeErrors.zh-CN";

/**
 * The Simplified Chinese catalog, drafted from the Traditional one with mainland terminology. Same keys and placeholders as English;
 * `catalogs.test.ts` fails the build otherwise.
 *
 * Chinese has one plural form, so a `.one` message reads the same as its
 * `.other`; both are kept so every catalog has the same key set.
 */
export const ZH_CN = {
  "common.yes": "是",
  "common.no": "否",
  "common.idSeparator": "、",

  "error.unknown": "发生了一个未预期的错误。",
  "error.retryHint": "可以重试。",
  "error.noRetryHint": "重试不会改变结果。",
  "error.withHint": "{message}{hint}",

  ...SAFE_ERRORS_ZH_CN,
  ...LABELS_ZH_CN,
  ...SHELL_ZH_CN,
  ...REVIEW_ZH_CN,
  ...LENS_ZH_CN,
  ...ROUTES_ZH_CN,
  ...INSPECTOR_ZH_CN,
  ...PAGES_ZH_CN,

  ...WORK_QUEUE_ZH_CN,

  // Operational Restore and System Health: see restore.zh-CN.ts.

  ...RESTORE_ZH_CN,

  // Record entry, the Portfolio family: see entry.zh-CN.ts.
  ...ENTRY_ZH_CN,
} as const satisfies Catalog;
