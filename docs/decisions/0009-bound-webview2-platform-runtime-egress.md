# 0009: Bound WebView2 platform-runtime egress

Status: Accepted
Date: 2026-08-17

## Context

The Windows Tauri shell uses no network plugin, loads a local packaged frontend and applies a fail-closed Content Security Policy. Even so, observing the whole process tree during a static launch, close and reopen showed the Microsoft WebView2 Runtime child process opening Microsoft HTTPS connections. Browser arguments that disable background networking, component updates, domain reliability reporting, sync, metrics and pings did not remove this platform-owned traffic.

The original requirement was zero non-loopback traffic from the whole process tree. Meeting it literally would require an organization-managed WebView2 policy, a narrowly bounded platform-runtime exception, or replacing the Tauri, React and WebView2 architecture.

## Decision drivers

- Keep the Windows-first Tauri and React architecture.
- Keep all Product Mission Control application and operational egress fail-closed.
- Do not change organization or Windows enterprise policy.
- Make the exception observable and mechanically narrower than a general WebView network waiver.

## Options considered

- Require the WebView2 `ExperimentationAndConfigurationServiceControl=RestrictedMode` policy before the app may run. Closest to zero process-tree traffic, but it makes a personal tool depend on an enterprise policy the user may not control.
- Bound a platform-runtime exception. Distinguish signed Microsoft WebView2 runtime and configuration traffic from application egress, while keeping the strict renderer CSP, the empty native permission set and process-tree observation.
- Replace the UI runtime. Might remove the dependency, but reopens the architecture.

## Decision

The bounded platform-runtime exception is accepted. It applies only when all of these hold:

- The root Product Mission Control process opens no non-loopback connection.
- The owning child executable is `msedgewebview2.exe`, has a valid Microsoft Authenticode signature, and descends from the Product Mission Control process.
- The connection uses HTTPS on port 443 and is classified only as WebView2 runtime or configuration traffic.
- The only non-Microsoft endpoint allowed is WebView2's DNS-over-HTTPS request to `https://chrome.cloudflare-dns.com/dns-query`. It must carry no application payload and does not extend to any other Cloudflare service.
- QUIC and every other remote renderer URL remain prohibited.
- The packaged renderer keeps its exact local-only CSP and has no remote window URL.
- The Tauri capability grants no plugin permissions.

The exception does not authorize any application payload to leave the machine: no Product, Portfolio, Stakeholder, Ledger, Vault, report, diagnostic, backup, credential, prompt, Work Packet, AI-provider, analytics, crash-report or update traffic. External AI stays disabled as described in [0002](0002-stage-ai-behind-governance.md).

## Consequences

- The Tauri and React architecture stays.
- Application egress stays deny-by-default and testable on its own.
- A local-first launch can still cause Microsoft-signed WebView2 traffic, and the product cannot promise a network-silent process tree without an external Windows policy or network control. This is stated rather than hidden.
- A renderer request also goes through WebView2 processes, so the CSP, the remote-window ban, the empty permission set, the signature check, the port restriction and the smoke test work only together.
- WebView2 behaviour may change; a WebView2 version change means rerunning the full smoke test.
- An organization may prohibit this traffic; the app must state the platform prerequisite and support an IT-managed RestrictedMode deployment.

## Current state

- `apps/desktop/src-tauri/tauri.conf.json` sets the local-only CSP and the WebView2 browser arguments, and `apps/desktop/src-tauri/capabilities/default.json` grants an empty permission list. No Tauri plugin is registered. The host registers its own typed commands, which read and write the local Ledger and read Vault files; none of them opens a network connection.
- `scripts/verify/check-policies.mjs` rejects an expanded CSP, extra capability permissions or keys, and direct dependencies on known network client packages.
- `scripts/verify/windows-smoke.ps1` launches the app, observes the full process tree, and fails on any connection outside the rules above.

## Revisit when

- WebView2 traffic comes from another process, port, signer or purpose.
- Any application payload appears in platform traffic.
- IT provides or requires WebView2 RestrictedMode.
- Tauri, WebView2 or the Windows deployment architecture changes.
