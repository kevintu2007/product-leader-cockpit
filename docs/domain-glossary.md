# Domain glossary

These terms are used with one meaning across the interface, the code and the tests. Each entry gives the definition and, under _Avoid_, the words it should not be confused with.

Some terms name concepts the application does not implement yet. Those entries say so. The application is a beta; [the architecture overview](architecture.md) describes what exists today.

## Product structure

**Product Mission Control**:
The complete workspace used to understand and operate a product portfolio.
_Avoid_: generic task manager, project dashboard

**Portfolio**:
The complete set of Products and strategic work overseen by the Head of Products.
_Avoid_: workspace, account

**Product**:
An enduring offering or solution that provides customer value and has its own outcomes and roadmap.
_Avoid_: Project, feature

**Initiative**:
A strategic body of work intended to produce a defined outcome. It can span several Products.
_Avoid_: epic, miscellaneous project

**Project**:
A time-bounded execution vehicle with scope, milestones and deliverables. It can support one or more Products.
_Avoid_: Product, Initiative

**Roadmap**:
The outcome-oriented sequence of planned Product changes and strategic commitments.
_Avoid_: task list, fixed promise

**Milestone**:
A verifiable checkpoint within a Project or Initiative.
_Avoid_: percentage complete

## Execution and governance

**KPI**:
A decision-relevant measure with a definition, owner, target, observation cadence and evidence source.
_Avoid_: any available number, vanity metric

**Action Request**:
A request waiting for its intended owner to accept, clarify or reject it.
_Avoid_: Action, assigned task

**Action**:
An accepted commitment with an owner, a status and a due date.
_Avoid_: Action Request, note

**Decision Request**:
A question waiting for a named decision owner to resolve it.
_Avoid_: Decision, discussion topic

**Decision**:
A resolved choice with its rationale, its impact and, where applicable, the Action Requests that follow from it. A Decision does not bypass the intended owner's acceptance of a commitment: follow-up work is created as Action Requests, not as Actions.
_Avoid_: Decision Request, opinion

**Risk**:
An uncertain event that may affect an outcome if it occurs. Recording that a Risk occurred creates an Issue.
_Avoid_: Issue, generic concern

**Issue**:
A condition that has occurred and is already affecting an outcome.
_Avoid_: Risk, action item

**Stakeholder**:
A person or organization with an interest, influence, responsibility or dependency related to the Portfolio. Stakeholders are records the user creates, not user accounts.
_Avoid_: user account, assignee only

## Knowledge and reporting

**Evidence**:
A traceable source that supports a KPI observation, a status, a conclusion, an Action or a Decision. Each Evidence reference carries a verification state.
_Avoid_: unattributed assertion, generated claim

**Product Vault**:
The user-owned Markdown knowledge area (an Obsidian-compatible folder) for narrative context, research, meeting notes, strategy and reports.
_Avoid_: Product Ledger, Git repository

**Product Ledger**:
The authoritative structured record of portfolio state, KPI observations, Actions, Decisions, Risks, Issues and their relationships. It is a local SQLite database.
_Avoid_: Product Vault, projection folder

**Projection**:
A disposable, read-only Markdown representation generated from the Product Ledger for search and navigation in the Vault. Edits to a projection are never read back into the Ledger.
_Avoid_: source record, editable note

**Attention Flag**:
A derived, reviewable condition such as needs info, overdue, blocked, at risk or superseded premise. Deriving a flag never changes the record's lifecycle state. See the [attention ranking policy](policies/attention-ranking.md).
_Avoid_: generic status, automatic commitment change

**Fact Pack** (not built):
A validated, period-specific set of operating facts and Evidence used to prepare a review or report.
_Avoid_: AI draft, narrative report

**Report Snapshot** (not built):
An approved, immutable representation of a report at a specific time.
_Avoid_: live dashboard, regenerating draft

**Deterministic Report Draft** (not built):
A report skeleton assembled locally from one validated Fact Pack and approved templates. All narrative language is written or edited by the user.
_Avoid_: AI-generated narrative, approved Report Snapshot

**Executive Voice Profile** (not built):
The versioned structure and language rules used to prepare concise, evidence-backed reports in the user's own voice.
_Avoid_: generic tone prompt, permission to imitate unreviewed text

**Approved Voice Sample** (not built):
A human-written or human-edited report sample explicitly approved for later Executive Voice Profile calibration.
_Avoid_: AI draft, raw conversation, unapproved edit

**Weekly Product Operating Review** (not built):
The recurring review of KPIs, progress, commitments, Decisions, Risks, Issues and evidence freshness across the Portfolio. The Reviews & Reports screen exists, but no review period can be approved yet.
_Avoid_: activity dump, generic weekly status

**Personal Operating Review** (not built):
The detailed private view of a Weekly Product Operating Review, used by the Head of Products.
_Avoid_: Leader Brief

**Leader Brief** (not built):
The concise, decision-oriented version of a Weekly Product Operating Review, prepared for the group leader.
_Avoid_: Personal Operating Review, AI summary

## Roles and surfaces

**Head of Products**:
The primary user, responsible for portfolio direction, product outcomes, cross-functional execution and executive communication.
_Avoid_: generic administrator, founder

**Product Chief of Staff** (not built):
A governed workflow role that guides evidence review, deterministic report preparation and human-controlled proposals. It is not a text-generating model. Any future AI output is an untrusted proposal behind provider policy and approval.
_Avoid_: autonomous agent, model name, chatbot

**Executive Cockpit**:
The exception-oriented home screen: where each Product stands, and which commitments, Decisions, Risks and Issues need attention.
_Avoid_: vanity dashboard, chat home page

**Work Queue**:
The operating screen for Action Requests, Actions, Decision Requests, Risks and Issues that can still be acted on. See the [Work Queue ordering policy](policies/work-queue-ordering.md).
_Avoid_: generic task list, Portfolio hierarchy

**Degraded Mode**:
A visible operating state in which an unavailable dependency, such as a missing Vault, limits specific capabilities while the safe, authoritative workflows remain available.
_Avoid_: silent failure, permission to bypass safety checks

**UI Contract**:
The versioned boundary the desktop interface is built against: routes, workflow states, typed command and query shapes, errors, accessibility behavior, localization and platform behavior. See the [UI contract](ui-contract.md).
_Avoid_: screenshot-only specification, mutable design suggestion

**Correlation ID**:
A safe, opaque identifier that connects a user-visible failure or operation to related local diagnostic events without exposing private payloads. Every error the interface shows carries one that the user can copy.
_Avoid_: error detail, entity name, secret, user-facing explanation

**Prepared Intent**:
A short-lived, version-bound, exact preview of one sensitive or high-impact domain command. It records the payload digest, targets, expected versions, effects and classification before approval. Prepared intents expire after five minutes.
_Avoid_: generic confirmation dialog, reusable permission

**Approval Receipt**:
A single-use record binding a human approval to one Prepared Intent and the exact digest the user acknowledged. It cannot override a policy denial and is consumed atomically on execution.
_Avoid_: blanket consent, permission bypass

## Data handling and AI governance

**Data Classification**:
The mandatory handling label on a record or source: Public, Internal, Confidential, Restricted or Unclassified. Derived material takes the most restrictive included classification, and Unclassified outranks all others.
_Avoid_: AI-generated sensitivity guess, optional tag

**Work Packet** (not built):
A purpose-bound, expiring package of explicitly selected content prepared for one governed AI operation after every egress check passes. No AI provider is connected to the application.
_Avoid_: prompt dump, background transmission, reusable unrestricted export

## Backup and recovery

**Operational Backup** (partly built):
A verified encrypted backup of the Product Ledger, schema and version information, non-secret settings and a manifest; rebuildable projections are excluded. Today the Ledger can write a snapshot of itself and verify it by hash, schema version and revision. Encryption, scheduling, restore and any user interface for backups are not built.
_Avoid_: file synchronization, Full Vault Archive

**Full Vault Archive** (not built):
A user-confirmed, verified, encrypted archive that adds user-owned Vault content and attachments to the Operational Backup scope and excludes rebuildable managed projections. Restore leaves projections out of sync until they are rebuilt.
_Avoid_: silent cloud upload, Operational Backup
