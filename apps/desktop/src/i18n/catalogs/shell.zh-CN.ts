import type { SHELL_EN } from "./shell.en";

/** 外壳与共用 overlay 的简体中文文字。键与英文完全相同。 */
export const SHELL_ZH_CN = {
  "shell.nav.primary": "主要导航",
  "noun.portfolioLens": "Portfolio Lens",
  "noun.products": "Products",
  "noun.evidence": "Evidence",
  "noun.productLedger": "Product Ledger",
  "noun.productVault": "Product Vault",
  "shell.routePlaceholder": "此界面尚未实现，等待所属垂直切片通过验证后才会开放。",
  "shell.theme.switchToDark": "切换为深色主题",
  "shell.theme.switchToLight": "切换为浅色主题",
  "shell.theme.dark": "深色主题",
  "shell.theme.light": "浅色主题",
  "shell.nav.attention.one": "{route}，{count} 件需要注意",
  "shell.nav.attention.other": "{route}，{count} 件需要注意",

  "policyStrip.degraded": "降级模式",
  "policyStrip.evidenceVerificationPending": "证据验证待处理",
  "policyStrip.outOfSync": "未同步",
  "policyStrip.cancelling": "取消中",
  "policyStrip.backupDue": "备份已到期",

  "errorDetail.correlationId": "Correlation ID",
  "errorDetail.copy": "复制",
  "errorDetail.copied": "已复制",
  "errorDetail.retry": "重试",

  "textScale.title": "文字缩放",
  "textScale.sampleBody":
    "早上好，今天先处理真正影响结果的事。有 4 件事需要注意，最前面的是一条回复期限已过的 Action Request。",
  "textScale.sampleLabel": "排在这里：承诺已经落空",

  "h2b.confirmPrompt": "请输入「{phrase}」以批准",
  "h2b.approve": "批准",
  "h2b.reject": "拒绝",
  "h2b.approved": "已批准",
  "h2b.rejected": "已拒绝",
  "h2b.recoveryEvidence": "已验证的恢复证据",
  "h2b.verifiedAt": "验证时间",
  "h2b.scope": "范围",
  "h2b.compatibility": "兼容性",
  "h2b.transferDetails": "外部传输详情",
  "h2b.provider": "服务提供者",
  "h2b.account": "账号",
  "h2b.purpose": "用途",
  "h2b.exactPayload": "确切传输内容",
  "h2b.irreversible": "此操作一经批准即无法收回：上述内容将实际传送至外部服务，且无法追回或撤销。",
  "policyStrip.backingUp": "备份中",
} as const satisfies Record<keyof typeof SHELL_EN, string>;
