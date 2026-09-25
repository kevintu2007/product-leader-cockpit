# 0002: Stage AI execution behind governance

Status: Accepted
Date: 2026-08-10

## Context

Product Chief of Staff workflows may benefit from external AI models, but operating data can be sensitive and the applicable organizational AI policy is not yet known. A model must not become the product's authority, and a human approval must not be able to bypass classification or a global denial.

## Decision

Product Chief of Staff is a governed workflow role, not an autonomous agent or a model identity. External AI is globally disabled. While it is disabled, the product may show a metadata-only local preview but cannot build, export, transmit or invoke an external-AI Work Packet.

If a later policy change enables external AI, every packet must still pass the selected provider, account and purpose policy, authoritative classification with inheritance, an exact-content preview, and a per-packet human approval. By classification:

- Public may be allowed through those checks.
- Internal additionally requires explicit organizational approval of provider, account, retention and purpose.
- Confidential, Restricted and Unclassified are denied.

AI output is untrusted. It cannot directly update the Product Ledger, overwrite the Product Vault, approve a report, send a message or change policy.

## Alternatives considered

- An in-app provider API first: rejected, because it would pick a provider early and force credential, retention, evaluation and egress decisions.
- Permanent external command-line execution only: rejected, because it would rule out a future approved in-app provider without improving the safety model.
- Local manual drafting only, permanently: rejected as a permanent rule, but kept as the complete fallback while external AI is disabled.
- Autonomous or background agent execution: rejected, because it conflicts with human control and least authority.

## Consequences

- Turning on the global switch is necessary but never sufficient.
- Every hosted model, whatever its interface, is subject to the same egress policy when it would receive operational data.
- A denial creates no serialized payload and no sensitive temporary file.
- Integrating a provider requires new policy, credential, evaluation and audit work.
- Manual drafting is a supported workflow, not an error state.

## Current state

No AI provider is connected to the application, and nothing builds or sends a Work Packet. The classification rule above is implemented as `classify_external_ai_eligibility` in `crates/pmc-domain/src/classification_governance.rs`, with inheritance in which Unclassified outranks every other level. The settings model in `crates/pmc-platform/src/settings.rs` has an `external_ai_enabled` switch that defaults to off; nothing else reads it yet. See [the security model](../security-model.md).

## Revisit when

An applicable organizational AI policy is recorded and a provider-specific design is approved.
