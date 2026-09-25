import type { SHELL_EN } from "./shell.en";

/** 한국어 셸과 공통 오버레이 문구. 키는 영어판과 같습니다. */
export const SHELL_KO = {
  "shell.nav.primary": "주 탐색",
  "noun.portfolioLens": "Portfolio Lens",
  "noun.products": "Products",
  "noun.evidence": "Evidence",
  "noun.productLedger": "Product Ledger",
  "noun.productVault": "Product Vault",
  "shell.routePlaceholder":
    "이 화면은 아직 사용할 수 없습니다. 담당 부분의 검증을 통과하면 사용할 수 있습니다.",
  "shell.theme.switchToDark": "어두운 테마로 전환",
  "shell.theme.switchToLight": "밝은 테마로 전환",
  "shell.theme.dark": "어두운 테마",
  "shell.theme.light": "밝은 테마",
  "shell.nav.attention.one": "{route}, 주의 필요 {count}건",
  "shell.nav.attention.other": "{route}, 주의 필요 {count}건",

  "policyStrip.degraded": "기능 제한 모드",
  "policyStrip.evidenceVerificationPending": "Evidence 검증 대기 중",
  "policyStrip.outOfSync": "동기화되지 않음",
  "policyStrip.cancelling": "취소하는 중",
  "policyStrip.backupDue": "백업할 때",

  "errorDetail.correlationId": "Correlation ID",
  "errorDetail.copy": "복사",
  "errorDetail.copied": "복사했습니다",
  "errorDetail.retry": "다시 시도",

  "textScale.title": "글자 크기",
  "textScale.sampleBody":
    "좋은 아침입니다. 오늘은 성과에 실제로 영향을 주는 일부터 시작하겠습니다. 주의가 필요한 항목은 4건이며, 맨 앞은 응답 기한이 지난 Action Request입니다.",
  "textScale.sampleLabel": "여기 있는 이유: 약속이 지켜지지 않았습니다",

  "h2b.confirmPrompt": "승인하려면 “{phrase}”를 입력하세요",
  "h2b.approve": "승인",
  "h2b.reject": "거부",
  "h2b.approved": "승인했습니다",
  "h2b.rejected": "거부했습니다",
  "h2b.recoveryEvidence": "검증된 복구 Evidence",
  "h2b.verifiedAt": "검증 일시",
  "h2b.scope": "범위",
  "h2b.compatibility": "호환성",
  "h2b.transferDetails": "외부 전송 세부 정보",
  "h2b.provider": "서비스 제공자",
  "h2b.account": "계정",
  "h2b.purpose": "목적",
  "h2b.exactPayload": "전송될 내용",
  "h2b.irreversible":
    "승인하면 되돌릴 수 없습니다. 위 내용은 외부 서비스로 실제로 전송되며, 회수하거나 되돌릴 수 없습니다.",
  "policyStrip.backingUp": "백업 중",
} as const satisfies Record<keyof typeof SHELL_EN, string>;
