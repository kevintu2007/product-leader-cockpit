# Changelog

This file records notable changes. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Version numbers follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.2.0-beta - 2026-09-25

The first beta, and the first version with a Windows installer. It can hold real records, with
verified, encrypted backups and restore. The working Ledger and Vault files are not encrypted by the
application.

### Added

- A per-user Windows installer (NSIS, no administrator rights), published with a SHA-256 checksum
  file. Uninstalling never deletes records, backups or Vault files.
- A first-run choice between your own workspace and a sample workspace, switchable later in Settings.
  The sample is clearly labelled, can be reset or deleted from your workspace, and is prepared by the
  app itself.
- A Getting started list on the Executive Cockpit of your workspace: backup folder, recovery
  passphrase, first verified backup, Vault folder, first Product. Each step is read from the real
  state, and the list disappears when all are done.
- Operational Backups: encrypted with age in passphrase mode, written to a folder you choose, read
  back and verified before they count. A recovery passphrase that is shown once and can optionally be
  remembered in Windows Credential Manager. Your workspace accepts changes only after a backup has
  verified.
- Restore from a backup, as a preview you can decline, including when the current Ledger cannot be
  opened. The current Ledger files are kept before anything is replaced.
- In-place upgrade of a Ledger written by an earlier supported version, after a verified backup.
- Choosing the Product Vault folder for your workspace, as an approved change made after a backup.
- Evidence from a file: choose a file inside the Vault folder; PMC records where it is and its
  fingerprint.
- Screens that create and edit records: Portfolios, Products, Initiatives, Projects, Milestones,
  Roadmaps, KPIs, Stakeholders and their relationships, Action Requests, Decision Requests, Risks
  (including their response) and Issues.
- System Health, which says whether PMC can read and keep your records.
- A host audit log, outside the Ledger, for backup, restore, Vault-folder and sample events.
- Six interface languages: English, Traditional Chinese, Simplified Chinese, Japanese, Korean and
  Spanish. The language follows Windows unless you choose one.
- A second launch brings a message instead of a second window.
- A user guide in English and Traditional Chinese.

### Changed

- The Work Queue shows Risk titles.
- File and folder pickers are opened by the host; no file path crosses the IPC boundary.

### Not yet available

- Review periods, Fact Packs and approved reports.
- Recurrence links between Issues. One sample Issue is titled "second occurrence" to show repeated
  work; no link is stored.
- Moving an Evidence file to a new location from the UI.
- Code signing, automatic updates and a full Vault archive.
- AI assistance.

## 0.1.0 - Not released

The first alpha. It ran on Windows from source and could be tried with the synthetic Training
workspace.

### Added

- A Tauri 2 desktop app with a React and TypeScript renderer. The host and the domain rules are written in Rust. The interface is in Traditional Chinese.
- The Product Ledger: a local SQLite store for records, audit events, Prepared Intents and approval receipts.
- The Executive Cockpit with the Portfolio Lens, which places Products by milestone timing and outcome observability and sizes them by verified Evidence coverage. It also lists attention items and why each one is where it is.
- The Portfolio table and the Product inspector, which has Structure, Evidence and People tabs.
- The People directory of Stakeholders, with each person's responsibilities, dependencies and outstanding requests.
- The Work Queue for Action Requests, Actions, Decision Requests, Risks and Issues, with filters, attention ordering and the next steps each record's state allows.
- Governed changes from the Work Queue:
  - Action Requests: accept, decline and withdraw.
  - Actions: start, link completion Evidence, complete, cancel and reopen.
  - Decision Requests: resolve, including follow-up Action Requests, and withdraw.
  - Risks: record an occurrence as a new Issue, or close.
  - Issues: resolve, close and reopen.
- The review sheet for changes that need approval. It shows a prepared preview, and you approve it or reject it. Each approval produces a receipt that can be used only once. A rejection is recorded. A preview expires after five minutes.
- Evidence-or-Judgment support checks for changes that need Evidence.
- Evidence actions in the Product inspector: link Evidence to a Product, pin a fingerprint, and check an Evidence file again.
- The Reviews & Reports screen, which lists the items the Work Queue flags.
- The Product Vault screen, which shows whether the Vault is available and the state of each Evidence reference.
- The Settings screen, with text size and the Ledger and Vault status. A light and dark theme switch is in the top bar.
- Safe error panels, each with a Correlation ID you can copy.
- `pmc-seed`, which builds the synthetic Training workspace, and the `--pmc-workspace=training` switch, which works in debug builds only.
- `npm run verify`, which runs formatting, lint, type, test, build, Clippy and policy checks, and Playwright end-to-end tests for the route, page title, keyboard and reflow contracts.

### Not yet available

- System Health (the screen is a placeholder).
- Review periods, Fact Packs and approved reports.
- Screens that create records.
- A way to choose a Product Vault for the Live workspace.
- Moving an Evidence file to a new location from the UI.
- Backup and restore.
- Upgrading a Ledger written by an older build.
- Installers.
- AI assistance.
