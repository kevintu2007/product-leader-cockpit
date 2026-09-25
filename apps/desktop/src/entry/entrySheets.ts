import type { Translator } from "../i18n/messages";
import { classificationName } from "../i18n/workLabels";
import { combineClassification } from "./classificationChoice";
import type {
  EntryRecordDto,
  InitiativeFields,
  KpiDefinitionFields,
  KpiObservationFields,
  MilestoneFields,
  ActionRequestFields,
  DecisionRequestFields,
  IssueFields,
  ProjectFields,
  RiskFields,
  RiskResponseFields,
  SimpleFields,
  StakeholderFields,
} from "./entryIpc";
import type { FieldSpec, SubmittedValues } from "./RecordSheet";
import type { FieldValues } from "./dirtyState";
import { utcMillisToLocal } from "./localDateTime";
import { TEXT_LIMITS } from "./utf8Bytes";

/**
 * The field lists the Portfolio-family sheets share, and the mapping from
 * what a sheet submits to what the host command takes. Every text field is
 * required because every domain text is (`BoundedText` refuses blank).
 */

export function simpleFieldSpecs(t: Translator): readonly FieldSpec[] {
  return [
    { kind: "short", name: "name", label: t("entry.field.name"), required: true },
    { kind: "long", name: "details", label: t("entry.field.details"), required: true },
    { kind: "classification", name: "classification" },
  ];
}

export function kpiFieldSpecs(t: Translator): readonly FieldSpec[] {
  return [
    { kind: "short", name: "name", label: t("entry.field.name"), required: true },
    { kind: "long", name: "definition", label: t("entry.field.definition"), required: true },
    { kind: "short", name: "owner", label: t("entry.field.owner"), required: true },
    { kind: "short", name: "target", label: t("entry.field.target"), required: true },
    { kind: "short", name: "cadence", label: t("entry.field.cadence"), required: true },
    { kind: "long", name: "source", label: t("entry.field.source"), required: true },
    { kind: "classification", name: "classification" },
  ];
}

export function observationFieldSpecs(t: Translator, timeZone: string): readonly FieldSpec[] {
  return [
    { kind: "short", name: "value", label: t("entry.field.value"), required: true },
    { kind: "datetime", name: "observedAt", label: t("entry.field.observedAt"), timeZone },
    { kind: "long", name: "source", label: t("entry.field.source"), required: true },
    {
      kind: "classification",
      name: "classification",
      inheritLabel: t("entry.classification.inherit"),
    },
  ];
}

// ---- The Delivery family (slice 6C) ---------------------------------------

export function initiativeFieldSpecs(t: Translator): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "name",
      label: t("entry.field.name"),
      required: true,
      limit: TEXT_LIMITS.deliveryName,
    },
    {
      kind: "long",
      name: "definedOutcome",
      label: t("entry.field.definedOutcome"),
      required: true,
      limit: TEXT_LIMITS.deliveryDetail,
    },
    { kind: "classification", name: "classification" },
  ];
}

export function projectFieldSpecs(t: Translator, timeZone: string): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "name",
      label: t("entry.field.name"),
      required: true,
      limit: TEXT_LIMITS.deliveryName,
    },
    { kind: "datetime", name: "startAt", label: t("entry.field.startAt"), timeZone },
    { kind: "datetime", name: "endAt", label: t("entry.field.endAt"), timeZone },
    { kind: "classification", name: "classification" },
  ];
}

/** The one rule across a Project's fields: it cannot end before it starts. */
export function projectPeriodProblem(t: Translator, values: SubmittedValues): string | null {
  const start = values.startAt;
  const end = values.endAt;
  return typeof start === "number" && typeof end === "number" && end < start
    ? t("entry.period.invalid")
    : null;
}

export function milestoneFieldSpecs(t: Translator, timeZone: string): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "name",
      label: t("entry.field.name"),
      required: true,
      limit: TEXT_LIMITS.deliveryName,
    },
    {
      kind: "long",
      name: "verificationCriteria",
      label: t("entry.field.verificationCriteria"),
      required: true,
      limit: TEXT_LIMITS.deliveryDetail,
    },
    { kind: "datetime", name: "dueAt", label: t("entry.field.dueAt"), timeZone },
    { kind: "classification", name: "classification" },
  ];
}

function millis(values: SubmittedValues, name: string): number {
  const value = values[name];
  return typeof value === "number" ? value : 0;
}

export function initiativeValues(values: SubmittedValues): InitiativeFields {
  return {
    name: text(values, "name"),
    definedOutcome: text(values, "definedOutcome"),
    classification: classification(values),
  };
}

export function projectValues(values: SubmittedValues): ProjectFields {
  return {
    name: text(values, "name"),
    startAtMillis: millis(values, "startAt"),
    endAtMillis: millis(values, "endAt"),
    classification: classification(values),
  };
}

export function milestoneValues(values: SubmittedValues): MilestoneFields {
  return {
    name: text(values, "name"),
    verificationCriteria: text(values, "verificationCriteria"),
    dueAtMillis: millis(values, "dueAt"),
    classification: classification(values),
  };
}

// ---- People (slice 6D) ----------------------------------------------------

/** A Stakeholder: the kind is chosen on a create and fixed after. */
export function stakeholderFieldSpecs(t: Translator, create: boolean): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "name",
      label: t("entry.field.name"),
      required: true,
      limit: TEXT_LIMITS.name,
    },
    ...(create
      ? [
          {
            kind: "choice" as const,
            name: "kind",
            label: t("entry.field.stakeholderKind"),
            emptyLabel: t("entry.stakeholderKind.none"),
            options: [
              { value: "person", label: t("people.kind.person") },
              { value: "organization", label: t("people.kind.organization") },
            ],
          },
        ]
      : []),
    { kind: "classification", name: "classification" },
  ];
}

export function stakeholderValues(
  values: SubmittedValues,
): StakeholderFields & { readonly kind: string } {
  return {
    name: text(values, "name"),
    kind: text(values, "kind"),
    classification: classification(values),
  };
}

/** The Stakeholders a request may name as its intended owner, as loaded. */
export interface OwnerCandidate {
  readonly id: string;
  readonly name: string;
}

function ownerField(t: Translator, owners: readonly OwnerCandidate[], label: string): FieldSpec {
  return {
    kind: "choice",
    name: "owner",
    label,
    required: false,
    emptyLabel: t("entry.owner.none"),
    options: owners.map((owner) => ({ value: owner.id, label: owner.name })),
  };
}

/** Risk and Issue never offer Unclassified; a request does not either — the
 * domain folds a request's classification into commitments, where
 * Unclassified is refused, so a draft that could never be submitted is not
 * offered. */
const WORK_CHOICES = { choices: "work" as const };

export function actionRequestFieldSpecs(
  t: Translator,
  owners: readonly OwnerCandidate[],
  timeZone: string,
): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "title",
      label: t("entry.field.title"),
      required: true,
      limit: TEXT_LIMITS.title,
    },
    { kind: "long", name: "details", label: t("entry.field.details"), required: true },
    ownerField(t, owners, t("entry.field.intendedOwner")),
    {
      kind: "datetime",
      name: "responseDueAt",
      label: t("entry.field.responseDueAt"),
      timeZone,
      required: false,
    },
    {
      kind: "datetime",
      name: "intendedActionDueAt",
      label: t("entry.field.intendedActionDueAt"),
      timeZone,
      required: false,
    },
    { kind: "classification", name: "classification", ...WORK_CHOICES },
  ];
}

export function decisionRequestFieldSpecs(
  t: Translator,
  owners: readonly OwnerCandidate[],
): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "subject",
      label: t("entry.field.subject"),
      required: true,
      limit: TEXT_LIMITS.title,
    },
    {
      kind: "long",
      name: "details",
      label: t("entry.field.details"),
      required: true,
      limit: TEXT_LIMITS.deliveryDetail,
    },
    ownerField(t, owners, t("entry.field.intendedOwner")),
    { kind: "classification", name: "classification", ...WORK_CHOICES },
  ];
}

export function issueFieldSpecs(t: Translator): readonly FieldSpec[] {
  return [
    {
      kind: "short",
      name: "title",
      label: t("entry.field.title"),
      required: true,
      limit: TEXT_LIMITS.title,
    },
    { kind: "long", name: "details", label: t("entry.field.details"), required: true },
    { kind: "classification", name: "classification", ...WORK_CHOICES },
  ];
}

export function riskFieldSpecs(t: Translator): readonly FieldSpec[] {
  return issueFieldSpecs(t);
}

export const RISK_RESPONSES = ["mitigate", "accept", "transfer", "avoid"] as const;

export function riskResponseFieldSpecs(
  t: Translator,
  owners: readonly OwnerCandidate[],
  timeZone: string,
): readonly FieldSpec[] {
  return [
    {
      kind: "choice",
      name: "response",
      label: t("entry.field.response"),
      emptyLabel: t("entry.response.none"),
      options: RISK_RESPONSES.map((response) => ({
        value: response,
        label: t(`entry.response.${response}`),
      })),
    },
    ownerField(t, owners, t("entry.field.riskOwner")),
    { kind: "long", name: "rationale", label: t("entry.field.rationale"), required: false },
    {
      kind: "short",
      name: "residualExposure",
      label: t("entry.field.residualExposure"),
      required: false,
      limit: TEXT_LIMITS.title,
    },
    {
      kind: "datetime",
      name: "nextReviewAt",
      label: t("entry.field.nextReviewAt"),
      timeZone,
      required: false,
    },
  ];
}

/** Accept and transfer need the owner, rationale, residual exposure and next
 * review (the domain refuses otherwise); said before the round trip. */
export function riskResponseProblem(t: Translator, values: SubmittedValues): string | null {
  const settled = values.response === "accept" || values.response === "transfer";
  if (!settled) {
    return null;
  }
  const complete =
    text(values, "owner") !== "" &&
    text(values, "rationale") !== "" &&
    text(values, "residualExposure") !== "" &&
    typeof values.nextReviewAt === "number";
  return complete ? null : t("entry.response.settledNeedsAll");
}

function optionalText(values: SubmittedValues, name: string): string | null {
  const value = text(values, name);
  return value === "" ? null : value;
}

function optionalMillis(values: SubmittedValues, name: string): number | null {
  const value = values[name];
  return typeof value === "number" ? value : null;
}

export function actionRequestValues(values: SubmittedValues): ActionRequestFields {
  return {
    title: text(values, "title"),
    details: text(values, "details"),
    intendedOwnerId: optionalText(values, "owner"),
    responseDueAtMillis: optionalMillis(values, "responseDueAt"),
    intendedActionDueAtMillis: optionalMillis(values, "intendedActionDueAt"),
    classification: classification(values),
  };
}

export function decisionRequestValues(values: SubmittedValues): DecisionRequestFields {
  return {
    subject: text(values, "subject"),
    details: text(values, "details"),
    intendedOwnerId: optionalText(values, "owner"),
    classification: classification(values),
  };
}

export function issueValues(values: SubmittedValues): IssueFields {
  return {
    title: text(values, "title"),
    details: text(values, "details"),
    classification: classification(values),
    recurrenceOfId: null,
  };
}

export function riskValues(values: SubmittedValues): RiskFields {
  return {
    title: text(values, "title"),
    details: text(values, "details"),
    classification: classification(values),
  };
}

export function riskResponseValues(values: SubmittedValues): RiskResponseFields {
  return {
    response: text(values, "response"),
    ownerId: optionalText(values, "owner"),
    rationale: optionalText(values, "rationale"),
    residualExposure: optionalText(values, "residualExposure"),
    nextReviewAtMillis: optionalMillis(values, "nextReviewAt"),
  };
}

export interface LinkCandidate {
  readonly id: string;
  readonly name: string;
  readonly classification: string;
  readonly version: number;
}

/**
 * What a link will record (§3.3): the combine of the anchor's own
 * classification and the chosen candidate's, in words, once one is chosen.
 */
export function linkClassificationPreview(
  t: Translator,
  anchorClassification: string,
  candidates: readonly LinkCandidate[],
  values: SubmittedValues,
): string | null {
  const chosen = candidates.find((candidate) => candidate.id === values.record);
  return chosen === undefined
    ? null
    : t("entry.link.classification", {
        classification: classificationName(
          t,
          combineClassification(anchorClassification, chosen.classification),
        ),
      });
}

export function linkFieldSpec(t: Translator, candidates: readonly LinkCandidate[]): FieldSpec {
  return {
    kind: "choice",
    name: "record",
    label: t("entry.field.record"),
    emptyLabel: candidates.length === 0 ? t("entry.candidates.none") : "",
    options: candidates.map((candidate) => ({
      value: candidate.id,
      label: t("entry.candidate", {
        name: candidate.name,
        classification: classificationName(t, candidate.classification),
        version: String(candidate.version),
      }),
    })),
  };
}

function text(values: SubmittedValues, name: string): string {
  const value = values[name];
  return typeof value === "string" ? value : "";
}

function classification(values: SubmittedValues): string | null {
  const value = values.classification;
  return typeof value === "string" && value !== "" ? value : null;
}

export function simpleValues(values: SubmittedValues): SimpleFields {
  return {
    name: text(values, "name"),
    details: text(values, "details"),
    classification: classification(values),
  };
}

export function kpiValues(values: SubmittedValues): KpiDefinitionFields {
  return {
    name: text(values, "name"),
    definition: text(values, "definition"),
    owner: text(values, "owner"),
    target: text(values, "target"),
    cadence: text(values, "cadence"),
    source: text(values, "source"),
    classification: classification(values),
  };
}

export function observationValues(values: SubmittedValues): KpiObservationFields {
  const observed = values.observedAt;
  return {
    value: text(values, "value"),
    observedAtMillis: typeof observed === "number" ? observed : 0,
    source: text(values, "source"),
    classification: classification(values),
  };
}

/** The values an edit sheet opens with, from the record as the host read it. */
export function initialValues(record: EntryRecordDto, timeZone: string): FieldValues {
  switch (record.kind) {
    case "portfolio":
    case "product":
    case "roadmap":
      return {
        name: record.name,
        details: record.details,
        classification: record.classification,
      };
    case "kpi_definition":
      return {
        name: record.name,
        definition: record.definition,
        owner: record.owner,
        target: record.target,
        cadence: record.cadence,
        source: record.source,
        classification: record.classification,
      };
    case "kpi_observation":
      return {
        value: record.value,
        observedAt: utcMillisToLocal(record.observedAtMillis, timeZone, true),
        source: record.source,
        classification: record.classification,
      };
    case "initiative":
      return {
        name: record.name,
        definedOutcome: record.definedOutcome,
        classification: record.classification,
      };
    case "project":
      return {
        name: record.name,
        startAt: utcMillisToLocal(record.startAtMillis, timeZone, true),
        endAt: utcMillisToLocal(record.endAtMillis, timeZone, true),
        classification: record.classification,
      };
    case "milestone":
      return {
        name: record.name,
        verificationCriteria: record.verificationCriteria,
        dueAt: utcMillisToLocal(record.dueAtMillis, timeZone, true),
        classification: record.classification,
      };
    case "stakeholder":
      return { name: record.name, classification: record.classification };
  }
}

const KINDS = [
  "portfolio",
  "product",
  "roadmap",
  "kpi_definition",
  "kpi_observation",
  "initiative",
  "project",
  "milestone",
  "stakeholder",
  "relationship",
  "action_request",
  "decision_request",
  "risk",
  "issue",
] as const;

type KnownKind = (typeof KINDS)[number];

function isKnownKind(kind: string): kind is KnownKind {
  return (KINDS as readonly string[]).includes(kind);
}

/** The kind of a saved record in words; the kind as sent when unknown. */
export function kindWord(t: Translator, kind: string): string {
  return isKnownKind(kind) ? t(`entry.kind.${kind}`) : kind;
}

/**
 * The records a link sheet may offer, from a listing: everything but an
 * observation (which is never linked), less the ids already present.
 */
export function linkCandidates(
  records: readonly EntryRecordDto[],
  present: ReadonlySet<string>,
): LinkCandidate[] {
  const candidates: LinkCandidate[] = [];
  for (const record of records) {
    if (
      record.kind !== "kpi_observation" &&
      record.kind !== "milestone" &&
      !present.has(record.id)
    ) {
      candidates.push({
        id: record.id,
        name: record.name,
        classification: record.classification,
        version: record.version,
      });
    }
  }
  return candidates;
}
