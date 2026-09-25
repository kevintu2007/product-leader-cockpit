import type { ListableKind } from "./entryIpc";

/**
 * The subjects a Stakeholder may be responsible for or depend on, as the
 * domain names them (`StakeholderSubject`), less Milestone: no listing of
 * Milestones exists yet, so none can be offered honestly.
 */
export const SUBJECT_KINDS = [
  "portfolio",
  "product",
  "initiative",
  "project",
  "roadmap",
  "kpi_definition",
] as const satisfies readonly ListableKind[];

export type SubjectKind = (typeof SUBJECT_KINDS)[number];

/** `responsibility` or `dependency`, as the domain persists them. */
export type Purpose = "responsibility" | "dependency";

export function isSubjectKind(value: string): value is SubjectKind {
  return (SUBJECT_KINDS as readonly string[]).includes(value);
}
