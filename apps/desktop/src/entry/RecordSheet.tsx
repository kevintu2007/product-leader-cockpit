import { useId, useState, type ReactNode } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { useT } from "../i18n/useT";
import { classificationName } from "../i18n/workLabels";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import { classificationChoices } from "./classificationChoice";
import { isDirty, type FieldValues } from "./dirtyState";
import type { EntryOutcomeDto } from "./entryIpc";
import { localToUtcMillis } from "./localDateTime";
import { TEXT_LIMITS, textProblem, utf8ByteLength } from "./utf8Bytes";

/**
 * One field of a record sheet. `short` and `long` are the domain's two text
 * bounds (§3.4); `classification` is chosen, never defaulted (§3.3), and may
 * offer "take it from the parent" as a real choice; `datetime` is entered in
 * the named zone and sent as the UTC instant (§3.5); `choice` picks one
 * existing record by id.
 */
export type FieldSpec =
  | {
      readonly kind: "short" | "long";
      readonly name: string;
      readonly label: string;
      readonly required: boolean;
      /** The domain's bound for this field when it is not `ShortText` /
       * `LongText` (the Delivery family's names are 200 bytes, its details
       * 4000). */
      readonly limit?: number;
    }
  | {
      readonly kind: "classification";
      readonly name: string;
      /** When set, one more option maps to `null`: the parent's classification. */
      readonly inheritLabel?: string;
      /** `work` leaves Unclassified out (Risk, Issue, the requests). */
      readonly choices?: "general" | "work";
    }
  | {
      readonly kind: "datetime";
      readonly name: string;
      readonly label: string;
      readonly timeZone: string;
      /** Optional when `false`: empty is sent as `null`. */
      readonly required?: boolean;
    }
  | {
      readonly kind: "choice";
      readonly name: string;
      readonly label: string;
      readonly options: readonly { readonly value: string; readonly label: string }[];
      readonly emptyLabel: string;
      /** Optional when `false`: nothing chosen is sent as the empty string. */
      readonly required?: boolean;
    };

/** What a sheet submits: trimmed text, a classification or `null`, an
 * instant in milliseconds, or a chosen id. */
export type SubmittedValues = Readonly<Record<string, string | number | null>>;

export interface RecordSheetProps {
  readonly title: string;
  readonly fields: readonly FieldSpec[];
  /** Text as the record holds it; a classification as its persisted word,
   * or absent for a create, so nothing is chosen until the person chooses. */
  readonly initial: FieldValues;
  readonly submitLabel: string;
  readonly submit: (values: SubmittedValues) => Promise<EntryOutcomeDto>;
  readonly onDone: (outcome: EntryOutcomeDto) => void;
  /** Closed without saving; a dirty sheet asked first (§3.9). */
  readonly onClose: () => void;
  /** The record moved past the version this sheet read (§3.8): the opener
   * re-reads it and opens a fresh sheet. */
  readonly onStale?: () => void;
  /** A rule across fields (a period's end before its start), checked once
   * every field passes on its own; the sentence to show, or `null`. */
  readonly validate?: (values: SubmittedValues) => string | null;
  /** What the submit will record beyond the fields (a link's classification,
   * §3.3), shown once every field passes; `null` while there is nothing to
   * say. */
  readonly preview?: (values: SubmittedValues) => string | null;
}

type Phase =
  | { readonly status: "editing" }
  | { readonly status: "confirmingDiscard" }
  | { readonly status: "sending" }
  | { readonly status: "failed"; readonly error: ResolvedSafeError };

/** The radio value for "the parent's classification"; never sent as such. */
const INHERIT = "__inherit__";

function textLimit(field: { readonly kind: "short" | "long"; readonly limit?: number }): number {
  return field.limit ?? (field.kind === "short" ? TEXT_LIMITS.short : TEXT_LIMITS.long);
}

function initialText(initial: FieldValues, name: string): string {
  const value = initial[name];
  return typeof value === "string" ? value : "";
}

/**
 * The generic record sheet (DG3 record-entry amendment §3): a focus-trapped
 * dialog with the record's fields, byte counters that count what the host
 * counts, a classification with nothing chosen until the person chooses, and
 * a close that asks when the sheet is dirty. The opener holds the request id
 * for the sheet's lifetime and reuses it on a retry. The host's refusal stays
 * the authority; this sheet refuses only what it can already see is wrong,
 * so an obvious mistake is caught before a round trip.
 */
export function RecordSheet({
  title,
  fields,
  initial,
  submitLabel,
  submit,
  onDone,
  onClose,
  onStale,
  validate,
  preview,
}: RecordSheetProps) {
  const t = useT();
  const baseId = useId();
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(fields.map((field) => [field.name, initialText(initial, field.name)])),
  );
  const [phase, setPhase] = useState<Phase>({ status: "editing" });

  const dirty = isDirty(
    Object.fromEntries(fields.map((field) => [field.name, initialText(initial, field.name)])),
    values,
  );
  const busy = phase.status === "sending";

  /** Why a field cannot be submitted yet, or `null` when it can. */
  function problem(field: FieldSpec): string | null {
    const value = values[field.name] ?? "";
    switch (field.kind) {
      case "short":
      case "long": {
        const limit = textLimit(field);
        switch (textProblem(value, limit)) {
          case "empty":
            return field.required ? t("entry.required") : null;
          case "tooLong":
            return t("entry.bytesOver", { used: utf8ByteLength(value), limit });
          case "control":
            return t("entry.control");
          case null:
            return null;
        }
      }
      // eslint-disable-next-line no-fallthrough -- every branch of the inner switch returns.
      case "classification":
        return value === "" ? t("entry.classification.none") : null;
      case "datetime":
        if (value === "") {
          return field.required === false ? null : t("entry.required");
        }
        return localToUtcMillis(value, field.timeZone) === undefined
          ? t("entry.datetime.invalid", { zone: field.timeZone })
          : null;
      case "choice":
        return value === "" && field.required !== false ? t("entry.required") : null;
    }
  }

  const problems = new Map(fields.map((field) => [field.name, problem(field)]));
  const fieldsPass = [...problems.values()].every((why) => why === null);
  const crossFieldProblem = fieldsPass && validate !== undefined ? validate(submitted()) : null;
  const canSubmit = !busy && fieldsPass && crossFieldProblem === null;
  const previewLine =
    fieldsPass && crossFieldProblem === null && preview !== undefined ? preview(submitted()) : null;

  function submitted(): SubmittedValues {
    const out: Record<string, string | number | null> = {};
    for (const field of fields) {
      const value = values[field.name] ?? "";
      switch (field.kind) {
        case "short":
        case "long":
          out[field.name] = value.trim();
          break;
        case "classification":
          out[field.name] = value === INHERIT ? null : value;
          break;
        case "datetime":
          // `problem` passed just before, so an entered value is exactly one
          // instant; an optional field left empty is `null`.
          out[field.name] = value === "" ? null : (localToUtcMillis(value, field.timeZone) ?? 0);
          break;
        case "choice":
          out[field.name] = value;
          break;
      }
    }
    return out;
  }

  const send = () => {
    if (!canSubmit) {
      return;
    }
    setPhase({ status: "sending" });
    submit(submitted()).then(
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
    if (dirty && phase.status !== "confirmingDiscard") {
      setPhase({ status: "confirmingDiscard" });
      return;
    }
    onClose();
  };

  function renderField(field: FieldSpec): ReactNode {
    const fieldId = `${baseId}-${field.name}`;
    const value = values[field.name] ?? "";
    const why = problems.get(field.name) ?? null;
    const set = (next: string) => {
      setValues((previous) => ({ ...previous, [field.name]: next }));
      if (phase.status === "failed" || phase.status === "confirmingDiscard") {
        setPhase({ status: "editing" });
      }
    };
    switch (field.kind) {
      case "short":
      case "long": {
        const limit = textLimit(field);
        const used = utf8ByteLength(value);
        const over = used > limit;
        // The count is always shown; a reason is shown once the person has
        // typed something, never on an untouched field.
        const note = !over && why !== null && value !== "" ? why : null;
        return (
          <div className="pmc-record-field" key={field.name}>
            <label className="pmc-h2b-confirmation-label" htmlFor={fieldId}>
              {field.label}
            </label>
            {field.kind === "short" ? (
              <input
                id={fieldId}
                className="pmc-h2b-confirmation-input"
                type="text"
                autoComplete="off"
                value={value}
                disabled={busy}
                aria-describedby={`${fieldId}-count`}
                onChange={(event) => {
                  set(event.target.value);
                }}
              />
            ) : (
              <textarea
                id={fieldId}
                className="pmc-h2b-confirmation-input pmc-record-textarea"
                rows={4}
                value={value}
                disabled={busy}
                aria-describedby={`${fieldId}-count`}
                onChange={(event) => {
                  set(event.target.value);
                }}
              />
            )}
            <p
              id={`${fieldId}-count`}
              className="pmc-record-count"
              data-over={over || note !== null ? "true" : undefined}
            >
              {over ? t("entry.bytesOver", { used, limit }) : t("entry.bytes", { used, limit })}
              {note !== null ? ` ${note}` : null}
            </p>
          </div>
        );
      }
      case "classification": {
        const choices = classificationChoices(field.choices ?? "general");
        return (
          <fieldset className="pmc-record-field pmc-record-classification" key={field.name}>
            <legend className="pmc-h2b-confirmation-label">
              {t("entry.field.classification")}
            </legend>
            <div className="pmc-record-choices">
              {field.inheritLabel !== undefined ? (
                <label className="pmc-passphrase-check">
                  <input
                    type="radio"
                    name={fieldId}
                    value={INHERIT}
                    checked={value === INHERIT}
                    disabled={busy}
                    onChange={() => {
                      set(INHERIT);
                    }}
                  />
                  {field.inheritLabel}
                </label>
              ) : null}
              {choices.map((choice) => (
                <label className="pmc-passphrase-check" key={choice}>
                  <input
                    type="radio"
                    name={fieldId}
                    value={choice}
                    checked={value === choice}
                    disabled={busy}
                    onChange={() => {
                      set(choice);
                    }}
                  />
                  <span className="pmc-classification-badge" data-classification={choice}>
                    {classificationName(t, choice)}
                  </span>
                </label>
              ))}
            </div>
            {value === "" ? (
              <p className="pmc-record-count">{t("entry.classification.none")}</p>
            ) : null}
          </fieldset>
        );
      }
      case "datetime":
        return (
          <div className="pmc-record-field" key={field.name}>
            <label className="pmc-h2b-confirmation-label" htmlFor={fieldId}>
              {field.label}
            </label>
            <input
              id={fieldId}
              className="pmc-h2b-confirmation-input"
              type="datetime-local"
              value={value}
              disabled={busy}
              aria-describedby={`${fieldId}-zone`}
              onChange={(event) => {
                set(event.target.value);
              }}
            />
            <p
              id={`${fieldId}-zone`}
              className="pmc-record-count"
              data-over={why !== null && value !== "" ? "true" : undefined}
            >
              {t("entry.zone", { zone: field.timeZone })}
              {why !== null && value !== "" ? ` ${why}` : null}
            </p>
          </div>
        );
      case "choice":
        return (
          <div className="pmc-record-field" key={field.name}>
            <label className="pmc-h2b-confirmation-label" htmlFor={fieldId}>
              {field.label}
            </label>
            <select
              id={fieldId}
              className="pmc-h2b-confirmation-input"
              value={value}
              disabled={busy || field.options.length === 0}
              onChange={(event) => {
                set(event.target.value);
              }}
            >
              <option value="">{field.emptyLabel}</option>
              {field.options.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>
        );
    }
  }

  const stale = phase.status === "failed" && phase.error.errorCode === "DOMAIN_CONFLICT";
  // Not `-title`: a field may be named `title`, and the ids must not collide.
  const titleId = `${baseId}-sheet-title`;

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId={titleId}
        onEscape={close}
        className="pmc-h2b-approval pmc-record-sheet"
      >
        <h2 id={titleId} className="pmc-section-title">
          {title}
        </h2>
        <form
          className="pmc-record-fields"
          onSubmit={(event) => {
            event.preventDefault();
            send();
          }}
        >
          {fields.map(renderField)}

          {crossFieldProblem !== null ? (
            <p className="pmc-record-count" data-over="true" role="status">
              {crossFieldProblem}
            </p>
          ) : null}
          {previewLine !== null ? (
            <p className="pmc-record-preview" role="status">
              {previewLine}
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

          {phase.status === "confirmingDiscard" ? (
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
                    setPhase({ status: "editing" });
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
              {stale && onStale !== undefined ? (
                <button type="button" className="pmc-h2a-approve" onClick={onStale}>
                  {t("entry.conflict.reload")}
                </button>
              ) : (
                <button
                  type="submit"
                  className="pmc-h2a-approve"
                  disabled={!canSubmit || stale}
                  aria-busy={busy}
                >
                  {submitLabel}
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
