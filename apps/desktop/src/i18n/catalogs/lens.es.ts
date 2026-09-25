import type { LENS_EN } from "./lens.en";

/** Textos en español del Portfolio Lens. Las claves son las del inglés. */
export const LENS_ES = {
  "lens.happened.unknown": "No hay hitos vinculados, así que no hay calendario que leer",
  "lens.happened.later.one": "Todos los hitos están a más de {days} día",
  "lens.happened.later.other": "Todos los hitos están a más de {days} días",
  "lens.happened.dueSoon.one": "Un hito vence en {days} día o menos",
  "lens.happened.dueSoon.other": "Un hito vence en {days} días o menos",
  "lens.happened.datePassed": "Ha pasado la fecha de un hito",
  "lens.happened.withEvidence": "{timing}; parte de la Evidence está: {verification}",
  "lens.timing.none": "No hay hitos vinculados",
  "lens.timing.line": "{state}, el más próximo {date}, {count} en total",
  "lens.observability.none": "No hay definiciones de KPI",
  "lens.observability.line": "{observed}/{defined} KPIs observados",
  "lens.observability.lineLatest": "{observed}/{defined} KPIs observados, el último {date}",
  "lens.coverage.none": "No hay Evidence vinculada",
  "lens.coverage.line": "{verified}/{linked} verificadas",

  "lens.mode.timing": "Hitos",
  "lens.mode.observability": "Observabilidad de resultados",
  "lens.mode.evidence": "Evidence",
  "lens.modeCopy.timing":
    "Marca los Products con un hito cuya fecha ya pasó. El eje horizontal son fechas de hitos; no significa que el trabajo esté atrasado.",
  "lens.modeCopy.observability":
    "Marca los Products con definiciones de KPI en los que menos de la mitad tiene una observación.",
  "lens.modeCopy.evidence":
    "Marca los Products con Evidence que no coincide con su registro o que no se ha verificado.",

  "lens.bubble.label":
    "{product}. {happened}. Hitos: {timing}. Observabilidad de resultados: {observability}. Evidence verificada: {coverage}.",
  "lens.tooltip.happened": "Qué pasó",
  "lens.tooltip.happenedLine": "{label}: {happened}",
  "lens.tooltip.impact": "Impacto",
  "lens.tooltip.impactLine": "{label}: nadie lo ha evaluado todavía",
  "lens.tooltip.next": "Siguiente paso",
  "lens.tooltip.nextLine": "{label}: selecciónalo para verlo en detalle a la derecha",
  "lens.tooltip.timing": "Hitos: {value}",
  "lens.tooltip.observability": "Observabilidad de resultados: {value}",
  "lens.tooltip.coverage": "Evidence verificada: {value}",
  "lens.canvas.label":
    "Gráfico de cuadrantes del Portfolio Lens; la tabla de abajo tiene los mismos datos",
  "lens.axis.y": "Observabilidad de resultados →",
  "lens.band.noMilestones": "Sin hitos",
  "lens.band.noKpis": "Sin definiciones de KPI",
  "lens.axis.x": "Fechas de hitos: {later} → {dueSoon} → {datePassed}",

  "lens.table.caption": "Ordenado por nombre de Product: un orden para explorar, no una prioridad.",
  "lens.table.product": "Product",
  "lens.table.timing": "Hitos",
  "lens.table.observability": "Observabilidad de resultados",
  "lens.table.coverage": "Evidence verificada",
  "lens.table.quadrant": "Cuadrante",
  "lens.table.view": "Ver",
  "lens.table.notEnoughData": "Datos insuficientes",
  "lens.table.selected": "Seleccionado",
  "lens.table.viewProduct": "Ver {product}",
  "lens.table.selectedProduct": "{product} seleccionado",

  "lens.contribution.versionless":
    "el vínculo no tiene versión propia; leído en la instantánea {revision}",
  "lens.contribution.version": "versión {version}",
  "lens.contribution.line": "{kind} {id} ({version}, {classification})",
  "lens.measures.heading": "Medidas del Lens para {product}",
  "lens.measures.ownClassification": "La clasificación propia del Product",
  "lens.measures.classificationFrom": "Establecida por {kind} {id}",
  "lens.measures.quadrant": "Cuadrante",
  "lens.measures.noQuadrant": "No hay datos suficientes para ubicarlo en un cuadrante",
  "lens.measures.timing": "Hitos",
  "lens.measures.observability": "Observabilidad de resultados",
  "lens.measures.coverage": "Evidence verificada",
  "lens.measures.stateCount": "{state}: {count}",
  "lens.measures.sharedProjects": "Projects compartidos",
  "lens.measures.sharedProjectList": "{ids} (también vinculados a otros Products)",
  "lens.measures.sources": "De dónde salen estas cifras ({count} registros)",
  "lens.measures.noSources": "No hay registros vinculados.",
} as const satisfies Record<keyof typeof LENS_EN, string>;
