/**
 * English copy for the app shell and the shared overlays: navigation, theme,
 * the policy strip, the safe error detail (O05), text scale (O06) and the
 * H2b approval sheet (O04).
 *
 * Route names (Executive Cockpit, Portfolio, ...) are not here: the frozen
 * Route Contract keeps them as canonical English identifiers in every
 * language (`shell/routes.ts`). Nor is the product name, Product Mission
 * Control, which is a name and reads the same in every language.
 */
export const SHELL_EN = {
  "shell.nav.primary": "Primary",
  // Product nouns shown as headings or labels. They are names, so they stay
  // English in every language, but they live here with the rest of the copy.
  "noun.portfolioLens": "Portfolio Lens",
  "noun.products": "Products",
  "noun.evidence": "Evidence",
  "noun.productLedger": "Product Ledger",
  "noun.productVault": "Product Vault",
  "shell.routePlaceholder":
    "This screen isn't available yet. It opens once its part of the app passes verification.",
  "shell.theme.switchToDark": "Switch to the dark theme",
  "shell.theme.switchToLight": "Switch to the light theme",
  "shell.theme.dark": "Dark theme",
  "shell.theme.light": "Light theme",
  "shell.nav.attention.one": "{route}, {count} item needs attention",
  "shell.nav.attention.other": "{route}, {count} items need attention",

  "policyStrip.degraded": "Degraded",
  "policyStrip.evidenceVerificationPending": "Evidence verification pending",
  "policyStrip.outOfSync": "Out of sync",
  "policyStrip.cancelling": "Cancelling",
  "policyStrip.backupDue": "Backup due",

  "errorDetail.correlationId": "Correlation ID",
  "errorDetail.copy": "Copy",
  "errorDetail.copied": "Copied",
  "errorDetail.retry": "Try again",

  "textScale.title": "Text size",
  "textScale.sampleBody":
    "Good morning. Start with what really moves results today. 4 items need attention; the first is an Action Request whose response is overdue.",
  "textScale.sampleLabel": "Placed here: a commitment was missed",

  "h2b.confirmPrompt": "Type “{phrase}” to approve",
  "h2b.approve": "Approve",
  "h2b.reject": "Reject",
  "h2b.approved": "Approved",
  "h2b.rejected": "Rejected",
  "h2b.recoveryEvidence": "Verified recovery evidence",
  "h2b.verifiedAt": "Verified at",
  "h2b.scope": "Scope",
  "h2b.compatibility": "Compatibility",
  "h2b.transferDetails": "External transfer details",
  "h2b.provider": "Provider",
  "h2b.account": "Account",
  "h2b.purpose": "Purpose",
  "h2b.exactPayload": "Exactly what is sent",
  "h2b.irreversible":
    "Once approved this can't be taken back: the content above is sent to the external service and can't be recalled or undone.",
  "policyStrip.backingUp": "Backing up",
} as const;
