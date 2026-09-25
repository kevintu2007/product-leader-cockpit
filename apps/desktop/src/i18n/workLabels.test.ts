import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { translatorFor } from "./catalogs";
import { EN } from "./catalogs/en";
import { COCKPIT_SENTENCE_KEYS, freshnessLabel, intentLabel } from "./workLabels";

/** One label group of the English catalog, keyed by the host identifier. The
 * other catalogs carry exactly the same keys (`catalogs.test.ts`). */
function group(name: string): Readonly<Record<string, string>> {
  const prefix = `label.${name}.`;
  return Object.fromEntries(
    Object.entries(EN)
      .filter(([key]) => key.startsWith(prefix))
      .map(([key, words]) => [key.slice(prefix.length), words]),
  );
}

/**
 * The words must cover every identifier the host can actually send. Rather
 * than restating the host's lists here (a second copy that drifts), these
 * tests read the Rust sources that define them, so an intent, reason or tier
 * added on the host fails this suite until a person gives it a word.
 */
function rust(path: string): string {
  return readFileSync(path, "utf8");
}

function quotedSnakeCase(source: string): string[] {
  return [...source.matchAll(/"([a-z]+(?:_[a-z]+)+)"/g)].map((match) => match[1] ?? "");
}

describe("work labels", () => {
  it("names every lifecycle intent the domain admits", () => {
    const stateIntents = quotedSnakeCase(rust("crates/pmc-domain/src/state_intents.rs"));
    const actions = rust("crates/pmc-domain/src/actions.rs");
    const requestTable = actions.slice(
      actions.indexOf("pub const fn request_allowed_intents"),
      actions.indexOf("pub const fn action_allowed_intents"),
    );
    const actionTable = actions.slice(
      actions.indexOf("pub const fn action_allowed_intents"),
      actions.indexOf("}", actions.indexOf("ActionState::Completed | ActionState::Cancelled")) + 1,
    );
    const intents = new Set([
      ...stateIntents,
      ...quotedSnakeCase(requestTable),
      ...quotedSnakeCase(actionTable),
    ]);

    expect(intents.size).toBeGreaterThan(10);
    for (const intent of intents) {
      expect(group("intent"), `intent ${intent} has no word`).toHaveProperty(intent);
    }
  });

  it("names every attention reason", () => {
    const attention = rust("crates/pmc-domain/src/attention.rs");
    const reasons = [...attention.matchAll(/Self::[A-Za-z]+ => "([a-z_]+)",/g)].map(
      (match) => match[1] ?? "",
    );

    expect(reasons.length).toBeGreaterThan(20);
    for (const reason of reasons) {
      expect(group("reason"), `reason ${reason} has no word`).toHaveProperty(reason);
    }
  });

  it("names every ranking tier by its stable identifier", () => {
    const ranking = rust("crates/pmc-application/src/attention_ranking.rs");
    const asStr = ranking.slice(ranking.indexOf("pub const fn as_str"));
    const tiers = [...asStr.matchAll(/Self::[A-Za-z]+ => "([A-Za-z]+)",/g)].map(
      (match) => match[1] ?? "",
    );

    expect(tiers).toHaveLength(6);
    for (const tier of tiers) {
      expect(group("tier"), `tier ${tier} has no word`).toHaveProperty(tier);
    }
  });

  it("names every lifecycle state the Work Queue reports", () => {
    const adapter = rust("crates/pmc-application/src/work_queue_adapter.rs");
    const states = [...adapter.matchAll(/State::[A-Za-z]+ => "([A-Za-z ]+)",/g)].map(
      (match) => match[1] ?? "",
    );

    expect(states.length).toBeGreaterThan(10);
    for (const state of states) {
      expect(group("state"), `state ${state} has no word`).toHaveProperty(state);
    }
  });

  it("names every persisted lifecycle state a write outcome can report", () => {
    const domain = rust("crates/pmc-domain/src/work_management.rs");
    const states = [
      ...domain.matchAll(
        /persisted_enum!\((?:ActionRequest|Action|DecisionRequest|Decision|Risk|Issue)State \{([^}]*)\}/g,
      ),
    ].flatMap((block) =>
      [...(block[1] ?? "").matchAll(/"([a-z_]+)"/g)].map((match) => match[1] ?? ""),
    );

    expect(states.length).toBeGreaterThan(15);
    for (const state of states) {
      expect(group("persistedState"), `state ${state} has no word`).toHaveProperty(state);
    }
  });

  describe("the O03 review sheet", () => {
    const commands = rust("apps/desktop/src-tauri/src/write_commands.rs");
    const domain = rust("crates/pmc-domain/src/work_management.rs");

    function between(source: string, start: string, end: string): string {
      const from = source.indexOf(start);
      const to = source.indexOf(end, from);
      expect(from, `anchor ${start}`).toBeGreaterThanOrEqual(0);
      expect(to, `anchor ${end}`).toBeGreaterThan(from);
      return source.slice(from, to);
    }

    function quoted(source: string): string[] {
      return [...source.matchAll(/"([a-z_]+)"/g)].map((match) => match[1] ?? "");
    }

    function covers(catalog: Readonly<Record<string, string>>, values: string[], min: number) {
      expect(values.length).toBeGreaterThanOrEqual(min);
      for (const value of values) {
        expect(catalog, `${value} has no word`).toHaveProperty(value);
      }
    }

    it("names every target kind and declared effect the host sends", () => {
      covers(group("targetKind"), quoted(between(commands, "fn target_dto", "fn effect_dto")), 14);
      covers(group("effect"), quoted(between(commands, "fn effect_dto", "fn source_role")), 34);
    });

    it("names every prepared intent type", () => {
      const types = between(
        domain,
        'Self::AcceptActionRequest => "accept_action_request"',
        "pub fn from_persisted",
      );
      covers(group("effect"), quoted(types.slice(0, types.indexOf("}"))), 12);
    });

    it("names every classification source role", () => {
      covers(
        group("sourceRole"),
        quoted(between(commands, "fn source_role", "fn support_dto")),
        10,
      );
    });

    it("names every support disposition, Evidence role and verification state", () => {
      covers(
        group("disposition"),
        quoted(between(commands, "fn support_dto", "fn verification_dto")),
        4,
      );
      covers(
        group("evidenceRole"),
        quoted(between(commands, "fn evidence_role", "// Decision Requests")),
        5,
      );
      covers(
        group("verification"),
        quoted(between(domain, "pub const fn kind_as_persisted", "/// What a support witness")),
        5,
      );
    });

    describe("the Executive Lens", () => {
      const lens = rust("crates/pmc-application/src/executive_lens.rs");

      // These identifiers are camelCase, so the snake_case matcher would miss them.
      const quotedIdentifiers = (source: string) =>
        [...source.matchAll(/"([A-Za-z_]+)"/g)].map((match) => match[1] ?? "");

      it("names every timing state and quadrant the host sends", () => {
        covers(
          group("timing"),
          quotedIdentifiers(between(lens, "impl TimingState", "pub const fn is_high")),
          4,
        );
        covers(group("quadrant"), quotedIdentifiers(between(lens, "impl Quadrant", "\n}\n")), 4);
      });

      it("names every kind of record a measure can stand on", () => {
        const kinds = [...lens.matchAll(/kind: "([a-z_]+)"/g)].map((match) => match[1] ?? "");
        covers(group("contribution"), kinds, 7);
      });

      it("names every relationship kind the Lens walks, by its persisted name", () => {
        const relationships = rust("crates/pmc-domain/src/relationships.rs");
        const walked = new Set(
          [...lens.matchAll(/RelationshipKind::([A-Za-z]+)/g)].map((match) => match[1] ?? ""),
        );
        const persisted = [...walked].map((variant) => {
          const found = new RegExp(`Self::${variant} => "([a-z_]+)"`).exec(relationships);
          expect(found, `RelationshipKind::${variant} has no persisted name`).not.toBeNull();
          return found?.[1] ?? "";
        });
        covers(group("relationshipKind"), persisted, 2);
      });
    });

    it("words every fixed sentence the Cockpit shows", () => {
      const aggregation = rust("crates/pmc-application/src/cockpit_aggregation.rs");
      const pulse = between(aggregation, "pub fn portfolio_pulse", "pub fn period_change");
      // Anchored inside the function: the variant is named earlier in the file too.
      const period = between(
        aggregation.slice(aggregation.indexOf("pub fn period_change")),
        "PeriodReadiness::NoApprovedPeriod",
        "PeriodReadiness::Approved",
      );
      // Sentences only: the provenance field names beside them have no space.
      const sentences = [...(pulse + period).matchAll(/"([A-Za-z][^"]* [^"]*)"/g)].map(
        (match) => match[1] ?? "",
      );
      covers(COCKPIT_SENTENCE_KEYS, sentences, 4);
      for (const key of Object.values(COCKPIT_SENTENCE_KEYS)) {
        expect(group("cockpit"), `${key} has no word`).toHaveProperty(key);
      }
    });

    it("names every Work Queue lifecycle type", () => {
      // The first `as_str` in the file is `WorkItemKind`'s.
      covers(
        group("workItemKind"),
        quoted(
          between(
            rust("crates/pmc-application/src/work_queue_composition.rs"),
            "pub const fn as_str(self)",
            "\n    }",
          ),
        ),
        5,
      );
    });

    it("names every owner module a record can report", () => {
      covers(
        group("owner"),
        quoted(
          between(
            between(
              rust("crates/pmc-application/src/route_composition.rs"),
              "impl OwnerModule",
              "\n}\n",
            ),
            "pub const fn as_str(self)",
            // The function's own closing brace; the outer slice already stops
            // before the impl's, so no trailing newline is left to match.
            "\n    }",
          ),
        ),
        10,
      );
    });

    it("names every Data Classification the domain persists", () => {
      covers(
        group("classification"),
        quoted(
          between(
            rust("crates/pmc-domain/src/classification.rs"),
            "pub const fn as_persisted",
            "pub fn from_persisted",
          ),
        ),
        5,
      );
    });

    it("names every Issue resolution type and every policy value", () => {
      covers(
        group("resolutionType"),
        quoted(between(domain, "persisted_enum!(IssueResolutionType", "});")),
        3,
      );
      covers(
        group("policy"),
        quoted(between(commands, "policy_result: match", "correlation_id: correlation")),
        3,
      );
    });
  });

  it("falls back to the identifier itself rather than rendering nothing", () => {
    const zh = translatorFor("zh-TW");
    expect(intentLabel(zh, "some_future_intent")).toBe("some_future_intent");
    expect(freshnessLabel(zh, "fresh")).toBe("最新");
    expect(freshnessLabel(translatorFor("en"), "fresh")).toBe("current");
  });
});
