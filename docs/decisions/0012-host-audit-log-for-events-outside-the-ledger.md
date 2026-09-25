# 0012: Record backup, restore and workspace events in a host audit log outside the Ledger

Status: Accepted
Date: 2026-09-22

## Context

The Ledger's own audit events belong to its record modules. Backup runs, the first creation of an
empty Live Ledger, restores and similar host actions also need an audit trail. Recording them inside
the Ledger has two problems: it needs a schema change for each new kind of event, and a restore
**replaces** the Ledger, so an audit of that restore kept inside it would be replaced with it.

## Options considered

- **Add a host module to the Ledger's audit table.** A restore would erase its own audit.
- **Record events in the backup registry.** The registry means "verified archives" only; overloading
  it would blur that contract.
- **A separate append-only file, owned by the host.**

## Decision

1. **One append-only file per profile**, `host-audit-v1.jsonl`, in the protected app-data folder
   beside the settings document, never inside a workspace. One JSON object per line.
2. **Event shape** (`pmc-host-audit/v1`): format, a host-minted event id, the host time, the workspace
   (`live` or `training`), an event code, an outcome (`succeeded`, `failed` or `refused`) and a small,
   closed set of non-secret facts per code. Never a path, a passphrase, a record's content or a
   person's name.
3. **The empty-Ledger bootstrap** is recorded at most once per Live workspace. It exempts only the
   creation of the empty Ledger: the first operating write still waits for a verified backup.
4. **Durability.** Appends are serialized by an exclusive lock file. Each event is one
   newline-terminated record followed by a flush request to the operating system; the action it
   records is not reported as done until that succeeds.
5. **Torn records.** If a crash leaves an unterminated last line, the next append first writes a
   newline, so the torn record stays on its own line. Readers skip any incomplete line. No line is
   ever rewritten or removed.
6. **Not an authority.** Nothing reads this log to decide what the Ledger contains. It answers "what
   did the host do, and when".

## Consequences

- Host events are recorded without a schema change and survive a restore.
- A reviewer reads two trails: the Ledger's audit events and this log.
- The file grows without bound; retention is not decided yet.

## Current state

- `crates/pmc-platform/src/host_audit.rs` writes the log. Its codes cover the empty-Ledger bootstrap,
  backup completion and failure, Product Vault folder changes, and the sample workspace's reset and
  delete.
- `crates/pmc-application/src/restore_service.rs` adds the restore codes: prepared, rejected, started
  and finished.
