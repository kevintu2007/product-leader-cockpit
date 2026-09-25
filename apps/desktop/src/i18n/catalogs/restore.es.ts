import type { RESTORE_EN } from "./restore.en";

/** Textos en español de Operational Restore y S11 System Health. */
export const RESTORE_ES = {
  "restore.open": "Restaurar desde una copia…",
  "restore.title": "Restaurar desde una copia",
  "restore.cancel": "Cancelar",
  "restore.choose.lede":
    "Elige una Operational Backup hecha en este equipo o en otro. Nada cambia hasta que confirmes el último paso.",
  "restore.choose.button": "Elegir archivo de copia…",
  "restore.choose.dialogTitle": "Elige la copia que quieres restaurar",
  "restore.choose.filterName": "Copias de PMC",
  "restore.file": "Archivo de copia: {name}",
  "restore.passphrase.label": "Frase de contraseña de esta copia",
  "restore.passphrase.check": "Comprobar la copia",
  "restore.checking": "Comprobando la copia… puede tardar un minuto.",
  "restore.backup": "Creada el {time}, con el esquema {schema} del Ledger, {count} registros.",
  "restore.recovery.running":
    "Antes de reemplazar nada, PMC hace una copia del espacio de trabajo actual. Puede tardar un minuto.",
  "restore.preview.lede":
    "Esto es exactamente lo que hace la restauración. Cualquier cambio antes de confirmar la cancela.",
  "restore.preview.backup": "Copia",
  "restore.preview.now": "Ahora",
  "restore.now": "{count} registros, último cambio el {time}.",
  "restore.now.noChange": "{count} registros, todavía sin cambios.",
  "restore.preview.replaced": "Se reemplaza",
  "restore.replaced":
    "El Product Ledger y los ajustes de la copia: idioma, zona horaria, tema, conservación, IA y registro.",
  "restore.preview.kept": "No se reemplaza",
  "restore.kept":
    "El Product Vault, la carpeta de copias y los ajustes de la frase de contraseña, y todas las demás copias.",
  "restore.preview.recovery": "Copia de recuperación",
  "restore.recovery.done": "El espacio de trabajo actual se copió y verificó el {time}.",
  "restore.needsUpgrade":
    "Esta copia viene de una versión anterior de PMC. Después de restaurarla, PMC pedirá actualizarla antes de poder usarla.",
  "restore.confirm.label": "Para confirmar, escribe la fecha de creación de la copia: {date}",
  "restore.confirm.button": "Reemplazar con esta copia",
  "restore.reject": "No restaurar",
  "restore.replacing": "Reemplazando… no cierres PMC.",
  "restore.result.restored":
    "Se restauró la copia del {time}. Lo que había antes quedó guardado como la copia del {recovery}.",
  "restore.result.unchanged": "La restauración falló. No se cambió nada.",
  "restore.result.putBack":
    "La restauración falló después de empezar a reemplazar. PMC devolvió el espacio de trabajo anterior.",
  "restore.result.recoveryFailed":
    "La restauración falló y PMC no pudo devolver el espacio de trabajo anterior. System Health indica qué copia de recuperación restaurar.",
  "restore.result.interrupted":
    "La restauración se detuvo antes de terminar. Ahora no cambiará nada más; cuando PMC vuelva a iniciarse, devolverá el espacio de trabajo anterior.",
  "restore.continue": "Continuar",
  "restore.openSystemHealth": "Abrir System Health",

  "policyStrip.restoring": "Restaurando",
  "backup.strip.restoring":
    "Hay una restauración en curso. Los registros y cambios nuevos esperan hasta que termine.",

  "health.headline": "Si PMC puede leer y guardar tus registros",
  "health.loading": "Leyendo el estado del sistema.",
  "health.unavailable": "No se puede leer el estado del sistema en este momento. {message}",
  "health.ledger.label": "Product Ledger",
  "health.ledger.ready": "Abierto y legible.",
  "health.ledger.replacing": "Una restauración lo está reemplazando ahora.",
  "health.ledger.restore_recovery_required":
    "No se abrió. Una restauración se detuvo a mitad de camino, así que el archivo actual puede no ser ni el Ledger anterior ni la copia.",
  "health.ledger.upgrade_required":
    "Todavía no se abrió. Viene de una versión anterior de PMC y primero hay que actualizarlo.",
  "health.ledger.open_failed": "No se pudo abrir.",
  "health.recoveryBackup":
    "Restaura la copia de recuperación {name} desde la carpeta de copias para volver a donde estabas.",
  "health.quit": "Salir de PMC",

  "safeError.desktop.restore_running":
    "Hay una restauración en curso. Vuelve a intentarlo cuando termine.",
  "safeError.desktop.ledger_unavailable":
    "El Product Ledger no está abierto. System Health indica por qué.",
  "safeError.desktop.restore_file_unreadable": "No se puede leer este archivo de copia.",
  "safeError.desktop.restore_wrong_passphrase": "Esta frase de contraseña no abre esta copia.",
  "safeError.desktop.restore_damaged":
    "Esta copia está dañada: falló una comprobación de su contenido.",
  "safeError.desktop.restore_newer_version":
    "Esta copia se hizo con una versión más reciente de PMC. Actualiza PMC para restaurarla.",
  "safeError.desktop.restore_unsupported_old":
    "Esta copia viene de una versión de PMC demasiado antigua para restaurarla.",
  "safeError.desktop.restore_recovery_backup_failed":
    "Falló la copia del espacio de trabajo actual, así que no se reemplazó nada.",
  "safeError.desktop.restore_preview_stale":
    "Esta vista previa ya no está al día. Vuelve a elegir la copia.",
  "safeError.desktop.restore_confirmation_mismatch":
    "La fecha escrita no es la fecha de creación de la copia.",
  "safeError.desktop.restore_failed_unchanged": "La restauración falló. No se cambió nada.",
  "safeError.desktop.restore_live_only":
    "La restauración solo se aplica al espacio de trabajo Live.",

  "upgrade.title": "Actualizar este espacio de trabajo",
  "upgrade.body":
    "PMC {version} guarda tus registros en un formato más nuevo. La actualización tarda un momento. PMC primero hace una copia de tu espacio de trabajo; si algo falla, no cambia nada.",
  "upgrade.fact.current": "Formato actual",
  "upgrade.fact.new": "Formato nuevo",
  "upgrade.schema": "esquema {schema}",
  "upgrade.fact.records": "Registros",
  "upgrade.fact.lastBackup": "Última copia verificada",
  "upgrade.fact.noBackup": "Ninguna",
  "upgrade.backupFirst":
    "Primero la copia: configura la carpeta de copias y la frase de contraseña, y después actualiza.",
  "upgrade.run": "Actualizar",
  "upgrade.backingUp": "Haciendo una copia de tu espacio de trabajo…",
  "upgrade.upgrading": "Actualizando…",
  "upgrade.result.upgraded":
    "Tu espacio de trabajo está actualizado. La copia del {time} lo guarda tal como estaba antes.",
  "upgrade.result.rolledBack": "La actualización no se completó. No se cambió nada.",
  "upgrade.result.unreadable":
    "Tu espacio de trabajo se actualizó, pero PMC no pudo abrirlo. La copia del {time} lo guarda tal como estaba antes.",
  "upgrade.result.unknown":
    "PMC no puede saber si la actualización se completó. La copia del {time} guarda tu espacio de trabajo tal como estaba antes.",
  "upgrade.retry": "Intentar de nuevo",
  "upgrade.blocked.title": "No se puede abrir este espacio de trabajo",
  "upgrade.newer":
    "Este espacio de trabajo se creó con una versión más reciente de PMC. Actualiza PMC para abrirlo.",
  "upgrade.unsupportedOld":
    "Este espacio de trabajo se creó con una versión de desarrollo de PMC y no se puede actualizar.",
  "health.ledger.unsupported_old":
    "No se abrió. Se creó con una versión de desarrollo de PMC y no se puede actualizar.",
  "health.ledger.newer_version": "No se abrió. Se creó con una versión más reciente de PMC.",
  "safeError.desktop.upgrade_live_only":
    "La actualización solo se aplica al espacio de trabajo Live.",
  "safeError.desktop.upgrade_not_required": "Este espacio de trabajo no necesita actualizarse.",
  "safeError.desktop.upgrade_backup_failed":
    "Falló la copia previa a la actualización, así que la actualización no empezó. Revisa la carpeta de copias y vuelve a intentarlo.",
  "safeError.desktop.upgrade_failed_unchanged":
    "La actualización no se completó. No se cambió nada.",
  "upgrade.trainingReset":
    "Este espacio de trabajo Training viene de una versión anterior de PMC. No se actualiza: restablécelo con el espacio de trabajo de ejemplo.",
  "safeError.desktop.upgrade_outcome_unknown":
    "PMC no puede saber si la actualización se completó. La copia hecha justo antes guarda tu espacio de trabajo tal como estaba.",
  "restore.now.unavailable":
    "PMC no puede abrir el Product Ledger actual, así que su número de registros y su último cambio no están disponibles.",
  "restore.recovery.preserved":
    "PMC guardó y volvió a comprobar una copia exacta de los archivos del Ledger actual como «{name}». La copia coincidió byte a byte. PMC no pudo verificarla como un Product Ledger utilizable, así que no es una Operational Backup ni cuenta como tu última copia verificada.",
  "restore.preserve.running":
    "Antes de reemplazar nada, PMC guarda una copia exacta de los archivos del Ledger actual y la comprueba. Puede tardar un minuto.",
  "restore.result.restoredPreserved":
    "Se restauró la copia de {time}. Los archivos que había aquí se guardan como «{name}».",
  "restore.result.putBackPreserved":
    "La restauración falló después de empezar el reemplazo. PMC volvió a dejar los archivos del Ledger anterior exactamente como estaban. El Product Ledger sigue sin poder abrirse. Puedes probar otra copia o salir de PMC.",
  "health.restore.backupFirst":
    "Antes de restaurar, PMC guarda una copia de los archivos del Ledger actual en la carpeta de copias. Configura primero la carpeta de copias y la frase de recuperación.",
  "restore.choose.recovery": "Usar la copia de recuperación «{name}»",
  "restore.choose.other": "Elegir otro archivo de copia…",
  "restore.result.sourceChanged":
    "Los archivos del Ledger actual cambiaron después de la vista previa, así que no se reemplazó nada. Vuelve a empezar la restauración para ver qué se reemplazaría ahora.",
} as const satisfies Record<keyof typeof RESTORE_EN, string>;
