import type { LENS_EN } from "./lens.en";

/** Portfolio Lens の日本語の文言。キーは英語版と同じです。 */
export const LENS_JA = {
  "lens.happened.unknown": "関連付けられたマイルストーンがないため、スケジュールが読み取れません",
  "lens.happened.later.one": "マイルストーンはすべて {days} 日以上先です",
  "lens.happened.later.other": "マイルストーンはすべて {days} 日以上先です",
  "lens.happened.dueSoon.one": "{days} 日以内に期日を迎えるマイルストーンがあります",
  "lens.happened.dueSoon.other": "{days} 日以内に期日を迎えるマイルストーンがあります",
  "lens.happened.datePassed": "日付を過ぎたマイルストーンがあります",
  "lens.happened.withEvidence": "{timing}。Evidence の状態：{verification}",
  "lens.timing.none": "関連付けられたマイルストーンなし",
  "lens.timing.line": "{state}、最も早い日付 {date}、全 {count} 件",
  "lens.observability.none": "KPI 定義なし",
  "lens.observability.line": "{observed}／{defined} 件の KPI に観測値あり",
  "lens.observability.lineLatest": "{observed}／{defined} 件の KPI に観測値あり、最新 {date}",
  "lens.coverage.none": "関連付けられた Evidence なし",
  "lens.coverage.line": "{verified}／{linked} 件が検証済み",

  "lens.mode.timing": "マイルストーン",
  "lens.mode.observability": "成果の可観測性",
  "lens.mode.evidence": "Evidence",
  "lens.modeCopy.timing":
    "マイルストーンの日付を過ぎた Product を強調します。横軸はマイルストーンの日付で、作業の遅延を意味するものではありません。",
  "lens.modeCopy.observability":
    "KPI 定義があるものの、観測値のある KPI が半分未満の Product を強調します。",
  "lens.modeCopy.evidence":
    "記録と内容が一致しない、または未検証の Evidence がある Product を強調します。",

  "lens.bubble.label":
    "{product}。{happened}。マイルストーン：{timing}。成果の可観測性：{observability}。検証済み Evidence：{coverage}。",
  "lens.tooltip.happened": "起きたこと",
  "lens.tooltip.happenedLine": "{label}：{happened}",
  "lens.tooltip.impact": "影響",
  "lens.tooltip.impactLine": "{label}：まだ誰も評価していません",
  "lens.tooltip.next": "次の手順",
  "lens.tooltip.nextLine": "{label}：選択すると右側に詳細が表示されます",
  "lens.tooltip.timing": "マイルストーン：{value}",
  "lens.tooltip.observability": "成果の可観測性：{value}",
  "lens.tooltip.coverage": "検証済み Evidence：{value}",
  "lens.canvas.label": "Portfolio Lens の象限図。下の表に同じデータがあります",
  "lens.axis.y": "成果の可観測性 ↑",
  "lens.band.noMilestones": "マイルストーンなし",
  "lens.band.noKpis": "KPI 定義なし",
  "lens.axis.x": "マイルストーンの日付：{later} → {dueSoon} → {datePassed}",

  "lens.table.caption":
    "Product 名の順に並んでいます。閲覧のための順序で、優先順位ではありません。",
  "lens.table.product": "Product",
  "lens.table.timing": "マイルストーン",
  "lens.table.observability": "成果の可観測性",
  "lens.table.coverage": "検証済み Evidence",
  "lens.table.quadrant": "象限",
  "lens.table.view": "表示",
  "lens.table.notEnoughData": "データ不足",
  "lens.table.selected": "選択中",
  "lens.table.viewProduct": "{product} を表示",
  "lens.table.selectedProduct": "{product} を選択中",

  "lens.contribution.versionless":
    "関連付け自体にバージョンはなく、スナップショット {revision} を基準にしています",
  "lens.contribution.version": "バージョン {version}",
  "lens.contribution.line": "{kind} {id}（{version}、{classification}）",
  "lens.measures.heading": "{product} の Lens 指標",
  "lens.measures.ownClassification": "Product 自体の分類",
  "lens.measures.classificationFrom": "{kind} {id} によって決まる分類",
  "lens.measures.quadrant": "象限",
  "lens.measures.noQuadrant": "データ不足のため、どの象限にも入りません",
  "lens.measures.timing": "マイルストーン",
  "lens.measures.observability": "成果の可観測性",
  "lens.measures.coverage": "検証済み Evidence",
  "lens.measures.stateCount": "{state}：{count} 件",
  "lens.measures.sharedProjects": "共有されている Project",
  "lens.measures.sharedProjectList": "{ids}（他の Product にも関連付けられています）",
  "lens.measures.sources": "これらの数値の根拠となるレコード（{count} 件）",
  "lens.measures.noSources": "関連付けられたレコードはありません。",
} as const satisfies Record<keyof typeof LENS_EN, string>;
