import type { SHELL_EN } from "./shell.en";

/** 日本語のシェルと共通オーバーレイの文言。キーは英語版と同じです。 */
export const SHELL_JA = {
  "shell.nav.primary": "メインナビゲーション",
  "noun.portfolioLens": "Portfolio Lens",
  "noun.products": "Products",
  "noun.evidence": "Evidence",
  "noun.productLedger": "Product Ledger",
  "noun.productVault": "Product Vault",
  "shell.routePlaceholder":
    "この画面はまだ利用できません。担当部分の検証が通ると利用できるようになります。",
  "shell.theme.switchToDark": "ダークテーマに切り替え",
  "shell.theme.switchToLight": "ライトテーマに切り替え",
  "shell.theme.dark": "ダークテーマ",
  "shell.theme.light": "ライトテーマ",
  "shell.nav.attention.one": "{route}、要注意 {count} 件",
  "shell.nav.attention.other": "{route}、要注意 {count} 件",

  "policyStrip.degraded": "機能制限モード",
  "policyStrip.evidenceVerificationPending": "Evidence の検証待ち",
  "policyStrip.outOfSync": "未同期",
  "policyStrip.cancelling": "キャンセル中",
  "policyStrip.backupDue": "バックアップの期限",

  "errorDetail.correlationId": "Correlation ID",
  "errorDetail.copy": "コピー",
  "errorDetail.copied": "コピーしました",
  "errorDetail.retry": "再試行",

  "textScale.title": "文字サイズ",
  "textScale.sampleBody":
    "おはようございます。今日は成果に本当に影響することから始めましょう。要注意は 4 件で、先頭は回答期限を過ぎた Action Request です。",
  "textScale.sampleLabel": "ここに並ぶ理由：約束が守られませんでした",

  "h2b.confirmPrompt": "承認するには「{phrase}」と入力してください",
  "h2b.approve": "承認",
  "h2b.reject": "却下",
  "h2b.approved": "承認しました",
  "h2b.rejected": "却下しました",
  "h2b.recoveryEvidence": "検証済みの復元 Evidence",
  "h2b.verifiedAt": "検証日時",
  "h2b.scope": "範囲",
  "h2b.compatibility": "互換性",
  "h2b.transferDetails": "外部への送信の詳細",
  "h2b.provider": "サービス提供者",
  "h2b.account": "アカウント",
  "h2b.purpose": "目的",
  "h2b.exactPayload": "送信される内容",
  "h2b.irreversible":
    "承認すると取り消せません。上記の内容は外部サービスに実際に送信され、取り戻すことも元に戻すこともできません。",
  "policyStrip.backingUp": "バックアップ中",
} as const satisfies Record<keyof typeof SHELL_EN, string>;
