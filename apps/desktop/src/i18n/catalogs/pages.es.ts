import type { PAGES_EN } from "./pages.en";

/** Textos en español de People, Reviews & Reports, Product Vault y Settings. Las claves son las del inglés. */
export const PAGES_ES = {
  "people.kind.person": "Persona",
  "people.kind.organization": "Organización",
  "people.none": "Ninguno",
  "people.headline": "Quién es responsable de qué y quién sigue esperando una respuesta",
  "people.lede":
    "Los Stakeholders y las relaciones de aquí los creas y mantienes tú; no son cuentas de usuario. Una solicitud que espera respuesta aún no es un compromiso: una Action Request se convierte en Action cuando se acepta.",
  "people.empty":
    "Aún no hay Stakeholders. Cuando crees alguno, aquí verás de qué es responsable y de qué depende.",
  "people.caption": "Ordenado por identificador. Se muestran del {from} al {to} de {total}.",
  "people.column.stakeholder": "Stakeholder",
  "people.column.kind": "Tipo",
  "people.column.responsible": "Responsable de",
  "people.column.dependsOn": "Depende de",
  "people.column.waiting": "Solicitudes que esperan respuesta",
  "people.column.classification": "Clasificación",
  "people.previous": "Página anterior",
  "people.next": "Página siguiente",

  "reviews.route": "los datos que necesita la Review",
  "reviews.headline": "Antes de una Review, mira lo que sigue abierto",
  "reviews.lede":
    "Aquí aparece lo que la Work Queue ha marcado, en el orden de atención del Executive Cockpit. Para actuar, ve a la Work Queue.",
  "reviews.period": "Periodo de revisión",
  "reviews.noPeriod": "No hay ningún periodo de revisión disponible: {reason}.",
  "reviews.notYet": "Esta versión aún no puede generar un Fact Pack ni aprobar un informe.",
  "reviews.open": "Marcado en la Work Queue",
  "reviews.openNone": "La Work Queue no ha marcado nada.",
  "reviews.count.one": "{count} en total.",
  "reviews.count.other": "{count} en total.",
  "reviews.goToQueue": "Actuar en la Work Queue",

  "vault.reason.notConfigured": "Este espacio de trabajo no tiene un Product Vault.",
  "vault.reason.invalidRoot":
    "La ubicación del Product Vault no existe, no es una carpeta o es un vínculo.",
  "vault.reason.unknown": "Se desconoce el motivo.",
  "vault.pausedBecause":
    "{reason} Fijar huellas y volver a observar están en pausa; vincular Evidence sigue funcionando.",
  "vault.headline": "Los archivos de Evidence y su situación actual",
  "vault.lede":
    "Los archivos están en tu propio Product Vault; el Ledger registra el uso, la huella, la verificación y la clasificación de cada Evidence.",
  "vault.available": "El Vault está disponible",
  "vault.unavailable": "El Vault no está disponible",
  "vault.availableHint":
    "Puedes fijar huellas o volver a observar desde la pestaña Evidence de un Product.",
  "vault.count": "{count} en total, ordenados por identificador.",
  "vault.empty":
    "Aún no hay Evidence en el Ledger. Vincula alguna desde la pestaña Evidence de un Product y aparecerá aquí.",
  "vault.column.evidence": "Evidence",
  "vault.column.role": "Uso",
  "vault.column.verification": "Verificación",
  "vault.column.fingerprint": "Huella",
  "vault.column.classification": "Clasificación",
  "vault.column.version": "Versión",
  "vault.roleUnset": "Sin definir",
  "vault.verificationOn": "{verification} ({date})",
  "vault.pinned": "Fijada",
  "vault.notPinned": "Sin fijar",

  "settings.headline": "Cómo se ve todo y cómo está este espacio de trabajo",
  "settings.lede":
    "El tamaño del texto solo afecta a la pantalla de este equipo y es independiente de la escala de pantalla de Windows. Cambia el tema con el botón de arriba a la derecha.",
  "settings.textSize": "Tamaño del texto",
  "settings.language": "Idioma",
  "settings.languageSystem": "Idioma del sistema ({language})",
  "settings.languageFailed": "No se guardó el idioma. {message}",
  "settings.sampleBody":
    "Buenos días. Empieza por lo que de verdad mueve los resultados hoy. Demo Product Atlas tiene un hito cuya fecha ya pasó; 2/2 KPIs observados.",
  "settings.sampleLabel": "Hitos: fecha pasada, el más próximo 2026‑08‑15",
  "settings.dataSources": "Fuentes de datos",
  "settings.loading": "Leyendo el estado del Ledger y del Vault.",
  "settings.unavailable": "Ahora no se puede leer el estado del Ledger o del Vault. {message}",
  "settings.ledgerReadable":
    "Legible. Versión de esquema {schema}; el Ledger está en la versión {revision}.",
  "settings.vaultNotSet": "Sin configurar",
  "vaultRoot.open.choose": "Elegir carpeta del Vault…",
  "vaultRoot.open.change": "Cambiar carpeta del Vault…",
  "vault.reason.changeUnresolved":
    "Un cambio de la carpeta del Vault se interrumpió y no pudo terminarse. Reinicia PMC para terminarlo.",
  "safeError.desktop.vault_change_unresolved":
    "Un cambio de la carpeta del Vault se interrumpió y no pudo terminarse, así que no se usa ningún Vault. Reinicia PMC para terminarlo.",
  "vaultRoot.title": "Carpeta del Product Vault",
  "vaultRoot.cancel": "Cancelar",
  "vaultRoot.choose.lede":
    "Elige la carpeta de la que este espacio de trabajo lee la Evidence. Antes de cambiar nada, PMC hace una copia de seguridad de este espacio y comprueba cada archivo de Evidence de la carpeta.",
  "vaultRoot.backupFirst":
    "Antes de cambiar el Vault, PMC hace una copia de seguridad de este espacio de trabajo para poder restaurarlo. Configura primero la carpeta de copias y la frase de recuperación.",
  "vaultRoot.choose.button": "Elegir carpeta del Vault…",
  "vaultRoot.choose.dialogTitle": "Elige la carpeta del Product Vault",
  "vaultRoot.folder": "Carpeta: {name}",
  "vaultRoot.preparing":
    "Haciendo una copia de seguridad de este espacio y comprobando cada archivo de Evidence de la nueva carpeta… puede tardar un minuto.",
  "vaultRoot.preview.lede":
    "Esto es exactamente lo que cambiará. No se mueve nada del Vault ni del Ledger.",
  "vaultRoot.preview.now": "Ahora",
  "vaultRoot.preview.new": "Nueva",
  "vaultRoot.preview.evidence": "Evidence",
  "vaultRoot.preview.noEvidence":
    "Ninguna Evidence hace referencia al Vault actual; no hay nada que volver a resolver.",
  "vaultRoot.preview.resolved":
    "{resolved} de {count} referencias tienen el mismo contenido en la nueva carpeta.",
  "vaultRoot.preview.recovery": "Evidencia de recuperación",
  "vaultRoot.preview.recoveryVerified": "Copia hecha y verificada a las {time}",
  "vaultRoot.confirm.phrase": "CAMBIAR VAULT",
  "vaultRoot.confirm.label": "Para confirmar, escribe {phrase} {code}",
  "vaultRoot.confirm.button": "Usar esta carpeta",
  "vaultRoot.reject": "No cambiar",
  "vaultRoot.changing": "Cambiando… no cierres PMC.",
  "vaultRoot.result.changed": "Este espacio de trabajo lee ahora la Evidence de «{name}».",
  "vaultRoot.result.notChanged": "No se cambió nada.",
  "vaultRoot.done": "Hecho",
  "safeError.desktop.vault_change_active":
    "Ya hay otro cambio de la carpeta del Vault en curso. Termínalo o cancélalo primero.",
  "safeError.desktop.vault_not_live":
    "Solo se puede cambiar la carpeta del Vault del espacio de trabajo Live.",
  "safeError.desktop.vault_folder_unusable":
    "Esa carpeta no puede ser un Vault. Elige una carpeta existente que puedas leer, que no sea un acceso directo ni un vínculo.",
  "safeError.desktop.vault_folder_unchanged":
    "Esa carpeta ya es el Vault de este espacio de trabajo.",
  "safeError.desktop.vault_folder_in_use":
    "PMC ya usa esa carpeta. Elige una que no sea la de PMC y que no contenga ni esté dentro de tu carpeta de copias de seguridad.",
  "safeError.desktop.vault_recovery_backup_stale":
    "La copia hecha para este cambio ya no coincide con el espacio de trabajo. Inténtalo de nuevo.",
  "safeError.desktop.vault_evidence_unpinned":
    "{count} referencias de Evidence no tienen una huella fijada, así que PMC no puede demostrar que sean los mismos archivos en la nueva carpeta. Fíjalas primero (Product → Evidence).",
  "safeError.desktop.vault_evidence_unresolved":
    "{count} referencias de Evidence no tienen el mismo contenido en la nueva carpeta. Mueve o restaura esos archivos primero.",
  "safeError.desktop.vault_preview_stale":
    "Esta vista previa está desactualizada. Vuelve a elegir la carpeta.",
  "safeError.desktop.vault_confirmation_mismatch": "El código no coincide con esta vista previa.",
  "safeError.desktop.vault_changed_but_not_recorded":
    "La carpeta se cambió, pero PMC no pudo registrar el cambio en su registro de auditoría.",
  "safeError.desktop.vault_change_not_recorded":
    "PMC no pudo registrar este paso, así que no se cambió nada. Inténtalo de nuevo.",
  "safeError.desktop.vault_change_failed":
    "No se pudo cambiar la carpeta del Vault. No se cambió nada. Inténtalo de nuevo.",
  "evidenceFile.open": "Añadir Evidence desde un archivo…",
  "evidenceFile.title": "Evidence desde un archivo",
  "evidenceFile.lede":
    "Elige un archivo dentro de la carpeta del Vault. PMC registra dónde está y su huella; el archivo se queda donde está.",
  "evidenceFile.choose": "Elegir archivo…",
  "evidenceFile.chooseAnother": "Elegir otro archivo…",
  "evidenceFile.dialogTitle": "Elige un archivo dentro del Product Vault",
  "evidenceFile.file": "Archivo: {name}",
  "evidenceFile.observed": "Observado {time}",
  "evidenceFile.changed": "El archivo cambió después de elegirlo.",
  "evidenceFile.existing":
    "Evidence {id} ya hace referencia a este archivo, así que no se crea otra.",
  "evidenceFile.existingLinked":
    "Evidence {id} ya hace referencia a este archivo y ya está vinculada a este Product.",
  "evidenceFile.linkExisting": "Vincularla a este Product",
  "evidenceFile.sameContent": "Evidence {id} tiene el mismo contenido.",
  "evidenceFile.create": "Crear",
  "evidenceFile.createAndLink": "Crear y vincular a este Product",
  "evidenceFile.cancel": "Cancelar",
  "evidenceFile.creating": "Creando…",
  "evidenceFile.linking": "Vinculando…",
  "evidenceFile.created": "Se creó Evidence {id}.",
  "evidenceFile.createdAndLinked": "Se creó Evidence {id} y se vinculó a {product}.",
  "evidenceFile.linkedExisting": "Se vinculó Evidence {id} a {product}.",
  "evidenceFile.createdNotLinked": "Se creó Evidence {id}; todavía no está vinculada.",
  "evidenceFile.retryLink": "Vincular ahora",
  "evidenceFile.done": "Listo",
  "safeError.desktop.evidence_file_outside_vault":
    "Elige un archivo dentro de la carpeta del Vault.",
  "safeError.desktop.evidence_file_unreadable": "No se pudo leer el archivo.",
  "safeError.desktop.evidence_file_choice_stale":
    "Esta elección está desactualizada. Elige el archivo de nuevo.",
  "safeError.desktop.sample_backup_refused": "El espacio de trabajo de ejemplo nunca se respalda.",
  "safeError.desktop.workspace_not_chosen": "Elige primero cómo empezar.",
  "safeError.desktop.sample_foreign":
    "La carpeta de ejemplo contiene algo que PMC no escribió, así que se dejó como está.",
  "safeError.desktop.sample_is_open":
    "Cambia primero a tu espacio de trabajo; los datos de ejemplo no pueden cambiar mientras están abiertos.",
  "safeError.desktop.sample_operation_in_progress":
    "Otro cambio en los datos de ejemplo no ha terminado. Inténtalo de nuevo en un momento.",
  "safeError.desktop.sample_nothing_to_delete": "No hay datos de ejemplo que borrar.",
  "safeError.desktop.sample_delete_not_prepared":
    "Este borrado ya no está pendiente. Vuelve a empezar.",
  "safeError.desktop.sample_delete_expired":
    "Esta vista previa ha caducado. No se borró nada. Vuelve a empezar.",
  "safeError.desktop.sample_delete_changed":
    "Los datos de ejemplo cambiaron desde la vista previa. No se borró nada. Vuelve a empezar.",
  "safeError.desktop.sample_confirmation_mismatch": "La frase no coincide. No se borró nada.",
  "safeError.desktop.sample_not_recorded":
    "PMC no pudo registrar este paso en su registro de auditoría. Inténtalo de nuevo.",
  "safeError.desktop.sample_failed":
    "No se pudieron preparar los datos de ejemplo. Tu espacio de trabajo no se ve afectado. Inténtalo de nuevo.",
  "safeError.desktop.workspace_not_first_run":
    "Esta elección solo se hace una vez, en el primer inicio.",
  "safeError.desktop.workspace_choice_not_saved":
    "No se pudo guardar la elección, así que PMC no se reinició. Inténtalo de nuevo.",
  "safeError.desktop.sample_reset_failed":
    "No se pudieron restablecer los datos de ejemplo. PMC los deja como estaban, ahora o en el próximo inicio.",
  "safeError.desktop.sample_delete_failed":
    "Los datos de ejemplo no se borraron, o no del todo. PMC vuelve a comprobarlo en el próximo inicio.",
  "safeError.desktop.sample_unavailable":
    "Ahora no se pueden leer los datos de ejemplo. No se cambió nada.",
  "firstRun.title": "Elige cómo empezar",
  "firstRun.lede":
    "Elige una opción para empezar. Puedes cambiar después en Configuración → Espacio de trabajo.",
  "firstRun.live.title": "Empezar con mi espacio de trabajo",
  "firstRun.live.body":
    "Se abre un espacio de trabajo vacío. Tus registros se quedan en este equipo.",
  "firstRun.live.setup":
    "Después, en Configuración: una carpeta de copias de seguridad, luego una frase de recuperación y luego la carpeta del Vault. Sin la frase, una copia de seguridad no se puede restaurar.",
  "firstRun.sample.title": "Aprender con datos de ejemplo",
  "firstRun.sample.body":
    "Datos sintéticos para aprender. Están separados de tu trabajo y puedes restablecerlos o borrarlos cuando quieras.",
  "firstRun.preparing": "Preparando los datos de ejemplo… puede tardar un minuto.",
  "firstRun.saving": "Guardando tu elección…",
  "firstRun.tryAgain": "Todavía no se ha elegido nada. Elige de nuevo cuando quieras.",
  "workspace.title": "Espacio de trabajo",
  "workspace.openLive": "Tu espacio de trabajo está abierto.",
  "workspace.openSample":
    "Los datos de ejemplo están abiertos. Son sintéticos y están separados de tu trabajo.",
  "workspace.sampleFellBack":
    "Se eligieron los datos de ejemplo, pero no se pudieron abrir, así que se abrió tu espacio de trabajo. Estado del sistema explica por qué.",
  "workspace.switchToSample": "Cambiar a los datos de ejemplo",
  "workspace.switchToLive": "Cambiar a mi espacio de trabajo",
  "workspace.switchToSample.confirm":
    "PMC se reiniciará y abrirá los datos de ejemplo. Tu espacio de trabajo no cambia.",
  "workspace.switchToLive.confirm": "PMC se reiniciará y abrirá tu espacio de trabajo.",
  "workspace.switchAndRestart": "Cambiar y reiniciar",
  "workspace.restarting": "PMC se está reiniciando…",
  "sampleWorkspace.badge": "Espacio de ejemplo",
  "sampleWorkspace.badge.open": "Abrir Configuración → Espacio de trabajo",
  "sampleWorkspace.manage": "Datos de ejemplo",
  "sampleWorkspace.fromLiveOnly":
    "Restablecer y borrar se ofrecen desde tu espacio de trabajo. Cambia a él primero.",
  "sampleWorkspace.cancel": "Cancelar",
  "sampleWorkspace.close": "Cerrar",
  "sampleWorkspace.done": "Listo",
  "sampleWorkspace.reset.open": "Restablecer datos de ejemplo…",
  "sampleWorkspace.reset": "Restablecer datos de ejemplo",
  "sampleWorkspace.reset.confirm":
    "Los datos de ejemplo vuelven a su estado inicial. Tu espacio de trabajo no se ve afectado.",
  "sampleWorkspace.resetting": "Restableciendo los datos de ejemplo… puede tardar un minuto.",
  "sampleWorkspace.reset.done": "Los datos de ejemplo volvieron a su estado inicial.",
  "sampleWorkspace.delete.open": "Borrar datos de ejemplo…",
  "sampleWorkspace.delete.title": "Borrar datos de ejemplo",
  "sampleWorkspace.delete.preparing": "Leyendo lo que contienen los datos de ejemplo…",
  "sampleWorkspace.delete.lede":
    "Esto es exactamente lo que se quita al borrar. Cualquier cambio antes de confirmar lo cancela.",
  "sampleWorkspace.delete.what": "Datos de ejemplo",
  "sampleWorkspace.delete.seed": "{seedId}, versión {version}",
  "sampleWorkspace.delete.unknown":
    "Esta vista previa describe un cambio que esta pantalla no puede mostrar, así que no se puede aprobar aquí. No se borró nada.",
  "sampleWorkspace.delete.parts": "Qué se quita",
  "sampleWorkspace.delete.part.ledger": "Su Product Ledger",
  "sampleWorkspace.delete.part.vault": "Su Vault sintético",
  "sampleWorkspace.delete.part.generated": "Sus archivos generados",
  "sampleWorkspace.delete.effect": "Efecto",
  "sampleWorkspace.delete.effectValue":
    "No se puede deshacer. Tu espacio de trabajo, la configuración y las copias de seguridad no se ven afectados.",
  "sampleWorkspace.delete.expires": "Vista previa válida hasta",
  "sampleWorkspace.delete.settingsRevision": "Revisión de la configuración",
  "sampleWorkspace.delete.inventory": "Resumen del contenido",
  "sampleWorkspace.delete.payload": "Resumen de la vista previa",
  "sampleWorkspace.delete.confirmLabel": "Escribe {phrase} para confirmar",
  "sampleWorkspace.delete.phrase": "BORRAR DATOS DE EJEMPLO",
  "sampleWorkspace.delete.confirm": "Borrar datos de ejemplo",
  "sampleWorkspace.delete.reject": "Conservarlos",
  "sampleWorkspace.delete.deleting": "Borrando los datos de ejemplo…",
  "sampleWorkspace.delete.deleted":
    "Los datos de ejemplo se borraron. Al cambiar a los datos de ejemplo se preparan de nuevo.",
  "sampleWorkspace.delete.notDeleted": "Los datos de ejemplo no se borraron. No cambió nada.",
  "health.ledger.first_run": "Aún no se abrió: primero elige cómo empezar.",
  "health.settings.setAside":
    "No se pudo leer la configuración, así que PMC la guardó aparte y empezó con la configuración predeterminada.",
  "health.settings.unavailable": "La configuración no se puede leer ni guardar ahora.",
  "health.sample.unresolved":
    "No se pudo completar un cambio pendiente en los datos de ejemplo, así que no están disponibles.",
  "health.sample.missing":
    "Se eligieron los datos de ejemplo, pero no están, así que se abrió tu espacio de trabajo.",
  "health.sample.foreign":
    "La carpeta de ejemplo contiene algo que PMC no escribió, así que se dejó como está y los datos de ejemplo no están disponibles.",
  "health.sample.cleanupPending":
    "Carpetas de ejemplo pendientes de quitar: {count}. PMC las quita en un inicio posterior.",
  "gettingStarted.title": "Primeros pasos",
  "gettingStarted.lede":
    "Configura tu espacio de trabajo en este orden. Cada paso se comprueba con lo que PMC puede ver, y esta lista desaparece cuando todos estén hechos.",
  "gettingStarted.backupFolder": "Elegir una carpeta de copias de seguridad",
  "gettingStarted.backupFolder.why":
    "Las copias se guardan en la carpeta que elijas, mejor en otra unidad.",
  "gettingStarted.passphrase": "Establecer una frase de recuperación",
  "gettingStarted.passphrase.why":
    "Las copias se cifran con ella. Sin ella no se puede restaurar una copia, y PMC no puede recuperar una frase perdida.",
  "gettingStarted.firstBackup": "Hacer la primera copia de seguridad",
  "gettingStarted.firstBackup.why":
    "PMC acepta registros y cambios nuevos solo después de verificar una copia.",
  "gettingStarted.vault": "Elegir la carpeta del Product Vault",
  "gettingStarted.vault.why":
    "La carpeta que guarda tus archivos de Evidence. PMC registra dónde está cada archivo y su huella; nunca copia archivos.",
  "gettingStarted.firstProduct": "Añadir tu primer Product",
  "gettingStarted.firstProduct.why":
    "En Portfolio, con «Nuevo Product…». El Cockpit lo coloca en el Portfolio Lens.",
  "gettingStarted.status.done": "Hecho",
  "gettingStarted.status.next": "Siguiente",
  "gettingStarted.status.waits": "Espera al paso anterior",
  "gettingStarted.status.unknown": "PMC aún no puede saberlo",
  "gettingStarted.openSettings": "Abrir Configuración",
  "gettingStarted.openPortfolio": "Abrir Portfolio",
  "settings.vaultAvailable": "Disponible",
  "settings.vaultUnavailable": "No disponible",
  "backup.headline": "Copias de seguridad",
  "backup.loading": "Leyendo el estado de las copias de seguridad.",
  "backup.dueLede":
    "Toca hacer una copia de seguridad. Hasta que una se verifique, este espacio de trabajo no acepta registros nuevos ni cambios.",
  "backup.folder.label": "Carpeta de copias",
  "backup.folder.notSet": "Sin configurar",
  "backup.folder.set": "Configurada",
  "backup.folder.available": "Disponible",
  "backup.folder.unavailable": "No disponible: conecta la unidad o elige otra carpeta",
  "backup.folder.choose": "Elegir carpeta…",
  "backup.folder.dialogTitle": "Elige dónde guarda PMC las copias",
  "backup.passphrase.label": "Frase de recuperación",
  "backup.passphrase.notSet": "Sin configurar",
  "backup.passphrase.session": "Configurada para esta sesión",
  "backup.passphrase.remembered": "Guardada en esta cuenta de Windows",
  "backup.passphrase.setUp": "Configurar frase…",
  "backup.last.label": "Última copia",
  "backup.last.none": "Aún no hay copias",
  "backup.last.at": "Verificada el {time}. La próxima vence el {next}.",
  "backup.orphans":
    "En esta carpeta hay {count} archivo(s) de copia que no están en los registros de PMC y no se cuentan.",
  "backup.run": "Hacer copia ahora",
  "backup.running": "Haciendo la copia… puede tardar un minuto.",
  "backup.done": "Copia hecha y verificada el {time}.",
  "backup.openBackups": "Abrir copias de seguridad",
  "backup.strip.due": "Haz primero una copia; después se vuelven a aceptar registros y cambios.",
  "backup.strip.running": "Haciendo la copia… los registros y cambios esperan a que termine.",
  "backup.passphrase.title": "Configurar la frase de recuperación",
  "backup.passphrase.generatedLede":
    "Esta frase se muestra una sola vez. Anótala o guárdala en un gestor de contraseñas y luego escríbela abajo.",
  "backup.passphrase.generating": "Creando una frase.",
  "backup.passphrase.retype": "Escríbela tal cual",
  "backup.passphrase.useOwn": "Usar la mía",
  "backup.passphrase.useGenerated": "Usar una frase generada",
  "backup.passphrase.ownRule":
    "Al menos 20 caracteres o seis palabras, y no solo contraseñas comunes.",
  "backup.passphrase.own": "Frase",
  "backup.passphrase.ownAgain": "Repite la frase",
  "backup.passphrase.acknowledge":
    "PMC no puede recuperar esta frase. Sin ella, estas copias no se pueden restaurar en este ni en otro equipo.",
  "backup.passphrase.remember": "Guardarla en esta cuenta de Windows para las copias automáticas.",
  "backup.passphrase.confirm": "Usar esta frase",
  "backup.passphrase.cancel": "Cancelar",
} as const satisfies Record<keyof typeof PAGES_EN, string>;
