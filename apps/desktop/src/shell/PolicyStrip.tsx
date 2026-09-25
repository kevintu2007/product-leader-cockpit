import { useT } from "../i18n/useT";

import type { MessageKey } from "../i18n/messages";

export type PolicyStripStateKind =
  | "degraded"
  | "evidence-verification-pending"
  | "out-of-sync"
  | "cancelling"
  | "backup-due"
  | "backing-up"
  | "restoring";

export interface PolicyStripState {
  readonly kind: PolicyStripStateKind;
  /** Message text in the active language, supplied by the state's owning slice (e.g. S3 for
   * `degraded`/`out-of-sync`, S7 for `backup-due`). This component only
   * hosts the shared strip; it never derives policy facts itself. */
  readonly message: string;
  /** One next step, e.g. "Open Backups" on Backup due (DG3 backup-setup
   * amendment §5). */
  readonly action?: { readonly label: string; readonly onSelect: () => void } | undefined;
}

export interface PolicyStripProps {
  readonly states: readonly PolicyStripState[];
}

const KIND_LABEL = {
  degraded: "policyStrip.degraded",
  "evidence-verification-pending": "policyStrip.evidenceVerificationPending",
  "out-of-sync": "policyStrip.outOfSync",
  cancelling: "policyStrip.cancelling",
  "backup-due": "policyStrip.backupDue",
  "backing-up": "policyStrip.backingUp",
  restoring: "policyStrip.restoring",
} as const satisfies Record<PolicyStripStateKind, MessageKey>;

/**
 * The persistent cross-route policy strip: surfaces
 * Degraded, Evidence verification pending, Out-of-sync, Cancelling, and
 * Backup Due wherever they apply, independent of which route is active
 * (DG3 State and Feedback Contract). Renders nothing when no state applies -- an empty strip is not
 * a visible "all clear" banner, it simply does not take up space.
 *
 * `role="status"`/`aria-live="polite"` announces a newly appearing state
 * without stealing focus, matching the DG3 testing decision to cover
 * "announcements". Each item names its own kind in text (not color alone),
 * so removing color leaves the strip fully legible.
 */
export function PolicyStrip({ states }: PolicyStripProps) {
  const t = useT();
  if (states.length === 0) {
    return null;
  }
  return (
    <div role="status" aria-live="polite" className="pmc-policy-strip">
      {states.map((state) => (
        <div key={state.kind} className="pmc-policy-strip-item" data-kind={state.kind}>
          <span className="pmc-policy-strip-label">{t(KIND_LABEL[state.kind])}</span>
          <span className="pmc-policy-strip-message">{state.message}</span>
          {state.action !== undefined && (
            <button
              type="button"
              className="pmc-policy-strip-action"
              onClick={state.action.onSelect}
            >
              {state.action.label}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
