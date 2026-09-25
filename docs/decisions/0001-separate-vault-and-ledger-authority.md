# 0001: Separate Product Vault and Product Ledger authority

Status: Accepted
Date: 2026-08-10

## Context

Product Mission Control needs transactional, queryable operating facts and user-owned narrative knowledge. Markdown alone cannot reliably enforce relationships and state transitions. A database-only design would replace the user's Obsidian-compatible notes and reduce portability. Letting both stores edit the same fact would create conflicts that have no clean resolution.

## Decision

The Product Vault is authoritative for user-owned narrative knowledge. The Product Ledger is authoritative for structured operating facts. The Ledger generates deterministic, read-only Markdown projections in the Vault's `Product Mission Control/Projections/` folder. Projections can be rebuilt at any time, never write back to the Ledger, and identify themselves as managed content.

## Alternatives considered

- Markdown or YAML only: rejected, because transactional invariants, relationships, migrations and consistent reporting would be fragile.
- Database only: rejected, because it would replace the user's own knowledge environment and weaken file ownership and portability.
- Two-way Markdown and SQLite synchronization: rejected, because conflict resolution would create a second authority and ambiguous recovery.

## Consequences

- Structured changes pass through the Rust domain and Ledger rules.
- Narrative editing stays in the Vault.
- The application links to Evidence and shows projection freshness instead of copying note content into the Ledger.
- A projection failure does not roll back a valid Ledger commit; projections are shown as out of sync until rebuilt.
- The managed folder is visible in the Vault but should not be edited; its notices and overwrite behaviour must be clear.

## Current state

The Ledger is a local SQLite database (`crates/pmc-ledger`). Projection generation (`crates/pmc-knowledge/src/projections.rs`) and a governed publication path (`crates/pmc-application/src/projection_publication.rs`) exist in the crates, but the desktop app does not publish projections yet. The desktop reads Vault files only to verify Evidence, and records the result in the Ledger. See [the architecture overview](../architecture.md).

## Revisit when

A workflow needs controlled two-way authoring and can define one unambiguous conflict authority.
