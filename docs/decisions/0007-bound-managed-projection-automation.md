# 0007: Bound managed-projection automation and escalate large rebuilds

Status: Accepted
Date: 2026-08-10

## Context

Managed projections are disposable read models, but rebuilding them can write tens of thousands of Markdown files and trigger re-indexing in the user's note tool. Treating every rebuild as background automation could make the desktop unusable or silently overwrite manual edits. Treating every small refresh as an approval would add needless friction.

## Decision drivers

- Keep ordinary synchronization quiet and bounded.
- Prevent large, unmeasured filesystem work from running automatically.
- Keep the Ledger authoritative and never import manual projection edits.
- Make overwrite scope, cancellation, partial results and foreground usability visible.

## Options considered

- Always rebuild automatically: simple, but the filesystem and indexing cost can surprise the user, and manual edits could be overwritten without review.
- Require approval for every projection write: explicit, but turns routine small updates into repetitive approvals.
- Bounded incremental automation with escalation to H2a: small, predictable updates run automatically; initial, full, large, uncertain or conflicting work needs an exact preview and approval.

## Decision

H1-Auto may publish only an incremental change set estimated at no more than 500 files and 30 seconds. Both limits must hold; a missing or unreliable estimate fails closed to H2a. Work may not be split into smaller batches to get under the limit.

Initial and full rebuilds, larger work, and any mismatch in hash, banner or managed metadata that suggests manual edits go through H2a. The preview lists the affected paths, the generated-only scope, the expected duration, and the rule that projection changes never write back to the Ledger. The user can copy or keep manual changes before approving the overwrite.

Generation and publication are separate phases. Cancellation or failure leaves the projections out of sync and reports committed, failed and untouched paths. The synchronized status returns only after deterministic content and hash verification. Benchmarks measure generation, publication, peak memory, peak temporary disk, file count, and whether the Cockpit, the Work Queue and the note tool stay usable during a rebuild.

## Consequences

- Small routine updates stay automatic, and large or surprising writes get a preview and approval.
- Splitting work cannot bypass the limit.
- Manual projection edits can be saved before an overwrite and never reach the Ledger.
- Reliable estimation and change-set planning are needed before execution.
- The 500-file and 30-second limits are provisional and may need calibration on measured Windows hardware.
- A bad estimate escalates instead of defaulting to automation.

## Current state

The limits are `MANAGED_PROJECTION_H1_AUTO_MAX_FILES` and `MANAGED_PROJECTION_H1_AUTO_MAX_DURATION_MILLIS` in `crates/pmc-domain/src/managed_projection_rebuild.rs`. `ManagedProjectionH1AutoAuthorization::authorize` is the only way to obtain an H1-Auto authorization, and it refuses an empty change set, an initial or full rebuild, an integrity conflict, or either limit being exceeded. The Ledger records each publication as H1-Auto or H2a, tracks the publishing, cancelling and terminal states, and refuses a new publication while another is in progress. The desktop app does not start projection publication yet, and the limits are still provisional.

## Revisit when

- Measurements show the limits are unsafe or needlessly conservative.
- Note-tool indexing behaviour changes materially.
- Projections stop being one file per record.
