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

Only part of this decision is built.

- Built: the migrations in `crates/pmc-ledger/src/sqlite/migrations.rs` are an ordered, append-only list, each with a checksum recorded in the Ledger. Opening a Ledger fails closed: `SqliteProductLedger::open` refuses a database that belongs to another application, has a newer schema, or has any schema other than the current one (46), and it validates the stored schema and migration registry against the code.
- Built: the Ledger can write a snapshot of itself and verify it (`create_verified_snapshot`, `verify_snapshot`).
- Not built: upgrading an existing Ledger in place. Migrations run only when a new, empty Ledger is created; upgrade paths and rollback after an injected failure are exercised in tests only. The pre-migration backup, the preflight preview for non-atomic migrations, and restore are not built.

## Revisit when

The Ledger moves to a storage engine with a different atomic migration or snapshot model.
