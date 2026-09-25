# 0010: Encrypt Operational Backups as tar.zst streamed through passphrase-mode age

Status: Accepted
Date: 2026-09-21, addendum 2026-09-22

## Context

A beta that holds real records must keep them recoverable. [0004](0004-forward-only-product-ledger-migrations.md)
already requires a verified backup before every schema migration, and ordinary writes wait until a
due backup has verified. The product owner decided that backups go to a folder the person picks, are
encrypted with a passphrase the person holds, can be restored on another computer, and can never be
recovered silently without that passphrase.

## Decision drivers

- A person must be able to recover their data with public tools if Product Mission Control is gone.
- The app must not own a cryptographic container format for ever.
- No secret, credential or Vault file may end up in an archive.
- An archive is listed as valid only after it has been read back and checked in full.

## Options considered

- **Argon2id and XChaCha20-Poly1305 in an app-specific container.** Good primitives, but the app
  would own framing, nonces, chunk order and downgrade rules for ever, and no standard tool could
  open it.
- **Encrypted ZIP.** No single assurance profile, weak legacy modes, and metadata exposure that is
  easy to misread.
- **tar and zstd, encrypted with age in passphrase mode.** A standard, documented format that the
  public `age` and `rage` tools can open.

## Decision

1. **Container.** An Operational Backup is one file: a deterministic tar stream, compressed with
   zstd, encrypted with age v1 using the scrypt passphrase recipient. No app header is added.
2. **Key derivation.** New archives use scrypt with N = 2^18, r = 8, p = 1 and a fresh 16-byte salt.
   Restore refuses a work factor above 2^20 before allocating memory.
3. **Contents.** A verified SQLite snapshot of the Ledger, the non-secret settings in scope, an
   authority inventory (every record's type, id and version), and a versioned manifest listing every
   member's name, size and SHA-256 plus the snapshot checksum. The manifest is inside the encryption.
   No secret, credential, Vault file or managed projection is included.
4. **Integrity.** Two checksums that are never merged: the snapshot checksum inside the manifest, and
   the SHA-256 of the finished archive, kept in the verified-archive registry.
5. **Verification before publication.** The archive is decrypted again in full, every member checksum
   recomputed, tar paths checked (no traversal, no links, exact member set), the snapshot reopened by
   a read-only inspector, and the authority inventory re-derived from it and compared byte for byte.
   Staging happens beside the destination and publication is an atomic rename; an interrupted archive
   is never listed as valid.
6. **Recovery passphrase.** By default a locally generated ten-word passphrase; a person may use
   their own of at least 20 characters or six words. It is shown once, typed back exactly, and
   acknowledged: PMC cannot recover it, and without it these backups cannot be restored on this or
   another computer. It is never logged or written to the Ledger, settings or an archive. There is no
   hidden recovery key.
7. **Unattended backups.** Only when the person ticks *Remember on this Windows account for automatic
   backups* is the passphrase kept in Windows Credential Manager; settings hold only a reference.
8. **Archive name.** `pmc-operational-<UTC time to the millisecond>-<archive id>-v1.tar.zst.age`.
9. **Standard recovery, without PMC:** `age -d archive.tar.zst.age | zstd -d | tar -x`.

## Consequences

- Data stays recoverable with public tools.
- The Rust `age` crate is pre-1.0 and has no published independent audit; this record claims none.
  Its SSH, plugin and armor features are disabled.
- Unattended backups store the passphrase on this Windows account; declining means PMC asks for it
  whenever a backup is due.
- A lost passphrase makes every archive made with it unrecoverable. This is stated in the interface
  at the moment the passphrase is set.

## Current state

- `crates/pmc-platform` holds the archive writer and reader, with `age` 0.12.1, `tar` 0.4.46 and
  `zstd` 0.13, default features off.
- `crates/pmc-application/src/operational_backup.rs` verifies each archive through
  `inspect_standalone_snapshot` from `crates/pmc-ledger/src/sqlite/inspection.rs`, a read-only
  inspector that never constructs a writable Ledger.
- `crates/pmc-application/src/restore_service.rs` re-derives the inventory from the snapshot it is
  about to install and compares it wholesale before a restore.

## Revisit when

- `age` publishes a 1.0 release or an audit, or an advisory affects the enabled features.
- Measured backup time or memory exceeds the targets.
- A full Vault archive is scheduled; it would reuse this container with its own profile.
