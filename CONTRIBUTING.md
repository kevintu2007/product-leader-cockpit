# Contributing

Thank you for your interest in Product Mission Control. The project is in beta.

## Issues

Issues are welcome.

When you report a bug, include:

- the steps to reproduce it
- what you expected, and what happened instead
- your Windows version
- the app version, or the commit you built
- the Correlation ID from the error panel, if one is shown

Use the sample workspace, or other made-up data, when you describe a problem. Do not post real personal or company data. Report security problems privately, as described in [SECURITY.md](SECURITY.md).

Feature ideas are welcome as issues too. Describe the problem you want solved, not just the screen you have in mind.

## Pull requests

Pull requests are not accepted during the beta. We may close them without review. If you have a fix, open an issue that describes it.

## Build and run

The prerequisites and commands are in [the README](README.md#build-from-source). To install the dependencies and run the app:

```powershell
npm ci
npm run desktop:dev
```

On first run, choose **Learn with sample data** to work on the synthetic sample workspace.

## Verification

`npm run verify` runs every required check and stops at the first failure. The checks run in this order:

1. `python scripts/validate-foundation.py .`: checks that required files and markers exist and that relative Markdown links resolve.
2. `node scripts/verify/failure-probe.mjs`: confirms that a failing check stops the run.
3. `npm run format:check`: runs Prettier.
4. `npm run lint`: runs ESLint and allows no warnings.
5. `npm run typecheck`: runs TypeScript.
6. `npm run test`: runs Vitest.
7. `npm run build`: builds the frontend for production.
8. `cargo metadata --locked --no-deps --format-version 1`: checks that `Cargo.lock` is current.
9. `cargo fmt --check`: checks Rust formatting.
10. `cargo fetch --locked`: downloads the locked Rust dependencies.
11. `cargo build --workspace --locked --offline`: builds the Rust workspace without network access.
12. `cargo clippy --workspace --locked --all-targets -- -D warnings`: runs Clippy and treats warnings as errors.
13. `cargo test --workspace --locked`: runs the Rust tests.
14. `node scripts/verify/dependency-evidence.mjs`: regenerates the dependency inventory and compares it with `docs/evidence/s1/dependency-evidence.json`.
15. `npm run policy:check`: runs the policy checks described under Conventions.

The full run takes a long time, mostly in the Rust build and tests.

The end-to-end tests are separate from `verify`. They drive the renderer in Chromium and check the route, page title, keyboard and reflow contracts:

```powershell
npx playwright install chromium
npm run test:e2e
```

To fix formatting, run `npm run format`.

## Conventions

The checks above enforce the conventions below, apart from the one noted as a design record:

- The renderer does not use the network or navigate. The policy check rejects `fetch`, `XMLHttpRequest`, `WebSocket`, `window.open`, `location` changes, and form or link targets in source code. Rust code may not use network APIs.
- The Tauri surface stays small:
  - The capability file grants no permissions.
  - The Content Security Policy and the WebView2 browser arguments must match the reviewed values exactly.
  - Every Tauri command must be on the reviewed command list, and every command on that list must be declared and registered.
  - There is no generic SQL, file-read or shell command.
- Rust owns the rules.
  - A command from the webview carries only the record ids and versions it read, the person's own text or choices, opaque tokens the host issued for a file or folder the person chose, and a request id used to make retries safe.
  - The host creates every other id, every timestamp, and the Prepared Intents, audit events and receipts.
  - No file or folder path passes between the webview and the host. File and folder pickers are opened by the host ([Decision 0011](docs/decisions/0011-host-owned-native-dialogs.md)).
- Rust code uses no `unwrap` or `expect` outside tests, and no `unsafe` outside tests except one reviewed call to the Windows `CompareStringOrdinal` API in `crates/pmc-platform/src/windows_names.rs`. The workspace lint settings enforce this; every other crate forbids `unsafe` outright.
- Product Ledger migrations only go forward:
  - Migrations are an ordered list, and each applied migration is recorded with a checksum.
  - There are no down migrations.
  - The Ledger opens only the schema version the build expects. An older supported Ledger is upgraded in place, after a verified backup.
  - [Decision 0004](docs/decisions/0004-forward-only-product-ledger-migrations.md) records the design.
- Dependencies are pinned, and each new one must have an approved license. The dependency inventory check enforces this.

## License

This project is source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE): you may use, study and change it for any noncommercial purpose. Commercial use needs a separate license from the author.
