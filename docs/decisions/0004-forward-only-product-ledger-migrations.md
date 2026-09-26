# 0004: Use forward-only Product Ledger migrations with verified recovery

Status: Accepted
Date: 2026-08-10

## Context

Product Ledger schema changes must preserve user data across application versions. Automatic down migrations often cannot reconstruct data that a newer schema removed or transformed, and they promise a reversibility they cannot deliver. Rolling back a transaction and restoring a pre-migration backup are different recovery mechanisms.

## Decision

Ledger migrations are append-only and forward-only. There is no automatic down migration. Before migrating, the application creates and verifies an Operational Backup. A migration whose operations can run atomically runs in one SQLite transaction, and a failure rolls that transaction back. A migration that cannot run atomically requires a preflight preview and explicit approval covering both the transformation and automatic recovery from the named verified backup. An incompatible combination of binary and schema fails closed instead of opening the Ledger unsafely.

## Alternatives considered

- Paired up and down migrations: rejected, because destructive or semantic transformations cannot reliably recover discarded data.
- Best-effort in-place migration without a backup: rejected, because a process, disk or implementation failure could leave no proven recovery path.
- Export and re-import into a new database for every change: kept for migrations that cannot be made safely transactional, but too expensive as the default.

## Consequences

- Rolling back the application may require restoring a compatible pre-migration backup; installing an older binary alone is not enough.
- Each release must declare which earlier schema versions it can upgrade.
- Migration tests cover supported upgrades, rollback of a failed atomic migration, recovery from a verified backup, reopening and retries.
- Backup creation, integrity checks, storage capacity and recovery messages are part of migration readiness.

## Current state

Implementation update, 2026-09-25 (0.2.0-beta):

- Built: the migrations in `crates/pmc-ledger/src/sqlite/migrations.rs` are an ordered, append-only list, each with a checksum recorded in the Ledger; the current schema is 48. Opening a Ledger fails closed: `SqliteProductLedger::open` refuses a database that belongs to another application, has a newer schema, or has any schema other than the current one, and it validates the stored schema and migration registry against the code.
- Built: upgrading an older Ledger in place. The Ledger is inspected read-only, backed up as a closed file and the backup verified, then upgraded in one transaction with foreign keys off before `BEGIN` and a foreign-key check before commit. Restore from a verified backup is built.
- Not built: the preflight preview and approval flow for a migration that cannot run atomically. Every migration shipped so far runs atomically.

## Revisit when

The Ledger moves to a storage engine with a different atomic migration or snapshot model.
