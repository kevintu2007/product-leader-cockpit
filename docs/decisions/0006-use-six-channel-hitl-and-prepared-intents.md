# 0006: Use six human-authority channels and prepared intents

Status: Accepted
Date: 2026-08-10

## Context

Product Mission Control mixes low-risk reads, ordinary commits, bounded automation, daily governance decisions, destructive recovery work and operations that policy forbids. A single confirmation dialog would either burden routine work until approval became mechanical, or under-protect irreversible operations. Two-step approval also has to prove that the payload executed is the payload the user saw, survive retries without duplicate effects, and refuse an approval that went stale because data or policy changed.

## Decision drivers

- Keep explicit human authority without causing approval fatigue.
- Prevent substitution between check and use, replay, duplicate execution and policy bypass.
- Make automation visible and bounded, instead of treating background execution as implicit authority.
- Offer cancellation only where an operation can stop without ambiguous state.
- Keep command contracts domain-specific and enforce them in Rust.

## Options considered

- One approve-or-deny tier: simple, but it puts routine acceptance, report approval, restore, deletion and external egress at one severity, which leads to weak prompts or mechanical approval.
- Per-command confirmation flags: flexible, but with no shared authority model they cannot express policy denial, recovery evidence or automation limits consistently, and they drift apart.
- Six channels with prepared intents: separates risk and authority and shares one two-step contract for high-impact operations, at the cost of more contract and test work.

## Decision

Execution channels:

- H0: read-only queries and deterministic derivation that persists nothing.
- H1-Auto: operations pre-authorized by a named, versioned policy and bounded by measurable scope. Each run is audited, reports failure visibly and offers safe cancellation where possible.
- H1-User: ordinary reversible commits started by one explicit user action.
- H2a: an exact preview and explicit approval, for daily governance and other material operations.
- H2b: an exact preview, a named confirmation and verified recovery evidence, for destructive or irreversible local operations. External egress has no real recovery point, so instead it requires the exact payload, provider, account, purpose, classification and an irreversible-transmission statement.
- H3: policy denial that cannot be overridden. Approval cannot bypass classification, provider policy, prompt-injection defences, authority boundaries or required recovery evidence.

Changing the policy that authorizes automation, retention, classification, privacy or egress is never H1-Auto; it needs H2a or stronger.

H2 commands are split into a prepare step and an approve-and-execute step. A short-lived Prepared Intent binds the target identifiers, expected versions, exact effects, classification, policy result, payload digest and any recovery or egress evidence. Execution requires the digest the user was actually shown, consumes a single-use Approval Receipt atomically, rechecks versions and policy, and carries an idempotency identifier. Any relevant change invalidates the approval, and a restart never replays one.

Cancellation is set per intent: not cancellable once submitted, cancellable during effect-free preparation, or stoppable only at listed safe boundaries. An accepted cancellation shows "cancelling" until the operation reaches a terminal outcome. Status and effect scope are separate: a cancelled operation with partial effects lists what was committed, what failed and what was untouched. Repeating an execution identifier returns the original outcome; running again needs a newly prepared and approved intent with a new identifier.

## Consequences

- Routine work stays usable while destructive and egress operations get stronger friction.
- Digest acknowledgement and version binding prevent substituted or stale approvals.
- Single-use receipts and idempotency make retries and restarts auditable.
- Automation cannot be used to slip a policy change into an unapproved write.
- Commands carry more typed metadata and need more state-machine tests.
- H2 flows need durable prepared-intent and outcome storage.
- Each intent needs an explicit cancellation rule instead of a generic cancel button.

## Current state

- H2a is wired end to end in the desktop for accepting an Action Request; completing, cancelling and reopening an Action; resolving a Decision Request; recording a Risk occurrence and closing a Risk; and resolving, closing and reopening an Issue. Each has prepare, approve-and-execute and reject commands in `apps/desktop/src-tauri/src/write_commands.rs`. Previews expire after five minutes, approval sends back the acknowledged payload digest, and a rejection is recorded without a receipt.
- H1-User commands in the desktop include declining and withdrawing Action Requests, starting an Action, withdrawing a Decision Request, and linking, pinning and re-observing Evidence.
- H2b exists in the domain and Ledger only for removing a relationship, with the single cancellation rule "not cancellable once submitted". It is not reachable from the desktop.
- H1-Auto exists in the domain and Ledger only for small managed-projection publications ([0007](0007-bound-managed-projection-automation.md)).
- No external egress exists.

## Revisit when

- Approval fatigue is measured, or a material operation fits no channel.
- Multi-user delegation, remote execution or a separate job service is introduced.
- Provider policy changes the external-egress evidence contract.
