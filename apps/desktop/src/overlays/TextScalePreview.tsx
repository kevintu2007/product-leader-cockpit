import type { CSSProperties, ReactNode } from "react";

import { APP_TEXT_SCALE_STEPS, type AppTextScaleStep } from "./appTextScale";
import { useT } from "../i18n/useT";
import { findRoute } from "../shell/routes";

export interface TextScalePreviewProps {
  readonly value: AppTextScaleStep;
  readonly onChange: (value: AppTextScaleStep) => void;
  /** Sample content to preview at the selected scale. Falls back to a
   * representative heading/body/label sample when omitted -- callers with
   * more specific content (e.g. Settings) may supply their own. */
  readonly sampleContent?: ReactNode;
}

/**
 * O06 Text-scale preview: "Immediate semantic
 * typography/reflow preview for five app-scale steps" (DG3 Overlay and
 * Contextual Surface Contract). A shared presentation component only --
 * persisting the chosen scale as a presentation setting is Settings'
 * (S10's) job, not this component's; it only reports the change via
 * `onChange`.
 *
 * Selection uses plain buttons with `aria-pressed`, matching
 * `Navigation.tsx`'s convention, rather than a full ARIA `radiogroup` with
 * roving-tabindex arrow-key handling -- DG3 requires every step reachable
 * by keyboard in logical order, not a specific widget pattern, and a
 * simple button group satisfies that with less complexity.
 */
export function TextScalePreview({ value, onChange, sampleContent }: TextScalePreviewProps) {
  const t = useT();
  return (
    <section aria-labelledby="pmc-text-scale-title" className="pmc-text-scale-preview">
      <h3 id="pmc-text-scale-title" className="pmc-section-title">
        {t("textScale.title")}
      </h3>
      <div className="pmc-text-scale-options">
        {APP_TEXT_SCALE_STEPS.map((step) => (
          <button
            key={step}
            type="button"
            aria-pressed={step === value}
            data-active={step === value}
            className="pmc-text-scale-option"
            onClick={() => {
              onChange(step);
            }}
          >
            {step}%
          </button>
        ))}
      </div>
      <div
        className="pmc-text-scale-sample"
        style={{ "--pmc-app-scale": (value / 100).toString() } as CSSProperties}
      >
        {sampleContent ?? <DefaultTextScaleSample />}
      </div>
    </section>
  );
}

function DefaultTextScaleSample() {
  const t = useT();
  return (
    <div className="pmc-text-scale-sample-content">
      <p className="pmc-route-title">{findRoute("executive-cockpit").label}</p>
      <p className="pmc-text-scale-sample-body">{t("textScale.sampleBody")}</p>
      <span className="pmc-text-scale-sample-label">{t("textScale.sampleLabel")}</span>
    </div>
  );
}
