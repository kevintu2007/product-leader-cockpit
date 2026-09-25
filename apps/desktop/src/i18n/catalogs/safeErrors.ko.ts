import type { SAFE_ERRORS_EN } from "./safeErrors.en";

/** 한국어 오류 메시지, 항목 이름, 다음 단계. 키는 영어판과 같습니다. */
export const SAFE_ERRORS_KO = {
  "safeError.desktop.snapshot_unavailable": "지금은 Product Ledger 스냅샷을 읽을 수 없습니다.",
  "safeError.desktop.snapshot_invalid":
    "Product Ledger 데이터가 일관성 검사를 통과하지 못해 아무것도 쓰지 않았습니다.",
  "safeError.desktop.product_not_found": "이 Product를 찾을 수 없습니다.",
  "safeError.desktop.product_detail_unattributable":
    "이 Product의 세부 정보 중 출처를 밝힐 수 없는 내용이 있어 표시하지 않았습니다.",
  "safeError.desktop.invalid_argument":
    "보낸 데이터의 형식이 올바르지 않습니다. 아무것도 쓰지 않았습니다.",
  "safeError.desktop.action_not_found": "이 Action을 찾을 수 없습니다.",
  "safeError.desktop.host_id_source_failed": "이 작업에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.desktop.host_identifier_failed": "이 작업에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.desktop.unsupported_preview":
    "이 화면에서 표시할 수 없는 종류의 미리 보기가 준비되어 표시하지 않았습니다.",
  "safeError.desktop.preview_already_consumed":
    "이 미리 보기는 이미 승인되었거나 거부되었습니다. 계속하려면 다시 준비하세요.",
  "safeError.desktop.evidence_not_found": "이 Evidence를 찾을 수 없습니다.",
  "safeError.desktop.evidence_already_pinned": "이 Evidence의 지문은 이미 고정되어 있습니다.",
  "safeError.desktop.evidence_version_conflict":
    "읽은 뒤에 이 Evidence가 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.desktop.evidence_path_containment_failed":
    "이 Evidence의 파일이 Product Vault 밖에 있어 읽지 않았습니다.",
  "safeError.desktop.evidence_source_not_observable":
    "지금은 이 Evidence의 파일을 읽을 수 없습니다.",
  "safeError.desktop.vault_not_configured":
    "이 작업 공간에는 아직 Product Vault가 설정되지 않았습니다. 설정에서 선택하세요.",
  "safeError.desktop.vault_root_unavailable": "지금은 Product Vault 폴더에 접근할 수 없습니다.",
  "safeError.desktop.settings_unavailable": "지금은 표시 설정을 읽거나 저장할 수 없습니다.",
  "safeError.desktop.backup_destination_unusable":
    "이 폴더에는 백업을 저장할 수 없습니다. 이 컴퓨터에 있고 쓰기 권한이 있는 폴더를 선택하세요. 바로 가기나 링크는 사용할 수 없습니다.",
  "safeError.desktop.passphrase_too_short":
    "암호 문구가 너무 짧습니다. 20자 이상 또는 6단어 이상을 사용하세요.",
  "safeError.desktop.passphrase_too_repetitive":
    "암호 문구에 반복이 너무 많습니다. 더 다양한 단어나 문자를 사용하세요.",
  "safeError.desktop.passphrase_too_long": "암호 문구가 너무 깁니다. 1,024자 이하로 사용하세요.",
  "safeError.desktop.passphrase_too_common":
    "이 암호 문구는 매우 흔한 비밀번호로만 이루어져 있습니다. 흔한 비밀번호가 아닌 단어를 사용하세요.",
  "safeError.desktop.credential_unavailable":
    "지금은 Windows 자격 증명 관리자에서 암호 문구를 저장하거나 읽을 수 없습니다.",
  "safeError.desktop.random_unavailable": "암호 문구를 만드는 데 필요한 난수를 얻지 못했습니다.",

  "safeError.ledger.open.unclaimed_database":
    "이 Product Ledger 파일은 이 앱에서 만든 것이 아닙니다.",
  "safeError.ledger.open.wrong_application":
    "이 파일은 Product Mission Control의 Ledger가 아닙니다.",
  "safeError.ledger.open.future_schema":
    "이 Product Ledger는 더 새로운 버전의 앱에서 만들어져 이 버전에서는 열 수 없습니다.",
  "safeError.ledger.open.unsupported_schema":
    "이 버전의 앱은 이 Product Ledger의 스키마 버전을 지원하지 않습니다.",
  "safeError.ledger.open.invalid_metadata": "Product Ledger의 메타데이터가 일관되지 않습니다.",
  "safeError.ledger.open.corrupt_database": "Product Ledger 파일이 손상되었습니다.",
  "safeError.ledger.open.policy_violation":
    "Product Ledger 연결 설정이 보안 정책을 충족하지 않습니다.",
  "safeError.ledger.open.busy": "Product Ledger가 사용 중입니다.",
  "safeError.ledger.open.storage_unavailable": "저장 장치를 사용할 수 없습니다.",
  "safeError.ledger.transaction.revision_conflict":
    "읽은 뒤에 데이터가 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.ledger.transaction.incompatible_ledger":
    "Product Ledger가 이 버전의 앱과 호환되지 않습니다.",
  "safeError.ledger.transaction.busy": "Product Ledger가 사용 중입니다.",
  "safeError.ledger.transaction.commit_failed":
    "쓰기가 완료되지 않았으며, 저장되었는지 확인할 수 없습니다.",
  "safeError.ledger.commit_failed": "쓰기가 완료되지 않았으며, 저장되었는지 확인할 수 없습니다.",
  "safeError.ledger.persistence_failed": "Product Ledger에 저장할 수 없었습니다.",
  "safeError.ledger.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.audit.failed": "이 작업의 감사 기록을 쓸 수 없어 아무것도 변경하지 않았습니다.",
  "safeError.version.overflow": "이 레코드의 버전 번호가 한도에 도달했습니다.",
  "safeError.classification.lowering_requires_governed_intent":
    "등급을 낮추려면 별도의 검토와 승인 절차가 필요합니다.",

  "safeError.action.not_found": "이 Action Request 또는 Action을 찾을 수 없습니다.",
  "safeError.action_request.already_exists": "같은 식별자의 Action Request가 이미 있습니다.",
  "safeError.action.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.action.approval_denied":
    "이 작업은 권한이 없거나, 정책에 의해 거부되었거나, Evidence 또는 Judgment가 충분하지 않습니다.",
  "safeError.action.preview_expired_or_changed":
    "미리 보기가 만료되었거나 확인 전에 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.action.internal": "이 작업을 처리하는 중에 내부 오류가 발생했습니다.",
  "safeError.action.validation_failed": "필수 항목(담당자나 기한 등)이 빠져 있습니다.",
  "safeError.action.domain_conflict":
    "레코드의 현재 상태나 버전과 충돌합니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.action.classification_lowering_not_a_lowering":
    "제안한 등급이 현재 등급보다 낮지 않습니다.",
  "safeError.action.completion_evidence_already_linked":
    "이 Evidence는 이미 이 Action의 완료 Evidence로 연결되어 있습니다.",

  "safeError.decision.not_found": "이 Decision Request 또는 Decision을 찾을 수 없습니다.",
  "safeError.decision.already_exists": "같은 식별자의 Decision이 이미 있습니다.",
  "safeError.decision.conflict":
    "Decision Request의 현재 상태나 버전과 충돌합니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.decision.request_transition_conflict":
    "Decision Request의 현재 상태에서는 이 작업을 할 수 없습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.decision.approval_denied":
    "이 작업은 권한이 없거나 Evidence 또는 Judgment가 충분하지 않습니다.",
  "safeError.decision.preview_changed":
    "미리 보기가 만료되었거나 확인 전에 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.decision.internal": "이 작업을 처리하는 중에 내부 오류가 발생했습니다.",
  "safeError.decision.classification_lowering_not_a_lowering":
    "제안한 등급이 현재 등급보다 낮지 않습니다.",

  "safeError.risk.not_found": "이 Risk를 찾을 수 없습니다.",
  "safeError.risk.owner_not_found": "담당자로 지정한 Stakeholder를 찾을 수 없습니다.",
  "safeError.risk.next_review_before_epoch": "다음 검토 날짜는 1970년 이전일 수 없습니다.",
  "safeError.risk.already_exists": "같은 식별자의 Risk가 이미 있습니다.",
  "safeError.risk.classification": "이 Risk의 등급을 선택하세요.",
  "safeError.risk.accepted_fields_required":
    "Risk를 수용하거나 이전하려면 사유와 검토일이 필요합니다.",
  "safeError.risk.conflict": "Risk의 현재 상태와 충돌합니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.risk.stale_or_illegal":
    "Risk가 변경되었거나 현재 상태에서는 이 작업을 할 수 없습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.risk.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.risk.preview_changed":
    "미리 보기가 만료되었거나 확인 전에 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.risk.security_denied": "이 Risk에 대한 작업 권한이 없습니다.",
  "safeError.risk.evidence_unavailable": "지금은 이 작업에 필요한 Evidence를 읽을 수 없습니다.",
  "safeError.risk.infrastructure": "이 Risk를 처리하는 중에 내부 오류가 발생했습니다.",

  "safeError.issue.not_found": "이 Issue를 찾을 수 없습니다.",
  "safeError.issue.exists": "같은 식별자의 Issue가 이미 있습니다.",
  "safeError.issue.classification_required": "이 Issue의 등급을 선택하세요.",
  "safeError.issue.stale_or_illegal":
    "Issue가 변경되었거나 현재 상태에서는 이 작업을 할 수 없습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.issue.invalid_intent": "이 Issue의 현재 상태에서는 이 작업을 할 수 없습니다.",
  "safeError.issue.prepared_operation_mismatch":
    "이 미리 보기는 다른 작업을 위해 준비되었습니다. 다시 준비하세요.",
  "safeError.issue.preview_changed":
    "미리 보기가 만료되었거나 확인 전에 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.issue.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.issue.recurrence_not_supported": "이 Issue는 재발로 기록할 수 없습니다.",
  "safeError.issue.security_denied": "이 Issue에 대한 작업 권한이 없습니다.",
  "safeError.issue.infrastructure": "이 Issue를 처리하는 중에 내부 오류가 발생했습니다.",
  "safeError.issue.classification_lowering_not_a_lowering":
    "제안한 등급이 현재 등급보다 낮지 않습니다.",

  "safeError.evidence.not_found": "이 Evidence를 찾을 수 없습니다.",
  "safeError.evidence_reference.not_found": "이 Evidence를 찾을 수 없습니다.",
  "safeError.evidence.already_exists": "같은 식별자의 Evidence가 이미 있습니다.",
  "safeError.evidence.path_already_referenced":
    "이 파일을 가리키는 다른 Evidence 참조가 방금 만들어졌습니다. 파일을 다시 선택하면 볼 수 있습니다.",
  "safeError.evidence.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.evidence.persistence_failed": "Product Ledger에 이 Evidence를 저장할 수 없었습니다.",
  "safeError.evidence.pin_fingerprint_already_pinned":
    "이 Evidence의 지문은 이미 고정되어 있습니다.",
  "safeError.evidence.pin_path_mismatch":
    "읽은 파일이 이 Evidence에 기록된 경로와 일치하지 않아 지문을 고정하지 않았습니다.",
  "safeError.evidence.relocation_path_unchanged": "새 위치가 현재 위치와 같습니다.",
  "safeError.evidence.relocation_path_mismatch": "읽은 파일이 지정한 새 위치와 일치하지 않습니다.",
  "safeError.evidence.relocation_fingerprint_mismatch":
    "새 위치의 파일은 지문이 달라 같은 파일이 아닙니다.",
  "safeError.evidence.relocation_fingerprint_unpinned":
    "이 Evidence에는 고정된 지문이 없어 이동 후에도 같은 파일인지 확인할 수 없습니다.",
  "safeError.evidence.supersession_source_mismatch":
    "이 대체는 다른 Evidence를 위해 준비되었습니다.",
  "safeError.evidence.supersession_source_already_superseded": "이 Evidence는 이미 대체되었습니다.",
  "safeError.evidence.supersession_replacement_is_source":
    "Evidence를 자기 자신으로 대체할 수 없습니다.",
  "safeError.evidence.supersession_not_a_genuine_replacement":
    "대체할 파일이 대체될 Evidence와 같은 파일입니다.",
  "safeError.evidence.supersession_lowers_classification":
    "대체 Evidence의 등급은 대체될 Evidence보다 낮을 수 없습니다.",
  "safeError.evidence.supersession_unclassified_replacement": "대체 Evidence의 등급을 선택하세요.",
  "safeError.evidence.supersession_missing_confirmation":
    "Evidence를 대체하려면 확인이 필요합니다.",
  "safeError.evidence.supersession_unauthorized_actor": "이 Evidence에 대한 작업 권한이 없습니다.",
  "safeError.evidence.supersession_prepared_intent_mismatch":
    "이 승인은 준비된 다른 대체에 대한 것입니다.",
  "safeError.evidence.supersession_digest_mismatch":
    "준비한 뒤에 대체 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.evidence.supersession_preview_changed":
    "확인 전에 미리 보기가 바뀌었습니다. 다시 준비하세요.",
  "safeError.evidence.supersession_expired": "미리 보기가 만료되었습니다. 다시 준비하세요.",

  "safeError.portfolio.not_found": "이 Portfolio를 찾을 수 없습니다.",
  "safeError.portfolio.already_exists": "같은 식별자의 Portfolio가 이미 있습니다.",
  "safeError.portfolio.stale_version":
    "읽은 뒤에 이 Portfolio가 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.portfolio.version_exhausted": "이 Portfolio의 버전 번호가 한도에 도달했습니다.",
  "safeError.portfolio.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.portfolio.repository_unavailable": "지금은 Portfolio 레코드를 읽을 수 없습니다.",
  "safeError.portfolio.fan_out_state_invalid":
    "Portfolio 상태의 불일치를 발견해 쓰기 전에 중단했습니다.",
  "safeError.portfolio.operation_ordinal_exhausted": "이 종류의 작업은 더 이상 기록할 수 없습니다.",
  "safeError.portfolio.audit_id_unavailable": "이 작업에 필요한 감사 식별자를 만들 수 없었습니다.",
  "safeError.portfolio.prepared_intent_id_unavailable":
    "이 미리 보기에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.portfolio.approval_receipt_id_unavailable":
    "이 승인에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.portfolio.classification_lowering_invalid":
    "이 Portfolio의 등급은 이런 방식으로 낮출 수 없습니다.",
  "safeError.portfolio.classification_lowering_not_a_lowering":
    "제안한 등급이 이 Portfolio의 현재 등급보다 낮지 않습니다.",
  "safeError.portfolio.classification_lowering_preview_changed":
    "미리 보기를 준비한 뒤에 이 Portfolio가 변경되었습니다. 다시 준비하세요.",
  "safeError.portfolio.classification_lowering_approval_mismatch":
    "이 승인은 다른 미리 보기에 대한 것입니다.",
  "safeError.portfolio.classification_lowering_approval_denied":
    "이 Portfolio의 등급을 낮추는 것은 승인되지 않았습니다.",

  "safeError.product.not_found": "이 Product를 찾을 수 없습니다.",
  "safeError.product.already_exists": "같은 식별자의 Product가 이미 있습니다.",
  "safeError.product.stale_version":
    "읽은 뒤에 이 Product가 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.product.version_exhausted": "이 Product의 버전 번호가 한도에 도달했습니다.",
  "safeError.product.prepared_intent_id_unavailable":
    "이 미리 보기에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.product.approval_receipt_id_unavailable":
    "이 승인에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.product.classification_lowering_invalid":
    "이 Product의 등급은 이런 방식으로 낮출 수 없습니다.",
  "safeError.product.classification_lowering_not_a_lowering":
    "제안한 등급이 이 Product의 현재 등급보다 낮지 않습니다.",
  "safeError.product.classification_lowering_preview_changed":
    "미리 보기를 준비한 뒤에 이 Product가 변경되었습니다. 다시 준비하세요.",
  "safeError.product.classification_lowering_approval_mismatch":
    "이 승인은 다른 미리 보기에 대한 것입니다.",
  "safeError.product.classification_lowering_approval_denied":
    "이 Product의 등급을 낮추는 것은 승인되지 않았습니다.",

  "safeError.roadmap.not_found": "이 Roadmap을 찾을 수 없습니다.",
  "safeError.roadmap.already_exists": "같은 식별자의 Roadmap이 이미 있습니다.",
  "safeError.roadmap.stale_version":
    "읽은 뒤에 이 Roadmap이 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.roadmap.version_exhausted": "이 Roadmap의 버전 번호가 한도에 도달했습니다.",
  "safeError.roadmap.prepared_intent_id_unavailable":
    "이 미리 보기에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.roadmap.approval_receipt_id_unavailable":
    "이 승인에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.roadmap.classification_lowering_invalid":
    "이 Roadmap의 등급은 이런 방식으로 낮출 수 없습니다.",
  "safeError.roadmap.classification_lowering_not_a_lowering":
    "제안한 등급이 이 Roadmap의 현재 등급보다 낮지 않습니다.",
  "safeError.roadmap.classification_lowering_preview_changed":
    "미리 보기를 준비한 뒤에 이 Roadmap이 변경되었습니다. 다시 준비하세요.",
  "safeError.roadmap.classification_lowering_approval_mismatch":
    "이 승인은 다른 미리 보기에 대한 것입니다.",
  "safeError.roadmap.classification_lowering_approval_denied":
    "이 Roadmap의 등급을 낮추는 것은 승인되지 않았습니다.",

  "safeError.kpi.not_found": "이 KPI를 찾을 수 없습니다.",
  "safeError.kpi.already_exists": "같은 식별자의 KPI가 이미 있습니다.",
  "safeError.kpi.stale_version": "읽은 뒤에 이 KPI가 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.kpi.version_exhausted": "이 KPI의 버전 번호가 한도에 도달했습니다.",
  "safeError.kpi.prepared_intent_id_unavailable":
    "이 미리 보기에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.kpi.approval_receipt_id_unavailable": "이 승인에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.kpi.classification_lowering_invalid":
    "이 KPI의 등급은 이런 방식으로 낮출 수 없습니다.",
  "safeError.kpi.classification_lowering_not_a_lowering":
    "제안한 등급이 이 KPI의 현재 등급보다 낮지 않습니다.",
  "safeError.kpi.classification_lowering_preview_changed":
    "미리 보기를 준비한 뒤에 이 KPI가 변경되었습니다. 다시 준비하세요.",
  "safeError.kpi.classification_lowering_approval_mismatch":
    "이 승인은 다른 미리 보기에 대한 것입니다.",
  "safeError.kpi.classification_lowering_approval_denied":
    "이 KPI의 등급을 낮추는 것은 승인되지 않았습니다.",

  "safeError.kpi.observation.not_found": "이 KPI 관측값을 찾을 수 없습니다.",
  "safeError.kpi.observation.already_exists": "같은 식별자의 KPI 관측값이 이미 있습니다.",
  "safeError.kpi.observation.stale_version":
    "읽은 뒤에 이 KPI 관측값이 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.kpi.observation.version_exhausted": "이 KPI 관측값의 버전 번호가 한도에 도달했습니다.",
  "safeError.kpi.observation.prepared_intent_id_unavailable":
    "이 미리 보기에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.kpi.observation.approval_receipt_id_unavailable":
    "이 승인에 필요한 식별자를 만들 수 없었습니다.",
  "safeError.kpi.observation.classification_lowering_invalid":
    "이 KPI 관측값의 등급은 이런 방식으로 낮출 수 없습니다.",
  "safeError.kpi.observation.classification_lowering_not_a_lowering":
    "제안한 등급이 이 KPI 관측값의 현재 등급보다 낮지 않습니다.",
  "safeError.kpi.observation.classification_lowering_preview_changed":
    "미리 보기를 준비한 뒤에 이 KPI 관측값이 변경되었습니다. 다시 준비하세요.",
  "safeError.kpi.observation.classification_lowering_approval_mismatch":
    "이 승인은 다른 미리 보기에 대한 것입니다.",
  "safeError.kpi.observation.classification_lowering_approval_denied":
    "이 KPI 관측값의 등급을 낮추는 것은 승인되지 않았습니다.",

  "safeError.delivery.not_found": "이 Initiative, Project 또는 Milestone을 찾을 수 없습니다.",
  "safeError.delivery.already_exists": "같은 식별자의 레코드가 이미 있습니다.",
  "safeError.delivery.stale_version":
    "읽은 뒤에 이 레코드가 변경되었습니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.delivery.version_exhausted": "이 레코드의 버전 번호가 한도에 도달했습니다.",
  "safeError.delivery.conflict": "레코드의 현재 상태와 충돌합니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.delivery.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.delivery.persistence_failed": "Product Ledger에 저장할 수 없었습니다.",
  "safeError.delivery.invalid_period": "시작일이 종료일보다 늦습니다.",
  "safeError.delivery.preview_changed":
    "미리 보기가 만료되었거나 확인 전에 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.delivery.validation.invalid_field": "허용되지 않는 값이 입력된 항목이 있습니다.",
  "safeError.delivery.validation.invalid_text": "비어 있거나 너무 긴 텍스트 항목이 있습니다.",
  "safeError.delivery.validation.invalid_period": "시작일이 종료일보다 늦습니다.",
  "safeError.delivery.classification.lowering_denied": "이 등급을 낮추는 것은 허용되지 않습니다.",
  "safeError.delivery.classification_lowering_invalid":
    "이 레코드의 등급은 이런 방식으로 낮출 수 없습니다.",
  "safeError.delivery.classification_lowering_not_a_lowering":
    "제안한 등급이 현재 등급보다 낮지 않습니다.",
  "safeError.delivery.classification_lowering_preview_changed":
    "미리 보기를 준비한 뒤에 이 레코드가 변경되었습니다. 다시 준비하세요.",

  "safeError.relationship.not_found": "이 관계를 찾을 수 없습니다.",
  "safeError.relationship.conflict": "관계의 현재 상태와 충돌합니다. 다시 읽은 후 다시 시도하세요.",
  "safeError.relationship.idempotency_conflict":
    "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.relationship.persistence_failed": "Product Ledger에 이 관계를 저장할 수 없었습니다.",
  "safeError.relationship.milestone_subject_not_supported":
    "Milestone은 이 종류의 관계의 대상이 될 수 없습니다.",
  "safeError.relationship.classification.unclassified_or_lowering_denied":
    "연결하는 레코드보다 낮지 않은 등급을 선택하세요.",
  "safeError.relationship.removal.authorization_denied": "이 관계를 삭제할 권한이 없습니다.",
  "safeError.relationship.removal.policy_denied": "정책상 이 관계는 삭제할 수 없습니다.",
  "safeError.relationship.removal.confirmation_mismatch":
    "확인 문구가 일치하지 않습니다. 표시된 그대로 입력하세요.",
  "safeError.relationship.removal.preview_expired_or_changed":
    "미리 보기가 만료되었거나 확인 전에 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.relationship.removal.too_late_to_cancel":
    "이 삭제는 이미 승인되어 취소할 수 없습니다.",

  "safeError.projection.idempotency_conflict": "이 요청 식별자는 이미 다른 작업에 사용되었습니다.",
  "safeError.projection.persistence_failed": "Product Ledger에 이 프로젝션을 저장할 수 없었습니다.",
  "safeError.projection.rebuild_operation_not_found": "이 프로젝션 재구성을 찾을 수 없습니다.",
  "safeError.projection.rebuild_operation_terminal": "이 프로젝션 재구성은 이미 끝났습니다.",
  "safeError.projection.rebuild_prepared_intent_not_found":
    "준비된 재구성을 찾을 수 없습니다. 다시 준비하세요.",
  "safeError.projection.rebuild_prepared_intent_consumed":
    "이 준비된 재구성은 이미 사용되었습니다. 다시 준비하세요.",
  "safeError.projection.rebuild_prepared_intent_mismatch":
    "이 승인은 준비된 다른 재구성에 대한 것입니다.",
  "safeError.projection.rebuild_preview_changed":
    "확인 전에 재구성 미리 보기가 바뀌었습니다. 다시 준비하세요.",
  "safeError.projection.rebuild_preview_expired":
    "재구성 미리 보기가 만료되었습니다. 다시 준비하세요.",
  "safeError.projection.rebuild_digest_mismatch":
    "준비한 뒤에 재구성 내용이 바뀌었습니다. 다시 준비하세요.",
  "safeError.projection.rebuild_missing_confirmation": "프로젝션 재구성에는 확인이 필요합니다.",
  "safeError.projection.rebuild_unauthorized_actor": "프로젝션에 대한 작업 권한이 없습니다.",
  "safeError.projection.rebuild_h1_auto_not_permitted":
    "이 재구성은 검토가 필요해 자동으로 실행할 수 없습니다.",
  "safeError.projection.rebuild_publication_in_flight":
    "다른 프로젝션 게시가 아직 진행 중입니다. 완료된 후 다시 시도하세요.",
  "safeError.projection.rebuild_empty_change_set": "재구성할 내용이 없습니다.",
  "safeError.projection.rebuild_changes_not_canonical":
    "계획된 변경이 예상 형식이 아니어서 재구성하지 않았습니다.",
  "safeError.projection.rebuild_unplanned_item":
    "재구성 중 계획에 없는 항목을 발견해 중단했습니다.",
  "safeError.projection.rebuild_incomplete_report": "재구성 보고서가 불완전해 확인할 수 없습니다.",

  "field.delivery.name": "이름",
  "field.initiative.defined_outcome": "정의된 성과",
  "field.milestone.verification_criteria": "검증 기준",
  "field.project.time_range": "기간",

  "nextStep.issue.prepare_resolve": "해결을 준비하세요.",
  "nextStep.issue.prepare_close_or_reopen": "종료 또는 다시 열기를 준비하세요.",
  "nextStep.issue.no_transition": "종료된 Issue에는 다음 단계가 없습니다.",
  "nextStep.issue.refresh_and_reprepare": "다시 읽은 후 다시 준비하세요.",
  "nextStep.risk.update_response_or_prepare_transition":
    "대응 방식을 업데이트하거나 상태 전환을 준비하세요.",
  "nextStep.risk.no_transition": "이 Risk에는 다음 단계가 없습니다.",
  "nextStep.risk.refresh_and_reprepare": "다시 읽은 후 다시 준비하세요.",
  "safeError.desktop.backup_due": "백업할 때가 되었습니다. 먼저 백업한 다음 다시 시도하세요.",
  "safeError.desktop.backup_running": "백업 중입니다. 끝난 후 다시 시도하세요.",
  "safeError.desktop.backup_destination_not_set": "먼저 백업 폴더를 선택하세요.",
  "safeError.desktop.backup_destination_unavailable":
    "백업 폴더에 접근할 수 없습니다. 드라이브를 연결하거나 다른 폴더를 선택하세요.",
  "safeError.desktop.backup_passphrase_required": "먼저 복구 암호 문구를 설정하세요.",
  "safeError.desktop.backup_verification_failed":
    "백업이 검증을 통과하지 못해 보관하지 않았습니다. 다시 시도하세요.",
  "safeError.desktop.backup_failed": "백업이 완료되지 않았습니다. 다시 시도하세요.",
  "safeError.desktop.restore_ledger_locked":
    "다른 프로그램이 현재 Ledger 파일을 열고 있습니다. 그 프로그램을 닫고 다시 시도하세요. 변경된 것은 없습니다.",
  "safeError.desktop.restore_preservation_failed":
    "PMC가 현재 Ledger 파일을 보존하고 확인할 수 없었습니다. 변경된 것은 없으며 복원을 계속할 수 없습니다.",
  "safeError.desktop.restore_state_unreadable":
    "PMC가 복원 상태 기록을 읽을 수 없어 복원하지 않습니다. 변경된 것은 없습니다.",
  "safeError.desktop.restore_recovery_backup_missing":
    "복구용 백업이 백업 폴더에 없거나 내용이 바뀌었습니다. 다른 백업을 선택하세요.",
} as const satisfies Record<keyof typeof SAFE_ERRORS_EN, string>;
