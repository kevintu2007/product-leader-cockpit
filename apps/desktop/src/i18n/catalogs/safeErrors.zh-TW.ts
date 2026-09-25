import type { SAFE_ERRORS_EN } from "./safeErrors.en";

/** 繁體中文的錯誤訊息、欄位名稱與下一步。鍵與英文完全相同。 */
export const SAFE_ERRORS_ZH_TW = {
  "safeError.desktop.snapshot_unavailable": "目前無法讀取 Product Ledger 的快照。",
  "safeError.desktop.snapshot_invalid":
    "Product Ledger 的資料沒有通過一致性驗證，畫面拒絕在它之上寫入。",
  "safeError.desktop.product_not_found": "找不到這個 Product。",
  "safeError.desktop.product_detail_unattributable":
    "Product 詳情有一筆無法標明來源的內容，畫面拒絕顯示未經溯源的資料。",
  "safeError.desktop.invalid_argument": "送出的資料格式不正確，畫面沒有寫入任何內容。",
  "safeError.desktop.action_not_found": "找不到這個 Action。",
  "safeError.desktop.host_id_source_failed": "主機無法產生這次操作所需的識別碼。",
  "safeError.desktop.host_identifier_failed": "主機無法產生這次操作所需的識別碼。",
  "safeError.desktop.unsupported_preview": "主機準備了畫面無法呈現的預覽類型，已拒絕顯示。",
  "safeError.desktop.preview_already_consumed": "這個預覽已經被核准或拒絕；要繼續請重新準備。",
  "safeError.desktop.evidence_not_found": "找不到這份 Evidence。",
  "safeError.desktop.evidence_already_pinned": "這份 Evidence 的指紋已經釘選過了。",
  "safeError.desktop.evidence_version_conflict":
    "這份 Evidence 在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.desktop.evidence_path_containment_failed":
    "這份 Evidence 的檔案不在 Product Vault 裡，所以沒有讀取。",
  "safeError.desktop.evidence_source_not_observable": "目前讀不到這份 Evidence 的檔案。",
  "safeError.desktop.vault_not_configured": "這個工作區還沒有設定 Product Vault，請到設定裡選擇。",
  "safeError.desktop.vault_root_unavailable": "目前無法存取 Product Vault 資料夾。",
  "safeError.desktop.settings_unavailable": "目前無法讀取或儲存顯示設定。",
  "safeError.desktop.backup_destination_unusable":
    "這個資料夾不能存放備份。請選擇這台電腦上已存在、你可以寫入的資料夾，不能是捷徑或連結。",
  "safeError.desktop.passphrase_too_short": "這組密碼太短。請至少使用 20 個字元或 6 個單字。",
  "safeError.desktop.passphrase_too_repetitive": "這組密碼重複太多。請使用更多不同的單字或字元。",
  "safeError.desktop.passphrase_too_long": "這組密碼太長。最多 1,024 個字元。",
  "safeError.desktop.passphrase_too_common":
    "這組密碼只由非常常見的密碼組成。請改用不是常見密碼的單字。",
  "safeError.desktop.credential_unavailable": "Windows 認證管理員目前無法儲存或讀取這組密碼。",
  "safeError.desktop.random_unavailable": "應用程式無法取得產生密碼所需的亂數。",

  "safeError.ledger.open.unclaimed_database": "Product Ledger 檔案尚未由本應用程式建立。",
  "safeError.ledger.open.wrong_application": "這個檔案不是 Product Mission Control 的 Ledger。",
  "safeError.ledger.open.future_schema":
    "Product Ledger 來自較新的版本，這個版本的應用程式無法開啟。",
  "safeError.ledger.open.unsupported_schema":
    "Product Ledger 的結構版本不受這個版本的應用程式支援。",
  "safeError.ledger.open.invalid_metadata": "Product Ledger 的中繼資料不一致。",
  "safeError.ledger.open.corrupt_database": "Product Ledger 檔案已損毀。",
  "safeError.ledger.open.policy_violation": "Product Ledger 的連線設定不符合安全政策。",
  "safeError.ledger.open.busy": "Product Ledger 目前忙碌中。",
  "safeError.ledger.open.storage_unavailable": "儲存裝置目前無法使用。",
  "safeError.ledger.transaction.revision_conflict": "資料在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.ledger.transaction.incompatible_ledger": "Product Ledger 與這個版本的應用程式不相容。",
  "safeError.ledger.transaction.busy": "Product Ledger 目前忙碌中。",
  "safeError.ledger.transaction.commit_failed": "寫入未能完成；系統無法確認是否已儲存。",
  "safeError.ledger.commit_failed": "寫入未能完成；系統無法確認是否已儲存。",
  "safeError.ledger.persistence_failed": "Product Ledger 無法儲存這筆資料。",
  "safeError.ledger.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.audit.failed": "這次操作的稽核紀錄無法寫入，所以沒有做任何變更。",
  "safeError.version.overflow": "這筆紀錄的版本號已達上限。",
  "safeError.classification.lowering_requires_governed_intent":
    "降低分級必須經過獨立的審閱與核准步驟。",

  "safeError.action.not_found": "找不到這個 Action Request 或 Action。",
  "safeError.action_request.already_exists": "已經有相同識別碼的 Action Request。",
  "safeError.action.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.action.approval_denied":
    "這個操作未獲授權、被政策拒絕，或 Evidence／Judgment 不足以支持它。",
  "safeError.action.preview_expired_or_changed":
    "預覽已逾期，或內容在你確認之前已變更；請重新準備。",
  "safeError.action.internal": "主機處理這個操作時發生內部錯誤。",
  "safeError.action.validation_failed": "這個操作缺少必要的欄位（例如負責人或到期日）。",
  "safeError.action.domain_conflict": "這個操作與記錄目前的狀態或版本衝突，請重新讀取後再試。",
  "safeError.action.classification_lowering_not_a_lowering": "提出的分級並非降級。",
  "safeError.action.completion_evidence_already_linked":
    "這份 Evidence 已經連結為這筆 Action 的完成證據。",

  "safeError.decision.not_found": "找不到這個 Decision Request 或 Decision。",
  "safeError.decision.already_exists": "已經有相同識別碼的 Decision。",
  "safeError.decision.conflict":
    "這個操作與 Decision Request 目前的狀態或版本衝突，請重新讀取後再試。",
  "safeError.decision.request_transition_conflict":
    "Decision Request 目前的狀態不允許這個操作，請重新讀取後再試。",
  "safeError.decision.approval_denied": "這個操作未獲授權、或 Evidence／Judgment 不足以支持它。",
  "safeError.decision.preview_changed": "預覽已逾期，或內容在你確認之前已變更；請重新準備。",
  "safeError.decision.internal": "主機處理這個操作時發生內部錯誤。",
  "safeError.decision.classification_lowering_not_a_lowering": "提出的分級並非降級。",

  "safeError.risk.not_found": "找不到這個 Risk。",
  "safeError.risk.owner_not_found": "找不到指定為負責人的 Stakeholder。",
  "safeError.risk.next_review_before_epoch": "下次檢視日期不能早於 1970 年。",
  "safeError.risk.already_exists": "已經有相同識別碼的 Risk。",
  "safeError.risk.classification": "請為這個 Risk 選擇分級。",
  "safeError.risk.accepted_fields_required": "接受或轉移 Risk 需要填寫理由與複查日期。",
  "safeError.risk.conflict": "這個操作與 Risk 目前的狀態衝突，請重新讀取後再試。",
  "safeError.risk.stale_or_illegal":
    "Risk 已被更改，或目前的狀態不允許這個操作；請重新讀取後再試。",
  "safeError.risk.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.risk.preview_changed": "預覽已逾期，或內容在你確認之前已變更；請重新準備。",
  "safeError.risk.security_denied": "這個 Risk 的操作未獲授權。",
  "safeError.risk.evidence_unavailable": "目前讀不到這個操作需要的 Evidence。",
  "safeError.risk.infrastructure": "主機處理這個 Risk 時發生內部錯誤。",

  "safeError.issue.not_found": "找不到這個 Issue。",
  "safeError.issue.exists": "已經有相同識別碼的 Issue。",
  "safeError.issue.classification_required": "請為這個 Issue 選擇分級。",
  "safeError.issue.stale_or_illegal":
    "Issue 已被更改，或目前的狀態不允許這個操作；請重新讀取後再試。",
  "safeError.issue.invalid_intent": "這個 Issue 目前的狀態不允許這個操作。",
  "safeError.issue.prepared_operation_mismatch": "這個預覽是為另一個操作準備的，請重新準備。",
  "safeError.issue.preview_changed": "預覽已逾期，或內容在你確認之前已變更；請重新準備。",
  "safeError.issue.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.issue.recurrence_not_supported": "這個 Issue 不能記錄為再次發生。",
  "safeError.issue.security_denied": "這個 Issue 的操作未獲授權。",
  "safeError.issue.infrastructure": "主機處理這個 Issue 時發生內部錯誤。",
  "safeError.issue.classification_lowering_not_a_lowering": "提出的分級並非降級。",

  "safeError.evidence.not_found": "找不到這份 Evidence。",
  "safeError.evidence_reference.not_found": "找不到這份 Evidence。",
  "safeError.evidence.already_exists": "已經有相同識別碼的 Evidence。",
  "safeError.evidence.path_already_referenced":
    "剛剛已有另一筆 Evidence 參照指向這個檔案。請重新選擇檔案查看。",
  "safeError.evidence.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.evidence.persistence_failed": "Product Ledger 無法儲存這份 Evidence。",
  "safeError.evidence.pin_fingerprint_already_pinned": "這份 Evidence 的指紋已經釘選過了。",
  "safeError.evidence.pin_path_mismatch":
    "讀到的檔案與這份 Evidence 記錄的路徑不符，沒有釘選指紋。",
  "safeError.evidence.relocation_path_unchanged": "新的位置與目前的位置相同。",
  "safeError.evidence.relocation_path_mismatch": "讀到的檔案與你提供的新位置不符。",
  "safeError.evidence.relocation_fingerprint_mismatch": "新位置上的檔案指紋不同，不是同一個檔案。",
  "safeError.evidence.relocation_fingerprint_unpinned":
    "這份 Evidence 沒有釘選指紋，無法確認搬移後仍是同一個檔案。",
  "safeError.evidence.supersession_source_mismatch": "這個取代是為另一份 Evidence 準備的。",
  "safeError.evidence.supersession_source_already_superseded": "這份 Evidence 已經被取代過了。",
  "safeError.evidence.supersession_replacement_is_source": "Evidence 不能取代自己。",
  "safeError.evidence.supersession_not_a_genuine_replacement":
    "用來取代的檔案與被取代的 Evidence 是同一個檔案。",
  "safeError.evidence.supersession_lowers_classification":
    "用來取代的 Evidence 分級不能低於被取代的 Evidence。",
  "safeError.evidence.supersession_unclassified_replacement": "請為用來取代的 Evidence 選擇分級。",
  "safeError.evidence.supersession_missing_confirmation": "取代 Evidence 需要你的確認。",
  "safeError.evidence.supersession_unauthorized_actor": "這份 Evidence 的操作未獲授權。",
  "safeError.evidence.supersession_prepared_intent_mismatch":
    "這個核准對應的是另一個已準備的取代。",
  "safeError.evidence.supersession_digest_mismatch": "準備之後取代內容已有變更，請重新準備。",
  "safeError.evidence.supersession_preview_changed": "預覽在你確認之前已變更，請重新準備。",
  "safeError.evidence.supersession_expired": "預覽已逾期，請重新準備。",

  "safeError.portfolio.not_found": "找不到這個 Portfolio。",
  "safeError.portfolio.already_exists": "已經有相同識別碼的 Portfolio。",
  "safeError.portfolio.stale_version": "這個 Portfolio 在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.portfolio.version_exhausted": "這個 Portfolio 的版本號已達上限。",
  "safeError.portfolio.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.portfolio.repository_unavailable": "目前無法讀取 Portfolio 紀錄。",
  "safeError.portfolio.fan_out_state_invalid": "主機發現 Portfolio 狀態不一致，已在寫入前停止。",
  "safeError.portfolio.operation_ordinal_exhausted": "主機已無法再記錄這類操作。",
  "safeError.portfolio.audit_id_unavailable": "主機無法產生這次操作所需的稽核識別碼。",
  "safeError.portfolio.prepared_intent_id_unavailable": "主機無法產生這個預覽所需的識別碼。",
  "safeError.portfolio.approval_receipt_id_unavailable": "主機無法產生這次核准所需的識別碼。",
  "safeError.portfolio.classification_lowering_invalid": "這個 Portfolio 的分級不能這樣降低。",
  "safeError.portfolio.classification_lowering_not_a_lowering":
    "提出的分級並不低於這個 Portfolio 目前的分級。",
  "safeError.portfolio.classification_lowering_preview_changed":
    "準備預覽之後這個 Portfolio 已被更改，請重新準備。",
  "safeError.portfolio.classification_lowering_approval_mismatch": "這個核准對應的是另一個預覽。",
  "safeError.portfolio.classification_lowering_approval_denied":
    "降低這個 Portfolio 的分級沒有獲得核准。",

  "safeError.product.not_found": "找不到這個 Product。",
  "safeError.product.already_exists": "已經有相同識別碼的 Product。",
  "safeError.product.stale_version": "這個 Product 在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.product.version_exhausted": "這個 Product 的版本號已達上限。",
  "safeError.product.prepared_intent_id_unavailable": "主機無法產生這個預覽所需的識別碼。",
  "safeError.product.approval_receipt_id_unavailable": "主機無法產生這次核准所需的識別碼。",
  "safeError.product.classification_lowering_invalid": "這個 Product 的分級不能這樣降低。",
  "safeError.product.classification_lowering_not_a_lowering":
    "提出的分級並不低於這個 Product 目前的分級。",
  "safeError.product.classification_lowering_preview_changed":
    "準備預覽之後這個 Product 已被更改，請重新準備。",
  "safeError.product.classification_lowering_approval_mismatch": "這個核准對應的是另一個預覽。",
  "safeError.product.classification_lowering_approval_denied":
    "降低這個 Product 的分級沒有獲得核准。",

  "safeError.roadmap.not_found": "找不到這個 Roadmap。",
  "safeError.roadmap.already_exists": "已經有相同識別碼的 Roadmap。",
  "safeError.roadmap.stale_version": "這個 Roadmap 在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.roadmap.version_exhausted": "這個 Roadmap 的版本號已達上限。",
  "safeError.roadmap.prepared_intent_id_unavailable": "主機無法產生這個預覽所需的識別碼。",
  "safeError.roadmap.approval_receipt_id_unavailable": "主機無法產生這次核准所需的識別碼。",
  "safeError.roadmap.classification_lowering_invalid": "這個 Roadmap 的分級不能這樣降低。",
  "safeError.roadmap.classification_lowering_not_a_lowering":
    "提出的分級並不低於這個 Roadmap 目前的分級。",
  "safeError.roadmap.classification_lowering_preview_changed":
    "準備預覽之後這個 Roadmap 已被更改，請重新準備。",
  "safeError.roadmap.classification_lowering_approval_mismatch": "這個核准對應的是另一個預覽。",
  "safeError.roadmap.classification_lowering_approval_denied":
    "降低這個 Roadmap 的分級沒有獲得核准。",

  "safeError.kpi.not_found": "找不到這個 KPI。",
  "safeError.kpi.already_exists": "已經有相同識別碼的 KPI。",
  "safeError.kpi.stale_version": "這個 KPI 在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.kpi.version_exhausted": "這個 KPI 的版本號已達上限。",
  "safeError.kpi.prepared_intent_id_unavailable": "主機無法產生這個預覽所需的識別碼。",
  "safeError.kpi.approval_receipt_id_unavailable": "主機無法產生這次核准所需的識別碼。",
  "safeError.kpi.classification_lowering_invalid": "這個 KPI 的分級不能這樣降低。",
  "safeError.kpi.classification_lowering_not_a_lowering": "提出的分級並不低於這個 KPI 目前的分級。",
  "safeError.kpi.classification_lowering_preview_changed":
    "準備預覽之後這個 KPI 已被更改，請重新準備。",
  "safeError.kpi.classification_lowering_approval_mismatch": "這個核准對應的是另一個預覽。",
  "safeError.kpi.classification_lowering_approval_denied": "降低這個 KPI 的分級沒有獲得核准。",

  "safeError.kpi.observation.not_found": "找不到這筆 KPI 觀測。",
  "safeError.kpi.observation.already_exists": "已經有相同識別碼的 KPI 觀測。",
  "safeError.kpi.observation.stale_version":
    "這筆 KPI 觀測在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.kpi.observation.version_exhausted": "這筆 KPI 觀測的版本號已達上限。",
  "safeError.kpi.observation.prepared_intent_id_unavailable": "主機無法產生這個預覽所需的識別碼。",
  "safeError.kpi.observation.approval_receipt_id_unavailable": "主機無法產生這次核准所需的識別碼。",
  "safeError.kpi.observation.classification_lowering_invalid": "這筆 KPI 觀測的分級不能這樣降低。",
  "safeError.kpi.observation.classification_lowering_not_a_lowering":
    "提出的分級並不低於這筆 KPI 觀測目前的分級。",
  "safeError.kpi.observation.classification_lowering_preview_changed":
    "準備預覽之後這筆 KPI 觀測已被更改，請重新準備。",
  "safeError.kpi.observation.classification_lowering_approval_mismatch":
    "這個核准對應的是另一個預覽。",
  "safeError.kpi.observation.classification_lowering_approval_denied":
    "降低這筆 KPI 觀測的分級沒有獲得核准。",

  "safeError.delivery.not_found": "找不到這個 Initiative、Project 或 Milestone。",
  "safeError.delivery.already_exists": "已經有相同識別碼的紀錄。",
  "safeError.delivery.stale_version": "這筆紀錄在你讀取之後已被更改，請重新讀取後再試。",
  "safeError.delivery.version_exhausted": "這筆紀錄的版本號已達上限。",
  "safeError.delivery.conflict": "這個操作與紀錄目前的狀態衝突，請重新讀取後再試。",
  "safeError.delivery.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.delivery.persistence_failed": "Product Ledger 無法儲存這筆資料。",
  "safeError.delivery.invalid_period": "開始日期晚於結束日期。",
  "safeError.delivery.preview_changed": "預覽已逾期，或內容在你確認之前已變更；請重新準備。",
  "safeError.delivery.validation.invalid_field": "有欄位的值不被允許。",
  "safeError.delivery.validation.invalid_text": "有文字欄位是空的或太長。",
  "safeError.delivery.validation.invalid_period": "開始日期晚於結束日期。",
  "safeError.delivery.classification.lowering_denied": "不允許降低這個分級。",
  "safeError.delivery.classification_lowering_invalid": "這筆紀錄的分級不能這樣降低。",
  "safeError.delivery.classification_lowering_not_a_lowering": "提出的分級並非降級。",
  "safeError.delivery.classification_lowering_preview_changed":
    "準備預覽之後這筆紀錄已被更改，請重新準備。",

  "safeError.relationship.not_found": "找不到這個關係。",
  "safeError.relationship.conflict": "這個操作與關係目前的狀態衝突，請重新讀取後再試。",
  "safeError.relationship.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.relationship.persistence_failed": "Product Ledger 無法儲存這個關係。",
  "safeError.relationship.milestone_subject_not_supported": "Milestone 不能作為這類關係的對象。",
  "safeError.relationship.classification.unclassified_or_lowering_denied":
    "請選擇一個不低於它所連結紀錄的分級。",
  "safeError.relationship.removal.authorization_denied": "移除這個關係未獲授權。",
  "safeError.relationship.removal.policy_denied": "政策不允許移除這個關係。",
  "safeError.relationship.removal.confirmation_mismatch":
    "確認文字不符，請照畫面上的文字完整輸入。",
  "safeError.relationship.removal.preview_expired_or_changed":
    "預覽已逾期，或內容在你確認之前已變更；請重新準備。",
  "safeError.relationship.removal.too_late_to_cancel": "這個移除已經核准，無法取消。",

  "safeError.projection.idempotency_conflict": "同一個請求識別碼已被用於不同的操作。",
  "safeError.projection.persistence_failed": "Product Ledger 無法儲存這個投影。",
  "safeError.projection.rebuild_operation_not_found": "找不到這次投影重建。",
  "safeError.projection.rebuild_operation_terminal": "這次投影重建已經結束。",
  "safeError.projection.rebuild_prepared_intent_not_found": "找不到已準備的重建，請重新準備。",
  "safeError.projection.rebuild_prepared_intent_consumed": "這個已準備的重建已經用過，請重新準備。",
  "safeError.projection.rebuild_prepared_intent_mismatch": "這個核准對應的是另一個已準備的重建。",
  "safeError.projection.rebuild_preview_changed": "重建預覽在你確認之前已變更，請重新準備。",
  "safeError.projection.rebuild_preview_expired": "重建預覽已逾期，請重新準備。",
  "safeError.projection.rebuild_digest_mismatch": "準備之後重建內容已有變更，請重新準備。",
  "safeError.projection.rebuild_missing_confirmation": "投影重建需要你的確認。",
  "safeError.projection.rebuild_unauthorized_actor": "投影的操作未獲授權。",
  "safeError.projection.rebuild_h1_auto_not_permitted": "這次重建需要審閱，不能自動執行。",
  "safeError.projection.rebuild_publication_in_flight":
    "另一次投影發布仍在進行中，請等它完成後再試。",
  "safeError.projection.rebuild_empty_change_set": "沒有需要重建的內容。",
  "safeError.projection.rebuild_changes_not_canonical":
    "計畫中的變更不符合預期格式，沒有進行重建。",
  "safeError.projection.rebuild_unplanned_item": "重建時發現不在計畫中的項目，已停止。",
  "safeError.projection.rebuild_incomplete_report": "重建報告不完整，無法確認。",

  "field.delivery.name": "名稱",
  "field.initiative.defined_outcome": "預期成果",
  "field.milestone.verification_criteria": "驗證標準",
  "field.project.time_range": "時間範圍",

  "nextStep.issue.prepare_resolve": "準備解決。",
  "nextStep.issue.prepare_close_or_reopen": "準備關閉或重新開啟。",
  "nextStep.issue.no_transition": "已關閉的 Issue 沒有下一步。",
  "nextStep.issue.refresh_and_reprepare": "重新讀取後再準備一次。",
  "nextStep.risk.update_response_or_prepare_transition": "更新應對方式，或準備狀態轉移。",
  "nextStep.risk.no_transition": "這個 Risk 沒有下一步。",
  "nextStep.risk.refresh_and_reprepare": "重新讀取後再準備一次。",
  "safeError.desktop.backup_due": "備份已到期。請先完成備份，再試一次。",
  "safeError.desktop.backup_running": "正在備份。請等備份完成後再試一次。",
  "safeError.desktop.backup_destination_not_set": "請先選擇備份資料夾。",
  "safeError.desktop.backup_destination_unavailable":
    "無法存取備份資料夾。請接上磁碟，或選擇其他資料夾。",
  "safeError.desktop.backup_passphrase_required": "請先設定復原密語。",
  "safeError.desktop.backup_verification_failed":
    "這份備份沒有通過檢查，所以沒有保留。請再試一次。",
  "safeError.desktop.backup_failed": "備份沒有完成。請再試一次。",
  "safeError.desktop.restore_ledger_locked":
    "目前的 Ledger 檔案被其他程式開著。請關閉那個程式後再試一次。沒有任何變更。",
  "safeError.desktop.restore_preservation_failed":
    "PMC 無法保存並核對目前的 Ledger 檔案。沒有任何變更，還原無法繼續。",
  "safeError.desktop.restore_state_unreadable":
    "PMC 讀不到還原狀態紀錄，所以不會進行還原。沒有任何變更。",
  "safeError.desktop.restore_recovery_backup_missing":
    "找不到那份復原用備份：它已不在備份資料夾，或內容被改動過。請選擇其他備份。",
} as const satisfies Record<keyof typeof SAFE_ERRORS_EN, string>;
