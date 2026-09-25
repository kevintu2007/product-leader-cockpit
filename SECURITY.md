# Security

Product Mission Control is a local-first Windows desktop app. It has no server component, and no user accounts or sign-in.

## Scope

In scope: the desktop app, both its host and its renderer, and how the app handles the data files listed below.

What the app does today:

- It runs no network service, and the Rust code uses no network API. A policy check in `npm run verify` enforces this.
- The renderer's Content Security Policy only allows the app itself and its host. The same policy check blocks network, navigation and native-plugin APIs in the renderer source code.
- The Tauri capability file grants no plugin permissions.
- The host exposes only a fixed list of typed commands, and the policy check verifies that list. There is no generic SQL, file-read or shell command.
- The Microsoft WebView2 runtime can still open its own connections, which the app does not control. These connections carry no app data. [Decision 0009](docs/decisions/0009-bound-webview2-platform-runtime-egress.md) records their limits.
- The desktop app has no AI provider connected and sends no data to any external service.

## Where data lives

- The Product Ledger is a SQLite file under `%LOCALAPPDATA%\ProductMissionControlDesktop\workspaces\`. Your workspace and the sample workspace each have a folder there. The Ledger holds records, audit events and approval receipts.
- The Product Vault is a folder of Markdown and Evidence files that belongs to you; you choose it in Settings. The Ledger stores only references, verification states and fingerprints. No file or folder path passes between the renderer and the host.
- Backups go to a folder you choose, encrypted with a recovery passphrase you hold. The passphrase is stored only if you choose to have it remembered, and then in Windows Credential Manager.
- The sample workspace holds only synthetic data, written by the app. It is always labelled in the window title and the top bar.

The app does not encrypt the Ledger or the Vault; only backups are encrypted. Anyone who can read your Windows user profile can read the Ledger.

## The installer

The beta installer is not code-signed, so Windows SmartScreen warns about it. Check the published SHA-256 checksum before you run it; the [user guide](docs/user-guide/walkthrough.md#download-and-check-the-installer) shows how.

[docs/security-model.md](docs/security-model.md) describes the full model.

## Supported versions

| Version                   | Supported |
| ------------------------- | --------- |
| Latest 0.2.x beta release | Yes       |
| Any earlier build         | No        |

Fixes go into the latest 0.2.x release only.

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open this repository's **Security** tab and choose **Report a vulnerability**. Do not open a public issue, pull request or discussion about a vulnerability.

Include in your report:

- the app version, or the commit you built
- your Windows version
- the steps to reproduce the problem
- what an attacker would need, and what they could gain

Use synthetic data only. Do not include real personal or company data.

You will get a reply through the private advisory. Please give us time to release a fix before you disclose the vulnerability publicly.
