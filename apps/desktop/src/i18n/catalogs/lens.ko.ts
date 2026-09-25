import type { LENS_EN } from "./lens.en";

/** Portfolio Lens의 한국어 문구. 키는 영어판과 같습니다. */
export const LENS_KO = {
  "lens.happened.unknown": "연결된 마일스톤이 없어 일정을 읽을 수 없습니다",
  "lens.happened.later.one": "모든 마일스톤이 {days}일 이상 남았습니다",
  "lens.happened.later.other": "모든 마일스톤이 {days}일 이상 남았습니다",
  "lens.happened.dueSoon.one": "{days}일 안에 기한이 되는 마일스톤이 있습니다",
  "lens.happened.dueSoon.other": "{days}일 안에 기한이 되는 마일스톤이 있습니다",
  "lens.happened.datePassed": "날짜가 지난 마일스톤이 있습니다",
  "lens.happened.withEvidence": "{timing}. Evidence 상태: {verification}",
  "lens.timing.none": "연결된 마일스톤 없음",
  "lens.timing.line": "{state}, 가장 이른 날짜 {date}, 총 {count}개",
  "lens.observability.none": "KPI 정의 없음",
  "lens.observability.line": "KPI {defined}개 중 {observed}개에 관측값 있음",
  "lens.observability.lineLatest": "KPI {defined}개 중 {observed}개에 관측값 있음, 최근 {date}",
  "lens.coverage.none": "연결된 Evidence 없음",
  "lens.coverage.line": "{linked}개 중 {verified}개 검증됨",

  "lens.mode.timing": "마일스톤",
  "lens.mode.observability": "성과 관측 가능성",
  "lens.mode.evidence": "Evidence",
  "lens.modeCopy.timing":
    "마일스톤 날짜가 지난 Product를 강조합니다. 가로축은 마일스톤 날짜이며 작업이 늦었다는 뜻은 아닙니다.",
  "lens.modeCopy.observability":
    "KPI 정의는 있지만 관측값이 있는 KPI가 절반 미만인 Product를 강조합니다.",
  "lens.modeCopy.evidence":
    "기록과 내용이 일치하지 않거나 검증되지 않은 Evidence가 있는 Product를 강조합니다.",

  "lens.bubble.label":
    "{product}. {happened}. 마일스톤: {timing}. 성과 관측 가능성: {observability}. 검증된 Evidence: {coverage}.",
  "lens.tooltip.happened": "일어난 일",
  "lens.tooltip.happenedLine": "{label}: {happened}",
  "lens.tooltip.impact": "영향",
  "lens.tooltip.impactLine": "{label}: 아직 아무도 평가하지 않았습니다",
  "lens.tooltip.next": "다음 단계",
  "lens.tooltip.nextLine": "{label}: 선택하면 오른쪽에 세부 정보가 표시됩니다",
  "lens.tooltip.timing": "마일스톤: {value}",
  "lens.tooltip.observability": "성과 관측 가능성: {value}",
  "lens.tooltip.coverage": "검증된 Evidence: {value}",
  "lens.canvas.label": "Portfolio Lens 사분면 차트. 아래 표에 같은 데이터가 있습니다",
  "lens.axis.y": "성과 관측 가능성 →",
  "lens.band.noMilestones": "마일스톤 없음",
  "lens.band.noKpis": "KPI 정의 없음",
  "lens.axis.x": "마일스톤 날짜: {later} → {dueSoon} → {datePassed}",

  "lens.table.caption":
    "Product 이름순으로 정렬했습니다. 둘러보기 위한 순서이며 우선순위가 아닙니다.",
  "lens.table.product": "Product",
  "lens.table.timing": "마일스톤",
  "lens.table.observability": "성과 관측 가능성",
  "lens.table.coverage": "검증된 Evidence",
  "lens.table.quadrant": "사분면",
  "lens.table.view": "보기",
  "lens.table.notEnoughData": "데이터 부족",
  "lens.table.selected": "선택됨",
  "lens.table.viewProduct": "{product} 보기",
  "lens.table.selectedProduct": "{product} 선택됨",

  "lens.contribution.versionless":
    "연결 자체에는 버전이 없으며 스냅샷 {revision}을 기준으로 합니다",
  "lens.contribution.version": "버전 {version}",
  "lens.contribution.line": "{kind} {id}({version}, {classification})",
  "lens.measures.heading": "{product}의 Lens 지표",
  "lens.measures.ownClassification": "Product 자체의 등급",
  "lens.measures.classificationFrom": "{kind} {id}에 의해 정해진 등급",
  "lens.measures.quadrant": "사분면",
  "lens.measures.noQuadrant": "데이터가 부족해 어느 사분면에도 들어가지 않습니다",
  "lens.measures.timing": "마일스톤",
  "lens.measures.observability": "성과 관측 가능성",
  "lens.measures.coverage": "검증된 Evidence",
  "lens.measures.stateCount": "{state}: {count}개",
  "lens.measures.sharedProjects": "공유된 Project",
  "lens.measures.sharedProjectList": "{ids}(다른 Product에도 연결되어 있습니다)",
  "lens.measures.sources": "이 수치의 근거가 되는 레코드({count}개)",
  "lens.measures.noSources": "연결된 레코드가 없습니다.",
} as const satisfies Record<keyof typeof LENS_EN, string>;
