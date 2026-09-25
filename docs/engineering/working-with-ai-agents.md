# Working with AI agents

[How this was built](story.md) tells the story. This page describes the operating model: who does
what, which rules apply, and what has to be true before a change is accepted.

## Roles

| Role | Who | Responsibility |
| --- | --- | --- |
| Product owner | Me | Sets each goal, decides every question that changes a rule or a frozen contract, and accepts user-facing work from screenshots of the real app |
| Orchestrator | Claude Code | Reads the foundation, plans one slice at a time, writes tests first, implements, runs verification, reports honestly |
| Reviewer | Codex | Independent review of every slice before it is accepted; evaluation of mechanism and architecture designs before I decide |
| Bounded workers | Local models, other hosted models | Narrow, low-risk tasks with a clear output contract |

The roles changed once. In the first two weeks Codex orchestrated and Claude Code reviewed with a
fresh, read-only context. From 26 August the roles swapped, and from 5 September the orchestrator
ran on Claude Opus with Codex reviews tiered by risk. The rule that survived every change: a writer
never accepts its own work.

## Routing by risk

Work goes to the cheapest capable tool first and escalates only with a reason:

1. Deterministic tools: compilers, linters, formatters, test runners, policy scripts.
2. Local models for bounded drafting and classification.
3. A standard hosted model for ordinary review.
4. A stronger model for architecture, mechanism design and cross-layer impact, with the reason
   stated each time.

Every call to a reviewer names its model explicitly. A missing model flag once silently fell back
to a global default, so the default is never trusted.

## Rules

- **Frozen foundation.** The domain model, schema, core interfaces and directory layout are defined
  and frozen. Agents fill in behaviour; they do not restructure. A conflict with a frozen contract
  stops the work and comes to me.
- **One slice at a time, test first.** Each slice is a single, reviewable change with its own tests.
- **Hard stop after two failures.** Two consecutive failed test runs or builds end the attempt. The
  agent keeps the failing state and hands control back instead of escalating to a bigger model or
  spawning more agents.
- **No self-authorised scope.** Agents do not open issues, write decision records or split work into
  new tasks without my explicit approval.
- **Never claim done on shape alone.** A schema value, a status, a field or a documented
  precondition counts only when a real code path both produces and enforces it. Where possible the
  expected result is re-derived and compared as a whole instead of checking one hash or flag.
- **Report honestly.** What was verified and what was not is stated in those words. A failed check
  is reported with its output; a skipped step is named.

## Gates a change passes

1. Tests written and passing for the slice.
2. Local checks: formatting, lint, type checks, clippy with warnings as errors, the affected tests.
3. Independent review. Every finding is answered: fixed with evidence, or explained. At most two
   review passes per slice.
4. Full verification with `npm run verify`: documentation links, formatting, lint, type checks,
   frontend tests and build, the locked Rust build, clippy, the whole Rust test suite, dependency
   license evidence and security policy checks.
5. For anything a person sees: screenshots of the real app, reviewed and accepted by me.
6. Only then a commit on the main branch.

## Decisions stay with a person

When a design question has more than one reasonable answer, the agent does not choose. The reviewer
evaluates the options first, then the question comes to me with a recommendation. The decisions and
their reasons are recorded, and later work is checked against them.

The same rule shapes the product itself. Its approval flow asks a person to approve an exact
preview, records a rejection, and never treats closing a window as a decision. The way the software
was built and the way it behaves follow one idea: the system prepares and explains, a person decides.
