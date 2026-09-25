import { useEffect, useRef, useState } from "react";

import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { useT } from "../i18n/useT";
import { classificationName } from "../i18n/workLabels";
import { FocusTrapDialog } from "../overlays/FocusTrapDialog";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type {
  ChosenEvidenceFileDto,
  EvidenceFileActions,
  EvidenceFileClassification,
  EvidenceMatchDto,
} from "./evidenceFileIpc";

/** From O01: the Product a new reference is linked to right after it is
 * created. Absent from S08, which creates only (§4.6). */
export interface EvidenceFileProduct {
  readonly id: string;
  readonly label: string;
  /** Evidence already linked to this Product, as the inspector read it. */
  readonly linkedEvidenceIds: ReadonlySet<string>;
  readonly link: (
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<unknown>;
}

export interface EvidenceFromFileSheetProps {
  readonly actions: EvidenceFileActions;
  readonly product?: EvidenceFileProduct | undefined;
  /** The sheet closed and nothing was written. */
  readonly onClose: () => void;
  /** Something was written (a reference, a link, or both); the caller reads
   * again. */
  readonly onFinished: () => void;
  readonly newClientRequestId?: () => string;
}

/** Never Unclassified: that is the state this sheet exists to avoid (§4.4). */
const CHOICES: readonly EvidenceFileClassification[] = [
  "public",
  "internal",
  "confidential",
  "restricted",
];

function defaultClientRequestId(): string {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `evidence-file-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

/** A file the host holds behind `token`, with this choice's request id. */
interface Chosen {
  readonly token: string;
  readonly fileName: string;
  readonly observedAtMillis: number | null;
  readonly existing: EvidenceMatchDto | null;
  readonly sameContent: readonly EvidenceMatchDto[];
  /** One per chosen file: a retry of the create (or of a create the file
   * changed under) is the same create; a different file is a new one. */
  readonly clientRequestId: string;
  /** Set when the host observed different bytes at create: the person
   * confirms the new observation before anything is created (§4.2). */
  readonly changed: boolean;
}

type Step =
  | { readonly kind: "choose" }
  | { readonly kind: "chosen"; readonly chosen: Chosen }
  | { readonly kind: "creating"; readonly chosen: Chosen }
  | { readonly kind: "linking"; readonly evidenceId: string }
  | {
      readonly kind: "done";
      readonly how: "created" | "createdAndLinked" | "linkedExisting";
      readonly evidenceId: string;
    }
  | {
      // Created, and the link after it failed: a retry resumes the link
      // under the same request id, never a second create (§4.6).
      readonly kind: "notLinked";
      readonly evidence: EvidenceMatchDto;
      readonly linkRequestId: string;
      readonly error: ResolvedSafeError;
    };

function chosenFrom(dto: ChosenEvidenceFileDto, clientRequestId: string): Chosen | null {
  if (!dto.chosen || dto.token === null) {
    return null;
  }
  return {
    token: dto.token,
    fileName: dto.fileName ?? "",
    observedAtMillis: dto.observedAtMillis,
    existing: dto.existing,
    sameContent: dto.sameContent,
    clientRequestId,
    changed: false,
  };
}

/**
 * O01 Evidence tab / S08 → "Add Evidence from a file…" (DG3 Vault-root and
 * Evidence-from-file amendment §4, H1-User): choose a file inside the Vault,
 * see when it was observed and what already refers to it, choose its
 * classification, create — and from O01, link it to the Product.
 *
 * The file itself never reaches the webview: the host holds it behind a
 * token, observes it again at create, and pins what it reads then.
 */
export function EvidenceFromFileSheet({
  actions,
  product,
  onClose,
  onFinished,
  newClientRequestId = defaultClientRequestId,
}: EvidenceFromFileSheetProps) {
  const t = useT();
  const [step, setStep] = useState<Step>({ kind: "choose" });
  const [picking, setPicking] = useState(false);
  const [classification, setClassification] = useState<EvidenceFileClassification | undefined>(
    undefined,
  );
  const [error, setError] = useState<ResolvedSafeError | null>(null);
  // Something landed: closing then means "read again", not "nothing changed".
  const wrote = useRef(false);
  // The host holds a chosen file until it is created or the sheet lets go.
  const holding = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (holding.current) {
        holding.current = false;
        void actions.discardEvidenceFileChoice().catch(() => undefined);
      }
    };
  }, [actions]);

  const locked = picking || step.kind === "creating" || step.kind === "linking";

  const close = () => {
    if (locked) {
      return;
    }
    if (holding.current) {
      holding.current = false;
      void actions.discardEvidenceFileChoice().catch(() => undefined);
    }
    if (wrote.current) {
      onFinished();
    } else {
      onClose();
    }
  };

  const choose = () => {
    if (picking) {
      return;
    }
    setPicking(true);
    setError(null);
    actions
      .chooseEvidenceFile(t("evidenceFile.dialogTitle"))
      .then(
        (dto) => {
          const chosen = chosenFrom(dto, newClientRequestId());
          if (chosen === null) {
            return; // Cancelled: whatever was chosen before stays chosen.
          }
          holding.current = true;
          if (!mounted.current) {
            holding.current = false;
            void actions.discardEvidenceFileChoice().catch(() => undefined);
            return;
          }
          setClassification(undefined);
          setStep({ kind: "chosen", chosen });
        },
        (reason: unknown) => {
          setError(resolveRejection(reason, t));
        },
      )
      .finally(() => {
        setPicking(false);
      });
  };

  /** `back`: the chosen file to return to when linking an existing
   * reference fails; `null` when the reference was just created here. */
  const link = (evidence: EvidenceMatchDto, linkRequestId: string, back: Chosen | null) => {
    if (product === undefined) {
      return;
    }
    const created = back === null;
    setError(null);
    setStep({ kind: "linking", evidenceId: evidence.evidenceId });
    product.link(evidence.evidenceId, evidence.version, linkRequestId).then(
      () => {
        wrote.current = true;
        setStep({
          kind: "done",
          how: created ? "createdAndLinked" : "linkedExisting",
          evidenceId: evidence.evidenceId,
        });
      },
      (reason: unknown) => {
        const failure = resolveRejection(reason, t);
        if (back === null) {
          setStep({ kind: "notLinked", evidence, linkRequestId, error: failure });
        } else {
          setError(failure);
          setStep({ kind: "chosen", chosen: back });
        }
      },
    );
  };

  const create = () => {
    if (step.kind !== "chosen" || classification === undefined) {
      return;
    }
    const { chosen } = step;
    setError(null);
    setStep({ kind: "creating", chosen });
    actions.createEvidenceFromFile(chosen.token, classification, chosen.clientRequestId).then(
      (result) => {
        if (result.outcome === "created" && result.evidence !== null) {
          // The host keeps the choice (a retry after a lost reply replays
          // this create); the sheet lets it go when it closes.
          wrote.current = true;
          if (product === undefined) {
            setStep({ kind: "done", how: "created", evidenceId: result.evidence.evidenceId });
          } else {
            link(result.evidence, `${chosen.clientRequestId}-link`, null);
          }
          return;
        }
        if (result.outcome === "already_referenced" && result.evidence !== null) {
          // Another reference named this file first: offer that one.
          setStep({ kind: "chosen", chosen: { ...chosen, existing: result.evidence } });
          return;
        }
        // The file changed since it was chosen: show the new observation
        // and let the person confirm it; the same token now holds it.
        setStep({
          kind: "chosen",
          chosen: { ...chosen, observedAtMillis: result.observedAtMillis, changed: true },
        });
      },
      (reason: unknown) => {
        setError(resolveRejection(reason, t));
        setStep({ kind: "chosen", chosen });
      },
    );
  };

  const productLabel = product?.label ?? "";

  return (
    <div className="pmc-dialog-backdrop">
      <FocusTrapDialog
        titleId="pmc-evidence-file-title"
        descriptionId="pmc-evidence-file-lede"
        onEscape={close}
        className="pmc-h2b-approval pmc-evidence-file-sheet"
      >
        <h2 id="pmc-evidence-file-title" className="pmc-section-title">
          {t("evidenceFile.title")}
        </h2>

        {step.kind === "choose" && (
          <>
            <p id="pmc-evidence-file-lede" className="pmc-h2a-summary">
              {t("evidenceFile.lede")}
            </p>
            <div className="pmc-h2a-actions">
              <button
                type="button"
                className="pmc-h2a-approve"
                disabled={picking}
                aria-busy={picking}
                onClick={choose}
              >
                {t("evidenceFile.choose")}
              </button>
              <button type="button" className="pmc-h2a-reject" disabled={picking} onClick={close}>
                {t("evidenceFile.cancel")}
              </button>
            </div>
          </>
        )}

        {(step.kind === "chosen" || step.kind === "creating") && (
          <ChosenFile
            chosen={step.chosen}
            busy={step.kind === "creating" || picking}
            product={product}
            classification={classification}
            onClassification={setClassification}
            onCreate={create}
            onLinkExisting={(existing) => {
              link(existing, `${step.chosen.clientRequestId}-link`, step.chosen);
            }}
            onChooseAnother={choose}
            onCancel={close}
          />
        )}

        {step.kind === "creating" && (
          <p role="status" className="pmc-backups-progress">
            {t("evidenceFile.creating")}
          </p>
        )}

        {step.kind === "linking" && (
          <p id="pmc-evidence-file-lede" role="status" className="pmc-backups-progress">
            {t("evidenceFile.linking")}
          </p>
        )}

        {step.kind === "done" && (
          <>
            <p
              id="pmc-evidence-file-lede"
              role="status"
              className="pmc-backups-result"
              tabIndex={-1}
              ref={(node) => node?.focus()}
            >
              {step.how === "created"
                ? t("evidenceFile.created", { id: step.evidenceId })
                : step.how === "createdAndLinked"
                  ? t("evidenceFile.createdAndLinked", {
                      id: step.evidenceId,
                      product: productLabel,
                    })
                  : t("evidenceFile.linkedExisting", {
                      id: step.evidenceId,
                      product: productLabel,
                    })}
            </p>
            <div className="pmc-h2a-actions">
              <button type="button" className="pmc-h2a-approve" onClick={close}>
                {t("evidenceFile.done")}
              </button>
            </div>
          </>
        )}

        {step.kind === "notLinked" && (
          <>
            <p
              id="pmc-evidence-file-lede"
              role="alert"
              className="pmc-backups-result"
              tabIndex={-1}
              ref={(node) => node?.focus()}
            >
              {t("evidenceFile.createdNotLinked", { id: step.evidence.evidenceId })}
            </p>
            <SafeErrorDetail
              message={step.error.message}
              correlationId={step.error.correlationId}
              retryable={step.error.retryable}
              errorCode={step.error.errorCode}
            />
            <div className="pmc-h2a-actions">
              {/* Only when trying again could succeed; otherwise (the
                  Product changed, say) the Evidence tab's own link control
                  links it after the Product is read again. */}
              {step.error.retryable ? (
                <button
                  type="button"
                  className="pmc-h2a-approve"
                  onClick={() => {
                    link(step.evidence, step.linkRequestId, null);
                  }}
                >
                  {t("evidenceFile.retryLink")}
                </button>
              ) : null}
              <button type="button" className="pmc-h2a-reject" onClick={close}>
                {t("evidenceFile.done")}
              </button>
            </div>
          </>
        )}

        {error !== null && (
          <SafeErrorDetail
            message={error.message}
            correlationId={error.correlationId}
            retryable={error.retryable}
            errorCode={error.errorCode}
          />
        )}
      </FocusTrapDialog>
    </div>
  );
}

interface ChosenFileProps {
  readonly chosen: Chosen;
  readonly busy: boolean;
  readonly product: EvidenceFileProduct | undefined;
  readonly classification: EvidenceFileClassification | undefined;
  readonly onClassification: (value: EvidenceFileClassification) => void;
  readonly onCreate: () => void;
  readonly onLinkExisting: (existing: EvidenceMatchDto) => void;
  readonly onChooseAnother: () => void;
  readonly onCancel: () => void;
}

function ChosenFile({
  chosen,
  busy,
  product,
  classification,
  onClassification,
  onCreate,
  onLinkExisting,
  onChooseAnother,
  onCancel,
}: ChosenFileProps) {
  const t = useT();
  const { existing } = chosen;
  const alreadyLinked =
    existing !== null && product?.linkedEvidenceIds.has(existing.evidenceId) === true;

  return (
    <>
      <p id="pmc-evidence-file-lede" className="pmc-h2a-summary">
        {t("evidenceFile.file", { name: chosen.fileName })}
      </p>
      {chosen.changed ? (
        <p role="alert" className="pmc-h2a-summary">
          {t("evidenceFile.changed")}
        </p>
      ) : null}
      {chosen.observedAtMillis !== null ? (
        <p className="pmc-h2a-summary">
          {t("evidenceFile.observed", { time: formatReadAt(chosen.observedAtMillis) })}
        </p>
      ) : null}

      {existing !== null ? (
        <>
          <p role="status" className="pmc-h2a-summary">
            {alreadyLinked
              ? t("evidenceFile.existingLinked", { id: existing.evidenceId })
              : t("evidenceFile.existing", { id: existing.evidenceId })}
          </p>
          <div className="pmc-h2a-actions">
            {product !== undefined && !alreadyLinked ? (
              <button
                type="button"
                className="pmc-h2a-approve"
                disabled={busy}
                onClick={() => {
                  onLinkExisting(existing);
                }}
              >
                {t("evidenceFile.linkExisting")}
              </button>
            ) : null}
            <button
              type="button"
              className="pmc-h2a-reject"
              disabled={busy}
              onClick={onChooseAnother}
            >
              {t("evidenceFile.chooseAnother")}
            </button>
            <button type="button" className="pmc-h2a-reject" disabled={busy} onClick={onCancel}>
              {t("evidenceFile.cancel")}
            </button>
          </div>
        </>
      ) : (
        <>
          {chosen.sameContent.map((match) => (
            <p key={match.evidenceId} role="note" className="pmc-h2a-summary">
              {t("evidenceFile.sameContent", { id: match.evidenceId })}
            </p>
          ))}
          <fieldset className="pmc-record-field pmc-record-classification">
            <legend className="pmc-h2b-confirmation-label">
              {t("entry.field.classification")}
            </legend>
            <div className="pmc-record-choices">
              {CHOICES.map((choice) => (
                <label className="pmc-passphrase-check" key={choice}>
                  <input
                    type="radio"
                    name="pmc-evidence-file-classification"
                    value={choice}
                    checked={classification === choice}
                    disabled={busy}
                    onChange={() => {
                      onClassification(choice);
                    }}
                  />
                  <span className="pmc-classification-badge" data-classification={choice}>
                    {classificationName(t, choice)}
                  </span>
                </label>
              ))}
            </div>
            {classification === undefined ? (
              <p className="pmc-record-count">{t("entry.classification.none")}</p>
            ) : null}
          </fieldset>
          <div className="pmc-h2a-actions">
            <button
              type="button"
              className="pmc-h2a-approve"
              disabled={busy || classification === undefined}
              aria-busy={busy}
              onClick={onCreate}
            >
              {product === undefined ? t("evidenceFile.create") : t("evidenceFile.createAndLink")}
            </button>
            <button
              type="button"
              className="pmc-h2a-reject"
              disabled={busy}
              onClick={onChooseAnother}
            >
              {t("evidenceFile.chooseAnother")}
            </button>
            <button type="button" className="pmc-h2a-reject" disabled={busy} onClick={onCancel}>
              {t("evidenceFile.cancel")}
            </button>
          </div>
        </>
      )}
    </>
  );
}
