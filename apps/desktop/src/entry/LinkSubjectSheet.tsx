import { useId, useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { useT } from "../i18n/useT";
import { classificationName } from "../i18n/workLabels";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { combineClassification } from "./classificationChoice";
import type { EntryActions, EntryOutcomeDto } from "./entryIpc";
import { linkCandidates, type LinkCandidate } from "./entrySheets";
import { isSubjectKind, SUBJECT_KINDS, type Purpose, type SubjectKind } from "./subjectKinds";

export interface LinkSubjectSheetProps {
  /** The Stakeholder as the host holds it: its own classification, not the
   * folded one the directory shows, so the link's classification can be
   * shown as it will be recorded. */
  readonly stakeholder: {
    readonly id: string;
    readonly label: string;
    readonly version: number;
    readonly classification: string;
  };
  readonly actions: Pick<EntryActions, "listEntryRecords" | "linkStakeholderSubject">;
  readonly clientRequestId: string;
  readonly onDone: (outcome: EntryOutcomeDto) => void;
  readonly onClose: () => void;
  /** The Stakeholder or the subject moved past the version read (§3.8). */
  readonly onStale: () => void;
}

type Candidates =
  | { readonly status: "idle" }
  | { readonly status: "loading"; readonly kind: SubjectKind }
  | {
      readonly status: "ready";
      readonly kind: SubjectKind;
      readonly candidates: readonly LinkCandidate[];
    }
  | { readonly status: "failed"; readonly kind: SubjectKind; readonly error: ResolvedSafeError };

type Phase =
  | { readonly status: "editing" }
  | { readonly status: "sending" }
  | { readonly status: "failed"; readonly error: ResolvedSafeError };

/**
 * S09: relate a Stakeholder to a subject (DG3 record-entry amendment §3,
 * slice 6D). Unlike the generic sheet, the record list depends on the kind
 * chosen first, so this sheet loads its candidates when the kind changes.
 * The purpose is chosen, never defaulted, like a classification. The host
 * folds the classification of both ends; nothing here computes it.
 */
export function LinkSubjectSheet({
  stakeholder,
  actions,
  clientRequestId,
  onDone,
  onClose,
  onStale,
}: LinkSubjectSheetProps) {
  const t = useT();
  const baseId = useId();
  const [kind, setKind] = useState<SubjectKind | "">("");
  const [subjectId, setSubjectId] = useState("");
  const [purpose, setPurpose] = useState<Purpose | "">("");
  const [candidates, setCandidates] = useState<Candidates>({ status: "idle" });
  const [phase, setPhase] = useState<Phase>({ status: "editing" });
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  // Only the latest kind's listing may land; an earlier one still on its
  // way is discarded rather than shown under the wrong kind.
  const listing = useRef(0);

  const chooseKind = (next: SubjectKind | "") => {
    setKind(next);
    setSubjectId("");
    setConfirmingDiscard(false);
    const sequence = listing.current + 1;
    listing.current = sequence;
    if (next === "") {
      setCandidates({ status: "idle" });
      return;
    }
    setCandidates({ status: "loading", kind: next });
    actions.listEntryRecords(next).then(
      (list) => {
        if (listing.current === sequence) {
          setCandidates({
            status: "ready",
            kind: next,
            candidates: linkCandidates(
              list.records.filter((record) => record.kind === next),
              new Set(),
            ),
          });
        }
      },
      (reason: unknown) => {
        if (listing.current === sequence) {
          setCandidates({ status: "failed", kind: next, error: resolveRejection(reason, t) });
        }
      },
    );
  };

  const busy = phase.status === "sending";
  const ready = candidates.status === "ready" ? candidates.candidates : [];
  const chosen = ready.find((candidate) => candidate.id === subjectId);
  const dirty = kind !== "" || purpose !== "";
  const canSubmit = !busy && kind !== "" && chosen !== undefined && purpose !== "";
  const stale = phase.status === "failed" && phase.error.errorCode === "DOMAIN_CONFLICT";

  const send = () => {
    if (busy || kind === "" || chosen === undefined || purpose === "") {
      return;
    }
    setPhase({ status: "sending" });
    actions
      .linkStakeholderSubject(
        stakeholder.id,
        stakeholder.version,
        kind,
        chosen.id,
        chosen.version,
        purpose,
        clientRequestId,
      )
      .then(
        (outcome) => {
          onDone(outcome);
        },
        (reason: unknown) => {
          setPhase({ status: "failed", error: resolveRejection(reason, t) });
        },
      );
  };

  const close = () => {
    if (busy) {
      return;
    }
    if (dirty && !confirmingDiscard) {
      setConfirmingDiscard(true);
      return;
    }
    onClose();
  };

  const titleId = `${baseId}-title`;
  const kindId = `${baseId}-kind`;
  const subjectFieldId = `${baseId}-subject`;

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId={titleId}
        onEscape={close}
        className="pmc-h2b-approval pmc-record-sheet"
      >
        <h2 id={titleId} className="pmc-section-title">
          {t("entry.title.link.subject", { stakeholder: stakeholder.label })}
        </h2>
        <form
          className="pmc-record-fields"
          onSubmit={(event) => {
            event.preventDefault();
            send();
          }}
        >
          <fieldset className="pmc-record-field pmc-record-classification">
            <legend className="pmc-h2b-confirmation-label">{t("entry.field.purpose")}</legend>
            <div className="pmc-record-choices">
              {(["responsibility", "dependency"] as const).map((option) => (
                <label className="pmc-passphrase-check" key={option}>
                  <input
                    type="radio"
                    name={`${baseId}-purpose`}
                    value={option}
                    checked={purpose === option}
                    disabled={busy}
                    onChange={() => {
                      setPurpose(option);
                      setConfirmingDiscard(false);
                    }}
                  />
                  {t(`entry.purpose.${option}`)}
                </label>
              ))}
            </div>
            {purpose === "" ? <p className="pmc-record-count">{t("entry.purpose.none")}</p> : null}
          </fieldset>

          <div className="pmc-record-field">
            <label className="pmc-h2b-confirmation-label" htmlFor={kindId}>
              {t("entry.field.subjectKind")}
            </label>
            <select
              id={kindId}
              className="pmc-h2b-confirmation-input"
              value={kind}
              disabled={busy}
              onChange={(event) => {
                const next = event.target.value;
                chooseKind(isSubjectKind(next) ? next : "");
              }}
            >
              <option value="">{t("entry.subjectKind.none")}</option>
              {SUBJECT_KINDS.map((option) => (
                <option key={option} value={option}>
                  {t(`entry.kind.${option}`)}
                </option>
              ))}
            </select>
          </div>

          <div className="pmc-record-field">
            <label className="pmc-h2b-confirmation-label" htmlFor={subjectFieldId}>
              {t("entry.field.record")}
            </label>
            <select
              id={subjectFieldId}
              className="pmc-h2b-confirmation-input"
              value={subjectId}
              disabled={busy || candidates.status !== "ready" || ready.length === 0}
              onChange={(event) => {
                setSubjectId(event.target.value);
                setConfirmingDiscard(false);
              }}
            >
              <option value="">
                {candidates.status === "ready" && ready.length === 0
                  ? t("entry.candidates.none")
                  : ""}
              </option>
              {ready.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {t("entry.candidate", {
                    name: candidate.name,
                    classification: classificationName(t, candidate.classification),
                    version: String(candidate.version),
                  })}
                </option>
              ))}
            </select>
            {candidates.status === "loading" ? (
              <p className="pmc-record-count" role="status">
                {t("entry.reading")}
              </p>
            ) : null}
            {candidates.status === "failed" ? (
              <SafeErrorDetail
                message={t("entry.readFailed", { message: candidates.error.message })}
                correlationId={candidates.error.correlationId}
                retryable={candidates.error.retryable}
              />
            ) : null}
          </div>

          {chosen !== undefined ? (
            <p className="pmc-record-preview" role="status">
              {t("entry.link.classification", {
                classification: classificationName(
                  t,
                  combineClassification(stakeholder.classification, chosen.classification),
                ),
              })}
            </p>
          ) : null}

          {phase.status === "failed" ? (
            stale ? (
              <p role="alert" className="pmc-h2b-irreversible-statement">
                {t("entry.conflict")}
              </p>
            ) : (
              <SafeErrorDetail
                message={t("entry.failed", { message: phase.error.message })}
                correlationId={phase.error.correlationId}
                errorCode={phase.error.errorCode}
                retryable={phase.error.retryable}
              />
            )
          ) : null}

          {confirmingDiscard ? (
            <div className="pmc-record-discard" role="group" aria-label={t("entry.dirty.question")}>
              <p className="pmc-h2b-irreversible-statement">{t("entry.dirty.question")}</p>
              <div className="pmc-h2a-buttons">
                <button type="button" className="pmc-h2a-reject" onClick={onClose}>
                  {t("entry.dirty.discard")}
                </button>
                <button
                  type="button"
                  className="pmc-h2a-approve"
                  onClick={() => {
                    setConfirmingDiscard(false);
                  }}
                >
                  {t("entry.dirty.keep")}
                </button>
              </div>
            </div>
          ) : null}

          <div className="pmc-h2a-actions">
            <p className="pmc-h2a-decision-status" role="status">
              {busy ? t("entry.saving") : ""}
            </p>
            <div className="pmc-h2a-buttons">
              {stale ? (
                <button type="button" className="pmc-h2a-approve" onClick={onStale}>
                  {t("entry.conflict.reload")}
                </button>
              ) : (
                <button
                  type="submit"
                  className="pmc-h2a-approve"
                  disabled={!canSubmit}
                  aria-busy={busy}
                >
                  {t("entry.linkConfirm")}
                </button>
              )}
              <button type="button" className="pmc-h2a-reject" disabled={busy} onClick={close}>
                {t("entry.cancel")}
              </button>
            </div>
          </div>
        </form>
      </FocusTrapDialog>
    </div>
  );
}
