import type { SHELL_EN } from "./shell.en";

/** Textos en español del marco de la app y las capas compartidas. Las claves son las del inglés. */
export const SHELL_ES = {
  "shell.nav.primary": "Principal",
  "noun.portfolioLens": "Portfolio Lens",
  "noun.products": "Products",
  "noun.evidence": "Evidence",
  "noun.productLedger": "Product Ledger",
  "noun.productVault": "Product Vault",
  "shell.routePlaceholder":
    "Esta pantalla aún no está disponible. Se abrirá cuando su parte de la app supere la verificación.",
  "shell.theme.switchToDark": "Cambiar al tema oscuro",
  "shell.theme.switchToLight": "Cambiar al tema claro",
  "shell.theme.dark": "Tema oscuro",
  "shell.theme.light": "Tema claro",
  "shell.nav.attention.one": "{route}, {count} elemento requiere atención",
  "shell.nav.attention.other": "{route}, {count} elementos requieren atención",

  "policyStrip.degraded": "Funcionamiento limitado",
  "policyStrip.evidenceVerificationPending": "Verificación de Evidence pendiente",
  "policyStrip.outOfSync": "Sin sincronizar",
  "policyStrip.cancelling": "Cancelando",
  "policyStrip.backupDue": "Copia de seguridad pendiente",

  "errorDetail.correlationId": "Correlation ID",
  "errorDetail.copy": "Copiar",
  "errorDetail.copied": "Copiado",
  "errorDetail.retry": "Reintentar",

  "textScale.title": "Tamaño del texto",
  "textScale.sampleBody":
    "Buenos días. Empieza por lo que de verdad mueve los resultados hoy. 4 elementos requieren atención; el primero es una Action Request cuya respuesta está vencida.",
  "textScale.sampleLabel": "Está aquí porque: se incumplió un compromiso",

  "h2b.confirmPrompt": "Escribe “{phrase}” para aprobar",
  "h2b.approve": "Aprobar",
  "h2b.reject": "Rechazar",
  "h2b.approved": "Aprobado",
  "h2b.rejected": "Rechazado",
  "h2b.recoveryEvidence": "Evidence de recuperación verificada",
  "h2b.verifiedAt": "Verificado el",
  "h2b.scope": "Alcance",
  "h2b.compatibility": "Compatibilidad",
  "h2b.transferDetails": "Detalles de la transferencia externa",
  "h2b.provider": "Proveedor",
  "h2b.account": "Cuenta",
  "h2b.purpose": "Finalidad",
  "h2b.exactPayload": "Exactamente lo que se envía",
  "h2b.irreversible":
    "Una vez aprobado, no se puede deshacer: el contenido anterior se envía al servicio externo y no se puede recuperar ni revertir.",
  "policyStrip.backingUp": "Haciendo copia",
} as const satisfies Record<keyof typeof SHELL_EN, string>;
