import type { INSPECTOR_EN } from "./inspector.en";

/** O01 Product 인스펙터의 한국어 문구. 키는 영어판과 같습니다. */
export const INSPECTOR_KO = {
  "inspector.route": "Product 세부 정보",
  "inspector.outOfSync":
    "Product 세부 정보의 두 원본이 서로 다른 Ledger 리비전을 읽어 아무것도 표시하지 않았습니다. 두 시점을 이어 붙이면 그럴듯하지만 틀린 화면이 됩니다.",
  "inspector.healthEvidence": "연결된 Evidence: {label}. 검증 상태: {verification}.",
  "inspector.healthRecord": "{kind} “{label}”: {reason}.",

  "inspector.pinConfirm":
    "{id}의 지문을 고정할까요? 이 Evidence가 지금 가리키는 파일을 읽고, 그 시점의 바이트 요약을 식별 정보로 기록합니다. 고정은 되돌릴 수 없습니다. 이후 내용이 같으면 “재관측”, 파일을 옮기면 “이동”, 내용이 바뀌면 “대체”가 되며 이 지문은 다시 쓰이지 않습니다.",
  "inspector.reobserveConfirm":
    "{id} 항목을 다시 관측할까요? 이 Evidence에 기록된 위치의 파일을 읽고, 관측 결과가 저장된 상태와 다를 때만 씁니다.",
  "inspector.confirmPin": "고정 확인",
  "inspector.confirmReobserve": "재관측 확인",
  "inspector.cancel": "취소",
  "inspector.sending": "보내는 중…",
  "inspector.written": "기록했습니다. 검증 상태는 이제 “{verification}”입니다.",
  "inspector.unchanged":
    "변화가 없습니다. 관측 결과가 저장된 상태와 같아 Ledger에 쓰지 않았습니다.",
  "inspector.close": "닫기",
  "inspector.pinFailed": "고정이 완료되지 않았습니다. {message}",
  "inspector.reobserveFailed": "재관측이 완료되지 않았습니다. {message}",
  "inspector.abandon": "이 작업 그만두기",
  "inspector.pin": "지문 고정",
  "inspector.reobserve": "재관측",

  "inspector.link": "Evidence를 이 Product에 연결",
  "inspector.linkLoading": "Evidence를 읽는 중…",
  "inspector.linkChoose": "{product}에 연결할 Evidence",
  "inspector.linkNone": "(연결할 수 있는 Evidence가 없습니다)",
  "inspector.linkCandidate": "{id}({verification}, {classification}, 버전 {version})",
  "inspector.linkConfirm": "연결 확인",
  "inspector.linked": "연결했습니다: {id}. 연결 시 등급: {classification}.",
  "inspector.linkFailed": "연결이 완료되지 않았습니다. {message}",

  "inspector.classification": "등급",
  "inspector.version": "버전",
  "inspector.fold":
    "표시 중인 {kind} {id}의 등급 때문에 이 인스펙터의 등급은 {classification}입니다.",
  "inspector.happened": "일어난 일",
  "inspector.nothingHappened": "지금은 주의가 필요한 상황이 없습니다.",
  "inspector.conditionLine": "{condition} {provenance}",
  "inspector.provenance": "출처: {owner} {id}, 버전 {version}",
  "inspector.impact": "영향",
  "inspector.impactUnassessed": "아직 아무도 평가하지 않았습니다",
  "inspector.tabs": "Product 세부 정보",
  "inspector.tab.structure": "구조",
  "inspector.tab.evidence": "Evidence",
  "inspector.tab.people": "사람",
  "inspector.structureNone": "Ledger에 이 Product와 관련된 구조가 없습니다.",
  "inspector.structureEntry": "{kind}: {label}({classification})",
  "inspector.structureEntryVia": "{kind}: {label}({classification}), {via}",
  "inspector.via": "{project} 경유",
  "inspector.vaultNotConfigured":
    "이 작업 공간에는 사용할 수 있는 Product Vault가 없습니다(설정되지 않음). 파일을 읽는 작업(지문 고정, 재관측)은 사용할 수 없습니다. Evidence 연결에는 영향이 없습니다.",
  "inspector.vaultUnavailable":
    "지금은 Product Vault를 읽을 수 없습니다(폴더가 없거나, 폴더가 아니거나, 링크입니다). 파일을 읽는 작업(지문 고정, 재관측)은 일시 중지되었습니다. Evidence 연결에는 영향이 없습니다.",
  "inspector.evidenceNone": "이 Product에 직접 연결된 Evidence가 없습니다.",
  "inspector.evidenceLine": "{id}: {verification} {classifications}",
  "inspector.evidenceLineUnpinned": "{id}: {verification} {unpinned} {classifications}",
  "inspector.unpinned": "(지문 고정 안 됨)",
  "inspector.evidenceClassifications": "(Evidence 등급 {classification}, 연결 시 등급 {atLink})",
  "inspector.peopleNone": "Ledger에는 이 Product를 맡거나 이 Product에 의존하는 사람이 없습니다.",
  "inspector.person.one": "{name}({purpose}, 다른 Product {count}개에도 관여)",
  "inspector.person.other": "{name}({purpose}, 다른 Product {count}개에도 관여)",
  "inspector.purpose.responsibility": "담당",
  "inspector.purpose.dependency": "의존",
  "inspector.carriedHeading":
    "이 사람이 지금 맡고 있는 일(담당자로서이며, 이 Product에 속한 것이 아닙니다)",
  "inspector.carriedNone": "지금 맡고 있는 작업 항목이 없습니다.",
  "inspector.carriedLine": "{kind} {label}({state}){attention} {intents}",
  "inspector.nextSteps": "수명 주기상 가능한 다음 단계: {intents}",
  "inspector.noNextSteps": "없음",
} as const satisfies Record<keyof typeof INSPECTOR_EN, string>;
