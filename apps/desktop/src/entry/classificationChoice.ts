/**
 * Classification is chosen, never defaulted (DG3 record-entry amendment
 * §3.3): a create sheet opens with nothing selected and cannot submit until
 * a value is chosen. Risk and Issue never offer Unclassified — the domain
 * refuses it — so the set of choices depends on what is being created.
 */

export type Classification = "public" | "internal" | "confidential" | "restricted" | "unclassified";

/** Nothing chosen yet: the sheet's initial state, never sent to the host. */
export type ClassificationChoice = Classification | undefined;

const EVERY_CHOICE: readonly Classification[] = [
  "public",
  "internal",
  "confidential",
  "restricted",
  "unclassified",
];

/** What the sheet offers: `work` (Risk, Issue) leaves Unclassified out. */
export function classificationChoices(kind: "general" | "work"): readonly Classification[] {
  return kind === "work" ? EVERY_CHOICE.filter((value) => value !== "unclassified") : EVERY_CHOICE;
}

/** A submit is possible only once a real choice was made. */
export function isChosen(choice: ClassificationChoice): choice is Classification {
  return choice !== undefined;
}

const RESTRICTION_RANK: Readonly<Record<Classification, number>> = {
  public: 0,
  internal: 1,
  confidential: 2,
  restricted: 3,
  unclassified: 4,
};

function isClassification(value: string): value is Classification {
  return Object.hasOwn(RESTRICTION_RANK, value);
}

/**
 * The classification a link between two records will record, as the domain
 * combines them (`DataClassification::combine`): Unclassified on either
 * side wins, otherwise the more restrictive. A link sheet shows this before
 * submit (§3.3); the host computes and stores its own.
 */
export function combineClassification(a: string, b: string): string {
  if (!isClassification(a) || !isClassification(b)) {
    return "unclassified";
  }
  if (a === "unclassified" || b === "unclassified") {
    return "unclassified";
  }
  return RESTRICTION_RANK[a] >= RESTRICTION_RANK[b] ? a : b;
}
