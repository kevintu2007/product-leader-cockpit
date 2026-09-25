import type { ROUTES_EN } from "./routes.en";

/** 각 화면의 한국어 문구. 키는 영어판과 같습니다. */
export const ROUTES_KO = {
  "route.loading": "읽는 중입니다: {route}.",
  "route.reload": "다시 읽기",
  "route.unavailable":
    "지금은 읽을 수 없습니다: {route}. 이전 데이터는 표시하지 않았습니다. {message}",
  "route.outOfSync":
    "읽는 동안 Ledger가 업데이트되어 두 번 읽은 결과가 일치하지 않았으므로 이번에는 아무것도 표시하지 않았습니다.",
  "route.ledgerRevision": "Ledger 버전",
  "route.readAt": "읽은 시각",

  "cockpit.headline": "좋은 아침입니다. 오늘은 성과에 실제로 영향을 주는 일부터 시작하겠습니다.",
  "cockpit.lede":
    "마일스톤 날짜, 성과 관측 가능성, 검증된 Evidence로 각 Product를 봅니다. 순위를 매기거나 확신도를 추정하지 않습니다.",
  "cockpit.lensModes": "강조 방식",
  "cockpit.lensEmpty":
    "Ledger에 아직 Product가 없습니다. Product를 만들면 마일스톤, KPI, Evidence에 따라 여기에 배치됩니다.",
  "cockpit.lensLegend":
    "원이 클수록 검증된 Evidence의 비율이 높고, 점선 원은 연결된 Evidence가 없음을 뜻합니다. 두 축 모두 데이터가 있을 때만 사분면에 들어갑니다.",
  "cockpit.asideEmpty":
    "차트나 표에서 Product를 선택하면 지표, 근거 레코드, 현재 상태가 여기에 표시됩니다.",
  "cockpit.period": "기간 비교",
  "cockpit.periodUnavailable": "비교할 수 없음: {reason}",
  "cockpit.pulse": "Portfolio 현황",
  "cockpit.pulse.milestones": "Milestones",
  "cockpit.pulse.commitments": "Commitments",
  "cockpit.pulse.kpis": "KPIs",
  "cockpit.pulse.from": "출처: {owner}",
  "cockpit.attention": "주의가 필요한 항목",
  "cockpit.attentionNone": "지금은 주의가 필요한 항목이 없습니다.",
  "cockpit.placedBecause": "여기 있는 이유: {tier}",
  "cockpit.briefing": "요약",
  "cockpit.briefingNone": "지금은 Portfolio 전체에서 주의가 필요한 항목이 없습니다.",
  "cockpit.briefingTop.one": "주의가 필요한 항목은 {count}건입니다. 맨 앞은 “{label}”: {reason}.",
  "cockpit.briefingTop.other": "주의가 필요한 항목은 {count}건입니다. 맨 앞은 “{label}”: {reason}.",
  "cockpit.briefingWhy": "맨 앞에 있는 이유: {tier}.",
  "cockpit.briefingNoPeriod":
    "비교할 기간이 아직 없어 이 항목들이 나아지는지 나빠지는지 알 수 없습니다.",

  "portfolio.headline": "Product 포트폴리오 목록",
  "portfolio.lede":
    "각 Product의 마일스톤, 성과 관측 가능성, 검증된 Evidence와 담당자가 맡고 있는 표시된 작업입니다.",
  "portfolio.showing": "전체 {total}개 중 {from}~{to}번째를 표시하고 있습니다.",
  "portfolio.empty": "Ledger에 아직 Product가 없습니다. Product를 만들면 여기에 나열됩니다.",
  "portfolio.emptyPage": "이 페이지에는 Product가 없습니다. 전체 {total}개가 있습니다.",
  "portfolio.firstPage": "첫 페이지로 돌아가기",
  "portfolio.nextPage": "다음 페이지 보기",
  "portfolio.column.flagged": "담당자의 표시된 작업",
  "portfolio.column.classification": "등급",
  "portfolio.flaggedNone": "없음",
  "portfolio.flaggedCount": "{count}건",
  "portfolio.asideEmpty":
    "표에서 Product를 선택하면 지표, 근거 레코드, 현재 상태가 여기에 표시됩니다.",
  "productAside.label": "선택한 Product",
} as const satisfies Record<keyof typeof ROUTES_EN, string>;
