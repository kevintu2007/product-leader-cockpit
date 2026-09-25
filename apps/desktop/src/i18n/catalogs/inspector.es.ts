import type { INSPECTOR_EN } from "./inspector.en";

/** Textos en español de O01, el inspector de Product. Las claves son las del inglés. */
export const INSPECTOR_ES = {
  "inspector.route": "Detalle del Product",
  "inspector.outOfSync":
    "Las dos fuentes del detalle de este Product leyeron revisiones distintas del Ledger, así que no se muestra nada. Unir dos momentos en un solo inspector parecería correcto y sería erróneo.",
  "inspector.healthEvidence": "La Evidence {label} está vinculada; verificación: {verification}.",
  "inspector.healthRecord": "{kind} “{label}”: {reason}.",

  "inspector.pinConfirm":
    "¿Fijar una huella para {id}? La app lee el archivo en el que está ahora esta Evidence y registra un resumen de sus bytes como su identidad. Una huella fijada es permanente: el mismo contenido más adelante es una “nueva observación”, un archivo movido es una “reubicación” y un contenido cambiado es un “reemplazo”; ninguno de ellos reescribe esta huella.",
  "inspector.reobserveConfirm":
    "¿Volver a observar {id}? La app lee el archivo en la ubicación que registra esta Evidence y solo escribe si lo que ve difiere de lo guardado.",
  "inspector.confirmPin": "Confirmar la huella",
  "inspector.confirmReobserve": "Confirmar la nueva observación",
  "inspector.cancel": "Cancelar",
  "inspector.sending": "Enviando…",
  "inspector.written": "Escrito: la verificación ahora es {verification}.",
  "inspector.unchanged":
    "Sin cambios: lo observado coincide con lo guardado, así que no se escribió en el Ledger.",
  "inspector.close": "Cerrar",
  "inspector.pinFailed": "La huella no se fijó. {message}",
  "inspector.reobserveFailed": "La nueva observación no terminó. {message}",
  "inspector.abandon": "Abandonar esta acción",
  "inspector.pin": "Fijar huella",
  "inspector.reobserve": "Volver a observar",

  "inspector.link": "Vincular Evidence a este Product",
  "inspector.linkLoading": "Leyendo la Evidence…",
  "inspector.linkChoose": "Evidence que se vinculará a {product}",
  "inspector.linkNone": "(no queda Evidence por vincular)",
  "inspector.linkCandidate": "{id} ({verification}, {classification}, versión {version})",
  "inspector.linkConfirm": "Confirmar el vínculo",
  "inspector.linked": "Se vinculó {id}; clasificación al vincular: {classification}.",
  "inspector.linkFailed": "El vínculo no terminó. {message}",

  "inspector.classification": "Clasificación",
  "inspector.version": "Versión",
  "inspector.fold":
    "Este inspector se muestra como {classification} porque {kind} {id}, que aparece en él, tiene esa clasificación.",
  "inspector.happened": "Qué pasó",
  "inspector.nothingHappened": "Ahora mismo nada requiere atención.",
  "inspector.conditionLine": "{condition} {provenance}",
  "inspector.provenance": "Fuente: {owner} {id}, versión {version}",
  "inspector.impact": "Impacto",
  "inspector.impactUnassessed": "Nadie lo ha evaluado todavía",
  "inspector.tabs": "Detalle del Product",
  "inspector.tab.structure": "Estructura",
  "inspector.tab.evidence": "Evidence",
  "inspector.tab.people": "Personas",
  "inspector.structureNone": "Nada en el Ledger está estructurado en torno a este Product.",
  "inspector.structureEntry": "{kind}: {label} ({classification})",
  "inspector.structureEntryVia": "{kind}: {label} ({classification}), {via}",
  "inspector.via": "a través de {project}",
  "inspector.vaultNotConfigured":
    "Este espacio de trabajo aún no tiene un Product Vault (sin definir). Las acciones que leen archivos (fijar huella, volver a observar) no están disponibles; vincular Evidence sigue funcionando.",
  "inspector.vaultUnavailable":
    "Ahora no se puede leer el Product Vault (la carpeta no existe, no es una carpeta o es un vínculo). Las acciones que leen archivos (fijar huella, volver a observar) están en pausa; vincular Evidence sigue funcionando.",
  "inspector.evidenceNone": "No hay Evidence vinculada directamente a este Product.",
  "inspector.evidenceLine": "{id}: {verification} {classifications}",
  "inspector.evidenceLineUnpinned": "{id}: {verification} {unpinned} {classifications}",
  "inspector.unpinned": "(sin huella fijada)",
  "inspector.evidenceClassifications":
    "(clasificación de la Evidence {classification}; clasificación al vincular {atLink})",
  "inspector.peopleNone": "Nadie en el Ledger es responsable de este Product ni depende de él.",
  "inspector.person.one": "{name} ({purpose}; también vinculado a {count} Product más)",
  "inspector.person.other": "{name} ({purpose}; también vinculado a {count} Products más)",
  "inspector.purpose.responsibility": "responsable",
  "inspector.purpose.dependency": "depende de él",
  "inspector.carriedHeading":
    "Lo que lleva esta persona ahora (por su responsabilidad, no porque pertenezca a este Product)",
  "inspector.carriedNone": "Ahora no lleva ningún elemento de trabajo.",
  "inspector.carriedLine": "{kind} {label} ({state}){attention} {intents}",
  "inspector.nextSteps": "Siguientes pasos que permite el ciclo de vida: {intents}",
  "inspector.noNextSteps": "ninguno",
} as const satisfies Record<keyof typeof INSPECTOR_EN, string>;
