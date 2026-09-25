import type { ROUTES_EN } from "./routes.en";

/** 各画面の日本語の文言。キーは英語版と同じです。 */
export const ROUTES_JA = {
  "route.loading": "{route} を読み込んでいます。",
  "route.reload": "読み込み直す",
  "route.unavailable": "現在 {route} を読み込めません。古いデータは表示していません。{message}",
  "route.outOfSync":
    "読み込み中に Ledger が更新され、2 回の読み込み結果が一致しなかったため、今回は何も表示していません。",
  "route.ledgerRevision": "Ledger バージョン",
  "route.readAt": "読み込み日時",

  "cockpit.headline": "おはようございます。今日は成果に本当に影響することから始めましょう。",
  "cockpit.lede":
    "マイルストーンの日付、成果の可観測性、検証済み Evidence で各 Product を見ます。順位付けも確信度の推定もしません。",
  "cockpit.lensModes": "強調の方法",
  "cockpit.lensEmpty":
    "Ledger にはまだ Product がありません。Product を作成すると、マイルストーン、KPI、Evidence に基づいてここに配置されます。",
  "cockpit.lensLegend":
    "円が大きいほど検証済み Evidence の割合が高く、点線の円は関連付けられた Evidence がないことを示します。両方の軸にデータがある場合のみ象限に入ります。",
  "cockpit.asideEmpty":
    "図または表で Product を選ぶと、その指標、根拠となるレコード、現在の状態がここに表示されます。",
  "cockpit.period": "期間比較",
  "cockpit.periodUnavailable": "比較できません：{reason}",
  "cockpit.pulse": "Portfolio の現状",
  "cockpit.pulse.milestones": "Milestones",
  "cockpit.pulse.commitments": "Commitments",
  "cockpit.pulse.kpis": "KPIs",
  "cockpit.pulse.from": "出所：{owner}",
  "cockpit.attention": "要注意事項",
  "cockpit.attentionNone": "現在、要注意事項はありません。",
  "cockpit.placedBecause": "ここに並ぶ理由：{tier}",
  "cockpit.briefing": "あなたへのまとめ",
  "cockpit.briefingNone": "現在、Portfolio 全体で注意が必要な事項はありません。",
  "cockpit.briefingTop.one": "要注意事項は {count} 件です。先頭は「{label}」：{reason}。",
  "cockpit.briefingTop.other": "要注意事項は {count} 件です。先頭は「{label}」：{reason}。",
  "cockpit.briefingWhy": "先頭にある理由：{tier}。",
  "cockpit.briefingNoPeriod":
    "比較できる期間がまだないため、これらが良くなっているのか悪くなっているのかは分かりません。",

  "portfolio.headline": "Product ポートフォリオ一覧",
  "portfolio.lede":
    "各 Product のマイルストーン、成果の可観測性、検証済み Evidence と、担当者が抱えるフラグ付きの作業です。",
  "portfolio.showing": "全 {total} 件中 {from}〜{to} 件目を表示しています。",
  "portfolio.empty":
    "Ledger にはまだ Product がありません。Product を作成すると、ここに一覧表示されます。",
  "portfolio.emptyPage": "このページに Product はありません。全部で {total} 件あります。",
  "portfolio.firstPage": "最初のページに戻る",
  "portfolio.nextPage": "次のページを表示",
  "portfolio.column.flagged": "担当者のフラグ付き作業",
  "portfolio.column.classification": "分類",
  "portfolio.flaggedNone": "なし",
  "portfolio.flaggedCount": "{count} 件",
  "portfolio.asideEmpty":
    "表で Product を選ぶと、その指標、根拠となるレコード、現在の状態がここに表示されます。",
  "productAside.label": "選択中の Product",
} as const satisfies Record<keyof typeof ROUTES_EN, string>;
