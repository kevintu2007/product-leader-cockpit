import type { RESTORE_EN } from "./restore.en";

/** Operational Restore와 S11 System Health의 한국어 문구. */
export const RESTORE_KO = {
  "restore.open": "백업에서 복원…",
  "restore.title": "백업에서 복원",
  "restore.cancel": "취소",
  "restore.choose.lede":
    "이 컴퓨터나 다른 컴퓨터에서 만든 Operational Backup을 선택하세요. 마지막 단계를 확인하기 전까지는 아무것도 바뀌지 않습니다.",
  "restore.choose.button": "백업 파일 선택…",
  "restore.choose.dialogTitle": "복원할 백업 선택",
  "restore.choose.filterName": "PMC 백업",
  "restore.file": "백업 파일: {name}",
  "restore.passphrase.label": "이 백업의 암호 문구",
  "restore.passphrase.check": "백업 확인",
  "restore.checking": "백업을 확인하는 중… 1분 정도 걸릴 수 있습니다.",
  "restore.backup": "{time}에 생성, Ledger schema {schema}, 레코드 {count}개.",
  "restore.recovery.running":
    "무엇이든 바꾸기 전에 PMC가 현재 작업 공간을 백업합니다. 1분 정도 걸릴 수 있습니다.",
  "restore.preview.lede":
    "복원이 하는 일은 정확히 아래와 같습니다. 확인하기 전에 무엇이든 바뀌면 이 복원은 취소됩니다.",
  "restore.preview.backup": "백업",
  "restore.preview.now": "현재",
  "restore.now": "레코드 {count}개, 마지막 변경 {time}.",
  "restore.now.noChange": "레코드 {count}개, 아직 변경 없음.",
  "restore.preview.replaced": "바뀌는 것",
  "restore.replaced":
    "Product Ledger와 백업에 담긴 설정: 언어, 시간대, 테마, 보존 기간, AI 및 로그 설정.",
  "restore.preview.kept": "바뀌지 않는 것",
  "restore.kept": "Product Vault, 백업 폴더와 암호 문구 설정, 그리고 다른 모든 백업.",
  "restore.preview.recovery": "복구용 백업",
  "restore.recovery.done": "현재 작업 공간을 {time}에 백업하고 검증했습니다.",
  "restore.needsUpgrade":
    "이 백업은 이전 버전의 PMC에서 만든 것입니다. 복원한 뒤 사용하기 전에 PMC가 업그레이드를 요청합니다.",
  "restore.confirm.label": "확인하려면 백업 생성 날짜를 입력하세요: {date}",
  "restore.confirm.button": "이 백업으로 바꾸기",
  "restore.reject": "복원하지 않기",
  "restore.replacing": "바꾸는 중… PMC를 닫지 마세요.",
  "restore.result.restored":
    "{time}의 백업을 복원했습니다. 이전 내용은 {recovery}의 백업으로 저장되어 있습니다.",
  "restore.result.unchanged": "복원하지 못했습니다. 아무것도 바뀌지 않았습니다.",
  "restore.result.putBack":
    "바꾸기 시작한 뒤 복원하지 못했습니다. PMC가 이전 작업 공간을 되돌려 놓았습니다.",
  "restore.result.recoveryFailed":
    "복원하지 못했고 PMC가 이전 작업 공간을 되돌려 놓지 못했습니다. System Health에서 복원할 복구용 백업을 알려 줍니다.",
  "restore.result.interrupted":
    "복원이 끝나기 전에 멈췄습니다. 지금은 더 이상 바뀌는 것이 없고, PMC를 다시 시작하면 이전 작업 공간을 되돌려 놓습니다.",
  "restore.continue": "계속",
  "restore.openSystemHealth": "System Health 열기",

  "policyStrip.restoring": "복원 중",
  "backup.strip.restoring": "복원 중입니다. 새 레코드와 변경은 복원이 끝난 뒤에 받습니다.",

  "health.headline": "PMC가 레코드를 읽고 보관할 수 있는지",
  "health.loading": "시스템 상태를 읽는 중입니다.",
  "health.unavailable": "지금은 시스템 상태를 읽을 수 없습니다. {message}",
  "health.ledger.label": "Product Ledger",
  "health.ledger.ready": "열려 있고 읽을 수 있습니다.",
  "health.ledger.replacing": "복원이 지금 바꾸는 중입니다.",
  "health.ledger.restore_recovery_required":
    "열지 않았습니다. 복원이 중간에 멈춰서, 지금 파일은 원래 Ledger도 백업도 아닐 수 있습니다.",
  "health.ledger.upgrade_required":
    "아직 열지 않았습니다. 이전 버전의 PMC에서 온 것이라 먼저 업그레이드해야 합니다.",
  "health.ledger.open_failed": "열 수 없었습니다.",
  "health.recoveryBackup":
    "백업 폴더에서 복구용 백업 {name}을(를) 복원하면 원래 상태로 돌아갈 수 있습니다.",
  "health.quit": "PMC 종료",

  "safeError.desktop.restore_running": "복원 중입니다. 끝난 후 다시 시도하세요.",
  "safeError.desktop.ledger_unavailable":
    "Product Ledger가 열려 있지 않습니다. 이유는 System Health에서 확인하세요.",
  "safeError.desktop.restore_file_unreadable": "이 백업 파일을 읽을 수 없습니다.",
  "safeError.desktop.restore_wrong_passphrase": "이 암호 문구로는 이 백업을 열 수 없습니다.",
  "safeError.desktop.restore_damaged": "이 백업은 손상되었습니다. 내용 확인에 실패했습니다.",
  "safeError.desktop.restore_newer_version":
    "이 백업은 더 새로운 버전의 PMC에서 만든 것입니다. 복원하려면 PMC를 업데이트하세요.",
  "safeError.desktop.restore_unsupported_old":
    "이 백업은 너무 오래된 버전의 PMC에서 만든 것이라 복원할 수 없습니다.",
  "safeError.desktop.restore_recovery_backup_failed":
    "현재 작업 공간을 백업하지 못해 아무것도 바꾸지 않았습니다.",
  "safeError.desktop.restore_preview_stale":
    "이 미리 보기는 더 이상 최신이 아닙니다. 백업을 다시 선택하세요.",
  "safeError.desktop.restore_confirmation_mismatch": "입력한 날짜가 백업 생성 날짜와 다릅니다.",
  "safeError.desktop.restore_failed_unchanged": "복원하지 못했습니다. 아무것도 바뀌지 않았습니다.",
  "safeError.desktop.restore_live_only": "복원은 Live 작업 공간에만 적용됩니다.",

  "upgrade.title": "이 작업 공간 업그레이드",
  "upgrade.body":
    "PMC {version}은(는) 더 새로운 형식으로 레코드를 저장합니다. 업그레이드는 금방 끝납니다. PMC가 먼저 작업 공간을 백업하며, 어느 단계든 실패하면 아무것도 바뀌지 않습니다.",
  "upgrade.fact.current": "현재 형식",
  "upgrade.fact.new": "새 형식",
  "upgrade.schema": "schema {schema}",
  "upgrade.fact.records": "레코드 수",
  "upgrade.fact.lastBackup": "마지막으로 검증된 백업",
  "upgrade.fact.noBackup": "없음",
  "upgrade.backupFirst": "먼저 백업: 백업 폴더와 암호 문구를 설정한 뒤 업그레이드하세요.",
  "upgrade.run": "업그레이드",
  "upgrade.backingUp": "작업 공간을 백업하는 중…",
  "upgrade.upgrading": "업그레이드하는 중…",
  "upgrade.result.upgraded":
    "작업 공간을 업그레이드했습니다. {time}의 백업에 이전 상태가 저장되어 있습니다.",
  "upgrade.result.rolledBack": "업그레이드를 완료하지 못했습니다. 아무것도 바뀌지 않았습니다.",
  "upgrade.result.unreadable":
    "작업 공간은 업그레이드되었지만 PMC가 열 수 없었습니다. {time}의 백업에 이전 상태가 저장되어 있습니다.",
  "upgrade.result.unknown":
    "PMC가 업그레이드 완료 여부를 알 수 없습니다. {time}의 백업에 이전 작업 공간이 저장되어 있습니다.",
  "upgrade.retry": "다시 시도",
  "upgrade.blocked.title": "이 작업 공간을 열 수 없습니다",
  "upgrade.newer":
    "이 작업 공간은 더 새로운 버전의 PMC에서 만들었습니다. 열려면 PMC를 업데이트하세요.",
  "upgrade.unsupportedOld": "이 작업 공간은 개발 버전의 PMC에서 만들어 업그레이드할 수 없습니다.",
  "health.ledger.unsupported_old":
    "열지 않았습니다. 개발 버전의 PMC에서 만들어 업그레이드할 수 없습니다.",
  "health.ledger.newer_version": "열지 않았습니다. 더 새로운 버전의 PMC에서 만들었습니다.",
  "safeError.desktop.upgrade_live_only": "업그레이드는 Live 작업 공간에만 적용됩니다.",
  "safeError.desktop.upgrade_not_required": "이 작업 공간은 업그레이드할 필요가 없습니다.",
  "safeError.desktop.upgrade_backup_failed":
    "업그레이드 전 백업에 실패해 업그레이드를 시작하지 않았습니다. 백업 폴더를 확인하고 다시 시도하세요.",
  "safeError.desktop.upgrade_failed_unchanged":
    "업그레이드를 완료하지 못했습니다. 아무것도 바뀌지 않았습니다.",
  "upgrade.trainingReset":
    "이 Training 작업 공간은 이전 버전의 PMC에서 온 것입니다. 업그레이드하지 않으니 샘플 작업 공간으로 초기화하세요.",
  "safeError.desktop.upgrade_outcome_unknown":
    "PMC가 업그레이드 완료 여부를 알 수 없습니다. 직전에 만든 백업에 이전 작업 공간이 저장되어 있습니다.",
  "restore.now.unavailable":
    "PMC가 현재 Product Ledger를 열 수 없어 기록 수와 마지막 변경 시각을 알 수 없습니다.",
  "restore.recovery.preserved":
    "PMC가 현재 Ledger 파일의 정확한 사본을 “{name}”(으)로 보존하고 다시 확인했습니다. 사본은 바이트 단위로 일치했습니다. 사용 가능한 Product Ledger인지는 확인할 수 없으므로 Operational Backup이 아니며, 마지막으로 검증된 백업에도 포함되지 않습니다.",
  "restore.preserve.running":
    "무엇이든 교체하기 전에 PMC가 현재 Ledger 파일의 정확한 사본을 보존하고 확인합니다. 1분 정도 걸릴 수 있습니다.",
  "restore.result.restoredPreserved":
    "{time}의 백업을 복원했습니다. 이전에 있던 파일은 “{name}”(으)로 보존되어 있습니다.",
  "restore.result.putBackPreserved":
    "교체를 시작한 뒤 복원에 실패했습니다. PMC가 이전 Ledger 파일을 그대로 되돌려 놓았습니다. Product Ledger는 여전히 열 수 없습니다. 다른 백업을 시도하거나 PMC를 종료하세요.",
  "health.restore.backupFirst":
    "복원하기 전에 PMC가 현재 Ledger 파일의 사본을 백업 폴더에 보존합니다. 먼저 백업 폴더와 복구 암호 문구를 설정하세요.",
  "restore.choose.recovery": "복구용 백업 “{name}” 사용",
  "restore.choose.other": "다른 백업 파일 선택…",
  "restore.result.sourceChanged":
    "미리보기 이후 현재 Ledger 파일이 바뀌어 아무것도 교체하지 않았습니다. 복원을 다시 시작해 지금 교체될 내용을 확인하세요.",
} as const satisfies Record<keyof typeof RESTORE_EN, string>;
