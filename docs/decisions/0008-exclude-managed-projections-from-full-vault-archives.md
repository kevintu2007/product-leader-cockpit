# 0008: Exclude managed projections from Full Vault Archives

Status: Accepted
Date: 2026-08-10

## Context

A Full Vault Archive adds user-owned Vault notes and attachments to the Operational Backup. Managed projections live inside the Vault folder but are deterministic, rebuildable views of the Ledger. Storing possibly tens of thousands of projection files in every retained weekly archive would add archive time, temporary disk pressure and steady-state storage without adding anything that cannot be rebuilt. Excluding them needs an explicit rule for what happens on restore.

## Decision drivers

- Preserve irreplaceable user notes and attachments.
- Avoid storing generated files again and again.
- Keep archive scope and restore behaviour unambiguous.
- Prevent a restore from claiming that projections are synchronized when they are not.

## Options considered

- Include managed projections: a restore starts with the last archived views, but archive size and file count grow a lot, the views may already be stale, and they duplicate Ledger data.
- Exclude them and rebuild after restore: smaller archives and a single authority, at the cost of an explicit out-of-sync state and a verified rebuild before projections count as current.

## Decision

A Full Vault Archive contains the Operational Backup scope plus user-owned Vault notes and attachments. It excludes the managed `Product Mission Control/Projections/` folder. A restore marks projections out of sync and offers the rebuild path from [0007](0007-bound-managed-projection-automation.md). The synchronized status returns only after a deterministic rebuild and integrity check.

Capacity planning covers 14 retained daily Operational Backups, eight weekly Full Vault Archives, and enough headroom to create and verify a replacement before pruning.

## Consequences

- Weekly archives do not repeatedly store rebuildable files, and capacity needs reflect only data that cannot be rebuilt.
- Restored projections cannot pass as current.
- Projection views are unavailable or degraded after a restore until the rebuild finishes.
- If the rebuild fails, the Ledger and Vault content are still restored, and projections stay visibly out of sync with recovery guidance.
- Manifest tests must prove that only the managed projection folder is excluded.

## Current state

Implementation update, 2026-09-25 (0.2.0-beta): Operational Backups exist, encrypted and verified, with due-backup handling and restore (see [0010](0010-age-encrypted-operational-backup-archives.md)). A restore already marks managed projections out of sync, as this decision requires. The Full Vault Archive itself is still not built: Vault notes and attachments are not archived.

## Revisit when

- Managed projections become non-deterministic or user-authored.
- Rebuilding after a restore takes unacceptably long on measured hardware.
- The backup destination or archive format changes the cost of file count materially.
