# 0003: Use a Windows-first cross-platform core

Status: Accepted
Date: 2026-08-10

## Context

The first user works on Windows, but the product should keep a credible path to a future macOS build. A native Windows-only stack would simplify the first release and make that path expensive. A browser-only application would reduce platform coupling but weaken local filesystem access, secure credential storage and desktop integration.

## Decision

Use Tauri 2, React, TypeScript, Vite and Rust, with explicit platform adapters. Windows is the only verified environment. Core domain behaviour, storage contracts and UI state must not embed Windows-only paths or APIs. macOS is architecture-ready but not supported until a native build, signing, integration and smoke tests exist.

## Alternatives considered

- Electron: a mature desktop ecosystem, with a larger runtime and resource footprint than this local-first tool needs.
- WinUI: strong Windows integration, but it would make a macOS build much more expensive.
- Browser or PWA: a portable UI, but a weaker fit for controlled local Vault access, OS credential storage and offline desktop work.

## Consequences

- Platform-specific behaviour sits behind adapters with contract tests.
- Windows packaging, display scaling, filesystem and credential behaviour are release requirements.
- macOS readiness adds design and abstraction cost before macOS delivers anything to users.
- Nothing in the documentation or the UI may imply macOS support.

## Current state

The desktop app is `apps/desktop` (React and TypeScript, built with Vite) with its Rust host in `apps/desktop/src-tauri`. Platform concerns such as workspace locations and the Windows credential store live in `crates/pmc-platform`. Installer packaging is turned off (`bundle.active` is `false` in `tauri.conf.json`). See [the architecture overview](../architecture.md).

## Revisit when

- Tauri blocks a critical capability.
- macOS becomes a committed release target.
- Measured desktop performance misses its targets.
