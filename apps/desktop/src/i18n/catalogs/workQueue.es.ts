import type { WORK_QUEUE_EN } from "./workQueue.en";

/** Textos en español de la Work Queue y sus formularios. Las claves son las del inglés. */
export const WORK_QUEUE_ES = {
  "workQueue.caption.one":
    "Ordenado según la política de clasificación aprobada. Se muestran del {from} al {to} de {count} elemento.",
  "workQueue.caption.other":
    "Ordenado según la política de clasificación aprobada. Se muestran del {from} al {to} de {count} elementos.",

  "wq.offer.prepareAccept": "Preparar la aceptación",
  "wq.offer.decline": "Rechazar",
  "wq.offer.withdraw": "Retirar",
  "wq.offer.start": "Empezar",
  "wq.offer.link": "Vincular Evidence",
  "wq.offer.prepareComplete": "Preparar la finalización",
  "wq.offer.prepareCancel": "Preparar la cancelación",
  "wq.offer.prepareReopen": "Preparar la reapertura",
  "wq.offer.prepareResolve": "Preparar la resolución",
  "wq.offer.prepareOccurrence": "Preparar el registro de que ocurrió",
  "wq.offer.prepareClose": "Preparar el cierre",

  "wq.noDeadline": "No hay fecha límite registrada",
  "wq.noResponseDeadline": "Sin fecha límite de respuesta",

  "wq.review.accept.title": "Aprobar y ejecutar: aceptar {label}",
  "wq.review.accept.summary":
    "Al aprobar se crea una Action a partir del resumen que confirmaste y se vincula esta Request a ella. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.accept.approve": "Aprobar y aceptar",
  "wq.review.complete.title": "Aprobar y ejecutar: completar {label}",
  "wq.review.complete.summary":
    "Al aprobar, esta Action se marca como completada, respaldada por la Evidence o el Judgment de la vista previa. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.complete.approve": "Aprobar y completar",
  "wq.review.cancel.title": "Aprobar y ejecutar: cancelar {label}",
  "wq.review.cancel.summary":
    "Al aprobar, esta Action se cancela por el motivo que escribiste. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.cancel.approve": "Aprobar y cancelar",
  "wq.review.reopen.title": "Aprobar y ejecutar: reabrir {label}",
  "wq.review.reopen.summary":
    "Al aprobar, esta Action se reabre en el modo que elegiste. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.reopen.approve": "Aprobar y reabrir",
  "wq.review.resolveDecision.title": "Aprobar y ejecutar: resolver {label}",
  "wq.review.resolveDecision.summary":
    "Al aprobar se crea la Decision de la vista previa y cada Action Request de seguimiento que incluye. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.resolveDecision.approve": "Aprobar y resolver",
  "wq.review.occurrence.title": "Aprobar y ejecutar: registrar que {label} ocurrió",
  "wq.review.occurrence.summary":
    "Al aprobar, este Risk se marca como ocurrido y se crea el Issue {issue} indicado en la vista previa (la app asignó ese id; el que ves es el que se escribirá). Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.occurrence.approve": "Aprobar y registrar",
  "wq.review.closeRisk.title": "Aprobar y ejecutar: cerrar {label}",
  "wq.review.closeRisk.summary":
    "Al aprobar, este Risk se cierra por el motivo que escribiste. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.closeRisk.approve": "Aprobar y cerrar",
  "wq.review.resolveIssue.title": "Aprobar y ejecutar: resolver {label}",
  "wq.review.resolveIssue.summary":
    "Al aprobar, este Issue se marca como resuelto con la Evidence de la vista previa. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.resolveIssue.approve": "Aprobar y resolver",
  "wq.review.closeIssue.title": "Aprobar y ejecutar: cerrar {label}",
  "wq.review.closeIssue.summary":
    "Al aprobar, este Issue se cierra con la Evidence de verificación de la vista previa. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.closeIssue.approve": "Aprobar y cerrar",
  "wq.review.reopenIssue.title": "Aprobar y ejecutar: reabrir {label}",
  "wq.review.reopenIssue.summary":
    "Al aprobar, este Issue se reabre con el motivo que escribiste y la Evidence de la verificación fallida. Un rechazo queda registrado y esta vista previa ya no podrá aprobarse.",
  "wq.review.reopenIssue.approve": "Aprobar y reabrir",

  "wq.done.declined": "Se rechazó {id} (versión {version}).",
  "wq.done.withdrawn": "Se retiró {id} (versión {version}).",
  "wq.done.started": "Se empezó {id} (versión {version}).",
  "wq.done.linked": "Se vinculó la Evidence {evidence} a {id} (versión {version}).",
  "wq.noWritePath": "Esta app no tiene una vía de escritura.",
  "wq.settled.completed": "Se completó {id} (versión {version}, estado: {state}).",
  "wq.settled.cancelled": "Se canceló {id} (versión {version}, estado: {state}).",
  "wq.settled.reopened": "Se reabrió {id} (versión {version}, estado: {state}).",
  "wq.settled.accepted":
    "Se creó la Action {action} y se vinculó a {request}. Comprobante {receipt}.",
  "wq.settled.occurred":
    "Se registró que ocurrió (el Risk está ahora en la versión {version}) y se creó el Issue {issue}.",
  "wq.settled.riskClosed": "Se cerró {id} (versión {version}).",
  "wq.settled.issue": "{id} ahora está en estado {state} (versión {version}).",
  "wq.settled.decision.one":
    "Se creó la Decision {decision} y {count} Action Request de seguimiento. Comprobante {receipt}.",
  "wq.settled.decision.other":
    "Se creó la Decision {decision} y {count} Action Requests de seguimiento. Comprobante {receipt}.",
  "wq.notice.gone": "{label} ya no está en la lista, así que no se volvió a preparar.",
  "wq.notice.notAcceptable": "{id} ya no se puede aceptar, así que no se volvió a preparar.",
  "wq.notice.rejected":
    "La vista previa anterior se rechazó. Vuelve a introducir los datos y prepárala de nuevo.",
  "wq.notice.held":
    "Se cerró por ahora la revisión de “{label}”; no se aceptó ni se rechazó. Para continuar, usa “Volver a la revisión” en su fila; cuando la vista previa caduque, la revisión te ofrecerá prepararla de nuevo.",

  "wq.detail.close": "Cerrar",
  "wq.detail.attention": "Requiere atención",
  "wq.detail.noAttention": "Nada requiere atención",
  "wq.uncertainty": "(hechos en que se basa: {freshness}; puede que ya no sean válidos)",
  "wq.uncertaintyDegraded":
    "(hechos en que se basa: {freshness}, y la fuente funciona de forma limitada; puede que ya no sean válidos)",
  "wq.detail.deadline": "Fecha límite",
  "wq.detail.promised": "Finalización prometida",
  "wq.detail.placement": "Por qué está aquí",
  "wq.detail.allowed": "El estado permite",
  "wq.none": "Ninguno",
  "wq.detail.owner": "Registro de",
  "wq.detail.version": "Versión",
  "wq.detail.next": "Siguiente paso",

  "wq.backToReview": "Volver a la revisión",
  "wq.sending": "Enviando…",
  "wq.actionFailed": "Esta acción no terminó. {message}",
  "wq.abandon": "Abandonar esta acción",
  "wq.cancel": "Cancelar",
  "wq.reason.closeRisk": "Motivo del cierre",
  "wq.reason.decline": "Motivo del rechazo",
  "wq.reason.withdraw": "Motivo de la retirada",
  "wq.reason.cancel": "Motivo de la cancelación",
  "wq.reason.reopen": "Motivo de la reapertura",
  "wq.confirm.closePreview": "Preparar la vista previa del cierre",
  "wq.confirm.decline": "Confirmar el rechazo",
  "wq.confirm.withdraw": "Confirmar la retirada",
  "wq.confirm.cancelPreview": "Preparar la vista previa de la cancelación",
  "wq.confirm.reopenPreview": "Preparar la vista previa de la reapertura",
  "wq.confirm.resolvePreview": "Preparar la vista previa de la resolución",
  "wq.confirm.completePreview": "Preparar la vista previa de la finalización",
  "wq.reopenMode": "Modo de reapertura",
  "wq.reopenMode.completed": "Reabrir una Action completada",
  "wq.reopenMode.cancelled": "Reanudar una Action cancelada",
  "wq.issueEvidence.resolve":
    "Evidence de que este Issue está resuelto (elige las que quieras; las tres transiciones necesitan Evidence)",
  "wq.issueEvidence.close": "Evidence de que la resolución se verificó (elige las que quieras)",
  "wq.issueEvidence.reopen": "Evidence de que la verificación falló (elige las que quieras)",
  "wq.resolutionType": "Resolución",
  "wq.resolutionType.resolved": "Resuelto",
  "wq.resolutionType.workaround": "Con una solución alternativa",
  "wq.resolutionType.acceptedImpact": "Impacto aceptado",
  "wq.reason.resolve": "Motivo de la resolución",
  "wq.evidenceLoading": "Leyendo la Evidence…",
  "wq.evidenceNone": "No hay Evidence en el Ledger.",
  "wq.evidenceOption.pinned": "{id}: {verification}, fijada, {classification} (versión {version})",
  "wq.evidenceOption.unpinned":
    "{id}: {verification}, sin fijar, {classification} (versión {version})",
  "wq.judgment.issue":
    "Justificación del Judgment (déjala vacía para no adjuntar ninguno; es obligatoria si la Evidence elegida está verificada solo en parte, y no puede respaldar Evidence sin verificar o que no coincide con su registro)",
  "wq.judgment.complete":
    "Justificación del Judgment (déjala vacía para no adjuntar ninguno; es obligatoria si la Evidence vinculada no está verificada)",
  "wq.judgment.decision": "Justificación del Judgment (déjala vacía para no adjuntar ninguno)",
  "wq.judgmentClassification": "Clasificación del Judgment",
  "wq.linkLoading": "Leyendo la Evidence que puedes vincular…",
  "wq.linkChoose": "Evidence que se vinculará",
  "wq.linkPlaceholder": "(elige una)",
  "wq.linkNone": "No queda Evidence en el Ledger que se pueda vincular.",
  "wq.linkConfirm": "Confirmar el vínculo",
  "wq.decision.statement": "Decisión",
  "wq.decision.rationale": "Justificación",
  "wq.decision.impact": "Impacto",
  "wq.decision.evidence":
    "Evidence que respalda esta Decision (elige las que quieras; sin Evidence se necesita un Judgment)",
  "wq.followUps": "Action Requests de seguimiento (la app asigna un id a cada una)",
  "wq.followUp.subject": "Asunto",
  "wq.followUp.details": "Detalles",
  "wq.followUp.owner": "Responsable (id de Stakeholder)",
  "wq.followUp.due": "Fecha límite",
  "wq.followUp.classification": "Clasificación",
  "wq.followUp.remove": "Quitar esta",
  "wq.followUp.add": "Añadir una Action Request de seguimiento",

  "wq.outOfSync":
    "Las dos fuentes de la Work Queue leyeron revisiones distintas del Ledger, así que no se muestra ningún elemento. Unir dos momentos en una sola lista parecería correcto y sería erróneo.",
  "wq.headline": "Qué atender hoy",
  "wq.lede":
    "En este orden: compromisos incumplidos, trabajo bloqueado, Evidence con problemas, fechas límite cercanas y falta de responsable. Cada elemento indica por qué está donde está.",
  "wq.filters": "Filtrar por tipo (si no marcas ninguno, se muestra todo)",
  "wq.filterChip": "{kind} ({count})",
  "wq.onlyFlagged": "Solo los elementos que requieren atención",
  "wq.empty": "Ahora mismo no hay nada que hacer.",
  "wq.column.kind": "Tipo",
  "wq.column.item": "Elemento",
  "wq.column.attention": "Requiere atención",
  "wq.column.deadline": "Fecha límite",
  "wq.column.placement": "Por qué está aquí",
  "wq.column.next": "Siguiente paso",
  "wq.allowed": "El estado permite: {intents}",
  "wq.promised": "Finalización prometida el {date}",
  "wq.inDetail": "Actuar en el panel de detalle",
  "wq.previous": "Página anterior",
  "wq.next": "Página siguiente",
} as const satisfies Record<keyof typeof WORK_QUEUE_EN, string>;
