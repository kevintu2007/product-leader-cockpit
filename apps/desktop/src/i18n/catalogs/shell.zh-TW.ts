import type { SHELL_EN } from "./shell.en";

/** 外殼與共用 overlay 的繁體中文文字。鍵與英文完全相同。 */
export const SHELL_ZH_TW = {
  "shell.nav.primary": "主要導覽",
  "noun.portfolioLens": "Portfolio Lens",
  "noun.products": "Products",
  "noun.evidence": "Evidence",
  "noun.productLedger": "Product Ledger",
  "noun.productVault": "Product Vault",
  "shell.routePlaceholder": "此畫面尚未實作，等待所屬垂直切片通過驗證後才會開放。",
  "shell.theme.switchToDark": "切換為深色主題",
  "shell.theme.switchToLight": "切換為淺色主題",
  "shell.theme.dark": "深色主題",
  "shell.theme.light": "淺色主題",
  "shell.nav.attention.one": "{route}，{count} 件需要注意",
  "shell.nav.attention.other": "{route}，{count} 件需要注意",

  "policyStrip.degraded": "降級模式",
  "policyStrip.evidenceVerificationPending": "證據驗證待處理",
  "policyStrip.outOfSync": "未同步",
  "policyStrip.cancelling": "取消中",
  "policyStrip.backupDue": "備份已到期",

  "errorDetail.correlationId": "Correlation ID",
  "errorDetail.copy": "複製",
  "errorDetail.copied": "已複製",
  "errorDetail.retry": "重試",

  "textScale.title": "文字縮放",
  "textScale.sampleBody":
    "早安，今天先處理真正影響結果的事。有 4 件事需要注意，最前面的是一筆回覆期限已過的 Action Request。",
  "textScale.sampleLabel": "排在這裡：承諾已經落空",

  "h2b.confirmPrompt": "請輸入「{phrase}」以核准",
  "h2b.approve": "核准",
  "h2b.reject": "拒絕",
  "h2b.approved": "已核准",
  "h2b.rejected": "已拒絕",
  "h2b.recoveryEvidence": "已驗證的復原證據",
  "h2b.verifiedAt": "驗證時間",
  "h2b.scope": "範圍",
  "h2b.compatibility": "相容性",
  "h2b.transferDetails": "外部傳輸詳情",
  "h2b.provider": "服務提供者",
  "h2b.account": "帳號",
  "h2b.purpose": "用途",
  "h2b.exactPayload": "確切傳輸內容",
  "h2b.irreversible": "此操作一經核准即無法收回：上述內容將實際傳送至外部服務，且無法追回或撤銷。",
  "policyStrip.backingUp": "備份中",
} as const satisfies Record<keyof typeof SHELL_EN, string>;
