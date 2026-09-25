/**
 * Words for the identifiers the host sends about records.
 *
 * The host speaks in stable identifiers -- a lifecycle intent
 * (`prepare_accept_action_request`), an attention reason
 * (`action_request_response_overdue`), a ranking tier (`breachedCommitment`),
 * a state (`Open`). Those identifiers are contracts and must not be reworded;
 * a person should never have to read them. The words live in the catalogs
 * (`catalogs/labels.*.ts`, keys `label.<group>.<identifier>`); these helpers
 * look them up in the active language.
 *
 * Every lookup falls back to the identifier itself, so an identifier added on
 * the host without a word is visible rather than silently blank -- and
 * `workLabels.test.ts` reads the Rust sources to fail the build first.
 */
import type { Translator } from "./messages";

function word(t: Translator, group: string, identifier: string): string {
  return t.lookup(`label.${group}.${identifier}`, {}) ?? identifier;
}

export const intentLabel = (t: Translator, intent: string) => word(t, "intent", intent);
export const stateLabel = (t: Translator, state: string) => word(t, "state", state);
export const persistedStateLabel = (t: Translator, state: string) =>
  word(t, "persistedState", state);
export const reasonLabel = (t: Translator, reason: string) => word(t, "reason", reason);
export const tierLabel = (t: Translator, tier: string) => word(t, "tier", tier);

/** Where an item was placed: one of the tiers, or below all of them. */
export const placementLabel = (t: Translator, placement: string) =>
  t.lookup(`label.tier.${placement}`, {}) ?? word(t, "placement", placement);

export const ownerLabel = (t: Translator, owner: string) => word(t, "owner", owner);

/** Classification names in full English, as the design system requires. */
export const classificationName = (t: Translator, classification: string) =>
  word(t, "classification", classification);
export const freshnessLabel = (t: Translator, freshness: string) => word(t, "freshness", freshness);
export const targetKindLabel = (t: Translator, kind: string) => word(t, "targetKind", kind);
export const workItemKindLabel = (t: Translator, kind: string) => word(t, "workItemKind", kind);
export const timingStateLabel = (t: Translator, state: string) => word(t, "timing", state);
export const quadrantLabel = (t: Translator, quadrant: string) => word(t, "quadrant", quadrant);

/** The host's fixed Cockpit sentences arrive as English text; each has a
 * stable key here so every language can word it. */
export const COCKPIT_SENTENCE_KEYS: Readonly<Record<string, string>> = {
  "Milestones currently tracked across the Portfolio": "milestonesTracked",
  "Accepted Actions. A submitted Action Request is not counted until it is accepted":
    "acceptedActions",
  "KPIs with a definition in the Ledger": "kpisDefined",
  "no review period has been approved yet, so there is nothing to compare against":
    "noApprovedPeriod",
};

export function cockpitSentence(t: Translator, sentence: string): string {
  const key = Object.hasOwn(COCKPIT_SENTENCE_KEYS, sentence)
    ? COCKPIT_SENTENCE_KEYS[sentence]
    : undefined;
  return key === undefined ? sentence : word(t, "cockpit", key);
}

/** A Lens contribution in words: a relationship by what it relates. */
export function contributionLabel(t: Translator, kind: string, role: string): string {
  return kind === "relationship"
    ? word(t, "relationshipKind", role)
    : word(t, "contribution", kind);
}

export const effectLabel = (t: Translator, effect: string) => word(t, "effect", effect);
export const sourceRoleLabel = (t: Translator, role: string) => word(t, "sourceRole", role);
export const verificationLabel = (t: Translator, kind: string) => word(t, "verification", kind);
export const evidenceRoleLabel = (t: Translator, role: string) => word(t, "evidenceRole", role);
export const dispositionLabel = (t: Translator, disposition: string) =>
  word(t, "disposition", disposition);
export const resolutionTypeLabel = (t: Translator, type: string) => word(t, "resolutionType", type);
export const policyLabel = (t: Translator, value: string) => word(t, "policy", value);
