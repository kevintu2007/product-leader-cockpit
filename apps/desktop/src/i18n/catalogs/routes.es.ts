import type { ROUTES_EN } from "./routes.en";

/** Textos en español de cada pantalla. Las claves son las del inglés. */
export const ROUTES_ES = {
  "route.loading": "Leyendo {route}.",
  "route.reload": "Volver a leer",
  "route.unavailable": "Ahora no se puede leer {route}. No se muestra nada antiguo. {message}",
  "route.outOfSync":
    "El Ledger cambió mientras se leía, así que las dos lecturas no coinciden y esta vez no se muestra nada.",
  "route.ledgerRevision": "Versión del Ledger",
  "route.readAt": "Leído el",

  "cockpit.headline": "Buenos días. Empieza por lo que de verdad mueve los resultados hoy.",
  "cockpit.lede":
    "Cada Product según las fechas de sus hitos, la observabilidad de resultados y la Evidence verificada. Sin priorización ni estimación de confianza.",
  "cockpit.lensModes": "Marcado",
  "cockpit.lensEmpty":
    "Aún no hay Products en el Ledger. Cuando crees uno, se ubicará aquí según sus hitos, KPIs y Evidence.",
  "cockpit.lensLegend":
    "Un círculo más grande indica una mayor proporción de Evidence verificada; un círculo discontinuo indica que no hay Evidence vinculada. Un Product solo se ubica en un cuadrante cuando ambos ejes tienen datos.",
  "cockpit.asideEmpty":
    "Elige un Product en el gráfico o en la tabla para ver sus medidas, los registros que las sustentan y su situación.",
  "cockpit.period": "Comparación de periodos",
  "cockpit.periodUnavailable": "No se puede comparar: {reason}",
  "cockpit.pulse": "El Portfolio hoy",
  "cockpit.pulse.milestones": "Milestones",
  "cockpit.pulse.commitments": "Commitments",
  "cockpit.pulse.kpis": "KPIs",
  "cockpit.pulse.from": "Fuente: {owner}",
  "cockpit.attention": "Requiere atención",
  "cockpit.attentionNone": "Ahora mismo nada requiere atención.",
  "cockpit.placedBecause": "Está aquí porque: {tier}",
  "cockpit.briefing": "Tu resumen",
  "cockpit.briefingNone": "Ahora mismo nada en el Portfolio requiere tu atención.",
  "cockpit.briefingTop.one": "{count} asunto requiere atención. El primero es “{label}”: {reason}.",
  "cockpit.briefingTop.other":
    "{count} asuntos requieren atención. El primero es “{label}”: {reason}.",
  "cockpit.briefingWhy": "Va primero porque: {tier}.",
  "cockpit.briefingNoPeriod":
    "Aún no hay un periodo con el que comparar, así que no se puede saber si esto mejora o empeora.",

  "portfolio.headline": "El Portfolio de Products",
  "portfolio.lede":
    "Los hitos, la observabilidad de resultados y la Evidence verificada de cada Product, y el trabajo marcado que llevan sus responsables.",
  "portfolio.showing": "{total} en total; se muestran del {from} al {to}.",
  "portfolio.empty": "Aún no hay Products en el Ledger. Cuando crees alguno, aparecerá aquí.",
  "portfolio.emptyPage": "No hay Products en esta página; hay {total} en total.",
  "portfolio.firstPage": "Volver a la primera página",
  "portfolio.nextPage": "Mostrar la página siguiente",
  "portfolio.column.flagged": "Trabajo marcado de los responsables",
  "portfolio.column.classification": "Clasificación",
  "portfolio.flaggedNone": "Ninguno",
  "portfolio.flaggedCount": "{count}",
  "portfolio.asideEmpty":
    "Elige un Product en la tabla para ver sus medidas, los registros que las sustentan y su situación.",
  "productAside.label": "Product seleccionado",
} as const satisfies Record<keyof typeof ROUTES_EN, string>;
