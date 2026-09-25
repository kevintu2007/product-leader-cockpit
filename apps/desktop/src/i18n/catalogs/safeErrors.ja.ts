import type { SAFE_ERRORS_EN } from "./safeErrors.en";

/** 日本語のエラーメッセージ、項目名、次の手順。キーは英語版と同じです。 */
export const SAFE_ERRORS_JA = {
  "safeError.desktop.snapshot_unavailable":
    "現在 Product Ledger のスナップショットを読み取れません。",
  "safeError.desktop.snapshot_invalid":
    "Product Ledger のデータが整合性チェックを通過しなかったため、書き込みは行われていません。",
  "safeError.desktop.product_not_found": "この Product が見つかりません。",
  "safeError.desktop.product_detail_unattributable":
    "この Product の詳細に出所を示せない内容があるため、表示していません。",
  "safeError.desktop.invalid_argument":
    "送信されたデータの形式が正しくありません。何も書き込まれていません。",
  "safeError.desktop.action_not_found": "この Action が見つかりません。",
  "safeError.desktop.host_id_source_failed": "この操作に必要な識別子を生成できませんでした。",
  "safeError.desktop.host_identifier_failed": "この操作に必要な識別子を生成できませんでした。",
  "safeError.desktop.unsupported_preview":
    "この画面で表示できない種類のプレビューが準備されたため、表示していません。",
  "safeError.desktop.preview_already_consumed":
    "このプレビューはすでに承認または却下されています。続けるには準備し直してください。",
  "safeError.desktop.evidence_not_found": "この Evidence が見つかりません。",
  "safeError.desktop.evidence_already_pinned":
    "この Evidence のフィンガープリントはすでに固定されています。",
  "safeError.desktop.evidence_version_conflict":
    "この Evidence は読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.desktop.evidence_path_containment_failed":
    "この Evidence のファイルは Product Vault の外にあるため、読み取っていません。",
  "safeError.desktop.evidence_source_not_observable":
    "現在この Evidence のファイルを読み取れません。",
  "safeError.desktop.vault_not_configured":
    "このワークスペースにはまだ Product Vault が設定されていません。設定で選んでください。",
  "safeError.desktop.vault_root_unavailable":
    "現在 Product Vault のフォルダーにアクセスできません。",
  "safeError.desktop.settings_unavailable":
    "現在、表示設定を読み込むことも保存することもできません。",
  "safeError.desktop.backup_destination_unusable":
    "このフォルダーにはバックアップを保存できません。このコンピューター上にある、書き込み可能なフォルダーを選んでください。ショートカットやリンクは使えません。",
  "safeError.desktop.passphrase_too_short":
    "このパスフレーズは短すぎます。20 文字以上、または 6 単語以上にしてください。",
  "safeError.desktop.passphrase_too_repetitive":
    "このパスフレーズは繰り返しが多すぎます。もっと異なる単語や文字を使ってください。",
  "safeError.desktop.passphrase_too_long":
    "このパスフレーズは長すぎます。1,024 文字以内にしてください。",
  "safeError.desktop.passphrase_too_common":
    "このパスフレーズはよく使われるパスワードだけでできています。よく使われるパスワードではない単語を使ってください。",
  "safeError.desktop.credential_unavailable":
    "現在、Windows 資格情報マネージャーでパスフレーズを保存または読み取れません。",
  "safeError.desktop.random_unavailable": "パスフレーズの作成に必要な乱数を取得できませんでした。",

  "safeError.ledger.open.unclaimed_database":
    "この Product Ledger ファイルはこのアプリで作成されたものではありません。",
  "safeError.ledger.open.wrong_application":
    "このファイルは Product Mission Control の Ledger ではありません。",
  "safeError.ledger.open.future_schema":
    "この Product Ledger は新しいバージョンのアプリで作成されたため、このバージョンでは開けません。",
  "safeError.ledger.open.unsupported_schema":
    "このバージョンのアプリは、この Product Ledger のスキーマバージョンに対応していません。",
  "safeError.ledger.open.invalid_metadata": "Product Ledger のメタデータに不整合があります。",
  "safeError.ledger.open.corrupt_database": "Product Ledger ファイルが破損しています。",
  "safeError.ledger.open.policy_violation":
    "Product Ledger の接続設定がセキュリティポリシーを満たしていません。",
  "safeError.ledger.open.busy": "Product Ledger は現在使用中です。",
  "safeError.ledger.open.storage_unavailable": "ストレージデバイスを使用できません。",
  "safeError.ledger.transaction.revision_conflict":
    "読み込んだ後にデータが変更されました。読み込み直してから再度お試しください。",
  "safeError.ledger.transaction.incompatible_ledger":
    "Product Ledger はこのバージョンのアプリと互換性がありません。",
  "safeError.ledger.transaction.busy": "Product Ledger は現在使用中です。",
  "safeError.ledger.transaction.commit_failed":
    "書き込みが完了しませんでした。保存されたかどうかは確認できません。",
  "safeError.ledger.commit_failed":
    "書き込みが完了しませんでした。保存されたかどうかは確認できません。",
  "safeError.ledger.persistence_failed": "Product Ledger に保存できませんでした。",
  "safeError.ledger.idempotency_conflict": "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.audit.failed": "この操作の監査記録を書き込めなかったため、何も変更していません。",
  "safeError.version.overflow": "このレコードのバージョン番号は上限に達しています。",
  "safeError.classification.lowering_requires_governed_intent":
    "分類を下げるには、専用のレビューと承認の手順が必要です。",

  "safeError.action.not_found": "この Action Request または Action が見つかりません。",
  "safeError.action_request.already_exists": "同じ識別子の Action Request がすでに存在します。",
  "safeError.action.idempotency_conflict": "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.action.approval_denied":
    "この操作は許可されていないか、ポリシーで拒否されたか、Evidence または Judgment が十分ではありません。",
  "safeError.action.preview_expired_or_changed":
    "プレビューの期限が切れたか、確認前に内容が変わりました。準備し直してください。",
  "safeError.action.internal": "この操作の処理中に内部エラーが発生しました。",
  "safeError.action.validation_failed": "必須項目（担当者や期日など）が不足しています。",
  "safeError.action.domain_conflict":
    "レコードの現在の状態またはバージョンと矛盾しています。読み込み直してから再度お試しください。",
  "safeError.action.classification_lowering_not_a_lowering":
    "提案された分類は現在の分類より低くありません。",
  "safeError.action.completion_evidence_already_linked":
    "この Evidence はすでにこの Action の完了 Evidence として関連付けられています。",

  "safeError.decision.not_found": "この Decision Request または Decision が見つかりません。",
  "safeError.decision.already_exists": "同じ識別子の Decision がすでに存在します。",
  "safeError.decision.conflict":
    "Decision Request の現在の状態またはバージョンと矛盾しています。読み込み直してから再度お試しください。",
  "safeError.decision.request_transition_conflict":
    "Decision Request の現在の状態ではこの操作はできません。読み込み直してから再度お試しください。",
  "safeError.decision.approval_denied":
    "この操作は許可されていないか、Evidence または Judgment が十分ではありません。",
  "safeError.decision.preview_changed":
    "プレビューの期限が切れたか、確認前に内容が変わりました。準備し直してください。",
  "safeError.decision.internal": "この操作の処理中に内部エラーが発生しました。",
  "safeError.decision.classification_lowering_not_a_lowering":
    "提案された分類は現在の分類より低くありません。",

  "safeError.risk.not_found": "この Risk が見つかりません。",
  "safeError.risk.owner_not_found": "所有者として指定した Stakeholder が見つかりません。",
  "safeError.risk.next_review_before_epoch": "次回レビュー日は 1970 年より前にできません。",
  "safeError.risk.already_exists": "同じ識別子の Risk がすでに存在します。",
  "safeError.risk.classification": "この Risk の分類を選んでください。",
  "safeError.risk.accepted_fields_required":
    "Risk を受容または移転するには、理由と見直し日が必要です。",
  "safeError.risk.conflict":
    "Risk の現在の状態と矛盾しています。読み込み直してから再度お試しください。",
  "safeError.risk.stale_or_illegal":
    "Risk が変更されたか、現在の状態ではこの操作はできません。読み込み直してから再度お試しください。",
  "safeError.risk.idempotency_conflict": "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.risk.preview_changed":
    "プレビューの期限が切れたか、確認前に内容が変わりました。準備し直してください。",
  "safeError.risk.security_denied": "この Risk に対する操作は許可されていません。",
  "safeError.risk.evidence_unavailable": "この操作に必要な Evidence を現在読み取れません。",
  "safeError.risk.infrastructure": "この Risk の処理中に内部エラーが発生しました。",

  "safeError.issue.not_found": "この Issue が見つかりません。",
  "safeError.issue.exists": "同じ識別子の Issue がすでに存在します。",
  "safeError.issue.classification_required": "この Issue の分類を選んでください。",
  "safeError.issue.stale_or_illegal":
    "Issue が変更されたか、現在の状態ではこの操作はできません。読み込み直してから再度お試しください。",
  "safeError.issue.invalid_intent": "この Issue の現在の状態ではこの操作はできません。",
  "safeError.issue.prepared_operation_mismatch":
    "このプレビューは別の操作用に準備されたものです。準備し直してください。",
  "safeError.issue.preview_changed":
    "プレビューの期限が切れたか、確認前に内容が変わりました。準備し直してください。",
  "safeError.issue.idempotency_conflict": "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.issue.recurrence_not_supported": "この Issue は再発として記録できません。",
  "safeError.issue.security_denied": "この Issue に対する操作は許可されていません。",
  "safeError.issue.infrastructure": "この Issue の処理中に内部エラーが発生しました。",
  "safeError.issue.classification_lowering_not_a_lowering":
    "提案された分類は現在の分類より低くありません。",

  "safeError.evidence.not_found": "この Evidence が見つかりません。",
  "safeError.evidence_reference.not_found": "この Evidence が見つかりません。",
  "safeError.evidence.already_exists": "同じ識別子の Evidence がすでに存在します。",
  "safeError.evidence.path_already_referenced":
    "このファイルを指す別の Evidence 参照が作成されたところです。もう一度ファイルを選ぶと確認できます。",
  "safeError.evidence.idempotency_conflict":
    "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.evidence.persistence_failed":
    "Product Ledger にこの Evidence を保存できませんでした。",
  "safeError.evidence.pin_fingerprint_already_pinned":
    "この Evidence のフィンガープリントはすでに固定されています。",
  "safeError.evidence.pin_path_mismatch":
    "読み取ったファイルがこの Evidence の記録上のパスと一致しないため、フィンガープリントを固定していません。",
  "safeError.evidence.relocation_path_unchanged": "新しい場所は現在の場所と同じです。",
  "safeError.evidence.relocation_path_mismatch":
    "読み取ったファイルが指定された新しい場所と一致しません。",
  "safeError.evidence.relocation_fingerprint_mismatch":
    "新しい場所のファイルはフィンガープリントが異なり、同じファイルではありません。",
  "safeError.evidence.relocation_fingerprint_unpinned":
    "この Evidence にはフィンガープリントが固定されていないため、移動後も同じファイルだと確認できません。",
  "safeError.evidence.supersession_source_mismatch":
    "この置き換えは別の Evidence 用に準備されたものです。",
  "safeError.evidence.supersession_source_already_superseded":
    "この Evidence はすでに置き換えられています。",
  "safeError.evidence.supersession_replacement_is_source":
    "Evidence をそれ自身で置き換えることはできません。",
  "safeError.evidence.supersession_not_a_genuine_replacement":
    "置き換え先は、置き換え元の Evidence と同じファイルです。",
  "safeError.evidence.supersession_lowers_classification":
    "置き換え先の分類を、置き換え元の Evidence より低くすることはできません。",
  "safeError.evidence.supersession_unclassified_replacement":
    "置き換え先の Evidence の分類を選んでください。",
  "safeError.evidence.supersession_missing_confirmation":
    "Evidence を置き換えるには確認が必要です。",
  "safeError.evidence.supersession_unauthorized_actor":
    "この Evidence に対する操作は許可されていません。",
  "safeError.evidence.supersession_prepared_intent_mismatch":
    "この承認は別の準備済みの置き換えに対するものです。",
  "safeError.evidence.supersession_digest_mismatch":
    "準備した後に置き換えの内容が変わりました。準備し直してください。",
  "safeError.evidence.supersession_preview_changed":
    "確認前にプレビューが変わりました。準備し直してください。",
  "safeError.evidence.supersession_expired": "プレビューの期限が切れました。準備し直してください。",

  "safeError.portfolio.not_found": "この Portfolio が見つかりません。",
  "safeError.portfolio.already_exists": "同じ識別子の Portfolio がすでに存在します。",
  "safeError.portfolio.stale_version":
    "この Portfolio は読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.portfolio.version_exhausted": "この Portfolio のバージョン番号は上限に達しています。",
  "safeError.portfolio.idempotency_conflict":
    "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.portfolio.repository_unavailable": "現在 Portfolio のレコードを読み取れません。",
  "safeError.portfolio.fan_out_state_invalid":
    "Portfolio の状態に不整合が見つかったため、書き込む前に中止しました。",
  "safeError.portfolio.operation_ordinal_exhausted": "この種類の操作はこれ以上記録できません。",
  "safeError.portfolio.audit_id_unavailable": "この操作に必要な監査識別子を生成できませんでした。",
  "safeError.portfolio.prepared_intent_id_unavailable":
    "このプレビューに必要な識別子を生成できませんでした。",
  "safeError.portfolio.approval_receipt_id_unavailable":
    "この承認に必要な識別子を生成できませんでした。",
  "safeError.portfolio.classification_lowering_invalid":
    "この Portfolio の分類はこの方法では下げられません。",
  "safeError.portfolio.classification_lowering_not_a_lowering":
    "提案された分類はこの Portfolio の現在の分類より低くありません。",
  "safeError.portfolio.classification_lowering_preview_changed":
    "プレビューを準備した後にこの Portfolio が変更されました。準備し直してください。",
  "safeError.portfolio.classification_lowering_approval_mismatch":
    "この承認は別のプレビューに対するものです。",
  "safeError.portfolio.classification_lowering_approval_denied":
    "この Portfolio の分類を下げることは承認されませんでした。",

  "safeError.product.not_found": "この Product が見つかりません。",
  "safeError.product.already_exists": "同じ識別子の Product がすでに存在します。",
  "safeError.product.stale_version":
    "この Product は読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.product.version_exhausted": "この Product のバージョン番号は上限に達しています。",
  "safeError.product.prepared_intent_id_unavailable":
    "このプレビューに必要な識別子を生成できませんでした。",
  "safeError.product.approval_receipt_id_unavailable":
    "この承認に必要な識別子を生成できませんでした。",
  "safeError.product.classification_lowering_invalid":
    "この Product の分類はこの方法では下げられません。",
  "safeError.product.classification_lowering_not_a_lowering":
    "提案された分類はこの Product の現在の分類より低くありません。",
  "safeError.product.classification_lowering_preview_changed":
    "プレビューを準備した後にこの Product が変更されました。準備し直してください。",
  "safeError.product.classification_lowering_approval_mismatch":
    "この承認は別のプレビューに対するものです。",
  "safeError.product.classification_lowering_approval_denied":
    "この Product の分類を下げることは承認されませんでした。",

  "safeError.roadmap.not_found": "この Roadmap が見つかりません。",
  "safeError.roadmap.already_exists": "同じ識別子の Roadmap がすでに存在します。",
  "safeError.roadmap.stale_version":
    "この Roadmap は読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.roadmap.version_exhausted": "この Roadmap のバージョン番号は上限に達しています。",
  "safeError.roadmap.prepared_intent_id_unavailable":
    "このプレビューに必要な識別子を生成できませんでした。",
  "safeError.roadmap.approval_receipt_id_unavailable":
    "この承認に必要な識別子を生成できませんでした。",
  "safeError.roadmap.classification_lowering_invalid":
    "この Roadmap の分類はこの方法では下げられません。",
  "safeError.roadmap.classification_lowering_not_a_lowering":
    "提案された分類はこの Roadmap の現在の分類より低くありません。",
  "safeError.roadmap.classification_lowering_preview_changed":
    "プレビューを準備した後にこの Roadmap が変更されました。準備し直してください。",
  "safeError.roadmap.classification_lowering_approval_mismatch":
    "この承認は別のプレビューに対するものです。",
  "safeError.roadmap.classification_lowering_approval_denied":
    "この Roadmap の分類を下げることは承認されませんでした。",

  "safeError.kpi.not_found": "この KPI が見つかりません。",
  "safeError.kpi.already_exists": "同じ識別子の KPI がすでに存在します。",
  "safeError.kpi.stale_version":
    "この KPI は読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.kpi.version_exhausted": "この KPI のバージョン番号は上限に達しています。",
  "safeError.kpi.prepared_intent_id_unavailable":
    "このプレビューに必要な識別子を生成できませんでした。",
  "safeError.kpi.approval_receipt_id_unavailable": "この承認に必要な識別子を生成できませんでした。",
  "safeError.kpi.classification_lowering_invalid": "この KPI の分類はこの方法では下げられません。",
  "safeError.kpi.classification_lowering_not_a_lowering":
    "提案された分類はこの KPI の現在の分類より低くありません。",
  "safeError.kpi.classification_lowering_preview_changed":
    "プレビューを準備した後にこの KPI が変更されました。準備し直してください。",
  "safeError.kpi.classification_lowering_approval_mismatch":
    "この承認は別のプレビューに対するものです。",
  "safeError.kpi.classification_lowering_approval_denied":
    "この KPI の分類を下げることは承認されませんでした。",

  "safeError.kpi.observation.not_found": "この KPI 観測値が見つかりません。",
  "safeError.kpi.observation.already_exists": "同じ識別子の KPI 観測値がすでに存在します。",
  "safeError.kpi.observation.stale_version":
    "この KPI 観測値は読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.kpi.observation.version_exhausted":
    "この KPI 観測値のバージョン番号は上限に達しています。",
  "safeError.kpi.observation.prepared_intent_id_unavailable":
    "このプレビューに必要な識別子を生成できませんでした。",
  "safeError.kpi.observation.approval_receipt_id_unavailable":
    "この承認に必要な識別子を生成できませんでした。",
  "safeError.kpi.observation.classification_lowering_invalid":
    "この KPI 観測値の分類はこの方法では下げられません。",
  "safeError.kpi.observation.classification_lowering_not_a_lowering":
    "提案された分類はこの KPI 観測値の現在の分類より低くありません。",
  "safeError.kpi.observation.classification_lowering_preview_changed":
    "プレビューを準備した後にこの KPI 観測値が変更されました。準備し直してください。",
  "safeError.kpi.observation.classification_lowering_approval_mismatch":
    "この承認は別のプレビューに対するものです。",
  "safeError.kpi.observation.classification_lowering_approval_denied":
    "この KPI 観測値の分類を下げることは承認されませんでした。",

  "safeError.delivery.not_found": "この Initiative、Project、または Milestone が見つかりません。",
  "safeError.delivery.already_exists": "同じ識別子のレコードがすでに存在します。",
  "safeError.delivery.stale_version":
    "このレコードは読み込んだ後に変更されました。読み込み直してから再度お試しください。",
  "safeError.delivery.version_exhausted": "このレコードのバージョン番号は上限に達しています。",
  "safeError.delivery.conflict":
    "レコードの現在の状態と矛盾しています。読み込み直してから再度お試しください。",
  "safeError.delivery.idempotency_conflict":
    "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.delivery.persistence_failed": "Product Ledger に保存できませんでした。",
  "safeError.delivery.invalid_period": "開始日が終了日より後になっています。",
  "safeError.delivery.preview_changed":
    "プレビューの期限が切れたか、確認前に内容が変わりました。準備し直してください。",
  "safeError.delivery.validation.invalid_field": "許可されていない値が入力された項目があります。",
  "safeError.delivery.validation.invalid_text": "空欄または長すぎるテキスト項目があります。",
  "safeError.delivery.validation.invalid_period": "開始日が終了日より後になっています。",
  "safeError.delivery.classification.lowering_denied": "この分類を下げることは許可されていません。",
  "safeError.delivery.classification_lowering_invalid":
    "このレコードの分類はこの方法では下げられません。",
  "safeError.delivery.classification_lowering_not_a_lowering":
    "提案された分類は現在の分類より低くありません。",
  "safeError.delivery.classification_lowering_preview_changed":
    "プレビューを準備した後にこのレコードが変更されました。準備し直してください。",

  "safeError.relationship.not_found": "この関係が見つかりません。",
  "safeError.relationship.conflict":
    "関係の現在の状態と矛盾しています。読み込み直してから再度お試しください。",
  "safeError.relationship.idempotency_conflict":
    "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.relationship.persistence_failed": "Product Ledger にこの関係を保存できませんでした。",
  "safeError.relationship.milestone_subject_not_supported":
    "Milestone はこの種類の関係の対象にできません。",
  "safeError.relationship.classification.unclassified_or_lowering_denied":
    "関連付けるレコードより低くない分類を選んでください。",
  "safeError.relationship.removal.authorization_denied": "この関係の削除は許可されていません。",
  "safeError.relationship.removal.policy_denied": "ポリシーにより、この関係は削除できません。",
  "safeError.relationship.removal.confirmation_mismatch":
    "確認の入力が一致しません。表示どおりに正確に入力してください。",
  "safeError.relationship.removal.preview_expired_or_changed":
    "プレビューの期限が切れたか、確認前に内容が変わりました。準備し直してください。",
  "safeError.relationship.removal.too_late_to_cancel":
    "この削除はすでに承認されているため、取り消せません。",

  "safeError.projection.idempotency_conflict":
    "このリクエスト識別子はすでに別の操作で使われています。",
  "safeError.projection.persistence_failed":
    "Product Ledger にこのプロジェクションを保存できませんでした。",
  "safeError.projection.rebuild_operation_not_found":
    "このプロジェクションの再構築が見つかりません。",
  "safeError.projection.rebuild_operation_terminal":
    "このプロジェクションの再構築はすでに終了しています。",
  "safeError.projection.rebuild_prepared_intent_not_found":
    "準備済みの再構築が見つかりません。準備し直してください。",
  "safeError.projection.rebuild_prepared_intent_consumed":
    "この準備済みの再構築はすでに使われています。準備し直してください。",
  "safeError.projection.rebuild_prepared_intent_mismatch":
    "この承認は別の準備済みの再構築に対するものです。",
  "safeError.projection.rebuild_preview_changed":
    "確認前に再構築のプレビューが変わりました。準備し直してください。",
  "safeError.projection.rebuild_preview_expired":
    "再構築のプレビューの期限が切れました。準備し直してください。",
  "safeError.projection.rebuild_digest_mismatch":
    "準備した後に再構築の内容が変わりました。準備し直してください。",
  "safeError.projection.rebuild_missing_confirmation":
    "プロジェクションの再構築には確認が必要です。",
  "safeError.projection.rebuild_unauthorized_actor":
    "プロジェクションに対する操作は許可されていません。",
  "safeError.projection.rebuild_h1_auto_not_permitted":
    "この再構築にはレビューが必要なため、自動では実行できません。",
  "safeError.projection.rebuild_publication_in_flight":
    "別のプロジェクションの公開がまだ実行中です。完了してから再度お試しください。",
  "safeError.projection.rebuild_empty_change_set": "再構築する内容がありません。",
  "safeError.projection.rebuild_changes_not_canonical":
    "予定された変更が想定の形式ではないため、再構築していません。",
  "safeError.projection.rebuild_unplanned_item":
    "再構築中に計画にない項目が見つかったため、中止しました。",
  "safeError.projection.rebuild_incomplete_report":
    "再構築のレポートが不完全なため、確認できません。",

  "field.delivery.name": "名前",
  "field.initiative.defined_outcome": "定義された成果",
  "field.milestone.verification_criteria": "検証基準",
  "field.project.time_range": "期間",

  "nextStep.issue.prepare_resolve": "解決を準備してください。",
  "nextStep.issue.prepare_close_or_reopen": "クローズまたは再オープンを準備してください。",
  "nextStep.issue.no_transition": "クローズ済みの Issue に次の手順はありません。",
  "nextStep.issue.refresh_and_reprepare": "読み込み直してから、もう一度準備してください。",
  "nextStep.risk.update_response_or_prepare_transition":
    "対応方針を更新するか、状態遷移を準備してください。",
  "nextStep.risk.no_transition": "この Risk に次の手順はありません。",
  "nextStep.risk.refresh_and_reprepare": "読み込み直してから、もう一度準備してください。",
  "safeError.desktop.backup_due":
    "バックアップの期限です。先にバックアップしてから、もう一度お試しください。",
  "safeError.desktop.backup_running": "バックアップ中です。終わってからもう一度お試しください。",
  "safeError.desktop.backup_destination_not_set": "先にバックアップ先フォルダーを選んでください。",
  "safeError.desktop.backup_destination_unavailable":
    "バックアップ先フォルダーにアクセスできません。ドライブを接続するか、別のフォルダーを選んでください。",
  "safeError.desktop.backup_passphrase_required": "先に復元パスフレーズを設定してください。",
  "safeError.desktop.backup_verification_failed":
    "バックアップが検証に通らなかったため、保存しませんでした。もう一度お試しください。",
  "safeError.desktop.backup_failed": "バックアップが完了しませんでした。もう一度お試しください。",
  "safeError.desktop.restore_ledger_locked":
    "現在の Ledger ファイルを別のプログラムが開いています。そのプログラムを閉じてから、もう一度お試しください。何も変更されていません。",
  "safeError.desktop.restore_preservation_failed":
    "PMC は現在の Ledger ファイルを保存して確認できませんでした。何も変更されていません。復元は続行できません。",
  "safeError.desktop.restore_state_unreadable":
    "PMC は復元の状態記録を読み取れないため、復元を行いません。何も変更されていません。",
  "safeError.desktop.restore_recovery_backup_missing":
    "復旧用バックアップがバックアップ先フォルダーにないか、内容が変更されています。別のバックアップを選択してください。",
} as const satisfies Record<keyof typeof SAFE_ERRORS_EN, string>;
