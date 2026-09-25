# How this was built

[繁體中文](story.zh-TW.md)

My background is in tech marketing and product management, and in my company I am the person who
ties the products and projects together. Product Mission Control is the tool I built from the
perspective of a Head of Products: being responsible for a company's whole product portfolio means
keeping every product's and project's direction, outcomes, delivery commitments, decisions, risks
and the evidence behind every judgment in view at once. It was developed in my private repository
over the six weeks before this release, with each mechanism refined and verified along the way, and
then consolidated into this public repository so that others can see how it works and how it was
made.

## Why I built it

My job is to connect portfolio direction, product outcomes, delivery commitments, decisions,
risks and the evidence behind all of them. In practice those facts live in task trackers,
documents, dashboards and meeting notes. The picture goes stale, and status reporting eats the time
that should go into decisions.

I wanted one local workspace that does for a portfolio what an X-ray does for a doctor: it shows
where to look, ranks what needs attention, and says why. It does not make the diagnosis. Every
material change passes through an exact preview that I approve or reject, and every conclusion is
backed either by Evidence or by a Judgment I write down and own.

Three rules followed from that:

- The system surfaces and ranks; the person decides. Nothing is approved on my behalf.
- Nothing disappears silently. A rejection is recorded; closing a review without deciding is not.
- There is always a way back. Strict rules must never deadlock the work, so partly verified Evidence
  can move forward with a written Judgment, while unverified Evidence cannot.

## Thinking first, then building

I began by thinking the design and architecture through, and let that guide the development: what
the product is for, where the privacy boundaries sit, what the domain terms mean, how the
architecture is layered, how errors are handled, which commands verify the work and how the review
process would work. In the first week I put that foundation in place, using
[Project Kickoff Foundation](https://github.com/kevintu2007/project-kickoff-foundation), an agent
skill I had published earlier for exactly this step, with every item marked confirmed, proposed or
open. Everything built afterwards followed it.

Design came next, as a series of gates: a concept, then a design brief with an interactive
prototype, then a UI contract, frozen on 17 August 2026 before the first line of application code.

<p align="center">
  <img src="images/design-portfolio-command.png" width="820" alt="Design prototype of the Portfolio command screen: a Portfolio Lens comparing Products, an owner work list ranked by attention, and every measure carrying its definition">
</p>
<p align="center"><sub>An early design specimen of the Portfolio screen. Every number carries its definition, and nothing is ranked without a stated reason.</sub></p>

<table>
<tr>
<td width="50%"><img src="images/design-cockpit-prototype.png" alt="Interactive design prototype of the Executive Cockpit"></td>
<td width="50%"><img src="../user-guide/images/en/01-cockpit.png" alt="The shipped Executive Cockpit on synthetic data"></td>
</tr>
<tr>
<td><sub>The interactive prototype of the Executive Cockpit.</sub></td>
<td><sub>The Cockpit as shipped. The Lens axes changed after I found the original "decision pressure" axis had no honest per-Product source in the data.</sub></td>
</tr>
</table>

That last change is typical of the project. When a frozen design asked for something the domain
could not truthfully compute, the design was amended, in writing, instead of the software
inventing a number.

## How I work with AI agents

I direct the work; AI agents write most of the code. The arrangement is described in detail in
[Working with AI agents](working-with-ai-agents.md). In short:

- **Claude Code** is the orchestrator. It plans and implements one slice at a time, test first.
- **Codex** is the independent reviewer. A standard model reviews each change; a stronger model is
  brought in for architecture and mechanism design. A writer never accepts its own work.
- Local models handle bounded, low-risk tasks.
- I am the only person who can decide. I set each goal, answer every question that changes a rule,
  and accept every user-facing slice only after looking at screenshots of the real app.

The rules that keep this honest are short: the foundation is frozen and only filled in; two
consecutive failed test runs stop the work and hand it back to me; agents do not open issues or
widen scope on their own; and nothing is called done because its structure looks complete.

## What the review gates caught

The reviews were not a formality. A few of the findings, all fixed before the work moved on:

- **13 August, design review.** A report being written in the prototype would lose unsaved text
  when the screen re-rendered.
- **27 August, integration review.** Re-resolving an Issue appended stale Evidence, and replayed
  audit events carried the wrong correlation id. After the fix, reverting it made the new test fail
  again, which proved the fix was load-bearing.
- **31 August, review before closing a milestone.** Replaying an approved change read the record as
  it is now instead of as it was when approved, which broke restarts after a later edit. A follow-up
  audit found the same gap in seven of nine record families. All were fixed.
- **5 September, audit of the managed projections.** An approved hash was compared to nothing, and
  one "synchronised" state could not be reached by any shipped code path. That audit gave the
  project its most important rule: never claim a requirement is met because its shape exists;
  confirm that real code both produces and enforces it. The fix re-derives the expected output and
  compares it as a whole.
- **17 September, preparing this repository.** Capturing the tutorial screenshots exposed a
  screen that kept stale numbers after a write, and the export review found internal task
  identifiers throughout the code comments. Both were fixed before publication.

## Hard problems

The review findings above were about correctness. The problems below took design work: each was
evaluated by the reviewer first, decided by me in writing, and then built.

- **Replay as approved, not as now (31 August – 2 September).** Approving a prepared change must
  re-derive the whole snapshot and compare it with the preview as one thing. The first
  implementations replayed the record as it is now; an audit found the gap in seven of nine record
  families, and the fix became the rule that every later mechanism follows.
- **A ranking you can explain (September).** The Work Queue uses six named tiers, then the
  earliest deadline, then kind and identifier. There is no weighted score, so every position can be
  stated in one sentence, and adding a new attention reason does not compile until it is given a
  tier. The [attention ranking](../policies/attention-ranking.md) policy is the written form.
- **Verification three times faster (19 September).** Every write transaction rebuilt the
  canonical schema from all 46 migrations before checking the live one. Computing it once per
  process, and building SQLite optimised in test profiles, took four representative test binaries
  from 914 s to 294 s; the full `npm run verify` went from about 155 minutes to between 36 and 46.
  Measured first, decided after.
- **History that survives its Evidence (19 September).** An Issue's executed history was rebuilt
  from the Evidence as it stands now, so Evidence that later lost its verification made the whole
  Issue snapshot fail to load. History now reads the prepared-intent snapshot taken when the
  transition was prepared; two tests fail on the old decoder.
- **Creates that survive a restart (22 September).** A create reserves its identifier durably under
  the sheet's request id in its own transaction, so a retry from another launch names the same
  record instead of a second one. The proof that a reservation happened is a type nothing outside
  the Ledger crate can construct.
- **The backup gate (22 September).** In your own workspace, ordinary writes are refused until an
  encrypted backup has verified, and the gate must be held across the whole transaction. The first
  design deadlocked its own tests: the rule that stops work after two consecutive failures stopped it
  and handed it back. The redesign, evaluated with the reviewer, splits an admission lock from a
  short state lock and hands each backup run a ticket that releases itself.
- **Restore without `unsafe` (22 September).** Replacing a live SQLite file safely on Windows was
  designed as a two-step rename with every phase journaled in a control record; start-up finishes
  or rolls back whatever a crash interrupted, and never opens a half-replaced Ledger. Four review
  rounds: eight findings on the first, then a rollback phase that was not compare-and-set and a
  start-up path that could open a half-replaced file.
- **Upgrade in place (22 September).** An older Ledger is inspected read-only, backed up and
  verified as a closed file, then upgraded in one transaction with foreign keys off before `BEGIN`
  and a foreign-key check before commit. The review found a forgeable proof of that backup and a
  receipt that could be replayed; both were closed before the screen was built.
- **Paths never cross the boundary (21–23 September).** File and folder pickers are opened by the
  host; the renderer receives an opaque, expiring token and a name to show. Path containment refuses
  links and reparse points, and the one `unsafe` call in the codebase compares path names the way
  Windows does. What remains open is written down: a time-of-check window that only the Windows API
  can close is listed as a known limitation rather than hidden.

## From the private repository to this one

The private repository keeps the full history, the agent instructions and the review records. This
repository is produced from it by a one-way export: an allowlist of product files, documentation
rewritten against the code as it is today, and a scan that refuses any file carrying private paths,
addresses or internal references. Before publishing, the code comments were revised so each one
states its requirement in words rather than pointing at an internal task.

It is a beta: the first version with an installer, encrypted backups and restore, ready to hold real
records. The review periods, fact packs and approved report snapshots in the design are not built yet,
and no AI assistant is connected inside the app. The [README](../../README.md) lists exactly what
works today.

<p align="center">
  <img src="images/design-weekly-review.png" width="820" alt="Design prototype of the Weekly Review flow: exception disposition, a validated fact pack, human report authoring, an exact-preview approval and immutable snapshots">
</p>
<p align="center"><sub>Designed, not yet built: the Weekly Review flow, from exceptions to a validated fact pack, a report I write myself, an exact-preview approval and immutable snapshots.</sub></p>

## Timeline

| Date (2026) | Milestone |
| --- | --- |
| 10 Aug | Foundation, domain language and first design specifications |
| 12–15 Aug | Design brief and interactive prototype; UI contract frozen; agent roles and review routing set up |
| 17 Aug | First application code: the verified Tauri desktop workspace |
| 18 Aug | Product Ledger on SQLite, failing closed from its first schema |
| 24–29 Aug | Durable lifecycles, real writers for every command, the Evidence and Vault layer |
| 31 Aug – 2 Sep | Approval, completion and supersession flows; review found and fixed replay gaps |
| 3–7 Sep | First end-to-end reads through the desktop; Cockpit, Portfolio, People and Work Queue |
| 8–12 Sep | Synthetic training data, the governed write path in the UI, Risk and Issue lifecycles |
| 14–17 Sep | Interface rebuilt to the design, the first user guide, and the first public export |
| 19 Sep | Six interface languages; Risks by title in the Work Queue |
| 21–24 Sep | Encrypted backups, restore and in-place Ledger upgrade; record entry; the Vault folder and Evidence from a file; the sample workspace; the Windows installer; the Getting started list |

## By the numbers

Measured on the private repository on 24 September 2026:

- 472 commits over 46 days
- About 113,000 lines of Rust and 80,000 lines of Rust tests in 152 test files; about 36,000 lines
  of TypeScript with 41 test files
- 1,540 Rust tests and 432 frontend tests, all run by `npm run verify`
- 48 forward-only Ledger schema versions
- 108 IPC commands between the window and the Rust host
- 1,383 message keys in the English catalogs, carried into five more languages
- 27 development-journal entries, 17 specifications, 12 design reviews and 14 decision records in
  the private repository; 11 of the decision records are published here

| Week (2026) | Commits |
| --- | --- |
| 10–16 August | 34 |
| 17–23 August | 58 |
| 24–30 August | 138 |
| 31 August – 6 September | 100 |
| 7–13 September | 37 |
| 14–20 September | 23 |
| 21–24 September | 82 |

## Author

Created and maintained by **Kevin Yu-chang Tu**<br>
**Polatouche.K** · [@kevintu2007](https://github.com/kevintu2007) · [LinkedIn](https://www.linkedin.com/in/kevin-yu-chang-tu-a19b8469)
