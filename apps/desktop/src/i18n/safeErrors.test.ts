import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { EN } from "./catalogs/en";

/**
 * Every dotted string the host's Rust code contains is either a word a person
 * may be shown -- an error `messageKey`, a field name, a next step -- or one of
 * the identifiers below that never reach a screen as text. Reading the sources
 * rather than restating them means a key the host gains fails this suite
 * until someone either gives it words or says here why it needs none.
 */
const NOT_MESSAGES = new Set([
  // The file names of the manifest and members inside an Operational Backup
  // archive.
  "manifest.json",
  "ledger.sqlite3",
  "settings.json",
  // The single-instance lock file in the protected root (item ⑩).
  "instance.lock",
  // Host audit event codes (ADR 0012): identifiers in the audit log, never
  // shown.
  "backup.bootstrap_empty_authority",
  "backup.completed",
  "backup.failed",
  // The Product Vault authority-path change (item ⑦, H2b).
  "authority.vault_root_prepared",
  "authority.vault_root_rejected",
  "authority.vault_root_changed",
  "authority.vault_root_not_changed",
  // The sample workspace's reset and delete (item ⑨): audit codes.
  "sample.reset",
  "sample.reset_rolled_back",
  "sample.delete_prepared",
  "sample.delete_rejected",
  "sample.deleted",
  "sample.not_deleted",
  "restore.prepared",
  "restore.rejected",
  "restore.started",
  "restore.finished",
  // Audit event codes, prepared-intent types, confirmation domain tags,
  // settings keys and projection field names: identifiers, never prose.
  "action.prepared_rejected",
  "decision.prepared_rejected",
  "issue.prepared_rejected",
  "risk.prepared_rejected",
  "action.approval_rejected",
  "action.cancelled",
  "action.classification_lowered",
  "action.completed",
  "action.completion_evidence_linked",
  "action.created_from_request",
  "action.evidence_policy_denied",
  "action.execution_failed",
  "action.reopened",
  "action.started",
  "action.superseded_premise_flagged",
  "action.superseded_premise_marked",
  "action_request.accepted",
  "action_request.action_linked",
  "action_request.created",
  "action_request.created_from_decision",
  "action_request.declined",
  "action_request.mutated",
  "action_request.submitted",
  "action_request.superseded_premise_flagged",
  "action_request.superseded_premise_marked",
  "action_request.withdrawn",
  "decision.action_request_linked",
  "decision.approval_rejected",
  "decision.classification_lowered",
  "decision.created",
  "decision.execution_failed",
  "decision.prepare_denied",
  "decision.replacement_created",
  "decision.replacement_linked",
  "decision.superseded",
  "decision_request.created",
  "decision_request.decision_linked",
  "decision_request.resolved",
  "decision_request.submitted",
  "decision_request.withdrawn",
  "evidence.fingerprint_pinned",
  "evidence.linked",
  "evidence.reference_created",
  "evidence.reference_relocated",
  "evidence.reference_superseded",
  "evidence.verification_updated",
  "initiative.classification_lowered",
  "initiative.created",
  "initiative.updated",
  "issue.classification_lowered",
  "issue.closed",
  "issue.created",
  "issue.created_from_risk",
  "issue.execute_denied",
  "issue.prepare_denied",
  "issue.reopened",
  "issue.resolved",
  "kpi.classification_lowered",
  "kpi.definition.created",
  "kpi.definition.updated",
  "kpi.observation.classification.inherited",
  "kpi.observation.classification_lowered",
  "kpi.observation.created",
  "kpi.observation.updated",
  "milestone.classification.inherited",
  "milestone.classification_lowered",
  "milestone.created",
  "milestone.updated",
  "portfolio.classification_lowered",
  "portfolio.created",
  "portfolio.updated",
  "product.classification_lowered",
  "product.created",
  "product.updated",
  "project.classification_lowered",
  "project.created",
  "project.updated",
  "relationship.initiative_project.linked",
  "relationship.portfolio_initiative.linked",
  "relationship.portfolio_product.linked",
  "relationship.product_kpi.linked",
  "relationship.product_roadmap.linked",
  "relationship.project_product.linked",
  "relationship.removal.approval_rejected",
  "relationship.removal.cancelled_before_approval",
  "relationship.removal.execution_rejected",
  "relationship.remove",
  "relationship.remove.confirmation.v1",
  "relationship.removed",
  "relationship.stakeholder.created",
  "relationship.stakeholder.updated",
  "relationship.stakeholder_relationship.reclassified",
  "relationship.stakeholder_subject.linked",
  "risk.classification_lowered",
  "risk.classification_lowering_denied",
  "risk.close_denied",
  "risk.closed",
  "risk.created",
  "risk.issue_linked",
  "risk.occurred",
  "risk.occurrence_denied",
  "risk.response_updated",
  "roadmap.classification_lowered",
  "roadmap.created",
  "roadmap.updated",
]);

const SOURCES = ["apps/desktop/src-tauri/src", "crates"];

function rustFiles(): string[] {
  return SOURCES.flatMap((root) =>
    readdirSync(root, { recursive: true, encoding: "utf8" })
      .filter((path) => path.endsWith(".rs"))
      .map((path) => join(root, path))
      .filter((path) => root !== "crates" || /^crates[\\/][^\\/]+[\\/]src[\\/]/.test(path))
      // A module's own tests.rs is compiled only under #[cfg(test)].
      .filter((path) => !/[\\/]tests\.rs$/.test(path)),
  );
}

/** Dotted literals in production code. A `#[cfg(test)]` module (the attribute
 * followed by `mod`) runs to the end of its file in this workspace and is
 * skipped; a `#[cfg(test)]` on a single import is not a cut. */
function hostLiterals(): Set<string> {
  const found = new Set<string>();
  for (const file of rustFiles()) {
    const source = readFileSync(file, "utf8");
    const cut = source.search(
      /^\s*#\[cfg\(test\)\]\s*\n(?:\s*#\[[^\n]*\]\s*\n)*\s*(?:pub(?:\(crate\))? )?mod /m,
    );
    const production = cut === -1 ? source : source.slice(0, cut);
    for (const match of production.matchAll(/"([a-z][a-z_]*(?:\.[a-z0-9_]+)+)"/g)) {
      found.add(match[1] ?? "");
    }
  }
  return found;
}

const WORDED_PREFIXES = ["safeError.", "field.", "nextStep."];

function worded(literal: string): boolean {
  return WORDED_PREFIXES.some((prefix) => Object.hasOwn(EN, `${prefix}${literal}`));
}

describe("host message keys", () => {
  const literals = hostLiterals();

  it("finds the host's keys at all", () => {
    expect(literals.size).toBeGreaterThan(200);
  });

  it("gives every key the host can send a word, or says why it needs none", () => {
    const unworded = [...literals].filter((key) => !worded(key) && !NOT_MESSAGES.has(key)).sort();
    expect(unworded).toEqual([]);
  });

  it("keeps no word for a key the host no longer sends", () => {
    const stale = Object.keys(EN)
      .filter((key) => WORDED_PREFIXES.some((prefix) => key.startsWith(prefix)))
      .map((key) => key.slice(key.indexOf(".") + 1))
      .filter((key) => !literals.has(key))
      .sort();
    expect(stale).toEqual([]);
  });

  it("keeps no exclusion for an identifier the host no longer has", () => {
    expect([...NOT_MESSAGES].filter((key) => !literals.has(key)).sort()).toEqual([]);
  });

  it("never both words and excludes the same key", () => {
    expect([...NOT_MESSAGES].filter(worded).sort()).toEqual([]);
  });
});
