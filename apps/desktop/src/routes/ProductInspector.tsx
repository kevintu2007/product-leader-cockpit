import { useCallback, useEffect, useRef, useState } from "react";

import { Slotted } from "../i18n/Slotted";
import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import {
  classificationName,
  intentLabel,
  ownerLabel,
  reasonLabel,
  stateLabel,
  verificationLabel,
} from "../i18n/workLabels";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import type { Translator } from "../i18n/messages";
import type { EntryActions, EntryOutcomeDto, EntryRecordDto } from "../entry/entryIpc";
import {
  initialValues,
  initiativeFieldSpecs,
  initiativeValues,
  kindWord,
  kpiFieldSpecs,
  kpiValues,
  linkCandidates,
  linkClassificationPreview,
  linkFieldSpec,
  milestoneFieldSpecs,
  milestoneValues,
  observationFieldSpecs,
  observationValues,
  projectFieldSpecs,
  projectPeriodProblem,
  projectValues,
  simpleFieldSpecs,
  simpleValues,
  type LinkCandidate,
} from "../entry/entrySheets";
import { RecordSheet, type FieldSpec, type SubmittedValues } from "../entry/RecordSheet";
import { EvidenceFromFileSheet } from "../evidence/EvidenceFromFileSheet";
import type { EvidenceFileActions } from "../evidence/evidenceFileIpc";
import type {
  EvidenceLinkOutcomeDto,
  EvidenceReferencesDto,
  EvidenceWriteOutcomeDto,
  HealthReasonDto,
  ProductDetailDto,
  VaultStatusDto,
} from "./cockpitContract";

/**
 * O01 Product-health inspector, and the S02 detail half, per the
 * DG3 O01 amendment of 2026-09-07.
 *
 * Three things this surface is careful about, each because getting it wrong
 * would state something the Ledger does not hold:
 *
 * **Work is never the Product's.** The domain anchors work to people. Every
 * work item here appears under the person who carries it, beneath a heading
 * that says so, and there is no list of "this Product's Actions" -- the DTO
 * has no field for one. A reader who sees an overdue Action sees whose it is.
 *
 * **`影響` is a judgment, not a computation.** No source exists from which an
 * impact could be derived for a Product. Until a person records one, the
 * surface says "尚未有人評估" rather than filling the slot with severity,
 * rank, or anything else that would look like an assessment.
 *
 * **Classification is shown with its reason.** The whole inspector is
 * presented at the most restrictive classification of anything it exposes.
 * When that is above the Product's own, the child that forced it is named,
 * so the reader is not left with a label and no way to tell why.
 *
 * `發生` lists current conditions only. Nothing here reads history, so no
 * line claims that anything worsened, slipped, or changed.
 */
/**
 * The Evidence writes this surface may reach, approved by the
 * product owner on 2026-09-10 as a DG3 O01 amendment in the form of the
 * 2026-09-07 one.
 *
 * Both are H1-User: one explicit click whose whole effect is stated before
 * it runs, with no H2a review sheet. Neither takes a path -- the host uses
 * the path the Evidence record already stores -- so the boundary rule the
 * write path established holds here without an exception.
 *
 * Absent means this surface offers no Evidence writes at all, which is what
 * a test adapter or a read-only context gets.
 */
export interface EvidenceActions {
  readonly pinEvidenceFingerprint: (
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<EvidenceWriteOutcomeDto>;
  readonly reobserveEvidenceVerification: (
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<EvidenceWriteOutcomeDto>;
  /**
   * H0 reads the Evidence tab needs before offering anything: whether the
   * Vault can serve the two filesystem actions, and which references exist
   * so a person can pick one to link. Both read-only, both path-free.
   */
  readonly loadVaultStatus: () => Promise<VaultStatusDto>;
  readonly loadEvidenceReferences: () => Promise<EvidenceReferencesDto>;
  /** Ledger-only; stays available in Degraded Mode. */
  readonly linkEvidenceToProduct: (
    productId: string,
    evidenceId: string,
    expectedVersion: number,
    clientRequestId: string,
    /** The Product version the view read; the host refuses a Product that
     * changed since. Sent by "Add Evidence from a file…". */
    expectedProductVersion?: number,
  ) => Promise<EvidenceLinkOutcomeDto>;
  /**
   * "Add Evidence from a file…" (DG3 Vault-root and Evidence-from-file
   * amendment §4): create a reference for a file in the Vault and link it
   * to this Product. Absent means the tab does not offer it.
   */
  readonly evidenceFile?: EvidenceFileActions;
}

/** The link control's own state; separate from per-entry writes. */
type LinkPhase =
  | { readonly status: "idle" }
  | { readonly status: "loading" }
  | {
      readonly status: "choosing";
      readonly candidates: EvidenceReferencesDto["evidenceReferences"];
      readonly chosen: string;
    }
  | {
      readonly status: "sending";
      readonly clientRequestId: string;
      readonly evidenceId: string;
      readonly version: number;
    }
  | { readonly status: "done"; readonly evidenceId: string; readonly classificationAtLink: string }
  | {
      readonly status: "failed";
      readonly error: ResolvedSafeError;
      readonly clientRequestId: string;
      readonly evidenceId: string;
      readonly version: number;
    };

export interface ProductInspectorProps {
  readonly productId: string;
  readonly load: (productId: string) => Promise<ProductDetailDto>;
  readonly actions?: EvidenceActions;
  readonly newClientRequestId?: () => string;
  /**
   * The record-entry sheets this inspector offers (DG3 record-entry
   * amendment, slice 6B): edit the Product; create, edit and link a Roadmap
   * or KPI on the Structure tab; record a KPI observation. Absent means the
   * inspector reads only.
   */
  readonly entryActions?: EntryActions;
  /** The workspace's configured zone dates are entered in (§3.5). The
   * browser's only while the app has not read the setting. */
  readonly timeZone?: string;
  /** Called after a write lands, so the page around the inspector can re-read
   * what the write changed. The inspector re-reads its own detail either way. */
  readonly onLedgerChanged?: () => void;
}

/** The one entry sheet open at a time, with its request id. */
type EntrySheet =
  | {
      readonly kind: "editProduct";
      readonly record: EntryRecordDto;
      readonly clientRequestId: string;
    }
  | { readonly kind: "createRoadmap"; readonly clientRequestId: string }
  | { readonly kind: "createKpi"; readonly clientRequestId: string }
  | {
      readonly kind: "edit";
      readonly record: EntryRecordDto;
      readonly clientRequestId: string;
    }
  | {
      readonly kind: "observe";
      readonly kpiId: string;
      readonly kpiVersion: number;
      readonly kpiLabel: string;
      readonly clientRequestId: string;
    }
  | {
      readonly kind: "link";
      readonly target: "roadmap" | "kpi_definition" | "project";
      readonly candidates: readonly LinkCandidate[];
      /** The Product's own classification, as the host holds it (the
       * inspector's is the folded one), for what the link will record. */
      readonly productClassification: string;
      readonly clientRequestId: string;
    }
  // The Delivery family (slice 6C). A Project is created and linked to this
  // Product; a Milestone belongs to the Project it is entered under; an
  // Initiative is created and linked to that Project.
  | { readonly kind: "createProject"; readonly clientRequestId: string }
  | {
      readonly kind: "createMilestone";
      readonly projectId: string;
      readonly projectVersion: number;
      readonly projectLabel: string;
      readonly clientRequestId: string;
    }
  | {
      readonly kind: "createInitiative";
      readonly projectId: string;
      readonly projectVersion: number;
      readonly projectLabel: string;
      readonly clientRequestId: string;
    }
  | {
      readonly kind: "linkInitiative";
      readonly projectId: string;
      readonly projectVersion: number;
      readonly projectLabel: string;
      readonly projectClassification: string;
      readonly candidates: readonly LinkCandidate[];
      readonly clientRequestId: string;
    };

/** The kinds an edit sheet opens on the record as the host holds it. */
type EditableKind =
  "product" | "roadmap" | "kpi_definition" | "initiative" | "project" | "milestone";

function browserTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** The structure entries that open an edit sheet from this surface. */
function isEditableKind(kind: string): kind is Exclude<EditableKind, "product"> {
  return (
    kind === "roadmap" ||
    kind === "kpi_definition" ||
    kind === "initiative" ||
    kind === "project" ||
    kind === "milestone"
  );
}

/** One Evidence entry's own write state. Only one entry acts at a time. */
type EvidenceAction = "pin" | "reobserve";

type EvidencePhase =
  | { readonly status: "sending" }
  | { readonly status: "confirming"; readonly action: EvidenceAction }
  | { readonly status: "done"; readonly changed: boolean; readonly verification: string }
  | { readonly status: "failed"; readonly error: ResolvedSafeError };

interface EvidenceWorking {
  readonly evidenceId: string;
  readonly action: EvidenceAction;
  readonly clientRequestId: string;
  readonly phase: EvidencePhase;
}

function defaultClientRequestId(): string {
  const random =
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `o01-${random}`;
}

type Tab = "structure" | "evidence" | "people";

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly detail: ProductDetailDto };

/**
 * A load result remembers which Product it answers for. When the selected
 * Product changes, the previous answer is simply not for this one, so the
 * surface is loading again -- derived, rather than set from inside an effect.
 */
interface Loaded {
  readonly productId: string;
  readonly state: Exclude<LoadState, { readonly status: "loading" }>;
}

const KIND_LABELS: ReadonlyMap<string, string> = new Map([
  ["product", "Product"],
  ["initiative", "Initiative"],
  ["project", "Project"],
  ["milestone", "Milestone"],
  ["stakeholder", "Stakeholder"],
  ["kpi_definition", "KPI"],
  ["roadmap", "Roadmap"],
  ["evidence_reference", "Evidence"],
  ["action_request", "Action Request"],
  ["action", "Action"],
  ["decision_request", "Decision Request"],
  ["risk", "Risk"],
  ["issue", "Issue"],
]);

function kindLabel(kind: string): string {
  return KIND_LABELS.get(kind) ?? kind;
}

/** What a person is to the Product: responsible for it, or depending on it. */
function purposeLabel(t: Translator, purpose: string): string {
  return purpose === "responsibility" || purpose === "dependency"
    ? t(`inspector.purpose.${purpose}`)
    : purpose;
}

/**
 * A current condition in words. The host names the reason and the record it
 * is about; when it could not, its own sentence is shown unchanged rather
 * than a guess.
 */
function healthReasonText(reason: HealthReasonDto, t: Translator): string {
  const { reasonCode, subjectKind, subjectLabel } = reason;
  if (reasonCode === null || subjectKind === null || subjectLabel === null) {
    return reason.text;
  }
  if (subjectKind === "evidence") {
    return t("inspector.healthEvidence", {
      label: subjectLabel,
      verification: verificationLabel(t, reasonCode),
    });
  }
  return t("inspector.healthRecord", {
    kind: kindLabel(subjectKind),
    label: subjectLabel,
    reason: reasonLabel(t, reasonCode),
  });
}

export function ProductInspector({
  productId,
  load,
  actions,
  newClientRequestId = defaultClientRequestId,
  entryActions,
  timeZone = browserTimeZone(),
  onLedgerChanged,
}: ProductInspectorProps) {
  const t = useT();
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [tab, setTab] = useState<Tab>("structure");
  const [working, setWorking] = useState<EvidenceWorking | null>(null);
  const [vault, setVault] = useState<VaultStatusDto | null>(null);
  const [link, setLink] = useState<LinkPhase>({ status: "idle" });
  const [entrySheet, setEntrySheet] = useState<EntrySheet | null>(null);
  const [entryNotice, setEntryNotice] = useState<string | null>(null);
  const [entryOpening, setEntryOpening] = useState<ResolvedSafeError | null>(null);
  const [fileSheetOpen, setFileSheetOpen] = useState(false);
  // A notice a create-and-link prepared before the sheet closed; it replaces
  // the plain "created" line, since the record exists but is not linked.
  const pendingNotice = useRef<string | null>(null);

  const fetchDetail = useCallback(() => {
    if (actions !== undefined) {
      actions.loadVaultStatus().then(setVault, () => {
        setVault(null);
      });
    }
    load(productId).then(
      (detail) => {
        setLoaded({ productId, state: { status: "ready", detail } });
      },
      (reason: unknown) => {
        setLoaded({ productId, state: { status: "error", error: resolveRejection(reason, t) } });
      },
    );
  }, [actions, load, productId, t]);

  const retry = useCallback(() => {
    setLoaded(null);
    fetchDetail();
  }, [fetchDetail]);

  /**
   * Run one Evidence write and re-read the Product.
   *
   * `clientRequestId` is minted once per *submitted command* and reused
   * verbatim on a retry of that same command, so a retry can never become a
   * second write. The detail is re-read on success because this Product may
   * now be presented at a different classification: an Evidence entry that
   * moves to a more restrictive state folds upward into the whole inspector.
   */
  const send = useCallback(
    (
      evidenceId: string,
      expectedVersion: number,
      action: EvidenceAction,
      clientRequestId: string,
    ) => {
      if (actions === undefined) {
        return;
      }
      setWorking({ evidenceId, action, clientRequestId, phase: { status: "sending" } });
      // The version this view read. The host refuses if the record has
      // moved past it, so a click from a stale view never acts on a record
      // the person did not see.
      const call =
        action === "pin"
          ? actions.pinEvidenceFingerprint(evidenceId, expectedVersion, clientRequestId)
          : actions.reobserveEvidenceVerification(evidenceId, expectedVersion, clientRequestId);
      call.then(
        (outcome: EvidenceWriteOutcomeDto) => {
          setWorking({
            evidenceId,
            action,
            clientRequestId,
            phase: {
              status: "done",
              changed: outcome.changed,
              verification: outcome.evidence.verification.kind,
            },
          });
          fetchDetail();
          if (outcome.changed) {
            onLedgerChanged?.();
          }
        },
        (reason: unknown) => {
          setWorking({
            evidenceId,
            action,
            clientRequestId,
            phase: { status: "failed", error: resolveRejection(reason, t) },
          });
        },
      );
    },
    [actions, fetchDetail, onLedgerChanged, t],
  );

  const sendLink = useCallback(
    (evidenceId: string, version: number, clientRequestId: string) => {
      if (actions === undefined) {
        return;
      }
      setLink({ status: "sending", clientRequestId, evidenceId, version });
      actions.linkEvidenceToProduct(productId, evidenceId, version, clientRequestId).then(
        (outcome: EvidenceLinkOutcomeDto) => {
          setLink({
            status: "done",
            evidenceId: outcome.evidenceId,
            classificationAtLink: outcome.classificationAtLink,
          });
          // Linking does not advance the Evidence version, so nothing can be
          // inferred; the Product is re-read as the only source of truth.
          fetchDetail();
          onLedgerChanged?.();
        },
        (reason: unknown) => {
          setLink({
            status: "failed",
            error: resolveRejection(reason, t),
            clientRequestId,
            evidenceId,
            version,
          });
        },
      );
    },
    [actions, fetchDetail, onLedgerChanged, productId, t],
  );

  useEffect(() => {
    fetchDetail();
  }, [fetchDetail]);

  /** An entry write landed: say so, re-read this Product, tell the page. */
  const entrySaved = useCallback(
    (outcome: EntryOutcomeDto, how: "created" | "updated" | "linked") => {
      setEntrySheet(null);
      const prepared = pendingNotice.current;
      pendingNotice.current = null;
      setEntryNotice(
        prepared ??
          (how === "linked"
            ? t("entry.saved.linked", { id: outcome.id })
            : t(how === "created" ? "entry.saved.created" : "entry.saved.updated", {
                kind: kindWord(t, outcome.kind),
                id: outcome.id,
              })),
      );
      fetchDetail();
      onLedgerChanged?.();
    },
    [fetchDetail, onLedgerChanged, t],
  );

  /** Open an edit sheet on the record as the host holds it now. */
  const openEdit = useCallback(
    (kind: EditableKind, id: string) => {
      if (entryActions === undefined) {
        return;
      }
      setEntryOpening(null);
      entryActions.loadEntryRecord(kind, id).then(
        (record) => {
          setEntrySheet({
            kind: kind === "product" ? "editProduct" : "edit",
            record,
            clientRequestId: newClientRequestId(),
          });
        },
        (reason: unknown) => {
          setEntryOpening(resolveRejection(reason, t));
        },
      );
    },
    [entryActions, newClientRequestId, t],
  );

  const state: LoadState =
    loaded !== null && loaded.productId === productId ? loaded.state : { status: "loading" };

  if (state.status === "loading") {
    return (
      <p className="pmc-inspector-status" role="status">
        {t("route.loading", { route: t("inspector.route") })}
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="pmc-inspector-status">
        <button type="button" onClick={retry}>
          {t("route.reload")}
        </button>
        <SafeErrorDetail
          message={t("route.unavailable", {
            route: t("inspector.route"),
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { detail } = state;
  // `via` is "project:<id>"; name the Project where the structure lists it.
  const viaLabel = (via: string): string => {
    const id = via.startsWith("project:") ? via.slice("project:".length) : via;
    return (
      detail.structure.find((entry) => entry.kind === "project" && entry.id === id)?.label ?? via
    );
  };

  if (detail.state === "outOfSync" || detail.product === null) {
    return (
      <div className="pmc-inspector-status" role="alert">
        <p>{t("inspector.outOfSync")}</p>
        <button type="button" onClick={retry}>
          {t("route.reload")}
        </button>
      </div>
    );
  }

  const { product } = detail;
  const headingId = `pmc-inspector-${product.id}`;

  /**
   * The legal next actions for one Evidence entry.
   *
   * `釘選指紋` appears only while `pinned` is false, because a pin is the
   * reference's identity and is never rewritten: the same bytes are
   * re-observed, a move is a relocation, different bytes are a supersession.
   * The confirmation says that in the person's own terms before anything
   * runs, and names the Evidence by id -- **not** by file path, which this
   * surface never receives.
   *
   * One entry acts at a time. While another entry is mid-flight the rest
   * offer nothing, so a person cannot start a second write believing the
   * first was finished.
   */
  function evidenceActions(entry: ProductDetailDto["evidence"][number]) {
    if (actions === undefined) {
      return null;
    }
    const mine = working !== null && working.evidenceId === entry.id ? working : null;
    const vaultDown = vault !== null && !vault.available;
    // One write in flight across kinds: another entry mid-action, or the
    // link control anywhere past idle, withholds this entry's actions.
    const busy =
      (working !== null && working.evidenceId !== entry.id) || vaultDown || link.status !== "idle";

    if (mine?.phase.status === "confirming") {
      const permanent = mine.phase.action === "pin";
      return (
        <div className="pmc-inspector-evidence-confirm" role="group">
          <p>
            {permanent
              ? t("inspector.pinConfirm", { id: entry.id })
              : t("inspector.reobserveConfirm", { id: entry.id })}
          </p>
          <button
            type="button"
            onClick={() => {
              send(entry.id, entry.revision, mine.action, mine.clientRequestId);
            }}
          >
            {permanent ? t("inspector.confirmPin") : t("inspector.confirmReobserve")}
          </button>
          <button
            type="button"
            onClick={() => {
              setWorking(null);
            }}
          >
            {t("inspector.cancel")}
          </button>
        </div>
      );
    }

    if (mine?.phase.status === "sending") {
      return <span role="status">{t("inspector.sending")}</span>;
    }

    if (mine?.phase.status === "done") {
      const { changed, verification } = mine.phase;
      return (
        <p className="pmc-inspector-evidence-result" role="status">
          {changed
            ? t("inspector.written", { verification: verificationLabel(t, verification) })
            : t("inspector.unchanged")}
          <button
            type="button"
            onClick={() => {
              setWorking(null);
            }}
          >
            {t("inspector.close")}
          </button>
        </p>
      );
    }

    if (mine?.phase.status === "failed") {
      return (
        <div className="pmc-inspector-evidence-result">
          <SafeErrorDetail
            message={t(
              mine.action === "pin" ? "inspector.pinFailed" : "inspector.reobserveFailed",
              {
                message: mine.phase.error.message,
              },
            )}
            correlationId={mine.phase.error.correlationId}
            errorCode={mine.phase.error.errorCode}
            retryable={mine.phase.error.retryable}
            {...(mine.phase.error.retryable
              ? {
                  onRetry: () => {
                    // The same command, so the same request id: a retry can
                    // never become a second write.
                    send(entry.id, entry.revision, mine.action, mine.clientRequestId);
                  },
                }
              : {})}
          />
          <button
            type="button"
            onClick={() => {
              setWorking(null);
            }}
          >
            {t("inspector.abandon")}
          </button>
        </div>
      );
    }

    return (
      <div className="pmc-inspector-evidence-actions">
        {entry.pinned ? null : (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setWorking({
                evidenceId: entry.id,
                action: "pin",
                clientRequestId: newClientRequestId(),
                phase: { status: "confirming", action: "pin" },
              });
            }}
          >
            {t("inspector.pin")}
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          onClick={() => {
            setWorking({
              evidenceId: entry.id,
              action: "reobserve",
              clientRequestId: newClientRequestId(),
              phase: { status: "confirming", action: "reobserve" },
            });
          }}
        >
          {t("inspector.reobserve")}
        </button>
      </div>
    );
  }

  /**
   * Link an existing Evidence reference to this Product. The target is the
   * inspected Product, shown fixed rather than chosen: O01 reads
   * `evidence_links.target_type = 'product'`, and a generic target picker
   * would be a different surface. Candidates exclude what is already linked
   * here, and each carries the version the person is acting on.
   */
  function linkControl() {
    if (actions === undefined) {
      return null;
    }
    const linkedIds = new Set(detail.evidence.map((entry) => entry.id));

    if (link.status === "idle") {
      return (
        <div className="pmc-inspector-link">
          <button
            type="button"
            disabled={working !== null}
            onClick={() => {
              setLink({ status: "loading" });
              actions.loadEvidenceReferences().then(
                (references) => {
                  const candidates = references.evidenceReferences.filter(
                    (reference) => !linkedIds.has(reference.id),
                  );
                  setLink({ status: "choosing", candidates, chosen: candidates[0]?.id ?? "" });
                },
                () => {
                  setLink({ status: "idle" });
                },
              );
            }}
          >
            {t("inspector.link")}
          </button>
        </div>
      );
    }

    if (link.status === "loading") {
      return <span role="status">{t("inspector.linkLoading")}</span>;
    }

    if (link.status === "choosing") {
      const chosen = link.candidates.find((candidate) => candidate.id === link.chosen);
      return (
        <form
          className="pmc-inspector-link"
          onSubmit={(event) => {
            event.preventDefault();
            if (chosen !== undefined) {
              sendLink(chosen.id, chosen.version, newClientRequestId());
            }
          }}
        >
          <label>
            {t("inspector.linkChoose", { product: product.label })}
            <select
              value={link.chosen}
              onChange={(event) => {
                setLink({ ...link, chosen: event.target.value });
              }}
            >
              {link.candidates.length === 0 ? (
                <option value="">{t("inspector.linkNone")}</option>
              ) : null}
              {link.candidates.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {t("inspector.linkCandidate", {
                    id: candidate.id,
                    verification: verificationLabel(t, candidate.verification.kind),
                    classification: classificationName(t, candidate.classification),
                    version: String(candidate.version),
                  })}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={chosen === undefined}>
            {t("inspector.linkConfirm")}
          </button>
          <button
            type="button"
            onClick={() => {
              setLink({ status: "idle" });
            }}
          >
            {t("inspector.cancel")}
          </button>
        </form>
      );
    }

    if (link.status === "sending") {
      return <span role="status">{t("inspector.sending")}</span>;
    }

    if (link.status === "done") {
      return (
        <p className="pmc-inspector-link" role="status">
          {t("inspector.linked", {
            id: link.evidenceId,
            classification: classificationName(t, link.classificationAtLink),
          })}
          <button
            type="button"
            onClick={() => {
              setLink({ status: "idle" });
            }}
          >
            {t("inspector.close")}
          </button>
        </p>
      );
    }

    return (
      <div className="pmc-inspector-link">
        <SafeErrorDetail
          message={t("inspector.linkFailed", { message: link.error.message })}
          correlationId={link.error.correlationId}
          errorCode={link.error.errorCode}
          retryable={link.error.retryable}
          {...(link.error.retryable
            ? {
                onRetry: () => {
                  sendLink(link.evidenceId, link.version, link.clientRequestId);
                },
              }
            : {})}
        />
        <button
          type="button"
          onClick={() => {
            setLink({ status: "idle" });
          }}
        >
          {t("inspector.abandon")}
        </button>
      </div>
    );
  }

  /**
   * Open the link sheet for a Roadmap or KPI: every record of that kind the
   * Ledger holds, less those already in this Product's structure, each with
   * the version the person will act on.
   */
  function openLink(target: "roadmap" | "kpi_definition" | "project") {
    if (entryActions === undefined) {
      return;
    }
    setEntryOpening(null);
    const present = new Set(
      detail.structure.filter((entry) => entry.kind === target).map((entry) => entry.id),
    );
    // The Product's own classification comes from its entry record: the
    // inspector's `product.classification` is the folded one.
    Promise.all([
      entryActions.listEntryRecords(target),
      entryActions.loadEntryRecord("product", productId),
    ]).then(
      ([list, own]) => {
        setEntrySheet({
          kind: "link",
          target,
          candidates: linkCandidates(
            list.records.filter((record) => record.kind === target),
            present,
          ),
          productClassification: own.classification,
          clientRequestId: newClientRequestId(),
        });
      },
      (reason: unknown) => {
        setEntryOpening(resolveRejection(reason, t));
      },
    );
  }

  /**
   * Link an existing Initiative to one of this Product's Projects, at the
   * Project version this view read. Initiatives already reached through
   * that Project are left out.
   */
  function openLinkInitiative(project: {
    id: string;
    label: string;
    revision: number;
    classification: string;
  }) {
    if (entryActions === undefined) {
      return;
    }
    setEntryOpening(null);
    const present = new Set(
      detail.structure
        .filter((entry) => entry.kind === "initiative" && entry.via === `project:${project.id}`)
        .map((entry) => entry.id),
    );
    entryActions.listEntryRecords("initiative").then(
      (list) => {
        setEntrySheet({
          kind: "linkInitiative",
          projectId: project.id,
          projectVersion: project.revision,
          projectLabel: project.label,
          projectClassification: project.classification,
          candidates: linkCandidates(
            list.records.filter((record) => record.kind === "initiative"),
            present,
          ),
          clientRequestId: newClientRequestId(),
        });
      },
      (reason: unknown) => {
        setEntryOpening(resolveRejection(reason, t));
      },
    );
  }

  /**
   * A Roadmap or KPI created from this Product's Structure tab is linked to
   * the Product right after, under a request id derived from the sheet's: a
   * retry of the create replays the same record, and a retry of the link
   * replays the same link. If the link fails the record still exists, and
   * the notice says so and names the way to link it.
   */
  function createAndLink(
    create: Promise<EntryOutcomeDto>,
    clientRequestId: string,
    link: (outcome: EntryOutcomeDto, linkRequestId: string) => Promise<EntryOutcomeDto>,
    linkLabel: string,
  ): Promise<EntryOutcomeDto> {
    return create.then((outcome) =>
      link(outcome, `${clientRequestId}-link`).then(
        () => outcome,
        (reason: unknown) => {
          const error = resolveRejection(reason, t);
          pendingNotice.current = t("entry.createdNotLinked", {
            kind: kindWord(t, outcome.kind),
            id: outcome.id,
            message: error.message,
            link: linkLabel,
          });
          return outcome;
        },
      ),
    );
  }

  /**
   * What an edit sheet needs for a record of one kind; `null` for a kind
   * this surface does not edit (an observation, a Product, which has its
   * own sheet).
   */
  function editSheet(record: EntryRecordDto): {
    readonly kind: EditableKind;
    readonly title: string;
    readonly fields: readonly FieldSpec[];
    readonly submit: (values: SubmittedValues, clientRequestId: string) => Promise<EntryOutcomeDto>;
    readonly validate?: (values: SubmittedValues) => string | null;
  } | null {
    if (entryActions === undefined) {
      return null;
    }
    const entry = entryActions;
    switch (record.kind) {
      case "roadmap":
        return {
          kind: "roadmap",
          title: t("entry.title.edit.roadmap"),
          fields: simpleFieldSpecs(t),
          submit: (values, clientRequestId) =>
            entry.updateRoadmap(record.id, record.version, simpleValues(values), clientRequestId),
        };
      case "kpi_definition":
        return {
          kind: "kpi_definition",
          title: t("entry.title.edit.kpi"),
          fields: kpiFieldSpecs(t),
          submit: (values, clientRequestId) =>
            entry.updateKpiDefinition(
              record.id,
              record.version,
              kpiValues(values),
              clientRequestId,
            ),
        };
      case "initiative":
        return {
          kind: "initiative",
          title: t("entry.title.edit.initiative"),
          fields: initiativeFieldSpecs(t),
          submit: (values, clientRequestId) =>
            entry.updateInitiative(
              record.id,
              record.version,
              initiativeValues(values),
              clientRequestId,
            ),
        };
      case "project":
        return {
          kind: "project",
          title: t("entry.title.edit.project"),
          fields: projectFieldSpecs(t, timeZone),
          submit: (values, clientRequestId) =>
            entry.updateProject(record.id, record.version, projectValues(values), clientRequestId),
          validate: (values) => projectPeriodProblem(t, values),
        };
      case "milestone":
        return {
          kind: "milestone",
          title: t("entry.title.edit.milestone"),
          fields: milestoneFieldSpecs(t, timeZone),
          submit: (values, clientRequestId) =>
            entry.updateMilestone(
              record.id,
              record.version,
              milestoneValues(values),
              clientRequestId,
            ),
        };
      case "product":
      case "portfolio":
      case "kpi_observation":
      case "stakeholder":
        return null;
    }
  }

  function entrySheetElement() {
    if (entryActions === undefined || entrySheet === null) {
      return null;
    }
    const entry = entryActions;
    const close = () => {
      setEntrySheet(null);
    };
    switch (entrySheet.kind) {
      case "editProduct": {
        const { record, clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.edit.product")}
            fields={simpleFieldSpecs(t)}
            initial={initialValues(record, timeZone)}
            submitLabel={t("entry.save")}
            submit={(values) =>
              entry.updateProduct(record.id, record.version, simpleValues(values), clientRequestId)
            }
            onDone={(outcome) => {
              entrySaved(outcome, "updated");
            }}
            onClose={close}
            onStale={() => {
              close();
              openEdit("product", record.id);
            }}
          />
        );
      }
      case "createRoadmap": {
        const { clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.create.roadmap")}
            fields={simpleFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              createAndLink(
                entry.createRoadmap(simpleValues(values), clientRequestId),
                clientRequestId,
                (outcome, linkRequestId) =>
                  entry.linkProductRoadmap(
                    productId,
                    product.revision,
                    outcome.id,
                    outcome.version,
                    linkRequestId,
                  ),
                t("entry.link.roadmap"),
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "created");
            }}
            onClose={close}
          />
        );
      }
      case "createKpi": {
        const { clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.create.kpi")}
            fields={kpiFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              createAndLink(
                entry.createKpiDefinition(kpiValues(values), clientRequestId),
                clientRequestId,
                (outcome, linkRequestId) =>
                  entry.linkProductKpi(
                    productId,
                    product.revision,
                    outcome.id,
                    outcome.version,
                    linkRequestId,
                  ),
                t("entry.link.kpi"),
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "created");
            }}
            onClose={close}
          />
        );
      }
      case "edit": {
        const { record, clientRequestId } = entrySheet;
        // One sheet per kind: the title, the fields, the command and the
        // rule across fields, chosen from the record the host holds.
        const edit = editSheet(record);
        if (edit === null) {
          return null;
        }
        return (
          <RecordSheet
            title={edit.title}
            fields={edit.fields}
            initial={initialValues(record, timeZone)}
            submitLabel={t("entry.save")}
            submit={(values) => edit.submit(values, clientRequestId)}
            onDone={(outcome) => {
              entrySaved(outcome, "updated");
            }}
            onClose={close}
            onStale={() => {
              close();
              openEdit(edit.kind, record.id);
            }}
            {...(edit.validate === undefined ? {} : { validate: edit.validate })}
          />
        );
      }
      case "createProject": {
        const { clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.create.project")}
            fields={projectFieldSpecs(t, timeZone)}
            initial={{}}
            submitLabel={t("entry.create")}
            validate={(values) => projectPeriodProblem(t, values)}
            submit={(values) =>
              createAndLink(
                entry.createProject(projectValues(values), clientRequestId),
                clientRequestId,
                (outcome, linkRequestId) =>
                  entry.linkProjectProduct(
                    outcome.id,
                    outcome.version,
                    productId,
                    product.revision,
                    linkRequestId,
                  ),
                t("entry.link.project"),
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "created");
            }}
            onClose={close}
          />
        );
      }
      case "createMilestone": {
        const { projectId: parent, projectVersion, projectLabel, clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.create.milestone", { project: projectLabel })}
            fields={milestoneFieldSpecs(t, timeZone)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              entry.createMilestone(
                parent,
                projectVersion,
                milestoneValues(values),
                clientRequestId,
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "created");
            }}
            onClose={close}
          />
        );
      }
      case "createInitiative": {
        const { projectId: parent, projectVersion, projectLabel, clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.create.initiative", { project: projectLabel })}
            fields={initiativeFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              createAndLink(
                entry.createInitiative(initiativeValues(values), clientRequestId),
                clientRequestId,
                (outcome, linkRequestId) =>
                  entry.linkInitiativeProject(
                    outcome.id,
                    outcome.version,
                    parent,
                    projectVersion,
                    linkRequestId,
                  ),
                t("entry.link.initiative"),
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "created");
            }}
            onClose={close}
          />
        );
      }
      case "linkInitiative": {
        const {
          projectId: parent,
          projectVersion,
          projectLabel,
          projectClassification,
          candidates,
          clientRequestId,
        } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.link.initiative", { project: projectLabel })}
            fields={[linkFieldSpec(t, candidates)]}
            initial={{}}
            submitLabel={t("entry.linkConfirm")}
            preview={(values) =>
              linkClassificationPreview(t, projectClassification, candidates, values)
            }
            submit={(values) => {
              const chosen = candidates.find((candidate) => candidate.id === values.record);
              if (chosen === undefined) {
                return Promise.reject(new Error("no candidate chosen"));
              }
              return entry.linkInitiativeProject(
                chosen.id,
                chosen.version,
                parent,
                projectVersion,
                clientRequestId,
              );
            }}
            onDone={(outcome) => {
              entrySaved(outcome, "linked");
            }}
            onClose={close}
            onStale={() => {
              close();
              fetchDetail();
            }}
          />
        );
      }
      case "observe": {
        const { kpiId, kpiVersion, kpiLabel, clientRequestId } = entrySheet;
        return (
          <RecordSheet
            title={t("entry.title.create.observation", { kpi: kpiLabel })}
            fields={observationFieldSpecs(t, timeZone)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              entry.createKpiObservation(
                kpiId,
                kpiVersion,
                observationValues(values),
                clientRequestId,
              )
            }
            onDone={(outcome) => {
              entrySaved(outcome, "created");
            }}
            onClose={close}
          />
        );
      }
      case "link": {
        const { target, candidates, productClassification, clientRequestId } = entrySheet;
        const titleKey =
          target === "roadmap"
            ? "entry.title.link.roadmap"
            : target === "kpi_definition"
              ? "entry.title.link.kpi"
              : "entry.title.link.project";
        return (
          <RecordSheet
            title={t(titleKey, { product: product.label })}
            fields={[linkFieldSpec(t, candidates)]}
            initial={{}}
            submitLabel={t("entry.linkConfirm")}
            preview={(values) =>
              linkClassificationPreview(t, productClassification, candidates, values)
            }
            submit={(values) => {
              const chosen = candidates.find((candidate) => candidate.id === values.record);
              if (chosen === undefined) {
                return Promise.reject(new Error("no candidate chosen"));
              }
              switch (target) {
                case "roadmap":
                  return entry.linkProductRoadmap(
                    productId,
                    product.revision,
                    chosen.id,
                    chosen.version,
                    clientRequestId,
                  );
                case "kpi_definition":
                  return entry.linkProductKpi(
                    productId,
                    product.revision,
                    chosen.id,
                    chosen.version,
                    clientRequestId,
                  );
                case "project":
                  // The Project is the link's first endpoint.
                  return entry.linkProjectProduct(
                    chosen.id,
                    chosen.version,
                    productId,
                    product.revision,
                    clientRequestId,
                  );
              }
            }}
            onDone={(outcome) => {
              entrySaved(outcome, "linked");
            }}
            onClose={close}
            onStale={() => {
              close();
              fetchDetail();
            }}
          />
        );
      }
    }
  }

  const entryBusy = entrySheet !== null;

  return (
    <section className="pmc-inspector" aria-labelledby={headingId}>
      <div className="pmc-inspector-head">
        <h3 id={headingId}>{product.label}</h3>
        {entryActions === undefined ? null : (
          <div className="pmc-entry-actions" role="group" aria-label={t("entry.inspector.actions")}>
            <button
              type="button"
              disabled={entryBusy}
              onClick={() => {
                openEdit("product", productId);
              }}
            >
              {t("entry.edit")}
            </button>
          </div>
        )}
      </div>
      {entryNotice !== null ? (
        <p className="pmc-entry-notice" role="status">
          {entryNotice}
        </p>
      ) : null}
      {entryOpening !== null ? (
        <SafeErrorDetail
          message={t("entry.readFailed", { message: entryOpening.message })}
          correlationId={entryOpening.correlationId}
          retryable={entryOpening.retryable}
        />
      ) : null}
      <dl className="pmc-meta pmc-inspector-meta">
        <div>
          <dt>{t("inspector.classification")}</dt>
          <dd>{classificationName(t, product.classification)}</dd>
        </div>
        <div>
          <dt>{t("inspector.version")}</dt>
          <dd>{String(product.revision)}</dd>
        </div>
        <div>
          <dt>{t("route.readAt")}</dt>
          <dd>
            <time>{formatReadAt(product.asOfMillis)}</time>
          </dd>
        </div>
      </dl>
      {detail.classificationForcedBy ? (
        <p className="pmc-inspector-fold">
          {t("inspector.fold", {
            classification: classificationName(t, detail.classificationForcedBy.classification),
            kind: kindLabel(detail.classificationForcedBy.forcedByKind),
            id: detail.classificationForcedBy.forcedById,
          })}
        </p>
      ) : null}

      <h4>{t("inspector.happened")}</h4>
      {detail.healthReasons.length === 0 ? (
        <p>{t("inspector.nothingHappened")}</p>
      ) : (
        <ul className="pmc-inspector-conditions">
          {detail.healthReasons.map((reason) => (
            <li key={`${reason.sourceRecordId}:${reason.text}`}>
              {/* Attributed to what raised it, never to the Product. */}
              <Slotted
                text={
                  t.lookup("inspector.conditionLine", {
                    condition: healthReasonText(reason, t),
                  }) ?? ""
                }
                slots={{
                  provenance: (
                    <span className="pmc-inspector-provenance">
                      {t("inspector.provenance", {
                        owner: ownerLabel(t, reason.owner),
                        id: reason.sourceRecordId,
                        version: String(reason.sourceRevision),
                      })}
                    </span>
                  ),
                }}
              />
            </li>
          ))}
        </ul>
      )}

      <h4>{t("inspector.impact")}</h4>
      {/* A judgment slot. Nothing is computed into it. */}
      <p className="pmc-inspector-impact">
        {detail.impact === "unassessed" ? t("inspector.impactUnassessed") : detail.impact}
      </p>

      <div role="tablist" aria-label={t("inspector.tabs")}>
        {(["structure", "evidence", "people"] as const).map((candidate) => (
          <button
            key={candidate}
            type="button"
            role="tab"
            aria-selected={tab === candidate}
            aria-controls={`${headingId}-${candidate}`}
            onClick={() => {
              setTab(candidate);
            }}
          >
            {t(`inspector.tab.${candidate}`)}
          </button>
        ))}
      </div>

      {tab === "structure" ? (
        <div
          role="tabpanel"
          id={`${headingId}-structure`}
          aria-label={t("inspector.tab.structure")}
        >
          {entryActions === undefined ? null : (
            <div
              className="pmc-entry-actions"
              role="group"
              aria-label={t("entry.structure.actions")}
            >
              <button
                type="button"
                disabled={entryBusy}
                onClick={() => {
                  setEntrySheet({ kind: "createRoadmap", clientRequestId: newClientRequestId() });
                }}
              >
                {t("entry.new.roadmap")}
              </button>
              <button
                type="button"
                disabled={entryBusy}
                onClick={() => {
                  openLink("roadmap");
                }}
              >
                {t("entry.link.roadmap")}
              </button>
              <button
                type="button"
                disabled={entryBusy}
                onClick={() => {
                  setEntrySheet({ kind: "createKpi", clientRequestId: newClientRequestId() });
                }}
              >
                {t("entry.new.kpi")}
              </button>
              <button
                type="button"
                disabled={entryBusy}
                onClick={() => {
                  openLink("kpi_definition");
                }}
              >
                {t("entry.link.kpi")}
              </button>
              <button
                type="button"
                disabled={entryBusy}
                onClick={() => {
                  setEntrySheet({ kind: "createProject", clientRequestId: newClientRequestId() });
                }}
              >
                {t("entry.new.project")}
              </button>
              <button
                type="button"
                disabled={entryBusy}
                onClick={() => {
                  openLink("project");
                }}
              >
                {t("entry.link.project")}
              </button>
            </div>
          )}
          {detail.structure.length === 0 ? (
            <p>{t("inspector.structureNone")}</p>
          ) : (
            <ul className="pmc-inspector-structure">
              {detail.structure.map((entry) => (
                <li key={`${entry.kind}:${entry.id}`}>
                  {/* Reached through a Project, and said so: an association,
                      never containment. */}
                  {entry.via ? (
                    <Slotted
                      text={
                        t.lookup("inspector.structureEntryVia", {
                          kind: kindLabel(entry.kind),
                          label: entry.label,
                          classification: classificationName(t, entry.classification),
                        }) ?? ""
                      }
                      slots={{
                        via: (
                          <span className="pmc-inspector-via">
                            {t("inspector.via", { project: viaLabel(entry.via) })}
                          </span>
                        ),
                      }}
                    />
                  ) : (
                    t("inspector.structureEntry", {
                      kind: kindLabel(entry.kind),
                      label: entry.label,
                      classification: classificationName(t, entry.classification),
                    })
                  )}
                  {/* The records this surface edits in place; an observation
                      is recorded against its KPI, a Milestone and an
                      Initiative are entered under a Project. */}
                  {entryActions !== undefined && isEditableKind(entry.kind) ? (
                    <div
                      className="pmc-entry-actions"
                      {...(entry.kind === "project"
                        ? { role: "group", "aria-label": t("entry.project.actions") }
                        : {})}
                    >
                      <button
                        type="button"
                        disabled={entryBusy}
                        onClick={() => {
                          // Narrowed again here: the guard above does not
                          // reach into this handler.
                          if (isEditableKind(entry.kind)) {
                            openEdit(entry.kind, entry.id);
                          }
                        }}
                      >
                        {t("entry.edit")}
                      </button>
                      {entry.kind === "project" ? (
                        <>
                          <button
                            type="button"
                            disabled={entryBusy}
                            onClick={() => {
                              setEntrySheet({
                                kind: "createMilestone",
                                projectId: entry.id,
                                projectVersion: entry.revision,
                                projectLabel: entry.label,
                                clientRequestId: newClientRequestId(),
                              });
                            }}
                          >
                            {t("entry.new.milestone")}
                          </button>
                          <button
                            type="button"
                            disabled={entryBusy}
                            onClick={() => {
                              setEntrySheet({
                                kind: "createInitiative",
                                projectId: entry.id,
                                projectVersion: entry.revision,
                                projectLabel: entry.label,
                                clientRequestId: newClientRequestId(),
                              });
                            }}
                          >
                            {t("entry.new.initiative")}
                          </button>
                          <button
                            type="button"
                            disabled={entryBusy}
                            onClick={() => {
                              openLinkInitiative({
                                id: entry.id,
                                label: entry.label,
                                revision: entry.revision,
                                classification: entry.classification,
                              });
                            }}
                          >
                            {t("entry.link.initiative")}
                          </button>
                        </>
                      ) : null}
                      {entry.kind === "kpi_definition" ? (
                        <button
                          type="button"
                          disabled={entryBusy}
                          onClick={() => {
                            setEntrySheet({
                              kind: "observe",
                              kpiId: entry.id,
                              kpiVersion: entry.revision,
                              kpiLabel: entry.label,
                              clientRequestId: newClientRequestId(),
                            });
                          }}
                        >
                          {t("entry.new.observation")}
                        </button>
                      ) : null}
                    </div>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      {tab === "evidence" ? (
        <div role="tabpanel" id={`${headingId}-evidence`} aria-label={t("inspector.tab.evidence")}>
          {actions !== undefined && vault !== null && !vault.available ? (
            <p className="pmc-inspector-vault-degraded" role="status">
              {vault.reason === "notConfigured"
                ? t("inspector.vaultNotConfigured")
                : t("inspector.vaultUnavailable")}
            </p>
          ) : null}
          {detail.evidence.length === 0 ? (
            <p>{t("inspector.evidenceNone")}</p>
          ) : (
            <ul className="pmc-inspector-evidence">
              {detail.evidence.map((entry) => (
                <li key={entry.id}>
                  {/* Three classifications, three facts. */}
                  <Slotted
                    text={
                      t.lookup(
                        entry.pinned ? "inspector.evidenceLine" : "inspector.evidenceLineUnpinned",
                        { id: entry.id, verification: verificationLabel(t, entry.verification) },
                      ) ?? ""
                    }
                    slots={{
                      ...(entry.pinned
                        ? {}
                        : {
                            unpinned: (
                              <span className="pmc-inspector-unpinned">
                                {t("inspector.unpinned")}
                              </span>
                            ),
                          }),
                      classifications: (
                        <span className="pmc-inspector-classifications">
                          {t("inspector.evidenceClassifications", {
                            classification: classificationName(t, entry.classification),
                            atLink: classificationName(t, entry.classificationAtLink),
                          })}
                        </span>
                      ),
                    }}
                  />
                  {evidenceActions(entry)}
                </li>
              ))}
            </ul>
          )}
          {actions !== undefined ? linkControl() : null}
          {actions?.evidenceFile !== undefined ? (
            <div className="pmc-inspector-link">
              {/* Legal only while the Vault can be read; the notice above
                  says why when it cannot (§4). */}
              <button
                type="button"
                disabled={vault?.available !== true || working !== null || link.status !== "idle"}
                onClick={() => {
                  setFileSheetOpen(true);
                }}
              >
                {t("evidenceFile.open")}
              </button>
            </div>
          ) : null}
          {fileSheetOpen && actions?.evidenceFile !== undefined ? (
            <EvidenceFromFileSheet
              actions={actions.evidenceFile}
              product={{
                id: product.id,
                label: product.label,
                linkedEvidenceIds: new Set(detail.evidence.map((entry) => entry.id)),
                // The Product version this view read (§4 Boundary).
                link: (evidenceId, version, clientRequestId) =>
                  actions.linkEvidenceToProduct(
                    product.id,
                    evidenceId,
                    version,
                    clientRequestId,
                    product.revision,
                  ),
              }}
              newClientRequestId={newClientRequestId}
              onClose={() => {
                setFileSheetOpen(false);
              }}
              onFinished={() => {
                setFileSheetOpen(false);
                fetchDetail();
                onLedgerChanged?.();
              }}
            />
          ) : null}
        </div>
      ) : null}

      {tab === "people" ? (
        <div role="tabpanel" id={`${headingId}-people`} aria-label={t("inspector.tab.people")}>
          {detail.people.length === 0 ? (
            <p>{t("inspector.peopleNone")}</p>
          ) : (
            <ul className="pmc-inspector-people">
              {detail.people.map((person) => (
                <li key={`${person.id}:${person.purpose}`}>
                  <h5>
                    {t.plural("inspector.person", person.otherProductsAccountableFor, {
                      name: person.displayName,
                      purpose: purposeLabel(t, person.purpose),
                    })}
                  </h5>
                  {/* The framing the amendment exists for: carried by the
                      person, never owned by the Product. */}
                  <p className="pmc-inspector-carried-heading">{t("inspector.carriedHeading")}</p>
                  {person.carried.length === 0 ? (
                    <p>{t("inspector.carriedNone")}</p>
                  ) : (
                    <ul className="pmc-inspector-carried">
                      {person.carried.map((item) => (
                        <li key={`${item.kind}:${item.id}`}>
                          {/* Next steps are text, not buttons: lifecycle-
                              admissible is weaker than executable. */}
                          <Slotted
                            text={
                              t.lookup("inspector.carriedLine", {
                                kind: kindLabel(item.kind),
                                label: item.label,
                                state: stateLabel(t, item.stateLabel),
                              }) ?? ""
                            }
                            slots={{
                              attention:
                                item.attention.length > 0 ? (
                                  <ul>
                                    {item.attention.map((flag) => (
                                      <li key={flag.reason}>{reasonLabel(t, flag.reason)}</li>
                                    ))}
                                  </ul>
                                ) : null,
                              intents: (
                                <span className="pmc-inspector-intents">
                                  {t("inspector.nextSteps", {
                                    intents:
                                      item.lifecycleLegalIntents.length === 0
                                        ? t("inspector.noNextSteps")
                                        : item.lifecycleLegalIntents
                                            .map((intent) => intentLabel(t, intent))
                                            .join(t("common.idSeparator")),
                                  })}
                                </span>
                              ),
                            }}
                          />
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      {entrySheetElement()}
    </section>
  );
}
