import type { SAFE_ERRORS_EN } from "./safeErrors.en";

/** 简体中文的错误消息、字段名称与下一步。键与英文完全相同。 */
export const SAFE_ERRORS_ZH_CN = {
  "safeError.desktop.snapshot_unavailable": "目前无法读取 Product Ledger 的快照。",
  "safeError.desktop.snapshot_invalid":
    "Product Ledger 的数据没有通过一致性验证，界面拒绝在它之上写入。",
  "safeError.desktop.product_not_found": "找不到这个 Product。",
  "safeError.desktop.product_detail_unattributable":
    "Product 详情有一条无法标明来源的内容，界面拒绝显示未经溯源的数据。",
  "safeError.desktop.invalid_argument": "提交的数据格式不正确，界面没有写入任何内容。",
  "safeError.desktop.action_not_found": "找不到这个 Action。",
  "safeError.desktop.host_id_source_failed": "主机无法生成这次操作所需的标识符。",
  "safeError.desktop.host_identifier_failed": "主机无法生成这次操作所需的标识符。",
  "safeError.desktop.unsupported_preview": "主机准备了界面无法呈现的预览类型，已拒绝显示。",
  "safeError.desktop.preview_already_consumed": "这个预览已经被批准或拒绝；要继续请重新准备。",
  "safeError.desktop.evidence_not_found": "找不到这份 Evidence。",
  "safeError.desktop.evidence_already_pinned": "这份 Evidence 的指纹已经固定过了。",
  "safeError.desktop.evidence_version_conflict":
    "这份 Evidence 在你读取之后已被更改，请重新读取后再试。",
  "safeError.desktop.evidence_path_containment_failed":
    "这份 Evidence 的文件不在 Product Vault 里，所以没有读取。",
  "safeError.desktop.evidence_source_not_observable": "目前读不到这份 Evidence 的文件。",
  "safeError.desktop.vault_not_configured": "这个工作区还没有设置 Product Vault，请到设置里选择。",
  "safeError.desktop.vault_root_unavailable": "目前无法存取 Product Vault 文件夹。",
  "safeError.desktop.settings_unavailable": "目前无法读取或保存显示设置。",
  "safeError.desktop.backup_destination_unusable":
    "这个文件夹不能存放备份。请选择这台电脑上已存在、你可以写入的文件夹，不能是快捷方式或链接。",
  "safeError.desktop.passphrase_too_short": "这组密码太短。请至少使用 20 个字符或 6 个单词。",
  "safeError.desktop.passphrase_too_repetitive": "这组密码重复太多。请使用更多不同的单词或字符。",
  "safeError.desktop.passphrase_too_long": "这组密码太长。最多 1,024 个字符。",
  "safeError.desktop.passphrase_too_common":
    "这组密码只由非常常见的密码组成。请改用不是常见密码的单词。",
  "safeError.desktop.credential_unavailable": "Windows 凭据管理器目前无法保存或读取这组密码。",
  "safeError.desktop.random_unavailable": "应用程序无法获取生成密码所需的随机数。",

  "safeError.ledger.open.unclaimed_database": "Product Ledger 文件尚未由本应用程序创建。",
  "safeError.ledger.open.wrong_application": "这个文件不是 Product Mission Control 的 Ledger。",
  "safeError.ledger.open.future_schema":
    "Product Ledger 来自较新的版本，这个版本的应用程序无法打开。",
  "safeError.ledger.open.unsupported_schema":
    "Product Ledger 的结构版本不受这个版本的应用程序支持。",
  "safeError.ledger.open.invalid_metadata": "Product Ledger 的元数据不一致。",
  "safeError.ledger.open.corrupt_database": "Product Ledger 文件已损坏。",
  "safeError.ledger.open.policy_violation": "Product Ledger 的连线设置不符合安全政策。",
  "safeError.ledger.open.busy": "Product Ledger 目前忙碌中。",
  "safeError.ledger.open.storage_unavailable": "存储设备目前无法使用。",
  "safeError.ledger.transaction.revision_conflict": "数据在你读取之后已被更改，请重新读取后再试。",
  "safeError.ledger.transaction.incompatible_ledger": "Product Ledger 与这个版本的应用程序不兼容。",
  "safeError.ledger.transaction.busy": "Product Ledger 目前忙碌中。",
  "safeError.ledger.transaction.commit_failed": "写入未能完成；系统无法确认是否已保存。",
  "safeError.ledger.commit_failed": "写入未能完成；系统无法确认是否已保存。",
  "safeError.ledger.persistence_failed": "Product Ledger 无法保存这条数据。",
  "safeError.ledger.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.audit.failed": "这次操作的稽核记录无法写入，所以没有做任何变更。",
  "safeError.version.overflow": "这条记录的版本号已达上限。",
  "safeError.classification.lowering_requires_governed_intent":
    "降低分级必须经过独立的审核与批准步骤。",

  "safeError.action.not_found": "找不到这个 Action Request 或 Action。",
  "safeError.action_request.already_exists": "已经有相同标识符的 Action Request。",
  "safeError.action.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.action.approval_denied":
    "这个操作未获授权、被政策拒绝，或 Evidence／Judgment 不足以支持它。",
  "safeError.action.preview_expired_or_changed":
    "预览已逾期，或内容在你确认之前已变更；请重新准备。",
  "safeError.action.internal": "主机处理这个操作时发生内部错误。",
  "safeError.action.validation_failed": "这个操作缺少必要的字段（例如负责人或到期日）。",
  "safeError.action.domain_conflict": "这个操作与记录目前的状态或版本冲突，请重新读取后再试。",
  "safeError.action.classification_lowering_not_a_lowering": "提出的分级并非降级。",
  "safeError.action.completion_evidence_already_linked":
    "这份 Evidence 已经关联为这条 Action 的完成证据。",

  "safeError.decision.not_found": "找不到这个 Decision Request 或 Decision。",
  "safeError.decision.already_exists": "已经有相同标识符的 Decision。",
  "safeError.decision.conflict":
    "这个操作与 Decision Request 目前的状态或版本冲突，请重新读取后再试。",
  "safeError.decision.request_transition_conflict":
    "Decision Request 目前的状态不允许这个操作，请重新读取后再试。",
  "safeError.decision.approval_denied": "这个操作未获授权、或 Evidence／Judgment 不足以支持它。",
  "safeError.decision.preview_changed": "预览已逾期，或内容在你确认之前已变更；请重新准备。",
  "safeError.decision.internal": "主机处理这个操作时发生内部错误。",
  "safeError.decision.classification_lowering_not_a_lowering": "提出的分级并非降级。",

  "safeError.risk.not_found": "找不到这个 Risk。",
  "safeError.risk.owner_not_found": "找不到指定为负责人的 Stakeholder。",
  "safeError.risk.next_review_before_epoch": "下次检视日期不能早于 1970 年。",
  "safeError.risk.already_exists": "已经有相同标识符的 Risk。",
  "safeError.risk.classification": "请为这个 Risk 选择分级。",
  "safeError.risk.accepted_fields_required": "接受或转移 Risk 需要填写理由与复查日期。",
  "safeError.risk.conflict": "这个操作与 Risk 目前的状态冲突，请重新读取后再试。",
  "safeError.risk.stale_or_illegal":
    "Risk 已被更改，或目前的状态不允许这个操作；请重新读取后再试。",
  "safeError.risk.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.risk.preview_changed": "预览已逾期，或内容在你确认之前已变更；请重新准备。",
  "safeError.risk.security_denied": "这个 Risk 的操作未获授权。",
  "safeError.risk.evidence_unavailable": "目前读不到这个操作需要的 Evidence。",
  "safeError.risk.infrastructure": "主机处理这个 Risk 时发生内部错误。",

  "safeError.issue.not_found": "找不到这个 Issue。",
  "safeError.issue.exists": "已经有相同标识符的 Issue。",
  "safeError.issue.classification_required": "请为这个 Issue 选择分级。",
  "safeError.issue.stale_or_illegal":
    "Issue 已被更改，或目前的状态不允许这个操作；请重新读取后再试。",
  "safeError.issue.invalid_intent": "这个 Issue 目前的状态不允许这个操作。",
  "safeError.issue.prepared_operation_mismatch": "这个预览是为另一个操作准备的，请重新准备。",
  "safeError.issue.preview_changed": "预览已逾期，或内容在你确认之前已变更；请重新准备。",
  "safeError.issue.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.issue.recurrence_not_supported": "这个 Issue 不能记录为再次发生。",
  "safeError.issue.security_denied": "这个 Issue 的操作未获授权。",
  "safeError.issue.infrastructure": "主机处理这个 Issue 时发生内部错误。",
  "safeError.issue.classification_lowering_not_a_lowering": "提出的分级并非降级。",

  "safeError.evidence.not_found": "找不到这份 Evidence。",
  "safeError.evidence_reference.not_found": "找不到这份 Evidence。",
  "safeError.evidence.already_exists": "已经有相同标识符的 Evidence。",
  "safeError.evidence.path_already_referenced":
    "刚刚已有另一条 Evidence 引用指向这个文件。请重新选择文件查看。",
  "safeError.evidence.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.evidence.persistence_failed": "Product Ledger 无法保存这份 Evidence。",
  "safeError.evidence.pin_fingerprint_already_pinned": "这份 Evidence 的指纹已经固定过了。",
  "safeError.evidence.pin_path_mismatch":
    "读到的文件与这份 Evidence 记录的路径不符，没有固定指纹。",
  "safeError.evidence.relocation_path_unchanged": "新的位置与目前的位置相同。",
  "safeError.evidence.relocation_path_mismatch": "读到的文件与你提供的新位置不符。",
  "safeError.evidence.relocation_fingerprint_mismatch": "新位置上的文件指纹不同，不是同一个文件。",
  "safeError.evidence.relocation_fingerprint_unpinned":
    "这份 Evidence 没有固定指纹，无法确认移动后仍是同一个文件。",
  "safeError.evidence.supersession_source_mismatch": "这个取代是为另一份 Evidence 准备的。",
  "safeError.evidence.supersession_source_already_superseded": "这份 Evidence 已经被取代过了。",
  "safeError.evidence.supersession_replacement_is_source": "Evidence 不能取代自己。",
  "safeError.evidence.supersession_not_a_genuine_replacement":
    "用来取代的文件与被取代的 Evidence 是同一个文件。",
  "safeError.evidence.supersession_lowers_classification":
    "用来取代的 Evidence 分级不能低于被取代的 Evidence。",
  "safeError.evidence.supersession_unclassified_replacement": "请为用来取代的 Evidence 选择分级。",
  "safeError.evidence.supersession_missing_confirmation": "取代 Evidence 需要你的确认。",
  "safeError.evidence.supersession_unauthorized_actor": "这份 Evidence 的操作未获授权。",
  "safeError.evidence.supersession_prepared_intent_mismatch":
    "这个批准对应的是另一个已准备的取代。",
  "safeError.evidence.supersession_digest_mismatch": "准备之后取代内容已有变更，请重新准备。",
  "safeError.evidence.supersession_preview_changed": "预览在你确认之前已变更，请重新准备。",
  "safeError.evidence.supersession_expired": "预览已逾期，请重新准备。",

  "safeError.portfolio.not_found": "找不到这个 Portfolio。",
  "safeError.portfolio.already_exists": "已经有相同标识符的 Portfolio。",
  "safeError.portfolio.stale_version": "这个 Portfolio 在你读取之后已被更改，请重新读取后再试。",
  "safeError.portfolio.version_exhausted": "这个 Portfolio 的版本号已达上限。",
  "safeError.portfolio.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.portfolio.repository_unavailable": "目前无法读取 Portfolio 记录。",
  "safeError.portfolio.fan_out_state_invalid": "主机发现 Portfolio 状态不一致，已在写入前停止。",
  "safeError.portfolio.operation_ordinal_exhausted": "主机已无法再记录这类操作。",
  "safeError.portfolio.audit_id_unavailable": "主机无法生成这次操作所需的稽核标识符。",
  "safeError.portfolio.prepared_intent_id_unavailable": "主机无法生成这个预览所需的标识符。",
  "safeError.portfolio.approval_receipt_id_unavailable": "主机无法生成这次批准所需的标识符。",
  "safeError.portfolio.classification_lowering_invalid": "这个 Portfolio 的分级不能这样降低。",
  "safeError.portfolio.classification_lowering_not_a_lowering":
    "提出的分级并不低于这个 Portfolio 目前的分级。",
  "safeError.portfolio.classification_lowering_preview_changed":
    "准备预览之后这个 Portfolio 已被更改，请重新准备。",
  "safeError.portfolio.classification_lowering_approval_mismatch": "这个批准对应的是另一个预览。",
  "safeError.portfolio.classification_lowering_approval_denied":
    "降低这个 Portfolio 的分级没有获得批准。",

  "safeError.product.not_found": "找不到这个 Product。",
  "safeError.product.already_exists": "已经有相同标识符的 Product。",
  "safeError.product.stale_version": "这个 Product 在你读取之后已被更改，请重新读取后再试。",
  "safeError.product.version_exhausted": "这个 Product 的版本号已达上限。",
  "safeError.product.prepared_intent_id_unavailable": "主机无法生成这个预览所需的标识符。",
  "safeError.product.approval_receipt_id_unavailable": "主机无法生成这次批准所需的标识符。",
  "safeError.product.classification_lowering_invalid": "这个 Product 的分级不能这样降低。",
  "safeError.product.classification_lowering_not_a_lowering":
    "提出的分级并不低于这个 Product 目前的分级。",
  "safeError.product.classification_lowering_preview_changed":
    "准备预览之后这个 Product 已被更改，请重新准备。",
  "safeError.product.classification_lowering_approval_mismatch": "这个批准对应的是另一个预览。",
  "safeError.product.classification_lowering_approval_denied":
    "降低这个 Product 的分级没有获得批准。",

  "safeError.roadmap.not_found": "找不到这个 Roadmap。",
  "safeError.roadmap.already_exists": "已经有相同标识符的 Roadmap。",
  "safeError.roadmap.stale_version": "这个 Roadmap 在你读取之后已被更改，请重新读取后再试。",
  "safeError.roadmap.version_exhausted": "这个 Roadmap 的版本号已达上限。",
  "safeError.roadmap.prepared_intent_id_unavailable": "主机无法生成这个预览所需的标识符。",
  "safeError.roadmap.approval_receipt_id_unavailable": "主机无法生成这次批准所需的标识符。",
  "safeError.roadmap.classification_lowering_invalid": "这个 Roadmap 的分级不能这样降低。",
  "safeError.roadmap.classification_lowering_not_a_lowering":
    "提出的分级并不低于这个 Roadmap 目前的分级。",
  "safeError.roadmap.classification_lowering_preview_changed":
    "准备预览之后这个 Roadmap 已被更改，请重新准备。",
  "safeError.roadmap.classification_lowering_approval_mismatch": "这个批准对应的是另一个预览。",
  "safeError.roadmap.classification_lowering_approval_denied":
    "降低这个 Roadmap 的分级没有获得批准。",

  "safeError.kpi.not_found": "找不到这个 KPI。",
  "safeError.kpi.already_exists": "已经有相同标识符的 KPI。",
  "safeError.kpi.stale_version": "这个 KPI 在你读取之后已被更改，请重新读取后再试。",
  "safeError.kpi.version_exhausted": "这个 KPI 的版本号已达上限。",
  "safeError.kpi.prepared_intent_id_unavailable": "主机无法生成这个预览所需的标识符。",
  "safeError.kpi.approval_receipt_id_unavailable": "主机无法生成这次批准所需的标识符。",
  "safeError.kpi.classification_lowering_invalid": "这个 KPI 的分级不能这样降低。",
  "safeError.kpi.classification_lowering_not_a_lowering": "提出的分级并不低于这个 KPI 目前的分级。",
  "safeError.kpi.classification_lowering_preview_changed":
    "准备预览之后这个 KPI 已被更改，请重新准备。",
  "safeError.kpi.classification_lowering_approval_mismatch": "这个批准对应的是另一个预览。",
  "safeError.kpi.classification_lowering_approval_denied": "降低这个 KPI 的分级没有获得批准。",

  "safeError.kpi.observation.not_found": "找不到这条 KPI 观测。",
  "safeError.kpi.observation.already_exists": "已经有相同标识符的 KPI 观测。",
  "safeError.kpi.observation.stale_version":
    "这条 KPI 观测在你读取之后已被更改，请重新读取后再试。",
  "safeError.kpi.observation.version_exhausted": "这条 KPI 观测的版本号已达上限。",
  "safeError.kpi.observation.prepared_intent_id_unavailable": "主机无法生成这个预览所需的标识符。",
  "safeError.kpi.observation.approval_receipt_id_unavailable": "主机无法生成这次批准所需的标识符。",
  "safeError.kpi.observation.classification_lowering_invalid": "这条 KPI 观测的分级不能这样降低。",
  "safeError.kpi.observation.classification_lowering_not_a_lowering":
    "提出的分级并不低于这条 KPI 观测目前的分级。",
  "safeError.kpi.observation.classification_lowering_preview_changed":
    "准备预览之后这条 KPI 观测已被更改，请重新准备。",
  "safeError.kpi.observation.classification_lowering_approval_mismatch":
    "这个批准对应的是另一个预览。",
  "safeError.kpi.observation.classification_lowering_approval_denied":
    "降低这条 KPI 观测的分级没有获得批准。",

  "safeError.delivery.not_found": "找不到这个 Initiative、Project 或 Milestone。",
  "safeError.delivery.already_exists": "已经有相同标识符的记录。",
  "safeError.delivery.stale_version": "这条记录在你读取之后已被更改，请重新读取后再试。",
  "safeError.delivery.version_exhausted": "这条记录的版本号已达上限。",
  "safeError.delivery.conflict": "这个操作与记录目前的状态冲突，请重新读取后再试。",
  "safeError.delivery.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.delivery.persistence_failed": "Product Ledger 无法保存这条数据。",
  "safeError.delivery.invalid_period": "开始日期晚于结束日期。",
  "safeError.delivery.preview_changed": "预览已逾期，或内容在你确认之前已变更；请重新准备。",
  "safeError.delivery.validation.invalid_field": "有字段的值不被允许。",
  "safeError.delivery.validation.invalid_text": "有文字字段是空的或太长。",
  "safeError.delivery.validation.invalid_period": "开始日期晚于结束日期。",
  "safeError.delivery.classification.lowering_denied": "不允许降低这个分级。",
  "safeError.delivery.classification_lowering_invalid": "这条记录的分级不能这样降低。",
  "safeError.delivery.classification_lowering_not_a_lowering": "提出的分级并非降级。",
  "safeError.delivery.classification_lowering_preview_changed":
    "准备预览之后这条记录已被更改，请重新准备。",

  "safeError.relationship.not_found": "找不到这个关系。",
  "safeError.relationship.conflict": "这个操作与关系目前的状态冲突，请重新读取后再试。",
  "safeError.relationship.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.relationship.persistence_failed": "Product Ledger 无法保存这个关系。",
  "safeError.relationship.milestone_subject_not_supported": "Milestone 不能作为这类关系的对象。",
  "safeError.relationship.classification.unclassified_or_lowering_denied":
    "请选择一个不低于它所关联记录的分级。",
  "safeError.relationship.removal.authorization_denied": "移除这个关系未获授权。",
  "safeError.relationship.removal.policy_denied": "政策不允许移除这个关系。",
  "safeError.relationship.removal.confirmation_mismatch":
    "确认文字不符，请照界面上的文字完整输入。",
  "safeError.relationship.removal.preview_expired_or_changed":
    "预览已逾期，或内容在你确认之前已变更；请重新准备。",
  "safeError.relationship.removal.too_late_to_cancel": "这个移除已经批准，无法取消。",

  "safeError.projection.idempotency_conflict": "同一个请求标识符已被用于不同的操作。",
  "safeError.projection.persistence_failed": "Product Ledger 无法保存这个投影。",
  "safeError.projection.rebuild_operation_not_found": "找不到这次投影重建。",
  "safeError.projection.rebuild_operation_terminal": "这次投影重建已经结束。",
  "safeError.projection.rebuild_prepared_intent_not_found": "找不到已准备的重建，请重新准备。",
  "safeError.projection.rebuild_prepared_intent_consumed": "这个已准备的重建已经用过，请重新准备。",
  "safeError.projection.rebuild_prepared_intent_mismatch": "这个批准对应的是另一个已准备的重建。",
  "safeError.projection.rebuild_preview_changed": "重建预览在你确认之前已变更，请重新准备。",
  "safeError.projection.rebuild_preview_expired": "重建预览已逾期，请重新准备。",
  "safeError.projection.rebuild_digest_mismatch": "准备之后重建内容已有变更，请重新准备。",
  "safeError.projection.rebuild_missing_confirmation": "投影重建需要你的确认。",
  "safeError.projection.rebuild_unauthorized_actor": "投影的操作未获授权。",
  "safeError.projection.rebuild_h1_auto_not_permitted": "这次重建需要审核，不能自动执行。",
  "safeError.projection.rebuild_publication_in_flight":
    "另一次投影发布仍在进行中，请等它完成后再试。",
  "safeError.projection.rebuild_empty_change_set": "没有需要重建的内容。",
  "safeError.projection.rebuild_changes_not_canonical":
    "计划中的变更不符合预期格式，没有进行重建。",
  "safeError.projection.rebuild_unplanned_item": "重建时发现不在计划中的事项，已停止。",
  "safeError.projection.rebuild_incomplete_report": "重建报告不完整，无法确认。",

  "field.delivery.name": "名称",
  "field.initiative.defined_outcome": "预期成果",
  "field.milestone.verification_criteria": "验证标准",
  "field.project.time_range": "时间范围",

  "nextStep.issue.prepare_resolve": "准备解决。",
  "nextStep.issue.prepare_close_or_reopen": "准备关闭或重新打开。",
  "nextStep.issue.no_transition": "已关闭的 Issue 没有下一步。",
  "nextStep.issue.refresh_and_reprepare": "重新读取后再准备一次。",
  "nextStep.risk.update_response_or_prepare_transition": "更新应对方式，或准备状态转移。",
  "nextStep.risk.no_transition": "这个 Risk 没有下一步。",
  "nextStep.risk.refresh_and_reprepare": "重新读取后再准备一次。",
  "safeError.desktop.backup_due": "备份已到期。请先完成备份，再试一次。",
  "safeError.desktop.backup_running": "正在备份。请等备份完成后再试一次。",
  "safeError.desktop.backup_destination_not_set": "请先选择备份文件夹。",
  "safeError.desktop.backup_destination_unavailable":
    "无法访问备份文件夹。请连接磁盘，或选择其他文件夹。",
  "safeError.desktop.backup_passphrase_required": "请先设置恢复密语。",
  "safeError.desktop.backup_verification_failed":
    "这份备份没有通过检查，所以没有保留。请再试一次。",
  "safeError.desktop.backup_failed": "备份没有完成。请再试一次。",
  "safeError.desktop.restore_ledger_locked":
    "当前的 Ledger 文件被其他程序打开着。请关闭那个程序后再试一次。没有任何变更。",
  "safeError.desktop.restore_preservation_failed":
    "PMC 无法保存并核对当前的 Ledger 文件。没有任何变更，还原无法继续。",
  "safeError.desktop.restore_state_unreadable":
    "PMC 读不到还原状态记录，所以不会进行还原。没有任何变更。",
  "safeError.desktop.restore_recovery_backup_missing":
    "找不到那份恢复用备份：它已不在备份文件夹，或内容被改动过。请选择其他备份。",
} as const satisfies Record<keyof typeof SAFE_ERRORS_EN, string>;
