import { INSPECTOR_EN } from "./inspector.en";
import { LABELS_EN } from "./labels.en";
import { LENS_EN } from "./lens.en";
import { PAGES_EN } from "./pages.en";
import { REVIEW_EN } from "./review.en";
import { ROUTES_EN } from "./routes.en";
import { SHELL_EN } from "./shell.en";
import { WORK_QUEUE_EN } from "./workQueue.en";
import { RESTORE_EN } from "./restore.en";
import { ENTRY_EN } from "./entry.en";
import { SAFE_ERRORS_EN } from "./safeErrors.en";

/**
 * The canonical English catalog. Its keys are the only keys any catalog may
 * have, and its `{name}` placeholders are the parameters each message takes.
 *
 * `safeError.*` holds one message per key the host can put in a safe error
 * envelope's `messageKey`. A key is added when the command that emits it
 * ships -- never ahead of it -- so no catalog describes a failure no code path
 * raises. The host never sends prose; these are the only words for them.
 *
 * Canonical domain nouns (Product Ledger, Action Request, Decision, Evidence,
 * Judgment, Risk, Issue) keep their English names in every language.
 */
export const EN = {
  // Words shared by every surface.
  "common.yes": "yes",
  "common.no": "no",
  "common.idSeparator": ", ",

  // O05, the safe error detail.
  "error.unknown": "Something unexpected went wrong.",
  "error.retryHint": "You can try again.",
  "error.noRetryHint": "Trying again will not change the result.",
  // The error and what retrying would do, as two sentences in one place, so
  // each language decides their order and how they are joined.
  "error.withHint": "{message} {hint}",

  // Every host error, field name and next step: see safeErrors.en.ts.
  ...SAFE_ERRORS_EN,

  // Words for the host's record identifiers: see labels.en.ts.
  ...LABELS_EN,

  // The app shell and shared overlays: see shell.en.ts.
  ...SHELL_EN,

  // O03, the focused review: see review.en.ts.
  ...REVIEW_EN,

  // The Portfolio Lens: see lens.en.ts.
  ...LENS_EN,

  // The routes: see routes.en.ts.
  ...ROUTES_EN,

  // O01, the Product inspector: see inspector.en.ts.
  ...INSPECTOR_EN,

  // People, Reviews & Reports, Product Vault and Settings: see pages.en.ts.
  ...PAGES_EN,

  // S03, the Work Queue: see workQueue.en.ts.
  ...WORK_QUEUE_EN,

  // Operational Restore and System Health: see restore.en.ts.
  ...RESTORE_EN,

  // Record entry, the Portfolio family: see entry.en.ts.
  ...ENTRY_EN,
} as const;
