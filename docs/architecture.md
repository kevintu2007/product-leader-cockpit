# Architecture

Product Mission Control is a local-first Windows desktop application for one Head of Products. It
keeps structured portfolio and work records in a local SQLite database (the Product Ledger), reads
Evidence files from a user-owned Markdown folder (the Product Vault), and presents both through a
Tauri 2 window with a React and TypeScript renderer. There is no server, no account system and no
synchronisation. All domain rules, persistence, classification and filesystem access live in Rust;
the renderer only presents what the host returns.

This document describes the code of version 0.2.0-beta. Several parts of the intended product are
not built yet; the last section lists them.

Related documents: [UI contract](ui-contract.md), [security model](security-model.md),
[domain glossary](domain-glossary.md), [attention ranking policy](policies/attention-ranking.md),
[Work Queue ordering policy](policies/work-queue-ordering.md) and the
[architecture decision records](decisions/README.md).

## Crate map

The Cargo workspace has five library crates, the desktop host and one tool. Every crate denies
`unwrap`/`expect` and forbids `unsafe` code, through crate attributes or the workspace lints, with one
reviewed exception: `crates/pmc-platform/src/windows_names.rs` calls the Windows
`CompareStringOrdinal` API to compare path names the way Windows does.

| Crate | Owns | Depends on |
| --- | --- | --- |
| `crates/pmc-domain` | Record types and lifecycles (Portfolio, Product, Roadmap, KPI, Initiative, Project, Milestone, Stakeholder, relationships, Action Request, Action, Decision Request, Decision, Risk, Issue, Evidence reference), data classification, attention derivation, lifecycle-legal next steps, prepared intents and approval validation, audit events and the safe error envelope | `sha2` only |
| `crates/pmc-ledger` | The Product Ledger: SQLite persistence per record family, the canonical schema and forward-only migration registry, idempotency, prepared intents, approval receipts, audit rows, the two read snapshots, read-only inspection of a Ledger file of any supported version, snapshots for backups, and in-place upgrade. Also an in-memory transactional composition of the same domain services | `pmc-domain`, bundled `rusqlite` |
| `crates/pmc-knowledge` | Product Vault root validation, Evidence observation (content fingerprint and verification state), Obsidian URI intents, and the deterministic Markdown projection generator | `pmc-domain`, `pmc-platform` |
| `crates/pmc-platform` | The app-data settings root and settings document, Training and Live workspace identity, contained path resolution, Windows path-name comparison, atomic managed-file publication, a narrow URI launcher, the Windows Credential Manager adapter used for a remembered recovery passphrase, the encrypted backup archive (age, tar, zstd), the verified-archive registry and the host audit log | settings, time, Windows and archive crates |
| `crates/pmc-application` | Use cases: record entry, the Action Request, Action, Decision Request, Risk and Issue flows, the Evidence write facade and Evidence from a file, the Product Vault folder change, Operational Backup, restore, Ledger upgrade, the sample workspace, attention ranking, and the compositions behind the Cockpit, Portfolio, People, Product detail and Work Queue. Also projection publication orchestration | all four crates above |
| `apps/desktop/src-tauri` | The Tauri host: resolves the workspace, holds the Ledger, mints identifiers and timestamps, opens native file and folder dialogs, keeps a single instance, and exposes the IPC commands | `pmc-application`, `pmc-domain`, `pmc-ledger`, `pmc-platform`, `rfd` |
| `apps/desktop/src` | The React renderer: routes, the navigation shell, overlays, localisation and presentation state | `@tauri-apps/api`, React |
| `tools/pmc-seed` | A developer command that rebuilds the synthetic sample (Training) workspace. The desktop prepares the same sample itself through `pmc-application`. It takes no path and refuses the Live workspace | the library crates |

`scripts/verify/check-policies.mjs` pins the dependencies of the desktop host, its renderer and the
five library crates: a dependency that is not in the reviewed list, or whose name suggests a
filesystem, shell, network, SQL, URI or process capability, fails verification.

## Desktop host and renderer

At start-up the host takes the single-instance lock (a second launch shows a message and exits),
reads the settings document, and resolves one workspace: the one the person chose on the first-run
screen or in Settings, your own (Live) or the sample (Training). It then inspects the Ledger file
read-only before opening it. A current Ledger is opened and held behind a mutex for the life of the
process; each command holds it only for its own transaction. A Ledger from an earlier supported
version waits at the upgrade gate; one that cannot be opened waits at a recovery gate that can
restore a verified backup. A missing or unavailable Vault does not stop anything: every read and
every Ledger-only write still works, and the Vault is validated again on each operation that needs
it.

In your own workspace, ordinary writes are refused while a backup is due, until an Operational
Backup has verified.
The renderer talks to the host only through Tauri IPC. The Tauri capability file grants no plugin
permissions, the Content Security Policy allows `connect-src` only to `'self'` and the IPC origins,
and no window may load a remote URL.

### The IPC boundary

The host registers 108 commands. The same list is kept in `scripts/verify/check-policies.mjs`, which
fails if a command is declared or registered without being listed, or listed without being declared
and registered.

Read commands (H0) never write:

- status: Ledger, Vault, workspace, backup, passphrase, System Health, the upgrade gate
- routes: `get_executive_cockpit`, `get_portfolio_overview`, `get_people_directory`,
  `get_work_queue`, `get_product_detail`, plus the records a sheet edits
- `get_evidence_references`, `get_action_completion_context`, the display locale and time zone

Write commands are either ordinary one-click writes (H1) or the two halves of a governed write
(H2a, see below):

- Action Requests: prepare/approve/reject accept, decline, withdraw
- Actions: start, link completion Evidence, prepare/approve complete, cancel and reopen, reject
- Decision Requests: withdraw, prepare/approve resolve, reject
- Risks: prepare/approve record occurrence and close, reject
- Issues: prepare resolve, close and reopen, one approve command for all three, reject
- Evidence: pin fingerprint, re-observe verification, link to a Product, create from a chosen file
- Record entry: create and edit Portfolios, Products, Roadmaps, KPI definitions and observations,
  Initiatives, Projects, Milestones and Stakeholders, and link them; create Action Request and
  Decision Request drafts and submit them; create Risks and Issues; update a Risk's response
- Operations outside the Ledger's record families, stored in the settings file or the backup folder:
  choosing the workspace, the language, the backup folder and passphrase, running a backup,
  upgrading the Ledger
- Changes to authority, prepared and approved like H2a but with their own preview: the Product Vault
  folder, restoring a backup, deleting the sample data

What crosses the boundary is deliberately narrow. The renderer sends the identity and version of
the record it read, the person's own text (a rationale, a decision statement), the Evidence ids the
person picked, and one opaque `clientRequestId` per submitted command. The host mints everything
else: correlation ids, timestamps, prepared-intent, receipt, audit and new record ids. The actor is
fixed by the host. No command accepts or returns a file path: pin and re-observe use the path the
Evidence record already stores, and the host opens every file and folder dialog itself. The renderer
then receives an opaque, expiring selection token, a name to show, and limited facts such as the
observation time and matching Evidence ids and versions, never the path
([ADR 0011](decisions/0011-host-owned-native-dialogs.md)). For reads, the renderer supplies the "as of" instant used for
deadline comparisons.

Every failure returns one safe error envelope: `errorCode`, `messageKey`, typed `messageParams`,
`correlationId`, `retryable`, an optional protected `privateDetailRef`, and typed extensions such
as the current version on a conflict. SQLite rows, paths and diagnostics are never mapped into it.

## Authority boundaries

The Product Ledger is the authority for structured records: Portfolio, Products, Roadmaps, KPI
definitions and observations, Initiatives, Projects, Milestones, Stakeholders, relationships,
Action Requests, Actions, Decision Requests, Decisions, Risks, Issues, Evidence references and
links, versions, classification, idempotency, prepared intents, approval receipts and audit. It is
one SQLite file per workspace. The schema carries an application id and a version (currently 48);
migrations are append-only and forward-only, and the Ledger opens only the current version. A Ledger
from version 46 onward is upgraded in place, after a verified Operational Backup, and is validated
against exactly the migrations that produced its version before it is changed. A Ledger made by a
newer version of the application is never opened or restored over.

The Product Vault is the user's own folder of Markdown and Evidence files. The Ledger stores only
a Vault-relative path, a verification state and, once pinned, a content fingerprint for each
Evidence reference. Note bodies are never copied into the Ledger. A pinned fingerprint is permanent:
the same bytes are re-observed, and changed bytes are reported as an integrity mismatch rather than
silently re-verified. The sample workspace has a synthetic Vault the application writes; for your own
workspace you choose the Vault folder in Settings, as an approved change made after a backup.

Managed projections are generated, read-only Markdown files for Products, Projects, Actions,
Decisions, Risks and KPIs under `Product Mission Control/Projections/` in the Vault. They are
disposable views: manual edits are never imported into the Ledger. The generator, the publication
orchestration and the Ledger bookkeeping exist in the library crates and are tested, but the
desktop does not publish projections yet.

See [ADR 0009](decisions/0009-bound-webview2-platform-runtime-egress.md) for the one accepted
network exception and the [decision records](decisions/README.md) for the authority decisions.

## Governed writes

Ordinary writes (H1) run in one SQLite transaction: validate, check the expected version, write
the record, its version, its classification, the idempotency entry and the audit event, or roll
back entirely.

Higher-impact changes (H2a) take two explicit steps:

1. Prepare. The host validates the request against the current Ledger, resolves every
   classification source, evaluates the Evidence or Judgment requirement, and persists a prepared
   intent. The renderer receives the whole preview: the operation, targets with expected
   versions, declared effects, classification and where it came from, supporting Evidence with the
   versions it was read at, policy result, and a payload digest. Nothing is executed.
2. Approve or reject. Approval sends back the prepared-intent id and the digest the person saw. The
   host checks the actor, that the digest matches, that the preview has not expired, and then
   re-derives the authoritative snapshot and compares it with the preview as a whole. Any change to
   the operation, classification, sources, support or policy refuses the approval. On success the
   intent is marked consumed and a single-use approval receipt is written in the same transaction
   as the effect. Rejection is also durable and audited, and leaves the preview unusable.

Previews expire five minutes after preparation. An expired preview can still be rejected, but not
approved; the person prepares again. Closing the review sheet without deciding sends nothing.

Retries are idempotent. The renderer reuses the same `clientRequestId` when it retries a command,
so a retry returns the original outcome instead of writing twice, and a repeated prepare returns
the preview it already produced.

Some operations require support:

- Completing an Action and resolving, closing or reopening an Issue require Evidence. Verified
  Evidence satisfies the gate. Evidence that was verified earlier but cannot be re-checked, or that
  was read without a pinned fingerprint, satisfies it only together with a recorded human Judgment,
  and the outcome is marked as verification pending.
- Resolving a Decision Request accepts Evidence or Judgment; a Judgment alone is enough.
- Unverified Evidence, or Evidence whose bytes no longer match its fingerprint, always refuses.

## Read snapshots and out-of-sync

The Ledger answers reads through two snapshots, each read in its own transaction. The projection
snapshot carries Products, Projects, Actions, Decisions, Risks and KPIs. The composition snapshot
carries what the other routes need, including Action Requests, Decision Requests, Issues,
Stakeholders, Milestones, relationships, Evidence and KPI observation metadata (never KPI values).

The Cockpit, Portfolio, Product detail and Work Queue commands read both. If a write lands between
the two reads, their Ledger revisions differ and the command returns the state `outOfSync` with an
empty body, instead of combining two moments into one answer. The renderer says so and offers to
read again. The People command reads only the composition snapshot.

Compositions fold classification upward: a composed item is labelled with the most restrictive
classification of everything it exposes, and `Unclassified` absorbs everything else.

## Backups and the host audit log

Operational Backups are encrypted archives written to a folder the person chooses and verified in
full before they count ([ADR 0010](decisions/0010-age-encrypted-operational-backup-archives.md)).
Backups, restores, Vault-folder changes and the sample's reset and delete are recorded in a host
audit log outside the Ledger, so a restore cannot replace its own record
([ADR 0012](decisions/0012-host-audit-log-for-events-outside-the-ledger.md)).

## Not built yet

- Review periods, Fact Packs, report authoring and approved Report Snapshots.
- Product Chief of Staff and any AI provider. No AI provider is wired into the desktop, and the
  platform settings document defaults external AI to disabled.
- Superseding a Decision or an Evidence reference; relocating Evidence; lowering a classification
  from the desktop.
- Removing relationships and other H2b record operations in the desktop.
- A full Vault archive, backup retention settings, and a configurable display time zone.
- Projection publication from the desktop and any projection health display.