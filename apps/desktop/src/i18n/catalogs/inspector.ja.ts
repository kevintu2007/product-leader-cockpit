import type { INSPECTOR_EN } from "./inspector.en";

/** O01 Product インスペクターの日本語の文言。キーは英語版と同じです。 */
export const INSPECTOR_JA = {
  "inspector.route": "Product の詳細",
  "inspector.outOfSync":
    "Product の詳細の 2 つの情報源が異なる Ledger リビジョンを読み込んだため、何も表示していません。2 つの時点をつなぎ合わせると、正しく見えて誤った表示になります。",
  "inspector.healthEvidence": "Evidence {label} が関連付けられています。検証状態：{verification}。",
  "inspector.healthRecord": "{kind}「{label}」：{reason}。",

  "inspector.pinConfirm":
    "{id} のフィンガープリントを固定しますか？この Evidence が現在あるファイルを読み取り、その時点のバイト列の要約を識別情報として記録します。固定は取り消せません。以後、同じ内容なら「再観測」、ファイルの移動は「移動」、内容の変更は「置き換え」となり、このフィンガープリントは書き換えられません。",
  "inspector.reobserveConfirm":
    "{id} を再観測しますか？この Evidence に記録された場所のファイルを読み取り、観測結果が保存済みの状態と異なる場合のみ書き込みます。",
  "inspector.confirmPin": "固定を確定",
  "inspector.confirmReobserve": "再観測を確定",
  "inspector.cancel": "キャンセル",
  "inspector.sending": "送信中…",
  "inspector.written": "書き込みました。検証状態は現在「{verification}」です。",
  "inspector.unchanged":
    "変化はありません。観測結果が保存済みの状態と同じため、Ledger には書き込んでいません。",
  "inspector.close": "閉じる",
  "inspector.pinFailed": "固定は完了しませんでした。{message}",
  "inspector.reobserveFailed": "再観測は完了しませんでした。{message}",
  "inspector.abandon": "この操作を中止する",
  "inspector.pin": "フィンガープリントを固定",
  "inspector.reobserve": "再観測",

  "inspector.link": "Evidence をこの Product に関連付ける",
  "inspector.linkLoading": "Evidence を読み込んでいます…",
  "inspector.linkChoose": "{product} に関連付ける Evidence",
  "inspector.linkNone": "（関連付けられる Evidence がありません）",
  "inspector.linkCandidate": "{id}（{verification}、{classification}、バージョン {version}）",
  "inspector.linkConfirm": "関連付けを確定",
  "inspector.linked": "{id} を関連付けました。関連付け時の分類：{classification}。",
  "inspector.linkFailed": "関連付けは完了しませんでした。{message}",

  "inspector.classification": "分類",
  "inspector.version": "バージョン",
  "inspector.fold":
    "表示している {kind} {id} がこの分類のため、このインスペクターは {classification} として表示されています。",
  "inspector.happened": "起きたこと",
  "inspector.nothingHappened": "現在、注意が必要な状況はありません。",
  "inspector.conditionLine": "{condition}{provenance}",
  "inspector.provenance": "出所：{owner} {id}、バージョン {version}",
  "inspector.impact": "影響",
  "inspector.impactUnassessed": "まだ誰も評価していません",
  "inspector.tabs": "Product の詳細",
  "inspector.tab.structure": "構造",
  "inspector.tab.evidence": "Evidence",
  "inspector.tab.people": "人",
  "inspector.structureNone": "Ledger にはこの Product に関連する構造がありません。",
  "inspector.structureEntry": "{kind}：{label}（{classification}）",
  "inspector.structureEntryVia": "{kind}：{label}（{classification}）、{via}",
  "inspector.via": "{project} 経由",
  "inspector.vaultNotConfigured":
    "このワークスペースには使用できる Product Vault がありません（未設定）。ファイルを読み取る操作（フィンガープリントの固定、再観測）は利用できません。Evidence の関連付けには影響しません。",
  "inspector.vaultUnavailable":
    "現在 Product Vault を読み取れません（フォルダーが存在しない、フォルダーではない、またはリンクです）。ファイルを読み取る操作（フィンガープリントの固定、再観測）は一時停止しています。Evidence の関連付けには影響しません。",
  "inspector.evidenceNone": "この Product に直接関連付けられた Evidence はありません。",
  "inspector.evidenceLine": "{id}：{verification}{classifications}",
  "inspector.evidenceLineUnpinned": "{id}：{verification}{unpinned}{classifications}",
  "inspector.unpinned": "（フィンガープリント未固定）",
  "inspector.evidenceClassifications":
    "（Evidence の分類 {classification}、関連付け時の分類 {atLink}）",
  "inspector.peopleNone": "Ledger には、この Product を担当している人も依存している人もいません。",
  "inspector.person.one": "{name}（{purpose}、他に {count} 件の Product に関与）",
  "inspector.person.other": "{name}（{purpose}、他に {count} 件の Product に関与）",
  "inspector.purpose.responsibility": "担当",
  "inspector.purpose.dependency": "依存",
  "inspector.carriedHeading":
    "この人が現在抱えているもの（担当として。この Product の所有物ではありません）",
  "inspector.carriedNone": "現在抱えている作業項目はありません。",
  "inspector.carriedLine": "{kind} {label}（{state}）{attention}{intents}",
  "inspector.nextSteps": "ライフサイクル上可能な次の手順：{intents}",
  "inspector.noNextSteps": "なし",
} as const satisfies Record<keyof typeof INSPECTOR_EN, string>;
