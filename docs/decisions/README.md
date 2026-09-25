# Architecture decisions

These records keep consequential technical choices together with the alternatives that were considered and the consequences that follow. Each record ends with a short section on how much of the decision is built today.

An accepted record is not rewritten to change its meaning. A later record replaces it and links back.

## Records

- [0001: Separate Product Vault and Product Ledger authority](0001-separate-vault-and-ledger-authority.md)
- [0002: Stage AI execution behind governance](0002-stage-ai-behind-governance.md)
- [0003: Use a Windows-first cross-platform core](0003-windows-first-cross-platform-core.md)
- [0004: Use forward-only Product Ledger migrations with verified recovery](0004-forward-only-product-ledger-migrations.md)
- [0006: Use six human-authority channels and prepared intents](0006-use-six-channel-hitl-and-prepared-intents.md)
- [0007: Bound managed-projection automation and escalate large rebuilds](0007-bound-managed-projection-automation.md)
- [0008: Exclude managed projections from Full Vault Archives](0008-exclude-managed-projections-from-full-vault-archives.md)
- [0009: Bound WebView2 platform-runtime egress](0009-bound-webview2-platform-runtime-egress.md)
- [0010: Encrypt Operational Backups as tar.zst streamed through passphrase-mode age](0010-age-encrypted-operational-backup-archives.md)
- [0011: Open file and folder pickers in the host, never in the webview](0011-host-owned-native-dialogs.md)
- [0012: Record backup, restore and workspace events in a host audit log outside the Ledger](0012-host-audit-log-for-events-outside-the-ledger.md)

The numbers are not contiguous. One record concerned internal development tooling rather than the product and is not included here.
