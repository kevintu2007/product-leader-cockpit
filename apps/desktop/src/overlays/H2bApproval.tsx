import { useState } from "react";

import { ClassificationBadge, type DataClassification } from "./ClassificationBadge";
import { FocusTrapDialog } from "./FocusTrapDialog";
import type { H2aPreviewField } from "./H2aFocusedReview";
import { useT } from "../i18n/useT";

export interface H2bLocalRecoveryEvidence {
  readonly kind: "local-recovery";
  /** Already-formatted for display (e.g. "2026-08-24 09:12 Asia/Taipei"). */
  readonly verifiedAt: string;
  readonly scope: string;
  readonly compatibility: string;
}

export interface H2bExternalEgressEvidence {
  readonly kind: "external-egress";
  readonly provider: string;
  readonly account: string;
  readonly purpose: string;
  /** Exact payload summary -- what will actually be sent, not a vague
   * description. */
  readonly exactPayloadSummary: string;
}

export type H2bEvidence = H2bLocalRecoveryEvidence | H2bExternalEgressEvidence;

export interface H2bApprovalProps {
  readonly title: string;
  readonly summary: string;
  readonly classification: DataClassification;
  readonly fields: readonly H2aPreviewField[];
  readonly evidence: H2bEvidence;
  /** The exact phrase the user must type to enable Approve -- DG0's "named
   * confirmation". Matching is exact (case-sensitive): a typed phrase is
   * reserved for high-impact irreversible operations, not routine
   * friction, so it should not silently accept a near match. */
  readonly confirmationPhrase: string;
  readonly onApprove: () => void;
  readonly onReject: () => void;
}

/**
 * O04 H2b approval route/sheet: "Exact preview,
 * named confirmation, verified recovery or egress-specific evidence,
 * approve/reject" (DG3 Overlay and Contextual Surface Contract). Extends
 * O03's shape with exactly the two things DG0 section 6.5 says H2b adds
 * over H2a: typed confirmation, and either local-data verified recovery
 * evidence (verification time, scope, compatibility) or, for external
 * egress, provider/account/purpose/exact-payload plus a fixed
 * irreversible-transmission statement this component itself renders --
 * every egress operation must show the identical wording, so it is not
 * left to each caller to phrase.
 *
 * Approve stays disabled until the typed confirmation exactly matches
 * `confirmationPhrase`; like O03, neither action can fire twice once a
 * decision is made.
 */
export function H2bApproval({
  title,
  summary,
  classification,
  fields,
  evidence,
  confirmationPhrase,
  onApprove,
  onReject,
}: H2bApprovalProps) {
  const t = useT();
  const [decision, setDecision] = useState<"approved" | "rejected" | null>(null);
  const [confirmationInput, setConfirmationInput] = useState("");
  const confirmed = confirmationInput === confirmationPhrase;

  function handleApprove() {
    if (decision || !confirmed) {
      return;
    }
    setDecision("approved");
    onApprove();
  }

  function handleReject() {
    if (decision) {
      return;
    }
    setDecision("rejected");
    onReject();
  }

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId="pmc-h2b-title"
        descriptionId="pmc-h2b-summary"
        onEscape={handleReject}
        className="pmc-h2b-approval"
      >
        <h2 id="pmc-h2b-title" className="pmc-section-title">
          {title}
        </h2>
        <p id="pmc-h2b-summary" className="pmc-h2a-summary">
          {summary}
        </p>
        <ClassificationBadge classification={classification} />
        <dl className="pmc-h2a-fields">
          {fields.map((field) => (
            <div key={field.label} className="pmc-h2a-field">
              <dt>{field.label}</dt>
              <dd>{field.value}</dd>
            </div>
          ))}
        </dl>
        <H2bEvidencePanel evidence={evidence} />
        <label className="pmc-h2b-confirmation-label" htmlFor="pmc-h2b-confirmation-input">
          {t("h2b.confirmPrompt", { phrase: confirmationPhrase })}
        </label>
        <input
          id="pmc-h2b-confirmation-input"
          type="text"
          className="pmc-h2b-confirmation-input"
          value={confirmationInput}
          onChange={(event) => {
            setConfirmationInput(event.target.value);
          }}
          disabled={decision !== null}
          autoComplete="off"
        />
        <div className="pmc-h2a-actions">
          <button
            type="button"
            className="pmc-h2a-approve"
            disabled={decision !== null || !confirmed}
            onClick={handleApprove}
          >
            {t("h2b.approve")}
          </button>
          <button
            type="button"
            className="pmc-h2a-reject"
            disabled={decision !== null}
            onClick={handleReject}
          >
            {t("h2b.reject")}
          </button>
        </div>
        <p role="status" className="pmc-h2a-decision-status">
          {decision === "approved" ? t("h2b.approved") : null}
          {decision === "rejected" ? t("h2b.rejected") : null}
        </p>
      </FocusTrapDialog>
    </div>
  );
}

function H2bEvidencePanel({ evidence }: { evidence: H2bEvidence }) {
  const t = useT();
  if (evidence.kind === "local-recovery") {
    return (
      <dl className="pmc-h2b-evidence" aria-label={t("h2b.recoveryEvidence")}>
        <div>
          <dt>{t("h2b.verifiedAt")}</dt>
          <dd>{evidence.verifiedAt}</dd>
        </div>
        <div>
          <dt>{t("h2b.scope")}</dt>
          <dd>{evidence.scope}</dd>
        </div>
        <div>
          <dt>{t("h2b.compatibility")}</dt>
          <dd>{evidence.compatibility}</dd>
        </div>
      </dl>
    );
  }
  return (
    <div className="pmc-h2b-evidence" aria-label={t("h2b.transferDetails")}>
      <dl>
        <div>
          <dt>{t("h2b.provider")}</dt>
          <dd>{evidence.provider}</dd>
        </div>
        <div>
          <dt>{t("h2b.account")}</dt>
          <dd>{evidence.account}</dd>
        </div>
        <div>
          <dt>{t("h2b.purpose")}</dt>
          <dd>{evidence.purpose}</dd>
        </div>
        <div>
          <dt>{t("h2b.exactPayload")}</dt>
          <dd>{evidence.exactPayloadSummary}</dd>
        </div>
      </dl>
      <p className="pmc-h2b-irreversible-statement">{t("h2b.irreversible")}</p>
    </div>
  );
}
