# Security model

Product Mission Control is a single-user desktop application that keeps its data on the local
machine. This document states what version 0.2.0-beta protects against, how, and what it does not
attempt. To report a vulnerability, see [SECURITY.md](../SECURITY.md).

## Assumptions and threats considered

The application assumes one person using their own Windows account. There are no user accounts,
no server, no synchronisation and no remote access. Within that setting the design addresses:

- the renderer (the WebView content) being made to reach the filesystem, the network, SQL or a
  shell;
- a stored or chosen path escaping the Product Vault;
- an approval applied to something other than what the person reviewed;
- disclosure by aggregation, where a list reveals more than its label says;
- synthetic sample data mixing with real data;
- loss of the records, and a backup that cannot actually be restored;
- application data leaving the machine;
- a tampered installer.

## Getting the installer

The beta installer is **not code-signed**. Windows SmartScreen therefore warns that it does not
recognise the app, and the warning cannot tell a genuine download from a tampered one. The release
publishes a SHA-256 checksum file next to the installer; check it before running the installer, as
the [user guide](user-guide/walkthrough.md#download-and-check-the-installer) shows. A mismatch means
the file is not the one that was published.

The installer is a per-user NSIS installer: it needs no administrator rights and writes the program
to `%LOCALAPPDATA%\Product Mission Control`. Its template is kept in the repository
(`apps/desktop/src-tauri/nsis/installer.nsi`) with the upstream option to delete application data on
uninstall removed, and the policy check pins the template's hash, so an uninstall never deletes
records, backups or Vault files. The release build refuses to run from a modified working tree and
records its commit and build inputs next to the installer.

## Data locations

Paths are derived by the application or chosen by the person in a native dialog opened by the host
(see [ADR 0011](decisions/0011-host-owned-native-dialogs.md)). No IPC command accepts a path from the
renderer, and none returns one. After a dialog, the renderer receives an opaque selection token, a
file or folder name to show, and limited facts such as when the file was observed or which existing
Evidence records have the same content.

| Data | Location | Notes |
| --- | --- | --- |
| Product Ledger | `%LOCALAPPDATA%\ProductMissionControlDesktop\workspaces\<live or training>\product-ledger.sqlite3` | One SQLite file per workspace. Not encrypted by the application |
| Settings and host audit log | `%LOCALAPPDATA%\ProductMissionControlDesktop\` | No secret is stored in either |
| Product Vault (your workspace) | A folder you choose | The Ledger stores Vault-relative references, verification states and fingerprints, never file contents |
| Product Vault (sample) | `...\workspaces\training\demo-vault\` | Synthetic files written by the app |
| Operational Backups | A folder you choose | Encrypted archives, see below |
| Recovery passphrase | Nowhere, unless you choose to have it remembered | Then in Windows Credential Manager, one entry per Windows account: a second PMC data folder on the same account that also remembers a passphrase replaces it |
| Presentation preferences | WebView local storage | Text scale and theme; never written to the Ledger |

The application directory and each workspace directory are checked when resolved: the path must be
canonical, stay under the application data folder, and contain no symbolic link or Windows reparse
point.

## Sample and Live workspaces

There are exactly two workspaces, represented as a typed value rather than a path: your workspace
(Live) and the sample workspace (Training). You choose between them on the first-run screen and in
Settings → Workspace; switching restarts the app. The sample workspace is always labelled: the top bar
shows a Sample workspace badge, and the window title ends in "Sample data" (in the chosen language,
or in English at first when the language follows Windows).

Synthetic data can be written only to the sample workspace: the platform crate refuses to issue a
seeding permit for Live, and the sample is filled through the same typed Ledger commands the app
uses, never raw SQL. Resetting or deleting the sample is offered only from your workspace, shows
exactly what it removes, and never touches your workspace, settings or backups.

## Backups and restore

Operational Backups are described in
[ADR 0010](decisions/0010-age-encrypted-operational-backup-archives.md). In short:

- each backup is a tar and zstd stream encrypted with age in passphrase mode (scrypt), readable with
  the public `age` tool;
- an archive is listed as valid only after it has been decrypted again in full and every member,
  the snapshot and the record inventory have been checked;
- your workspace accepts new records and changes only after a backup has verified, and a verified
  backup is made before every Ledger upgrade and before the Vault folder changes;
- the passphrase is shown once and typed back; PMC cannot recover it, and without it a backup cannot
  be restored on any computer;
- a restore is prepared as a preview that names what it replaces and what it keeps, and can be
  declined; before it replaces anything, PMC keeps a copy of the current Ledger files in the backup
  folder.

Backup, restore, Vault-folder and sample events are recorded in a host audit log outside the Ledger,
so a restore cannot erase its own record ([ADR 0012](decisions/0012-host-audit-log-for-events-outside-the-ledger.md)).

## Data classification

Every record carries one of five classifications: `public`, `internal`, `confidential`,
`restricted` or `unclassified`. `unclassified` is the default and is treated as the most
restrictive value.

When values are combined (`DataClassification::combine` in `crates/pmc-domain`), the result is the
more restrictive of the two, and `unclassified` absorbs everything else. Compositions use this to
fold classification upward: a Portfolio row, a People entry, a Lens point and the Product inspector
are each labelled with the most restrictive classification of everything they expose, and the
inspector and the Lens name the record that forced the label. An Evidence link records the combined
classification of both sides at the time of linking, kept separate from each side's current value.

A preview lists every classification source and the resolved result, and the digest the person
approves covers them. If any source changes before approval, the approval is refused.

The domain crate also maps classification to external-AI eligibility. Nothing in the application
calls an AI provider, so this mapping is not exercised by the running application.

## Renderer restrictions

The renderer can do only what the registered IPC commands allow. `scripts/verify/check-policies.mjs`
runs as part of verification and fails when:

- the Tauri capability file grants any permission, or covers a window other than `main`;
- `tauri.conf.json` has unexpected keys, a non-loopback development URL, a remote window URL, a
  Content Security Policy other than the reviewed one, a change to the fixed WebView2 browser
  arguments, or a bundle configuration other than the reviewed installer settings;
- a declared or registered IPC command is not in the reviewed list of 108, or a listed command is not
  both declared and registered;
- an IPC argument or DTO field carries a path;
- a Rust or TypeScript source file under `apps/` or `crates/` contains a generic SQL, file or shell
  command, a Tauri plugin or native process API (outside a few hash-pinned harnesses and the
  hash-pinned native dialog blocks), a direct Rust network API, or a renderer network or navigation
  API (`fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`, `sendBeacon`, `window.open`,
  assignments to `location`, and `<form>` or `<a>` elements with an `action` or `href` attribute);
- an npm or Cargo manifest adds a dependency outside its reviewed list, or one whose name suggests a
  filesystem, shell, HTTP, network, SQL, URI, opener or process capability.

The check also runs its own negative fixtures, so a weakened pattern that stops catching them fails.

The Content Security Policy is `default-src 'self'`, with `connect-src` limited to `'self'`,
`ipc:` and `http://ipc.localhost`, `object-src`, `base-uri` and `form-action` set to `'none'`,
styles limited to `'self'` and inline styles, and images limited to `'self'` and `data:`.

## Network egress

Nothing in the application opens a network connection, and the policy check above keeps network
clients out of the dependency graph and the source. There is no update check, telemetry or crash
reporting. The Microsoft WebView2 runtime that hosts the renderer can still make its own platform
connections. That exception, its limits, and how it is observed are recorded in
[ADR 0009](decisions/0009-bound-webview2-platform-runtime-egress.md). It does not permit any
application data to leave the machine.

## Product Vault path containment

Evidence references store a Vault-relative path. The domain rejects absolute paths, drive letters,
backslashes and empty, `.` or `..` segments. Before each read the host validates the Vault root
again (it must exist, be a directory, be canonical, and not be a link or reparse point) and resolves
the relative path with `resolve_contained_path`, which rejects any existing component that is a link
or reparse point and any result outside the root. A file chosen for new Evidence must be inside the
Vault folder; one outside it is refused.

Validation happens per operation, so it is never older than the read it guards. It does not close
the narrower race between validation and the read inside one operation; closing that on Windows
needs handle-relative file APIs and is not done.

## Errors and diagnostics

Every IPC failure is a safe envelope with a stable code, a message key, typed parameters, a
correlation id and a retryable flag. SQLite rows, file paths and internal diagnostics are not
mapped into it. The renderer shows a localised message and the correlation id, which can be copied.

## Out of scope

- An attacker who can act as the user, read the user's files or modify the application binary. The
  Ledger and the Vault are ordinary files and are not encrypted by the application; only backups
  are.
- Malware, a compromised operating system, and physical access.
- Multiple users, shared machines with shared accounts, and network services.
- Screen-reader conformance, which the current UI contract does not claim.
- Code signing, which the beta does not have.
