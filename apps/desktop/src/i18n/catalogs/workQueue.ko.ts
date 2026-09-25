import type { WORK_QUEUE_EN } from "./workQueue.en";

/** Work Queue의 한국어 문구. 키는 영어판과 같습니다. */
export const WORK_QUEUE_KO = {
  "workQueue.caption.one":
    "승인된 정렬 정책을 따릅니다. 전체 {count}건 중 {from}~{to}번째를 표시하고 있습니다.",
  "workQueue.caption.other":
    "승인된 정렬 정책을 따릅니다. 전체 {count}건 중 {from}~{to}번째를 표시하고 있습니다.",

  "wq.offer.prepareAccept": "수락 준비",
  "wq.offer.decline": "거절",
  "wq.offer.withdraw": "철회",
  "wq.offer.start": "시작",
  "wq.offer.link": "Evidence 연결",
  "wq.offer.prepareComplete": "완료 준비",
  "wq.offer.prepareCancel": "취소 준비",
  "wq.offer.prepareReopen": "다시 열기 준비",
  "wq.offer.prepareResolve": "해결 준비",
  "wq.offer.prepareOccurrence": "발생 기록 준비",
  "wq.offer.prepareClose": "종료 준비",

  "wq.noDeadline": "기한 기록 없음",
  "wq.noResponseDeadline": "응답 기한 없음",

  "wq.review.accept.title": "승인 후 실행: {label} 수락",
  "wq.review.accept.summary":
    "승인하면 확인한 요약으로 Action이 만들어지고 이 Request에 연결됩니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.accept.approve": "승인하고 수락",
  "wq.review.complete.title": "승인 후 실행: {label} 완료로 표시",
  "wq.review.complete.summary":
    "승인하면 이 Action은 미리 보기의 Evidence/Judgment를 근거로 완료됩니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.complete.approve": "승인하고 완료로 표시",
  "wq.review.cancel.title": "승인 후 실행: {label} 취소",
  "wq.review.cancel.summary":
    "승인하면 이 Action은 입력한 사유로 취소됩니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.cancel.approve": "승인하고 취소",
  "wq.review.reopen.title": "승인 후 실행: {label} 다시 열기",
  "wq.review.reopen.summary":
    "승인하면 이 Action은 선택한 방식으로 다시 열립니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.reopen.approve": "승인하고 다시 열기",
  "wq.review.resolveDecision.title": "승인 후 실행: {label} 해결",
  "wq.review.resolveDecision.summary":
    "승인하면 미리 보기의 Decision과 거기에 적힌 후속 Action Request가 모두 만들어집니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.resolveDecision.approve": "승인하고 해결",
  "wq.review.occurrence.title": "승인 후 실행: {label} 발생 기록",
  "wq.review.occurrence.summary":
    "승인하면 이 Risk는 발생함 상태가 되고 미리 보기에 적힌 Issue {issue}가 만들어집니다(이 id는 앱이 할당한 것이며, 표시된 그대로 기록됩니다). 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.occurrence.approve": "승인하고 발생 기록",
  "wq.review.closeRisk.title": "승인 후 실행: {label} 종료",
  "wq.review.closeRisk.summary":
    "승인하면 이 Risk는 입력한 사유로 종료됩니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.closeRisk.approve": "승인하고 종료",
  "wq.review.resolveIssue.title": "승인 후 실행: {label} 해결",
  "wq.review.resolveIssue.summary":
    "승인하면 이 Issue는 미리 보기의 Evidence를 근거로 해결됨 상태가 됩니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.resolveIssue.approve": "승인하고 해결",
  "wq.review.closeIssue.title": "승인 후 실행: {label} 종료",
  "wq.review.closeIssue.summary":
    "승인하면 이 Issue는 미리 보기의 검증 Evidence를 근거로 종료됩니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.closeIssue.approve": "승인하고 종료",
  "wq.review.reopenIssue.title": "승인 후 실행: {label} 다시 열기",
  "wq.review.reopenIssue.summary":
    "승인하면 이 Issue는 입력한 사유와 검증 실패 Evidence를 근거로 다시 열립니다. 거부하면 기록되며 이 미리 보기는 더 이상 승인할 수 없습니다.",
  "wq.review.reopenIssue.approve": "승인하고 다시 열기",

  "wq.done.declined": "거절했습니다: {id}(버전 {version}).",
  "wq.done.withdrawn": "철회했습니다: {id}(버전 {version}).",
  "wq.done.started": "시작했습니다: {id}(버전 {version}).",
  "wq.done.linked": "Evidence {evidence}, {id} 항목에 연결했습니다(버전 {version}).",
  "wq.noWritePath": "이 앱에는 쓰기 경로가 없습니다.",
  "wq.settled.completed": "완료로 표시했습니다: {id}(버전 {version}, 상태: {state}).",
  "wq.settled.cancelled": "취소했습니다: {id}(버전 {version}, 상태: {state}).",
  "wq.settled.reopened": "다시 열었습니다: {id}(버전 {version}, 상태: {state}).",
  "wq.settled.accepted":
    "Action {action} 항목을 만들어 {request} 항목에 연결했습니다. 영수증 {receipt}.",
  "wq.settled.occurred":
    "발생을 기록하고(Risk는 이제 버전 {version}) Issue {issue} 항목을 만들었습니다.",
  "wq.settled.riskClosed": "종료했습니다: {id}(버전 {version}).",
  "wq.settled.issue": "{id} 항목의 현재 상태: {state}(버전 {version}).",
  "wq.settled.decision.one":
    "Decision {decision} 항목과 후속 Action Request {count}건을 만들었습니다. 영수증 {receipt}.",
  "wq.settled.decision.other":
    "Decision {decision} 항목과 후속 Action Request {count}건을 만들었습니다. 영수증 {receipt}.",
  "wq.notice.gone": "현재 목록에 없어 다시 준비하지 않았습니다: {label}.",
  "wq.notice.notAcceptable":
    "더 이상 수락할 수 있는 상태가 아니어서 다시 준비하지 않았습니다: {id}.",
  "wq.notice.rejected": "원래 미리 보기는 거부되었습니다. 다시 입력한 후 다시 준비하세요.",
  "wq.notice.held":
    "“{label}” 검토를 잠시 닫았습니다. 수락도 거부도 하지 않았습니다. 계속하려면 해당 행의 “검토로 돌아가기”를 누르세요. 미리 보기가 만료된 후에는 검토에서 다시 준비할 수 있습니다.",

  "wq.detail.close": "닫기",
  "wq.detail.attention": "주의 필요",
  "wq.detail.noAttention": "주의가 필요한 점이 없습니다",
  "wq.uncertainty": "(근거가 된 사실: {freshness}. 지금도 유효하다고 보장할 수 없습니다)",
  "wq.uncertaintyDegraded":
    "(근거가 된 사실: {freshness}. 원본도 기능 제한 상태라 지금도 유효하다고 보장할 수 없습니다)",
  "wq.detail.deadline": "기한",
  "wq.detail.promised": "완료 약속일",
  "wq.detail.placement": "여기 있는 이유",
  "wq.detail.allowed": "상태상 가능한 작업",
  "wq.none": "없음",
  "wq.detail.owner": "기록한 곳",
  "wq.detail.version": "버전",
  "wq.detail.next": "다음 단계",

  "wq.backToReview": "검토로 돌아가기",
  "wq.sending": "보내는 중…",
  "wq.actionFailed": "이 작업이 완료되지 않았습니다. {message}",
  "wq.abandon": "이 작업 그만두기",
  "wq.cancel": "취소",
  "wq.reason.closeRisk": "종료 사유",
  "wq.reason.decline": "거절 사유",
  "wq.reason.withdraw": "철회 사유",
  "wq.reason.cancel": "취소 사유",
  "wq.reason.reopen": "다시 여는 사유",
  "wq.confirm.closePreview": "종료 미리 보기 만들기",
  "wq.confirm.decline": "거절 확인",
  "wq.confirm.withdraw": "철회 확인",
  "wq.confirm.cancelPreview": "취소 미리 보기 만들기",
  "wq.confirm.reopenPreview": "다시 열기 미리 보기 만들기",
  "wq.confirm.resolvePreview": "해결 미리 보기 만들기",
  "wq.confirm.completePreview": "완료 미리 보기 만들기",
  "wq.reopenMode": "다시 여는 방식",
  "wq.reopenMode.completed": "완료된 Action 다시 열기",
  "wq.reopenMode.cancelled": "취소된 Action 다시 시작",
  "wq.issueEvidence.resolve":
    "이 Issue가 해결되었음을 보여 주는 Evidence(여러 개 선택 가능. 세 가지 전환 모두 Evidence가 필요합니다)",
  "wq.issueEvidence.close": "해결이 검증되었음을 보여 주는 Evidence(여러 개 선택 가능)",
  "wq.issueEvidence.reopen": "검증에 실패했음을 보여 주는 Evidence(여러 개 선택 가능)",
  "wq.resolutionType": "해결 방식",
  "wq.resolutionType.resolved": "해결됨",
  "wq.resolutionType.workaround": "우회 방법으로 처리",
  "wq.resolutionType.acceptedImpact": "영향을 수용함",
  "wq.reason.resolve": "해결 사유",
  "wq.evidenceLoading": "Evidence를 읽는 중…",
  "wq.evidenceNone": "Ledger에 Evidence가 없습니다.",
  "wq.evidenceOption.pinned": "{id}: {verification}, 고정됨, {classification}(버전 {version})",
  "wq.evidenceOption.unpinned":
    "{id}: {verification}, 고정 안 됨, {classification}(버전 {version})",
  "wq.judgment.issue":
    "Judgment 사유(비워 두면 Judgment를 붙이지 않습니다. 선택한 Evidence가 일부만 검증된 경우 필수이지만, 검증되지 않았거나 기록과 내용이 일치하지 않는 Evidence를 통과시킬 수는 없습니다)",
  "wq.judgment.complete":
    "Judgment 사유(비워 두면 Judgment를 붙이지 않습니다. 연결된 Evidence가 검증되지 않은 경우 필수입니다)",
  "wq.judgment.decision": "Judgment 사유(비워 두면 Judgment를 붙이지 않습니다)",
  "wq.judgmentClassification": "Judgment 등급",
  "wq.linkLoading": "연결할 수 있는 Evidence를 읽는 중…",
  "wq.linkChoose": "연결할 Evidence",
  "wq.linkPlaceholder": "(선택하세요)",
  "wq.linkNone": "Ledger에 연결할 수 있는 Evidence가 없습니다.",
  "wq.linkConfirm": "연결 확인",
  "wq.decision.statement": "결정 내용",
  "wq.decision.rationale": "결정 사유",
  "wq.decision.impact": "영향",
  "wq.decision.evidence":
    "이 Decision을 뒷받침하는 Evidence(여러 개 선택 가능. Evidence를 선택하지 않으면 Judgment가 필요합니다)",
  "wq.followUps": "후속 Action Request(각각 앱이 식별자를 할당합니다)",
  "wq.followUp.subject": "제목",
  "wq.followUp.details": "내용",
  "wq.followUp.owner": "담당자(Stakeholder id)",
  "wq.followUp.due": "기한",
  "wq.followUp.classification": "등급",
  "wq.followUp.remove": "이 항목 삭제",
  "wq.followUp.add": "후속 Action Request 추가",

  "wq.outOfSync":
    "Work Queue의 두 원본이 서로 다른 Ledger 리비전을 읽어 항목을 표시하지 않았습니다. 두 시점을 이어 붙이면 그럴듯하지만 틀린 목록이 됩니다.",
  "wq.headline": "오늘 처리할 일",
  "wq.lede":
    "정렬 순서: 지켜지지 않은 약속, 멈춘 작업, Evidence에 문제가 있는 항목, 기한이 다가오는 항목, 책임자가 없는 항목. 각 항목에 여기 있는 이유를 표시합니다.",
  "wq.filters": "유형으로 필터링(아무것도 선택하지 않으면 모두 표시)",
  "wq.filterChip": "{kind} ({count})",
  "wq.onlyFlagged": "주의가 필요한 항목만",
  "wq.empty": "지금은 처리할 항목이 없습니다.",
  "wq.column.kind": "유형",
  "wq.column.item": "항목",
  "wq.column.attention": "주의 필요",
  "wq.column.deadline": "기한",
  "wq.column.placement": "여기 있는 이유",
  "wq.column.next": "다음 단계",
  "wq.allowed": "상태상 가능한 작업: {intents}",
  "wq.promised": "완료 약속일 {date}",
  "wq.inDetail": "세부 정보 창에서 처리",
  "wq.previous": "이전 페이지",
  "wq.next": "다음 페이지",
} as const satisfies Record<keyof typeof WORK_QUEUE_EN, string>;
