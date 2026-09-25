import type { PAGES_EN } from "./pages.en";

/** People, Reviews & Reports, Product Vault, Settings의 한국어 문구. 키는 영어판과 같습니다. */
export const PAGES_KO = {
  "people.kind.person": "개인",
  "people.kind.organization": "조직",
  "people.none": "없음",
  "people.headline": "누가 무엇을 맡고 있고, 누가 아직 응답을 기다리는지",
  "people.lede":
    "여기의 Stakeholder와 관계는 직접 만들고 관리하는 것이며 사용자 계정이 아닙니다. 응답을 기다리는 요청은 아직 약속이 아니며, 상대가 수락하면 Action Request가 Action이 됩니다.",
  "people.empty":
    "아직 Stakeholder가 없습니다. Stakeholder를 만들면 맡은 일과 의존하는 일이 여기에 표시됩니다.",
  "people.caption":
    "식별자순으로 정렬했습니다. 전체 {total}명 중 {from}~{to}번째를 표시하고 있습니다.",
  "people.column.stakeholder": "Stakeholder",
  "people.column.kind": "유형",
  "people.column.responsible": "담당",
  "people.column.dependsOn": "의존",
  "people.column.waiting": "응답 대기 중인 요청",
  "people.column.classification": "등급",
  "people.previous": "이전 페이지",
  "people.next": "다음 페이지",

  "reviews.route": "Review에 필요한 데이터",
  "reviews.headline": "Review 전에 아직 열려 있는 항목 확인하기",
  "reviews.lede":
    "Work Queue에서 현재 표시된 항목을 Executive Cockpit의 주의 순서대로 정렬했습니다. 처리하려면 Work Queue로 이동하세요.",
  "reviews.period": "검토 기간",
  "reviews.noPeriod": "사용할 수 있는 검토 기간이 없습니다: {reason}.",
  "reviews.notYet": "이 버전에서는 아직 Fact Pack을 만들거나 보고서를 승인할 수 없습니다.",
  "reviews.open": "Work Queue에서 표시된 항목",
  "reviews.openNone": "지금은 Work Queue에서 표시된 항목이 없습니다.",
  "reviews.count.one": "총 {count}건.",
  "reviews.count.other": "총 {count}건.",
  "reviews.goToQueue": "Work Queue에서 처리",

  "vault.reason.notConfigured": "이 작업 공간에는 Product Vault가 설정되어 있지 않습니다.",
  "vault.reason.invalidRoot": "Product Vault 위치가 없거나, 폴더가 아니거나, 링크입니다.",
  "vault.reason.unknown": "원인을 알 수 없습니다.",
  "vault.pausedBecause":
    "{reason} 지문 고정과 재관측을 일시 중지합니다. Evidence 연결에는 영향이 없습니다.",
  "vault.headline": "Evidence 파일과 현재 상태",
  "vault.lede":
    "파일은 사용자의 Product Vault에 있고, Ledger에는 각 Evidence의 용도, 지문, 검증 결과, 등급이 기록됩니다.",
  "vault.available": "Vault를 사용할 수 있습니다",
  "vault.unavailable": "Vault를 사용할 수 없습니다",
  "vault.availableHint": "Product의 Evidence 탭에서 지문을 고정하거나 다시 관측할 수 있습니다.",
  "vault.count": "총 {count}건, 식별자순으로 정렬했습니다.",
  "vault.empty":
    "Ledger에 아직 Evidence가 없습니다. Product의 Evidence 탭에서 연결하면 여기에 표시됩니다.",
  "vault.column.evidence": "Evidence",
  "vault.column.role": "용도",
  "vault.column.verification": "검증 상태",
  "vault.column.fingerprint": "지문",
  "vault.column.classification": "등급",
  "vault.column.version": "버전",
  "vault.roleUnset": "지정되지 않음",
  "vault.verificationOn": "{verification}({date})",
  "vault.pinned": "고정됨",
  "vault.notPinned": "고정되지 않음",

  "settings.headline": "표시 방식과 이 작업 공간의 상태",
  "settings.lede":
    "글자 크기는 이 컴퓨터의 화면에만 적용되며 Windows 배율 설정과는 별개입니다. 테마는 오른쪽 위 버튼으로 전환합니다.",
  "settings.textSize": "글자 크기",
  "settings.language": "언어",
  "settings.languageSystem": "시스템 언어({language})",
  "settings.languageFailed": "언어를 저장하지 못했습니다. {message}",
  "settings.sampleBody":
    "좋은 아침입니다. 오늘은 성과에 실제로 영향을 주는 일부터 시작하겠습니다. Demo Product Atlas에는 날짜가 지난 마일스톤이 있으며, KPI 2개 중 2개에 관측값이 있습니다.",
  "settings.sampleLabel": "마일스톤: 날짜 지남, 가장 이른 날짜 2026‑08‑15",
  "settings.dataSources": "데이터 원본",
  "settings.loading": "Ledger와 Vault 상태를 읽는 중입니다.",
  "settings.unavailable": "지금은 Ledger 또는 Vault 상태를 읽을 수 없습니다. {message}",
  "settings.ledgerReadable": "읽을 수 있습니다. 스키마 버전 {schema}, Ledger 버전 {revision}.",
  "settings.vaultNotSet": "설정되지 않음",
  "vaultRoot.open.choose": "Vault 폴더 선택…",
  "vaultRoot.open.change": "Vault 폴더 변경…",
  "vault.reason.changeUnresolved":
    "Vault 폴더 변경이 중단되어 완료하지 못했습니다. PMC를 다시 시작해 완료하세요.",
  "safeError.desktop.vault_change_unresolved":
    "Vault 폴더 변경이 중단되어 완료하지 못해 지금은 어떤 Vault도 사용하지 않습니다. PMC를 다시 시작해 완료하세요.",
  "vaultRoot.title": "Product Vault 폴더",
  "vaultRoot.cancel": "취소",
  "vaultRoot.choose.lede":
    "이 작업 공간이 Evidence를 읽을 폴더를 선택하세요. 변경하기 전에 PMC가 이 작업 공간을 백업하고 폴더 아래의 모든 Evidence 파일을 확인합니다.",
  "vaultRoot.backupFirst":
    "Vault를 변경하기 전에 PMC가 복원할 수 있도록 이 작업 공간을 백업합니다. 먼저 백업 폴더와 복구 암호 문구를 설정하세요.",
  "vaultRoot.choose.button": "Vault 폴더 선택…",
  "vaultRoot.choose.dialogTitle": "Product Vault 폴더 선택",
  "vaultRoot.folder": "폴더: {name}",
  "vaultRoot.preparing":
    "이 작업 공간을 백업한 뒤 새 폴더 아래의 모든 Evidence 파일을 확인하는 중입니다… 1분 정도 걸릴 수 있습니다.",
  "vaultRoot.preview.lede":
    "바뀌는 내용은 다음과 같습니다. Vault와 Ledger 안의 내용은 옮겨지지 않습니다.",
  "vaultRoot.preview.now": "현재",
  "vaultRoot.preview.new": "새 폴더",
  "vaultRoot.preview.evidence": "Evidence",
  "vaultRoot.preview.noEvidence":
    "현재 Vault를 참조하는 Evidence가 없어 다시 연결할 것이 없습니다.",
  "vaultRoot.preview.resolved": "새 폴더에서 {count}개 참조 중 {resolved}개가 같은 내용입니다.",
  "vaultRoot.preview.recovery": "복구 증거",
  "vaultRoot.preview.recoveryVerified": "{time}에 백업하고 검증함",
  "vaultRoot.confirm.phrase": "VAULT 변경",
  "vaultRoot.confirm.label": '확인하려면 "{phrase} {code}"를 입력하세요',
  "vaultRoot.confirm.button": "이 폴더 사용",
  "vaultRoot.reject": "변경하지 않음",
  "vaultRoot.changing": "변경하는 중입니다… PMC를 닫지 마세요.",
  "vaultRoot.result.changed": '이제 이 작업 공간은 "{name}"에서 Evidence를 읽습니다.',
  "vaultRoot.result.notChanged": "아무것도 바뀌지 않았습니다.",
  "vaultRoot.done": "완료",
  "safeError.desktop.vault_change_active":
    "다른 Vault 폴더 변경이 이미 진행 중입니다. 먼저 완료하거나 취소하세요.",
  "safeError.desktop.vault_not_live": "Live 작업 공간의 Vault 폴더만 변경할 수 있습니다.",
  "safeError.desktop.vault_folder_unusable":
    "이 폴더는 Vault로 쓸 수 없습니다. 읽을 수 있는 기존 폴더를 선택하고 바로 가기나 링크는 선택하지 마세요.",
  "safeError.desktop.vault_folder_unchanged": "이 폴더는 이미 이 작업 공간의 Vault입니다.",
  "safeError.desktop.vault_folder_in_use":
    "PMC가 이미 사용하는 폴더입니다. PMC 자체 폴더가 아니고 백업 폴더를 포함하거나 그 안에 있지 않은 폴더를 선택하세요.",
  "safeError.desktop.vault_recovery_backup_stale":
    "이번 변경을 위해 만든 백업이 더 이상 작업 공간과 일치하지 않습니다. 다시 시도하세요.",
  "safeError.desktop.vault_evidence_unpinned":
    "Evidence 참조 {count}개에 고정된 지문이 없어 새 폴더의 파일이 같은지 증명할 수 없습니다. 먼저 고정하세요(Product → Evidence).",
  "safeError.desktop.vault_evidence_unresolved":
    "Evidence 참조 {count}개가 새 폴더에서 같은 내용으로 확인되지 않습니다. 먼저 해당 파일을 옮기거나 복원하세요.",
  "safeError.desktop.vault_preview_stale": "이 미리 보기는 오래되었습니다. 폴더를 다시 선택하세요.",
  "safeError.desktop.vault_confirmation_mismatch": "코드가 이 미리 보기와 일치하지 않습니다.",
  "safeError.desktop.vault_changed_but_not_recorded":
    "폴더는 변경되었지만 PMC가 이 변경을 감사 로그에 기록하지 못했습니다.",
  "safeError.desktop.vault_change_not_recorded":
    "PMC가 이 단계를 기록하지 못해 아무것도 바뀌지 않았습니다. 다시 시도하세요.",
  "safeError.desktop.vault_change_failed":
    "Vault 폴더를 변경할 수 없습니다. 아무것도 바뀌지 않았습니다. 다시 시도하세요.",
  "evidenceFile.open": "파일에서 Evidence 추가…",
  "evidenceFile.title": "파일에서 Evidence 추가",
  "evidenceFile.lede":
    "Vault 폴더 안의 파일을 선택하세요. PMC는 위치와 지문을 기록하며, 파일은 그대로 둡니다.",
  "evidenceFile.choose": "파일 선택…",
  "evidenceFile.chooseAnother": "다른 파일 선택…",
  "evidenceFile.dialogTitle": "Product Vault 안의 파일 선택",
  "evidenceFile.file": "파일: {name}",
  "evidenceFile.observed": "{time}에 관측",
  "evidenceFile.changed": "선택한 뒤 파일이 변경되었습니다.",
  "evidenceFile.existing": "Evidence {id}가 이미 이 파일을 참조하므로 새로 만들지 않습니다.",
  "evidenceFile.existingLinked":
    "Evidence {id}가 이미 이 파일을 참조하고 있으며 이 Product에도 연결되어 있습니다.",
  "evidenceFile.linkExisting": "이 Product에 연결",
  "evidenceFile.sameContent": "Evidence {id}의 내용이 같습니다.",
  "evidenceFile.create": "만들기",
  "evidenceFile.createAndLink": "만들고 이 Product에 연결",
  "evidenceFile.cancel": "취소",
  "evidenceFile.creating": "만드는 중…",
  "evidenceFile.linking": "연결하는 중…",
  "evidenceFile.created": "Evidence {id}를 만들었습니다.",
  "evidenceFile.createdAndLinked": "Evidence {id}를 만들고 {product}에 연결했습니다.",
  "evidenceFile.linkedExisting": "Evidence {id}를 {product}에 연결했습니다.",
  "evidenceFile.createdNotLinked": "Evidence {id}를 만들었지만 아직 연결되지 않았습니다.",
  "evidenceFile.retryLink": "지금 연결",
  "evidenceFile.done": "완료",
  "safeError.desktop.evidence_file_outside_vault": "Vault 폴더 안의 파일을 선택하세요.",
  "safeError.desktop.evidence_file_unreadable": "파일을 읽을 수 없습니다.",
  "safeError.desktop.evidence_file_choice_stale":
    "이 선택은 오래되었습니다. 파일을 다시 선택하세요.",
  "safeError.desktop.sample_backup_refused": "샘플 작업 공간은 백업하지 않습니다.",
  "safeError.desktop.workspace_not_chosen": "먼저 시작 방법을 선택하세요.",
  "safeError.desktop.sample_foreign": "샘플 폴더에 PMC가 쓰지 않은 항목이 있어 그대로 두었습니다.",
  "safeError.desktop.sample_is_open":
    "먼저 내 작업 공간으로 전환하세요. 샘플 데이터는 열려 있는 동안 변경할 수 없습니다.",
  "safeError.desktop.sample_operation_in_progress":
    "샘플 데이터의 다른 변경이 아직 끝나지 않았습니다. 잠시 후 다시 시도하세요.",
  "safeError.desktop.sample_nothing_to_delete": "삭제할 샘플 데이터가 없습니다.",
  "safeError.desktop.sample_delete_not_prepared":
    "이 삭제는 더 이상 대기 중이 아닙니다. 다시 시작하세요.",
  "safeError.desktop.sample_delete_expired":
    "이 미리 보기는 만료되었습니다. 아무것도 삭제되지 않았습니다. 다시 시작하세요.",
  "safeError.desktop.sample_delete_changed":
    "미리 보기 이후 샘플 데이터가 바뀌었습니다. 아무것도 삭제되지 않았습니다. 다시 시작하세요.",
  "safeError.desktop.sample_confirmation_mismatch":
    "입력한 문구가 일치하지 않습니다. 아무것도 삭제되지 않았습니다.",
  "safeError.desktop.sample_not_recorded":
    "PMC가 이 단계를 감사 로그에 기록하지 못했습니다. 다시 시도하세요.",
  "safeError.desktop.sample_failed":
    "샘플 데이터를 준비하지 못했습니다. 내 작업 공간에는 영향이 없습니다. 다시 시도하세요.",
  "safeError.desktop.workspace_not_first_run": "이 선택은 처음 실행할 때 한 번만 합니다.",
  "safeError.desktop.workspace_choice_not_saved":
    "선택을 저장하지 못해 PMC가 다시 시작하지 않았습니다. 다시 시도하세요.",
  "safeError.desktop.sample_reset_failed":
    "샘플 데이터를 재설정하지 못했습니다. PMC가 지금 또는 다음 시작 때 원래대로 되돌립니다.",
  "safeError.desktop.sample_delete_failed":
    "샘플 데이터가 삭제되지 않았거나 완전히 삭제되지 않았습니다. PMC가 다음 시작 때 다시 확인합니다.",
  "safeError.desktop.sample_unavailable":
    "지금은 샘플 데이터를 읽을 수 없습니다. 아무것도 변경되지 않았습니다.",
  "firstRun.title": "시작 방법을 선택하세요",
  "firstRun.lede": "하나를 골라 시작하세요. 나중에 설정 → 작업 공간에서 바꿀 수 있습니다.",
  "firstRun.live.title": "내 작업 공간으로 시작",
  "firstRun.live.body": "빈 작업 공간이 열립니다. 기록은 이 컴퓨터에만 저장됩니다.",
  "firstRun.live.setup":
    "그다음 설정에서 백업 폴더, 복구 암호 문구, Vault 폴더 순서로 설정하세요. 암호 문구가 없으면 백업을 복원할 수 없습니다.",
  "firstRun.sample.title": "샘플 데이터로 배우기",
  "firstRun.sample.body":
    "학습용 합성 데이터입니다. 내 작업과 분리되어 있으며 언제든지 초기화하거나 삭제할 수 있습니다.",
  "firstRun.preparing": "샘플 데이터를 준비하는 중… 1분 정도 걸릴 수 있습니다.",
  "firstRun.saving": "선택을 저장하는 중…",
  "firstRun.tryAgain": "아직 선택되지 않았습니다. 준비되면 다시 선택하세요.",
  "workspace.title": "작업 공간",
  "workspace.openLive": "내 작업 공간이 열려 있습니다.",
  "workspace.openSample":
    "샘플 데이터가 열려 있습니다. 합성 데이터이며 내 작업과 분리되어 있습니다.",
  "workspace.sampleFellBack":
    "샘플 데이터를 선택했지만 열 수 없어서 내 작업 공간을 열었습니다. 이유는 시스템 상태에서 확인할 수 있습니다.",
  "workspace.switchToSample": "샘플 데이터로 전환",
  "workspace.switchToLive": "내 작업 공간으로 전환",
  "workspace.switchToSample.confirm":
    "PMC가 다시 시작되고 샘플 데이터를 엽니다. 내 작업 공간은 바뀌지 않습니다.",
  "workspace.switchToLive.confirm": "PMC가 다시 시작되고 내 작업 공간을 엽니다.",
  "workspace.switchAndRestart": "전환하고 다시 시작",
  "workspace.restarting": "PMC를 다시 시작하는 중…",
  "sampleWorkspace.badge": "샘플 작업 공간",
  "sampleWorkspace.badge.open": "설정 → 작업 공간 열기",
  "sampleWorkspace.manage": "샘플 데이터",
  "sampleWorkspace.fromLiveOnly":
    "초기화와 삭제는 내 작업 공간에서 할 수 있습니다. 먼저 전환하세요.",
  "sampleWorkspace.cancel": "취소",
  "sampleWorkspace.close": "닫기",
  "sampleWorkspace.done": "완료",
  "sampleWorkspace.reset.open": "샘플 데이터 초기화…",
  "sampleWorkspace.reset": "샘플 데이터 초기화",
  "sampleWorkspace.reset.confirm":
    "샘플 데이터가 처음 상태로 돌아갑니다. 내 작업 공간에는 영향이 없습니다.",
  "sampleWorkspace.resetting": "샘플 데이터를 초기화하는 중… 1분 정도 걸릴 수 있습니다.",
  "sampleWorkspace.reset.done": "샘플 데이터가 처음 상태로 돌아갔습니다.",
  "sampleWorkspace.delete.open": "샘플 데이터 삭제…",
  "sampleWorkspace.delete.title": "샘플 데이터 삭제",
  "sampleWorkspace.delete.preparing": "샘플 데이터의 내용을 읽는 중…",
  "sampleWorkspace.delete.lede":
    "삭제로 제거되는 내용은 다음과 같습니다. 확인하기 전에 무엇이든 바뀌면 이 삭제는 취소됩니다.",
  "sampleWorkspace.delete.what": "샘플 데이터",
  "sampleWorkspace.delete.seed": "{seedId}, 버전 {version}",
  "sampleWorkspace.delete.unknown":
    "이 미리 보기는 이 화면에서 보여 줄 수 없는 변경을 설명하므로 여기서 승인할 수 없습니다. 아무것도 삭제되지 않았습니다.",
  "sampleWorkspace.delete.parts": "제거되는 내용",
  "sampleWorkspace.delete.part.ledger": "샘플의 Product Ledger",
  "sampleWorkspace.delete.part.vault": "샘플의 합성 Vault",
  "sampleWorkspace.delete.part.generated": "생성된 파일",
  "sampleWorkspace.delete.effect": "영향",
  "sampleWorkspace.delete.effectValue":
    "되돌릴 수 없습니다. 내 작업 공간, 설정, 백업에는 영향이 없습니다.",
  "sampleWorkspace.delete.expires": "미리 보기 유효 기한",
  "sampleWorkspace.delete.settingsRevision": "설정 리비전",
  "sampleWorkspace.delete.inventory": "내용 다이제스트",
  "sampleWorkspace.delete.payload": "미리 보기 다이제스트",
  "sampleWorkspace.delete.confirmLabel": "확인하려면 {phrase}을(를) 입력하세요",
  "sampleWorkspace.delete.phrase": "샘플 데이터 삭제",
  "sampleWorkspace.delete.confirm": "샘플 데이터 삭제",
  "sampleWorkspace.delete.reject": "유지",
  "sampleWorkspace.delete.deleting": "샘플 데이터를 삭제하는 중…",
  "sampleWorkspace.delete.deleted":
    "샘플 데이터를 삭제했습니다. 샘플 데이터로 전환하면 다시 준비됩니다.",
  "sampleWorkspace.delete.notDeleted":
    "샘플 데이터는 삭제되지 않았습니다. 아무것도 바뀌지 않았습니다.",
  "health.ledger.first_run": "아직 열리지 않았습니다. 먼저 시작 방법을 선택하세요.",
  "health.settings.setAside":
    "설정을 읽을 수 없어서 PMC가 기존 설정을 따로 보관하고 기본 설정으로 시작했습니다.",
  "health.settings.unavailable": "지금은 설정을 읽거나 저장할 수 없습니다.",
  "health.sample.unresolved":
    "샘플 데이터에 대한 변경을 끝내지 못해 샘플 데이터를 사용할 수 없습니다.",
  "health.sample.missing": "샘플 데이터를 선택했지만 없어서 내 작업 공간을 열었습니다.",
  "health.sample.foreign":
    "샘플 폴더에 PMC가 쓰지 않은 항목이 있어 그대로 두었습니다. 샘플 데이터를 사용할 수 없습니다.",
  "health.sample.cleanupPending":
    "제거를 기다리는 샘플 폴더가 {count}개 남아 있습니다. PMC가 이후 시작할 때 제거합니다.",
  "gettingStarted.title": "시작하기",
  "gettingStarted.lede":
    "이 순서대로 작업 공간을 설정하세요. 각 단계는 PMC가 확인할 수 있는 상태로 판단하며, 모두 끝나면 이 목록은 사라집니다.",
  "gettingStarted.backupFolder": "백업 폴더 선택",
  "gettingStarted.backupFolder.why": "백업은 선택한 폴더에 저장됩니다. 다른 드라이브를 권장합니다.",
  "gettingStarted.passphrase": "복구 암호 문구 설정",
  "gettingStarted.passphrase.why":
    "백업은 이것으로 암호화됩니다. 이것이 없으면 백업을 복원할 수 없고, 잃어버린 암호 문구는 PMC가 되찾을 수 없습니다.",
  "gettingStarted.firstBackup": "첫 백업 만들기",
  "gettingStarted.firstBackup.why": "백업이 검증된 뒤에야 PMC가 새 기록과 변경을 받습니다.",
  "gettingStarted.vault": "Product Vault 폴더 선택",
  "gettingStarted.vault.why":
    "Evidence 파일을 두는 폴더입니다. PMC는 각 파일의 위치와 지문만 기록하며 파일을 복사하지 않습니다.",
  "gettingStarted.firstProduct": "첫 Product 추가",
  "gettingStarted.firstProduct.why":
    "Portfolio에서 “새 Product…”로 추가합니다. Cockpit이 Portfolio Lens에 배치합니다.",
  "gettingStarted.status.done": "완료",
  "gettingStarted.status.next": "다음",
  "gettingStarted.status.waits": "위 단계 완료 대기",
  "gettingStarted.status.unknown": "PMC가 아직 판단할 수 없음",
  "gettingStarted.openSettings": "설정 열기",
  "gettingStarted.openPortfolio": "Portfolio 열기",
  "settings.vaultAvailable": "사용 가능",
  "settings.vaultUnavailable": "사용 불가",
  "backup.headline": "백업",
  "backup.loading": "백업 상태를 읽는 중입니다.",
  "backup.dueLede":
    "백업할 때가 되었습니다. 백업이 검증될 때까지 이 작업 공간에서는 기록을 추가하거나 바꿀 수 없습니다.",
  "backup.folder.label": "백업 폴더",
  "backup.folder.notSet": "설정 안 됨",
  "backup.folder.set": "설정됨",
  "backup.folder.available": "사용 가능",
  "backup.folder.unavailable": "사용할 수 없음: 드라이브를 연결하거나 다른 폴더를 선택하세요",
  "backup.folder.choose": "폴더 선택…",
  "backup.folder.dialogTitle": "PMC가 백업을 보관할 폴더 선택",
  "backup.passphrase.label": "복구 암호 문구",
  "backup.passphrase.notSet": "설정 안 됨",
  "backup.passphrase.session": "이번 세션에 설정됨",
  "backup.passphrase.remembered": "이 Windows 계정에 저장됨",
  "backup.passphrase.setUp": "암호 문구 설정…",
  "backup.last.label": "마지막 백업",
  "backup.last.none": "아직 백업이 없습니다",
  "backup.last.at": "{time}에 검증됨. 다음 백업 기한은 {next}입니다.",
  "backup.orphans":
    "이 폴더에 PMC 기록에 없는 백업 파일이 {count}개 있으며, 개수에 포함하지 않습니다.",
  "backup.run": "지금 백업",
  "backup.running": "백업 중… 1분 정도 걸릴 수 있습니다.",
  "backup.done": "{time}에 백업하고 검증했습니다.",
  "backup.openBackups": "백업 열기",
  "backup.strip.due": "먼저 백업하면 다시 기록을 추가하거나 바꿀 수 있습니다.",
  "backup.strip.running": "백업 중… 끝날 때까지 기록 추가와 변경은 기다려 주세요.",
  "backup.passphrase.title": "복구 암호 문구 설정",
  "backup.passphrase.generatedLede":
    "이 암호 문구는 한 번만 표시됩니다. 적어 두거나 암호 관리자에 보관한 다음, 아래에 다시 입력하세요.",
  "backup.passphrase.generating": "암호 문구를 만드는 중입니다.",
  "backup.passphrase.retype": "그대로 입력",
  "backup.passphrase.useOwn": "직접 정한 암호 문구 사용",
  "backup.passphrase.useGenerated": "생성된 암호 문구 사용",
  "backup.passphrase.ownRule":
    "20자 이상 또는 여섯 단어 이상이어야 하며, 흔한 비밀번호로만 만들면 안 됩니다.",
  "backup.passphrase.own": "암호 문구",
  "backup.passphrase.ownAgain": "암호 문구 다시 입력",
  "backup.passphrase.acknowledge":
    "PMC는 이 암호 문구를 복구할 수 없습니다. 이 문구가 없으면 이 컴퓨터에서도 다른 컴퓨터에서도 이 백업을 복원할 수 없습니다.",
  "backup.passphrase.remember": "자동 백업을 위해 이 Windows 계정에 저장합니다.",
  "backup.passphrase.confirm": "이 암호 문구 사용",
  "backup.passphrase.cancel": "취소",
} as const satisfies Record<keyof typeof PAGES_EN, string>;
