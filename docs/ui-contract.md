# UI contract

This is the user-observable contract of the desktop as it ships in 0.2.0-beta: what the code in
`apps/desktop/src` actually does. Where the design describes something the code does not do, this
document says "not built yet" rather than describing it.

The interface is available in six languages: English, Traditional Chinese, Simplified Chinese,
Japanese, Korean and Spanish. It follows Windows until the person chooses one in Settings. Route
names and canonical domain terms (Product, Action Request, Evidence, and so on) stay in English in
every language. For how the host serves these
screens, see [architecture](architecture.md); for terms, see the
[domain glossary](domain-glossary.md).

## Shared rules

- The renderer shows what the host composed. Placement, quadrants, attention tiers, classification
  folds and legal next steps are decided in Rust; the renderer does not recompute them.
- Products are placed, never ranked. Product tables are in name order and say so. Only attention
  exceptions are ranked, by the [attention ranking policy](policies/attention-ranking.md) and the
  [Work Queue ordering policy](policies/work-queue-ordering.md).
- No progress bars or percentages. Ratios are shown as counts such as `2／3`.
- Work (Action Requests, Actions, Decision Requests, Risks, Issues) belongs to people, not to
  Products. It is never shown as a Product's own work.
- Every screen that reads the Ledger shows the Ledger revision and the time it was read.
- Lifecycle-legal next steps are shown as text. They say what a record's lifecycle admits, which is
  weaker than what may be executed; the actual commands are separate buttons, and each still runs
  the host's checks.

## Navigation

A compact rail lists seven primary destinations in a fixed order: Executive Cockpit, Portfolio,
Work Queue, Reviews & Reports, Product Vault, People and Settings. System Health sits apart at the
foot of the rail. The rail expands to show labels on hover or keyboard focus. Every destination is
a button; the current one has `aria-current="page"` and is marked by more than colour.

The Work Queue item shows a badge with the number of flagged items, and its accessible name says
the same in words. The badge is refreshed whenever the Work Queue is read.

The top bar shows the route name, today's date and the display time zone, and a light/dark theme
toggle. In the sample workspace it also shows a Sample workspace badge that opens Settings →
Workspace, and the window title ends in "— Sample data": in the chosen language, or in English at first
when the language follows Windows. Routing is
internal state; there are no URLs or browser history.

## Before the shell opens

Some states are handled by a full-window gate before any route is shown:

- **First run.** "Choose how to begin" offers two equal choices: start with your own workspace, or
  learn with sample data. Choosing the sample prepares it (this can take a minute) and restarts.
- **Upgrade.** A Ledger written by an earlier supported version is shown with its facts and the last
  verified backup; upgrading makes a backup first and is refused if that backup fails.
- **A Ledger that cannot be opened.** The gate says why and, where a verified backup exists, offers
  to restore it; a Ledger made by a newer version is never restored over.
- **A second launch** shows a native message that PMC is already running, instead of a second
  window.

## Executive Cockpit

The default route. In your own workspace, while setup is incomplete, a Getting started list comes
first: choose a backup folder, set a recovery passphrase, make the first backup, choose the Product
Vault folder, add the first Product. Each step's state is read from the host (backup status, Vault
status, the Cockpit's own Product count), never stored; only the next open step has a button, which
opens Settings or Portfolio; a step whose state cannot be read says so and claims nothing; the list
disappears when all five are done. It is never shown in the sample workspace. It has no progress
bar or count. Then:

1. The Portfolio Lens with a persistent inspector beside it.
2. Period comparison. It always says there is nothing to compare against, because review periods
   are not built. It never shows a zero change.
3. Portfolio pulse: counts of Milestones, commitments and KPIs, each with the definition of what
   was counted and the part of the Ledger it came from.
4. Attention items, in ranked order. Each names the record (type, title, classification), the
   reason, and why it is placed there.
5. A short summary: how many records need attention and which comes first, and a note that without
   a review period there is no telling whether things improved or worsened.

### Portfolio Lens

Each Product is a point placed by two measures, with a third as its size:

| Measure | Shown as | Unknown when |
| --- | --- | --- |
| Milestone timing | Horizontal position: later, due within 14 days, date passed | No linked Milestone |
| Outcome observability | Vertical position: KPI definitions with at least one observation, over all linked definitions | No linked KPI definition |
| Verified Evidence coverage | Point size: verified Evidence over all Evidence linked to the Product; a dashed outline means none is linked | No linked Evidence |

A passed Milestone date is described as a date that has passed, never as late or unfinished work,
because Milestones carry no completion state. Observability is high when at least half of the KPI
definitions have an observation. Timing is high when a date is due soon or has passed.

The four quadrants keep their names: Keep Momentum (low timing exposure, high observability),
Monitor Closely (high, high), Explore and Validate (low, low) and Prioritize Now (high, low). A
Product with an unknown axis is placed in a labelled band outside the quadrants and gets no
quadrant.

Three mode buttons (Milestones, Outcome observability, Evidence) change which Products are
emphasised and the description above the chart. They never move a point. The modes emphasise,
respectively, a passed Milestone date, low observability, and Evidence that is unverified or does
not match its fingerprint.

Hovering or focusing a point shows the Product name, What happened (what is currently the case),
Impact (always "not yet assessed"), Next step (select to view), and the three measures in words. A table
under the chart carries the same values and each row has a select button. Selecting a Product
(click, Enter or Space) fills the inspector and moves focus to its heading.

The inspector first shows the Lens measures for the selected Product: its effective
classification and the record that forced it, the quadrant, the three measures, Evidence counts
per verification state, Projects shared with other Products, and an expandable list of every record
behind the numbers with its version and classification. Below that is the Product inspector.

## Portfolio

A table of all Products in name order, 25 per page, with the three Lens measures, the quadrant, the
number of flagged work items carried by the people accountable for each Product, and a
classification that folds in those items. Each row's View button, at the right end of the row,
opens the same persistent inspector beside the table.

Above the table, the Portfolios list offers New Portfolio…, and Edit… and Link a Product… for each
Portfolio; the Products list offers New Product…. Each opens a record sheet with the fields, a byte
counter where a field has a limit, and a required classification. In your own workspace these writes
are refused until a backup has verified.

## Product inspector

Opened from the Cockpit or Portfolio. It shows the Product name, classification, version and read
time. When something the inspector shows is more restrictive than the Product itself, the whole
inspector is presented at that classification and the record that forced it is named.

- What happened lists current conditions, each attributed to the record that raised it with that
  record's version: the verification state of Product-linked Evidence, and attention flags on work
  carried by accountable people. It never says anything worsened, because no history is compared.
- Impact is a slot for human judgment. It always reads "not assessed by anyone yet"; recording an
  assessment is not built yet.
- Next step appears as the lifecycle-legal next steps listed under each carried work item, and as
  the Evidence actions below.

Three tabs:

- Structure: Projects, Milestones, Roadmaps and KPI definitions linked to the Product. Initiatives
  appear only through a Project and are labelled as reached through it. The tab offers New
  Roadmap…, New KPI…, New Project… and their "Link an existing …" counterparts; each Project offers
  Edit…, New Milestone…, New Initiative… and Link an existing Initiative…; a KPI can record an
  observation.
- Evidence: Evidence linked directly to the Product, with its verification state, whether a
  fingerprint is pinned, its current classification and its classification at link time. No file
  path is shown.
- People: Stakeholders responsible for or dependent on the Product, how many other Products each
  is accountable for, and the work each person currently carries, under a heading that says it is
  held through accountability and not owned by the Product.

The Evidence tab offers four actions, one entry at a time:

- Pin fingerprint, only while no fingerprint is pinned. A confirmation explains that pinning is
  permanent before anything runs.
- Re-observe. A confirmation explains that the Ledger is written only if the observed state
  differs. An unchanged result is reported as "no change, Ledger not written".
- Link Evidence to this Product. It lists Evidence not yet linked here, with state,
  classification and version, and reports the classification recorded at link time.
- Add Evidence from a file…. The host opens the Windows file dialog in the Vault folder; a file
  outside the Vault is refused. The sheet shows the file name and when it was observed, asks for a
  classification, and creates the Evidence and links it to the Product. The file is not copied.

The inspector reads the Vault status with the Product. When the Vault is unavailable it says why
(not configured, or the location is missing, not a folder, or a link), disables pin, re-observe and
Evidence from a file, and keeps link available. Relocating and superseding Evidence are not offered.

The tabs are buttons with tab roles; arrow-key navigation between tabs is not built yet.

## Work Queue

A table of Action Requests, Actions, Decision Requests, Risks and Issues, 25 per page, in the order
set by the [Work Queue ordering policy](policies/work-queue-ordering.md). Each row shows the type,
title, state, classification, its own read time, the lifecycle-legal next steps, every attention
flag in words (with a note when a flag rests on stale or degraded facts), the deadline, and why the
row is placed where it is. Type checkboxes (with counts, including zero) and an "only items that
need attention" checkbox filter the list. Decisions themselves are not listed. A recurring Issue is
titled as a further occurrence of the first.

Above the filters, New Action Request…, New Decision Request…, New Risk… and New Issue… open record
sheets; a new Action Request or Decision Request is created as a draft and then submitted.

Selecting a title opens a detail dialog with the same facts and the same actions; focus starts on
its close button, so opening it cannot start a command.

The actions column offers only what the record's lifecycle admits:

| Record | Direct actions | Actions that open a focused review |
| --- | --- | --- |
| Action Request | Decline, withdraw (each with a reason) | Accept |
| Action | Start, link completion Evidence | Complete (optional Judgment and its classification), cancel (reason), reopen (mode and reason) |
| Decision Request | Withdraw (reason) | Resolve (statement, rationale, impact, Evidence and/or Judgment, follow-up Action Requests) |
| Risk | Update the response… | Record occurrence, close (reason) |
| Issue | None | Resolve (type, reason, Evidence), close (Evidence), reopen (reason, Evidence) |

One row acts at a time. A failed command shows the safe error with retry (using the same request
id) and an option to abandon it.

### Focused review

Preparing an operation opens a modal review sheet that shows the host's whole preview: what will
change, the records and versions it targets, the declared effects, where the classification comes
from, policy and authority, the Evidence and Judgment support with source versions, the full
payload digest, the preparation record, and the expiry time with a live countdown. Times on this
sheet are shown in UTC.

- Approve sends the digest the person saw. It is disabled once the preview has expired.
- Reject is recorded in the Ledger and makes the preview unusable. It works even after expiry.
- "先不決定" (decide later), Escape, or a click outside the sheet closes it without sending
  anything. The row then offers "回到審閱單" (back to the review) and cannot prepare a second
  preview beside the pending one. Held reviews survive leaving and returning to the Work Queue.
- When the preview has expired, or the host reports it expired or changed, "prepare again" rejects
  the old preview, re-reads the queue, and either prepares an Accept again or reopens the row's form
  for the person's input.
- After a result, the sheet shows it and offers Close; the queue is then read again.

## People

A table of Stakeholders (people or organisations), 25 per page: responsibilities, dependencies, and
outstanding Action Requests listed separately, with each entry's effective classification folded
from everything it exposes. Stakeholders are relationships, not user accounts. New Stakeholder…
creates one; each row offers Edit… and Relate to a subject….

## Reviews & Reports

Lists the records the Work Queue currently flags, one entry per record, in the Cockpit's attention
order, with a button that opens the Work Queue. It states that review periods are not available and
that Fact Packs and approved reports are not built yet. Weekly Review, report authoring and
approved Report Snapshots are not built.

## Product Vault

Shows whether the Vault can be used (and why not), and a table of every Evidence reference in the
Ledger: purpose, verification state and date, whether a fingerprint is pinned, classification and
version. No paths are shown. Add Evidence from a file… creates Evidence without linking it to a
Product. Search, opening a note in Obsidian and projection health are not built yet.

## Settings

- **Workspace:** which workspace is open, and Switch to sample data / Switch to my workspace (each
  confirmed, then PMC restarts). From your own workspace only: Reset sample data… (confirmed) and
  Delete sample data…, a sheet that lists exactly what is removed, the preview's expiry, settings
  revision and digests, and requires typing a phrase. Reset and delete never touch your workspace,
  settings or backups.
- **Backups** (your workspace): the backup folder (chosen in a host dialog; only its name is shown),
  the recovery passphrase (generated or your own, shown once, typed back, acknowledged, optionally
  remembered on this Windows account), the last verified backup and when the next is due, Back up
  now, and Restore from a backup…: choose a backup file in a host dialog, type its passphrase, and PMC
  backs up the current workspace and then previews the backup, the current state, what is replaced,
  what is not, and that recovery backup. Restoring needs the backup's creation date typed; Don't
  restore leaves the workspace as it is and keeps that recovery backup.
- **Language:** follow Windows or one of the six languages.
- **Text size** with a live preview, and **Data sources**: the Ledger schema version and revision,
  and the Product Vault with its availability. In your own workspace the Vault row offers Choose Vault
  folder… (Change Vault folder… once set): PMC backs up the workspace, the host opens the folder
  dialog, and a preview shows the folder now and new, the Evidence affected and that backup; typing
  the confirmation phrase it shows and Use this folder applies it, Don't change leaves it.

Time zone, retention and policy settings are not built yet.

## System Health

States whether PMC can read and keep your records: whether the Product Ledger is open and readable,
being replaced by a restore, waiting for an upgrade or for recovery, or could not be opened, with
the reason; whether the settings can be read; and anything about the sample workspace that needs
attention.

## States and feedback

- Loading: each screen shows a status message while it reads.
- Empty: each list says plainly that there is nothing, and Portfolio distinguishes an empty Ledger
  from an empty page.
- Error: the screen shows a localised message that no earlier data is displayed, the correlation id
  with a copy button, and a button to read again. Raw message keys are never shown. A failure that
  is not a host safe error shows a generic message and no correlation id.
- Out of sync: when the host reports that its two reads disagreed, the Cockpit, Portfolio, Reviews
  & Reports, Work Queue and the inspector show no data, say why, and offer to read again.
- Degraded Vault: shown in the inspector's Evidence tab, the Product Vault screen and Settings.
- Backup due: in your own workspace, a strip under the top bar says that new records and changes
  are accepted again after a backup, with a button that opens Settings; it also reports a backup or
  restore in progress. Stale, partial success, cancelling, approval required and classification
  denied states are not built yet.

## Keyboard and focus

- All navigation, selections, forms and actions are native buttons and form controls, reachable
  with Tab and operated with Enter or Space.
- Dialogs trap focus, move focus inside when they open, and return it to the opener when they
  close. Escape closes the focused review without deciding (or closes it after a result) and closes
  the Work Queue detail dialog.
- Returning focus to the originating Product after inspecting it, and a command palette, are not
  built yet.

## Text scale, theme and time

- Text scale has five steps (90, 100, 110, 120 and 130 percent), independent of Windows display
  scaling. It applies to the whole app and is remembered in the WebView's local storage.
- The theme follows Windows until the person picks light or dark with the toggle; the choice is
  remembered the same way.
- Read times and dates are shown in the computer's local time zone; the focused review shows UTC.
  A configurable display time zone is not built yet.
