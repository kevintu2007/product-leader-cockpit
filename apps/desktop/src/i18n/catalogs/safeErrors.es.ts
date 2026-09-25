import type { SAFE_ERRORS_EN } from "./safeErrors.en";

/** Mensajes de error, nombres de campo y siguientes pasos en español. Las claves son las del inglés. */
export const SAFE_ERRORS_ES = {
  "safeError.desktop.snapshot_unavailable":
    "Ahora no se puede leer la instantánea del Product Ledger.",
  "safeError.desktop.snapshot_invalid":
    "Los datos del Product Ledger no superaron la comprobación de coherencia, así que no se escribió nada sobre ellos.",
  "safeError.desktop.product_not_found": "No se encuentra este Product.",
  "safeError.desktop.product_detail_unattributable":
    "Parte del detalle de este Product no tiene un origen rastreable, así que no se muestra.",
  "safeError.desktop.invalid_argument":
    "Los datos enviados no tenían el formato correcto. No se escribió nada.",
  "safeError.desktop.action_not_found": "No se encuentra esta Action.",
  "safeError.desktop.host_id_source_failed":
    "La app no pudo generar el identificador que necesita esta operación.",
  "safeError.desktop.host_identifier_failed":
    "La app no pudo generar el identificador que necesita esta operación.",
  "safeError.desktop.unsupported_preview":
    "La app preparó un tipo de vista previa que esta pantalla no puede mostrar, así que no se mostró.",
  "safeError.desktop.preview_already_consumed":
    "Esta vista previa ya se aprobó o se rechazó. Vuelve a prepararla para continuar.",
  "safeError.desktop.evidence_not_found": "No se encuentra esta Evidence.",
  "safeError.desktop.evidence_already_pinned": "La huella de esta Evidence ya está fijada.",
  "safeError.desktop.evidence_version_conflict":
    "Esta Evidence cambió después de que la leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.desktop.evidence_path_containment_failed":
    "El archivo de esta Evidence no está dentro del Product Vault, así que no se leyó.",
  "safeError.desktop.evidence_source_not_observable":
    "Ahora no se puede leer el archivo de esta Evidence.",
  "safeError.desktop.vault_not_configured":
    "Este espacio de trabajo aún no tiene un Product Vault. Elige uno en Configuración.",
  "safeError.desktop.vault_root_unavailable":
    "Ahora no se puede acceder a la carpeta del Product Vault.",
  "safeError.desktop.settings_unavailable":
    "Ahora no se puede leer ni guardar la configuración de visualización.",
  "safeError.desktop.backup_destination_unusable":
    "Esa carpeta no puede guardar copias de seguridad. Elige una carpeta existente en este equipo en la que puedas escribir, que no sea un acceso directo ni un vínculo.",
  "safeError.desktop.passphrase_too_short":
    "Esa frase de contraseña es demasiado corta. Usa al menos 20 caracteres o seis palabras.",
  "safeError.desktop.passphrase_too_repetitive":
    "Esa frase de contraseña se repite demasiado. Usa palabras o caracteres más variados.",
  "safeError.desktop.passphrase_too_long":
    "Esa frase de contraseña es demasiado larga. Usa como máximo 1024 caracteres.",
  "safeError.desktop.passphrase_too_common":
    "Esa frase de contraseña está hecha de contraseñas muy comunes. Usa palabras que no sean contraseñas comunes.",
  "safeError.desktop.credential_unavailable":
    "El Administrador de credenciales de Windows no pudo guardar ni leer la frase de contraseña ahora.",
  "safeError.desktop.random_unavailable":
    "La app no pudo obtener la aleatoriedad necesaria para crear una frase de contraseña.",

  "safeError.ledger.open.unclaimed_database": "Este archivo de Product Ledger no lo creó esta app.",
  "safeError.ledger.open.wrong_application":
    "Este archivo no es un Ledger de Product Mission Control.",
  "safeError.ledger.open.future_schema":
    "Este Product Ledger viene de una versión más reciente de la app y esta no puede abrirlo.",
  "safeError.ledger.open.unsupported_schema":
    "Esta versión de la app no admite la versión de esquema de este Product Ledger.",
  "safeError.ledger.open.invalid_metadata": "Los metadatos del Product Ledger no son coherentes.",
  "safeError.ledger.open.corrupt_database": "El archivo del Product Ledger está dañado.",
  "safeError.ledger.open.policy_violation":
    "La configuración de conexión del Product Ledger no cumple la política de seguridad.",
  "safeError.ledger.open.busy": "El Product Ledger está ocupado.",
  "safeError.ledger.open.storage_unavailable":
    "El dispositivo de almacenamiento no está disponible.",
  "safeError.ledger.transaction.revision_conflict":
    "Los datos cambiaron después de que los leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.ledger.transaction.incompatible_ledger":
    "El Product Ledger no es compatible con esta versión de la app.",
  "safeError.ledger.transaction.busy": "El Product Ledger está ocupado.",
  "safeError.ledger.transaction.commit_failed":
    "La escritura no terminó y no se puede confirmar si se guardó.",
  "safeError.ledger.commit_failed": "La escritura no terminó y no se puede confirmar si se guardó.",
  "safeError.ledger.persistence_failed": "El Product Ledger no pudo guardar esto.",
  "safeError.ledger.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.audit.failed":
    "No se pudo escribir el registro de auditoría de esta operación, así que no cambió nada.",
  "safeError.version.overflow": "Este registro alcanzó la versión más alta que puede tener.",
  "safeError.classification.lowering_requires_governed_intent":
    "Bajar una clasificación requiere su propio paso revisado y aprobado.",

  "safeError.action.not_found": "No se encuentra esta Action Request o Action.",
  "safeError.action_request.already_exists": "Ya existe una Action Request con este identificador.",
  "safeError.action.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.action.approval_denied":
    "Esto no está autorizado, la política lo rechazó o la Evidence o el Judgment no lo respaldan.",
  "safeError.action.preview_expired_or_changed":
    "La vista previa caducó o cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.action.internal": "La app tuvo un error interno al procesar esto.",
  "safeError.action.validation_failed":
    "Falta un campo obligatorio (por ejemplo, un responsable o una fecha límite).",
  "safeError.action.domain_conflict":
    "Esto entra en conflicto con el estado o la versión actual del registro. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.action.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual.",
  "safeError.action.completion_evidence_already_linked":
    "Esta Evidence ya está vinculada como Evidence de finalización de esta Action.",

  "safeError.decision.not_found": "No se encuentra esta Decision Request o Decision.",
  "safeError.decision.already_exists": "Ya existe una Decision con este identificador.",
  "safeError.decision.conflict":
    "Esto entra en conflicto con el estado o la versión actual de la Decision Request. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.decision.request_transition_conflict":
    "El estado de la Decision Request no permite esto ahora. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.decision.approval_denied":
    "Esto no está autorizado, o la Evidence o el Judgment no lo respaldan.",
  "safeError.decision.preview_changed":
    "La vista previa caducó o cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.decision.internal": "La app tuvo un error interno al procesar esto.",
  "safeError.decision.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual.",

  "safeError.risk.not_found": "No se encuentra este Risk.",
  "safeError.risk.owner_not_found": "No se encuentra el Stakeholder indicado como responsable.",
  "safeError.risk.next_review_before_epoch":
    "La fecha de la próxima revisión no puede ser anterior a 1970.",
  "safeError.risk.already_exists": "Ya existe un Risk con este identificador.",
  "safeError.risk.classification": "Elige una clasificación para este Risk.",
  "safeError.risk.accepted_fields_required":
    "Aceptar o transferir un Risk requiere su justificación y su fecha de revisión.",
  "safeError.risk.conflict":
    "Esto entra en conflicto con el estado actual del Risk. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.risk.stale_or_illegal":
    "El Risk cambió o su estado no permite esto. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.risk.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.risk.preview_changed":
    "La vista previa caducó o cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.risk.security_denied": "Esto no está autorizado para este Risk.",
  "safeError.risk.evidence_unavailable": "Ahora no se puede leer la Evidence que esto necesita.",
  "safeError.risk.infrastructure": "La app tuvo un error interno al procesar este Risk.",

  "safeError.issue.not_found": "No se encuentra este Issue.",
  "safeError.issue.exists": "Ya existe un Issue con este identificador.",
  "safeError.issue.classification_required": "Elige una clasificación para este Issue.",
  "safeError.issue.stale_or_illegal":
    "El Issue cambió o su estado no permite esto. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.issue.invalid_intent": "El estado de este Issue no permite esta operación.",
  "safeError.issue.prepared_operation_mismatch":
    "Esta vista previa se preparó para otra operación. Vuelve a prepararla.",
  "safeError.issue.preview_changed":
    "La vista previa caducó o cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.issue.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.issue.recurrence_not_supported":
    "Este Issue no se puede registrar como una recurrencia.",
  "safeError.issue.security_denied": "Esto no está autorizado para este Issue.",
  "safeError.issue.infrastructure": "La app tuvo un error interno al procesar este Issue.",
  "safeError.issue.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual.",

  "safeError.evidence.not_found": "No se encuentra esta Evidence.",
  "safeError.evidence_reference.not_found": "No se encuentra esta Evidence.",
  "safeError.evidence.already_exists": "Ya existe una Evidence con este identificador.",
  "safeError.evidence.path_already_referenced":
    "Se acaba de crear otra referencia de Evidence para este archivo. Vuelve a elegir el archivo para verla.",
  "safeError.evidence.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.evidence.persistence_failed": "El Product Ledger no pudo guardar esta Evidence.",
  "safeError.evidence.pin_fingerprint_already_pinned": "La huella de esta Evidence ya está fijada.",
  "safeError.evidence.pin_path_mismatch":
    "El archivo leído no coincide con la ruta registrada de esta Evidence, así que no se fijó ninguna huella.",
  "safeError.evidence.relocation_path_unchanged": "La nueva ubicación es la misma que la actual.",
  "safeError.evidence.relocation_path_mismatch":
    "El archivo leído no coincide con la nueva ubicación que indicaste.",
  "safeError.evidence.relocation_fingerprint_mismatch":
    "El archivo de la nueva ubicación no es el mismo: su huella es distinta.",
  "safeError.evidence.relocation_fingerprint_unpinned":
    "Esta Evidence no tiene una huella fijada, así que no se puede confirmar que tras moverlo sea el mismo archivo.",
  "safeError.evidence.supersession_source_mismatch":
    "Este reemplazo se preparó para otra Evidence.",
  "safeError.evidence.supersession_source_already_superseded": "Esta Evidence ya se reemplazó.",
  "safeError.evidence.supersession_replacement_is_source":
    "Una Evidence no puede reemplazarse a sí misma.",
  "safeError.evidence.supersession_not_a_genuine_replacement":
    "El reemplazo es el mismo archivo que la Evidence que reemplazaría.",
  "safeError.evidence.supersession_lowers_classification":
    "Un reemplazo no puede tener una clasificación más baja que la Evidence que reemplaza.",
  "safeError.evidence.supersession_unclassified_replacement":
    "Elige una clasificación para la Evidence de reemplazo.",
  "safeError.evidence.supersession_missing_confirmation":
    "Reemplazar una Evidence requiere tu confirmación.",
  "safeError.evidence.supersession_unauthorized_actor":
    "Esto no está autorizado para esta Evidence.",
  "safeError.evidence.supersession_prepared_intent_mismatch":
    "Esta aprobación es para otro reemplazo preparado.",
  "safeError.evidence.supersession_digest_mismatch":
    "El reemplazo cambió después de prepararse. Vuelve a prepararlo.",
  "safeError.evidence.supersession_preview_changed":
    "La vista previa cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.evidence.supersession_expired": "La vista previa caducó. Vuelve a prepararla.",

  "safeError.portfolio.not_found": "No se encuentra este Portfolio.",
  "safeError.portfolio.already_exists": "Ya existe un Portfolio con este identificador.",
  "safeError.portfolio.stale_version":
    "Este Portfolio cambió después de que lo leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.portfolio.version_exhausted":
    "Este Portfolio alcanzó la versión más alta que puede tener.",
  "safeError.portfolio.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.portfolio.repository_unavailable":
    "Ahora no se pueden leer los registros de Portfolio.",
  "safeError.portfolio.fan_out_state_invalid":
    "La app encontró un estado de Portfolio incoherente y se detuvo antes de escribir.",
  "safeError.portfolio.operation_ordinal_exhausted":
    "La app no puede registrar más operaciones de este tipo.",
  "safeError.portfolio.audit_id_unavailable":
    "La app no pudo generar el identificador de auditoría que necesita esta operación.",
  "safeError.portfolio.prepared_intent_id_unavailable":
    "La app no pudo generar el identificador que necesita esta vista previa.",
  "safeError.portfolio.approval_receipt_id_unavailable":
    "La app no pudo generar el identificador que necesita esta aprobación.",
  "safeError.portfolio.classification_lowering_invalid":
    "La clasificación de este Portfolio no se puede bajar de esta forma.",
  "safeError.portfolio.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual de este Portfolio.",
  "safeError.portfolio.classification_lowering_preview_changed":
    "Este Portfolio cambió después de preparar la vista previa. Vuelve a prepararla.",
  "safeError.portfolio.classification_lowering_approval_mismatch":
    "Esta aprobación es para otra vista previa.",
  "safeError.portfolio.classification_lowering_approval_denied":
    "No se aprobó bajar la clasificación de este Portfolio.",

  "safeError.product.not_found": "No se encuentra este Product.",
  "safeError.product.already_exists": "Ya existe un Product con este identificador.",
  "safeError.product.stale_version":
    "Este Product cambió después de que lo leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.product.version_exhausted":
    "Este Product alcanzó la versión más alta que puede tener.",
  "safeError.product.prepared_intent_id_unavailable":
    "La app no pudo generar el identificador que necesita esta vista previa.",
  "safeError.product.approval_receipt_id_unavailable":
    "La app no pudo generar el identificador que necesita esta aprobación.",
  "safeError.product.classification_lowering_invalid":
    "La clasificación de este Product no se puede bajar de esta forma.",
  "safeError.product.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual de este Product.",
  "safeError.product.classification_lowering_preview_changed":
    "Este Product cambió después de preparar la vista previa. Vuelve a prepararla.",
  "safeError.product.classification_lowering_approval_mismatch":
    "Esta aprobación es para otra vista previa.",
  "safeError.product.classification_lowering_approval_denied":
    "No se aprobó bajar la clasificación de este Product.",

  "safeError.roadmap.not_found": "No se encuentra este Roadmap.",
  "safeError.roadmap.already_exists": "Ya existe un Roadmap con este identificador.",
  "safeError.roadmap.stale_version":
    "Este Roadmap cambió después de que lo leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.roadmap.version_exhausted":
    "Este Roadmap alcanzó la versión más alta que puede tener.",
  "safeError.roadmap.prepared_intent_id_unavailable":
    "La app no pudo generar el identificador que necesita esta vista previa.",
  "safeError.roadmap.approval_receipt_id_unavailable":
    "La app no pudo generar el identificador que necesita esta aprobación.",
  "safeError.roadmap.classification_lowering_invalid":
    "La clasificación de este Roadmap no se puede bajar de esta forma.",
  "safeError.roadmap.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual de este Roadmap.",
  "safeError.roadmap.classification_lowering_preview_changed":
    "Este Roadmap cambió después de preparar la vista previa. Vuelve a prepararla.",
  "safeError.roadmap.classification_lowering_approval_mismatch":
    "Esta aprobación es para otra vista previa.",
  "safeError.roadmap.classification_lowering_approval_denied":
    "No se aprobó bajar la clasificación de este Roadmap.",

  "safeError.kpi.not_found": "No se encuentra este KPI.",
  "safeError.kpi.already_exists": "Ya existe un KPI con este identificador.",
  "safeError.kpi.stale_version":
    "Este KPI cambió después de que lo leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.kpi.version_exhausted": "Este KPI alcanzó la versión más alta que puede tener.",
  "safeError.kpi.prepared_intent_id_unavailable":
    "La app no pudo generar el identificador que necesita esta vista previa.",
  "safeError.kpi.approval_receipt_id_unavailable":
    "La app no pudo generar el identificador que necesita esta aprobación.",
  "safeError.kpi.classification_lowering_invalid":
    "La clasificación de este KPI no se puede bajar de esta forma.",
  "safeError.kpi.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual de este KPI.",
  "safeError.kpi.classification_lowering_preview_changed":
    "Este KPI cambió después de preparar la vista previa. Vuelve a prepararla.",
  "safeError.kpi.classification_lowering_approval_mismatch":
    "Esta aprobación es para otra vista previa.",
  "safeError.kpi.classification_lowering_approval_denied":
    "No se aprobó bajar la clasificación de este KPI.",

  "safeError.kpi.observation.not_found": "No se encuentra esta observación de KPI.",
  "safeError.kpi.observation.already_exists":
    "Ya existe una observación de KPI con este identificador.",
  "safeError.kpi.observation.stale_version":
    "Esta observación de KPI cambió después de que la leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.kpi.observation.version_exhausted":
    "Esta observación de KPI alcanzó la versión más alta que puede tener.",
  "safeError.kpi.observation.prepared_intent_id_unavailable":
    "La app no pudo generar el identificador que necesita esta vista previa.",
  "safeError.kpi.observation.approval_receipt_id_unavailable":
    "La app no pudo generar el identificador que necesita esta aprobación.",
  "safeError.kpi.observation.classification_lowering_invalid":
    "La clasificación de esta observación de KPI no se puede bajar de esta forma.",
  "safeError.kpi.observation.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual de esta observación de KPI.",
  "safeError.kpi.observation.classification_lowering_preview_changed":
    "Esta observación de KPI cambió después de preparar la vista previa. Vuelve a prepararla.",
  "safeError.kpi.observation.classification_lowering_approval_mismatch":
    "Esta aprobación es para otra vista previa.",
  "safeError.kpi.observation.classification_lowering_approval_denied":
    "No se aprobó bajar la clasificación de esta observación de KPI.",

  "safeError.delivery.not_found": "No se encuentra esta Initiative, Project o Milestone.",
  "safeError.delivery.already_exists": "Ya existe un registro con este identificador.",
  "safeError.delivery.stale_version":
    "Este registro cambió después de que lo leyeras. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.delivery.version_exhausted":
    "Este registro alcanzó la versión más alta que puede tener.",
  "safeError.delivery.conflict":
    "Esto entra en conflicto con el estado actual del registro. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.delivery.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.delivery.persistence_failed": "El Product Ledger no pudo guardar esto.",
  "safeError.delivery.invalid_period": "La fecha de inicio es posterior a la fecha de fin.",
  "safeError.delivery.preview_changed":
    "La vista previa caducó o cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.delivery.validation.invalid_field": "Un campo tiene un valor que no está permitido.",
  "safeError.delivery.validation.invalid_text":
    "Un campo de texto está vacío o es demasiado largo.",
  "safeError.delivery.validation.invalid_period":
    "La fecha de inicio es posterior a la fecha de fin.",
  "safeError.delivery.classification.lowering_denied": "No se permite bajar esta clasificación.",
  "safeError.delivery.classification_lowering_invalid":
    "La clasificación de este registro no se puede bajar de esta forma.",
  "safeError.delivery.classification_lowering_not_a_lowering":
    "La clasificación propuesta no es más baja que la actual.",
  "safeError.delivery.classification_lowering_preview_changed":
    "Este registro cambió después de preparar la vista previa. Vuelve a prepararla.",

  "safeError.relationship.not_found": "No se encuentra esta relación.",
  "safeError.relationship.conflict":
    "Esto entra en conflicto con el estado actual de la relación. Vuelve a cargar e inténtalo de nuevo.",
  "safeError.relationship.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.relationship.persistence_failed": "El Product Ledger no pudo guardar esta relación.",
  "safeError.relationship.milestone_subject_not_supported":
    "Un Milestone no puede ser el sujeto de este tipo de relación.",
  "safeError.relationship.classification.unclassified_or_lowering_denied":
    "Elige una clasificación que no sea más baja que la de los registros que conecta.",
  "safeError.relationship.removal.authorization_denied": "No está autorizado quitar esta relación.",
  "safeError.relationship.removal.policy_denied": "La política no permite quitar esta relación.",
  "safeError.relationship.removal.confirmation_mismatch":
    "La confirmación no coincide. Escríbela exactamente como se muestra.",
  "safeError.relationship.removal.preview_expired_or_changed":
    "La vista previa caducó o cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.relationship.removal.too_late_to_cancel":
    "Esta eliminación ya se aprobó, así que no se puede cancelar.",

  "safeError.projection.idempotency_conflict":
    "Este identificador de solicitud ya se usó para otra operación.",
  "safeError.projection.persistence_failed": "El Product Ledger no pudo guardar esta proyección.",
  "safeError.projection.rebuild_operation_not_found":
    "No se encuentra esta reconstrucción de proyección.",
  "safeError.projection.rebuild_operation_terminal":
    "Esta reconstrucción de proyección ya terminó.",
  "safeError.projection.rebuild_prepared_intent_not_found":
    "No se encuentra la reconstrucción preparada. Vuelve a prepararla.",
  "safeError.projection.rebuild_prepared_intent_consumed":
    "Esta reconstrucción preparada ya se usó. Vuelve a prepararla.",
  "safeError.projection.rebuild_prepared_intent_mismatch":
    "Esta aprobación es para otra reconstrucción preparada.",
  "safeError.projection.rebuild_preview_changed":
    "La vista previa de la reconstrucción cambió antes de que la confirmaras. Vuelve a prepararla.",
  "safeError.projection.rebuild_preview_expired":
    "La vista previa de la reconstrucción caducó. Vuelve a prepararla.",
  "safeError.projection.rebuild_digest_mismatch":
    "La reconstrucción cambió después de prepararse. Vuelve a prepararla.",
  "safeError.projection.rebuild_missing_confirmation":
    "Una reconstrucción de proyección requiere tu confirmación.",
  "safeError.projection.rebuild_unauthorized_actor":
    "Esto no está autorizado para las proyecciones.",
  "safeError.projection.rebuild_h1_auto_not_permitted":
    "Esta reconstrucción necesita una revisión; no puede ejecutarse automáticamente.",
  "safeError.projection.rebuild_publication_in_flight":
    "Otra publicación de proyección sigue en curso. Inténtalo de nuevo cuando termine.",
  "safeError.projection.rebuild_empty_change_set": "No hay nada que reconstruir.",
  "safeError.projection.rebuild_changes_not_canonical":
    "Los cambios planificados no tienen la forma esperada, así que no se reconstruyó nada.",
  "safeError.projection.rebuild_unplanned_item":
    "La reconstrucción encontró un elemento que no estaba en su plan, así que se detuvo.",
  "safeError.projection.rebuild_incomplete_report":
    "El informe de la reconstrucción está incompleto, así que no se puede confirmar.",

  "field.delivery.name": "nombre",
  "field.initiative.defined_outcome": "resultado definido",
  "field.milestone.verification_criteria": "criterios de verificación",
  "field.project.time_range": "periodo",

  "nextStep.issue.prepare_resolve": "Prepara una resolución.",
  "nextStep.issue.prepare_close_or_reopen": "Prepárate para cerrarlo o reabrirlo.",
  "nextStep.issue.no_transition": "Un Issue cerrado no tiene más pasos.",
  "nextStep.issue.refresh_and_reprepare": "Vuelve a cargar y luego prepáralo de nuevo.",
  "nextStep.risk.update_response_or_prepare_transition":
    "Actualiza la respuesta o prepara una transición.",
  "nextStep.risk.no_transition": "Este Risk no tiene más pasos.",
  "nextStep.risk.refresh_and_reprepare": "Vuelve a cargar y luego prepáralo de nuevo.",
  "safeError.desktop.backup_due":
    "Toca hacer una copia de seguridad. Hazla primero y vuelve a intentarlo.",
  "safeError.desktop.backup_running":
    "Se está haciendo una copia. Vuelve a intentarlo cuando termine.",
  "safeError.desktop.backup_destination_not_set": "Elige primero una carpeta de copias.",
  "safeError.desktop.backup_destination_unavailable":
    "No se puede acceder a la carpeta de copias. Conecta la unidad o elige otra carpeta.",
  "safeError.desktop.backup_passphrase_required": "Configura primero la frase de recuperación.",
  "safeError.desktop.backup_verification_failed":
    "La copia no pasó la comprobación y no se guardó. Vuelve a intentarlo.",
  "safeError.desktop.backup_failed": "La copia no se completó. Vuelve a intentarlo.",
  "safeError.desktop.restore_ledger_locked":
    "Otro programa tiene abiertos los archivos del Ledger actual. Ciérralo y vuelve a intentarlo. No se cambió nada.",
  "safeError.desktop.restore_preservation_failed":
    "PMC no pudo guardar y comprobar los archivos del Ledger actual. No se cambió nada. La restauración no puede continuar.",
  "safeError.desktop.restore_state_unreadable":
    "PMC no puede leer el registro del estado de la restauración, así que no restaurará. No se cambió nada.",
  "safeError.desktop.restore_recovery_backup_missing":
    "La copia de recuperación ya no está en la carpeta de copias o se modificó. Elige otra copia.",
} as const satisfies Record<keyof typeof SAFE_ERRORS_EN, string>;
