# 0011: Open file and folder pickers in the host, never in the webview

Status: Accepted
Date: 2026-09-21

## Context

The beta needs a person to choose three kinds of location: the folder backups go to, the Product
Vault folder, and a file to create Evidence from. Every surface so far keeps paths out of the
webview: the renderer names records by id and never sends or receives a file path. A webview-side
dialog plugin would hand the chosen path to the renderer and need a Tauri permission, both of which
the existing boundary forbids ([0009](0009-bound-webview2-platform-runtime-egress.md) keeps the
permission set empty).

## Options considered

- **`tauri-plugin-dialog`.** The renderer receives the path, and a Tauri permission is needed.
- **Typing a path in the renderer.** The renderer would author paths.
- **Native dialogs opened by the Rust host.** The host holds the path; the renderer only learns what
  it may show.

## Decision

1. **The host opens the dialog.** Native Windows dialogs are opened by the Rust host through `rfd`, a
   Windows-only dependency of the desktop crate, pinned to an exact version with default features
   off. No Tauri plugin is used and the capability manifests stay empty.
2. **One command per purpose.** Each picker is a narrow, purpose-named command. There is no generic
   "pick a path" or "read a path" command.
3. **No path crosses the IPC boundary, in either direction.** The host validates the choice
   (canonical, inside the allowed root where one applies, no links or reparse points) and either
   stores it itself or holds it behind an opaque, expiring selection token that later commands in the
   same flow redeem; a retry under the same request id redeems it again without a second effect. The
   renderer receives only the token and facts it may show, such as a file or folder name, when the
   file was observed, availability, or the ids of existing Evidence with the same content.
4. **The dialog code lives in one reviewed module**, and the policy check pins the content hash of
   each block that calls it.
5. **The policy check enforces the boundary.** It allows `rfd` only in the desktop crate, refuses
   dialog plugins, refuses IPC arguments or DTO fields that carry a path, and has negative fixtures
   for each refusal.
6. **A token is not a read capability.** Observing the chosen file keeps the existing containment and
   verification rules.

## Consequences

- The renderer cannot learn where the person's files are, and a compromised page cannot ask for an
  arbitrary path.
- Each new location the product needs costs one reviewed command and a policy update.
- A dialog is modal to the host; commands that open one must not hold the Ledger lock while it is
  open.

## Current state

- `apps/desktop/src-tauri/src/native_dialogs.rs` is the only caller of `rfd` (0.17.2). It holds the
  backup-folder picker, the Vault-folder picker, the Evidence file picker, the restore sheet's
  backup-file picker and the single-instance message box.
- `scripts/verify/check-policies.mjs` pins the hash of each of those blocks and refuses `rfd`
  anywhere else.
- Moving an existing Evidence file to a new location still has no picker and is not available on
  screen.
