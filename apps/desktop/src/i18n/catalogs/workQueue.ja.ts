import type { WORK_QUEUE_EN } from "./workQueue.en";

/** Work Queue の日本語の文言。キーは英語版と同じです。 */
export const WORK_QUEUE_JA = {
  "workQueue.caption.one":
    "承認済みの並び順ポリシーに従っています。全 {count} 件中 {from}〜{to} 件目を表示しています。",
  "workQueue.caption.other":
    "承認済みの並び順ポリシーに従っています。全 {count} 件中 {from}〜{to} 件目を表示しています。",

  "wq.offer.prepareAccept": "受け入れを準備",
  "wq.offer.decline": "辞退",
  "wq.offer.withdraw": "取り下げ",
  "wq.offer.start": "開始",
  "wq.offer.link": "Evidence を関連付け",
  "wq.offer.prepareComplete": "完了を準備",
  "wq.offer.prepareCancel": "キャンセルを準備",
  "wq.offer.prepareReopen": "再オープンを準備",
  "wq.offer.prepareResolve": "解決を準備",
  "wq.offer.prepareOccurrence": "発生の記録を準備",
  "wq.offer.prepareClose": "クローズを準備",

  "wq.noDeadline": "期限の記録なし",
  "wq.noResponseDeadline": "回答期限なし",

  "wq.review.accept.title": "承認して実行：{label} を受け入れる",
  "wq.review.accept.summary":
    "承認すると、確認した要約から Action が作成され、この Request に関連付けられます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.accept.approve": "承認して受け入れる",
  "wq.review.complete.title": "承認して実行：{label} を完了にする",
  "wq.review.complete.summary":
    "承認すると、この Action はプレビューの Evidence／Judgment を根拠に完了となります。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.complete.approve": "承認して完了にする",
  "wq.review.cancel.title": "承認して実行：{label} をキャンセルする",
  "wq.review.cancel.summary":
    "承認すると、この Action は入力した理由でキャンセルされます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.cancel.approve": "承認してキャンセルする",
  "wq.review.reopen.title": "承認して実行：{label} を再オープンする",
  "wq.review.reopen.summary":
    "承認すると、この Action は選んだモードで再オープンされます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.reopen.approve": "承認して再オープンする",
  "wq.review.resolveDecision.title": "承認して実行：{label} を解決する",
  "wq.review.resolveDecision.summary":
    "承認すると、プレビューの Decision と、そこに記載された後続の Action Request がすべて作成されます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.resolveDecision.approve": "承認して解決する",
  "wq.review.occurrence.title": "承認して実行：{label} の発生を記録する",
  "wq.review.occurrence.summary":
    "承認すると、この Risk は発生済みになり、プレビューに記載された Issue {issue} が作成されます（この id はアプリが割り当てたもので、表示されているものがそのまま書き込まれます）。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.occurrence.approve": "承認して発生を記録する",
  "wq.review.closeRisk.title": "承認して実行：{label} をクローズする",
  "wq.review.closeRisk.summary":
    "承認すると、この Risk は入力した理由でクローズされます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.closeRisk.approve": "承認してクローズする",
  "wq.review.resolveIssue.title": "承認して実行：{label} を解決する",
  "wq.review.resolveIssue.summary":
    "承認すると、この Issue はプレビューの Evidence をもとに解決済みになります。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.resolveIssue.approve": "承認して解決する",
  "wq.review.closeIssue.title": "承認して実行：{label} をクローズする",
  "wq.review.closeIssue.summary":
    "承認すると、この Issue はプレビューの検証 Evidence をもとにクローズされます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.closeIssue.approve": "承認してクローズする",
  "wq.review.reopenIssue.title": "承認して実行：{label} を再オープンする",
  "wq.review.reopenIssue.summary":
    "承認すると、この Issue は入力した理由と検証失敗の Evidence をもとに再オープンされます。却下すると記録され、このプレビューは以後承認できません。",
  "wq.review.reopenIssue.approve": "承認して再オープンする",

  "wq.done.declined": "{id} を辞退しました（バージョン {version}）。",
  "wq.done.withdrawn": "{id} を取り下げました（バージョン {version}）。",
  "wq.done.started": "{id} を開始しました（バージョン {version}）。",
  "wq.done.linked": "Evidence {evidence} を {id} に関連付けました（バージョン {version}）。",
  "wq.noWritePath": "このアプリには書き込み経路がありません。",
  "wq.settled.completed": "{id} を完了にしました（バージョン {version}、状態：{state}）。",
  "wq.settled.cancelled": "{id} をキャンセルしました（バージョン {version}、状態：{state}）。",
  "wq.settled.reopened": "{id} を再オープンしました（バージョン {version}、状態：{state}）。",
  "wq.settled.accepted": "Action {action} を作成し、{request} に関連付けました。受領 {receipt}。",
  "wq.settled.occurred":
    "発生を記録し（Risk は現在バージョン {version}）、Issue {issue} を作成しました。",
  "wq.settled.riskClosed": "{id} をクローズしました（バージョン {version}）。",
  "wq.settled.issue": "{id} は現在「{state}」です（バージョン {version}）。",
  "wq.settled.decision.one":
    "Decision {decision} と後続の Action Request {count} 件を作成しました。受領 {receipt}。",
  "wq.settled.decision.other":
    "Decision {decision} と後続の Action Request {count} 件を作成しました。受領 {receipt}。",
  "wq.notice.gone": "{label} は現在の一覧にないため、準備し直していません。",
  "wq.notice.notAcceptable": "{id} はもう受け入れられる状態ではないため、準備し直していません。",
  "wq.notice.rejected":
    "元のプレビューは却下されました。入力し直してから、もう一度準備してください。",
  "wq.notice.held":
    "「{label}」のレビューをいったん閉じました。受け入れも却下もしていません。続けるには、その行の「レビューに戻る」を押してください。プレビューの期限が切れた後は、レビューで準備し直せます。",

  "wq.detail.close": "閉じる",
  "wq.detail.attention": "要注意",
  "wq.detail.noAttention": "注意が必要な点はありません",
  "wq.uncertainty": "（根拠となる事実：{freshness}。現在も成り立つとは限りません）",
  "wq.uncertaintyDegraded":
    "（根拠となる事実：{freshness}。情報源も機能制限中のため、現在も成り立つとは限りません）",
  "wq.detail.deadline": "期限",
  "wq.detail.promised": "完了予定",
  "wq.detail.placement": "ここに並ぶ理由",
  "wq.detail.allowed": "状態上可能な操作",
  "wq.none": "なし",
  "wq.detail.owner": "記録元",
  "wq.detail.version": "バージョン",
  "wq.detail.next": "次の手順",

  "wq.backToReview": "レビューに戻る",
  "wq.sending": "送信中…",
  "wq.actionFailed": "この操作は完了しませんでした。{message}",
  "wq.abandon": "この操作を中止する",
  "wq.cancel": "キャンセル",
  "wq.reason.closeRisk": "クローズの理由",
  "wq.reason.decline": "辞退の理由",
  "wq.reason.withdraw": "取り下げの理由",
  "wq.reason.cancel": "キャンセルの理由",
  "wq.reason.reopen": "再オープンの理由",
  "wq.confirm.closePreview": "クローズのプレビューを作成",
  "wq.confirm.decline": "辞退を確定",
  "wq.confirm.withdraw": "取り下げを確定",
  "wq.confirm.cancelPreview": "キャンセルのプレビューを作成",
  "wq.confirm.reopenPreview": "再オープンのプレビューを作成",
  "wq.confirm.resolvePreview": "解決のプレビューを作成",
  "wq.confirm.completePreview": "完了のプレビューを作成",
  "wq.reopenMode": "再オープンのモード",
  "wq.reopenMode.completed": "完了した Action を再オープン",
  "wq.reopenMode.cancelled": "キャンセルした Action を再開",
  "wq.issueEvidence.resolve":
    "この Issue が解決したことを示す Evidence（複数選択可。3 つの遷移すべてに Evidence が必要です）",
  "wq.issueEvidence.close": "解決が検証されたことを示す Evidence（複数選択可）",
  "wq.issueEvidence.reopen": "検証に失敗したことを示す Evidence（複数選択可）",
  "wq.resolutionType": "解決方法",
  "wq.resolutionType.resolved": "解決済み",
  "wq.resolutionType.workaround": "回避策で対応",
  "wq.resolutionType.acceptedImpact": "影響を受容",
  "wq.reason.resolve": "解決の理由",
  "wq.evidenceLoading": "Evidence を読み込んでいます…",
  "wq.evidenceNone": "Ledger に Evidence がありません。",
  "wq.evidenceOption.pinned":
    "{id}：{verification}、固定済み、{classification}（バージョン {version}）",
  "wq.evidenceOption.unpinned":
    "{id}：{verification}、未固定、{classification}（バージョン {version}）",
  "wq.judgment.issue":
    "Judgment の理由（空欄の場合は Judgment を付けません。選んだ Evidence が一部しか検証されていない場合は必須ですが、未検証や記録と内容が一致しない Evidence を通すことはできません）",
  "wq.judgment.complete":
    "Judgment の理由（空欄の場合は Judgment を付けません。関連付けた Evidence が未検証の場合は必須です）",
  "wq.judgment.decision": "Judgment の理由（空欄の場合は Judgment を付けません）",
  "wq.judgmentClassification": "Judgment の分類",
  "wq.linkLoading": "関連付けられる Evidence を読み込んでいます…",
  "wq.linkChoose": "関連付ける Evidence",
  "wq.linkPlaceholder": "（選択してください）",
  "wq.linkNone": "Ledger に関連付けられる Evidence がありません。",
  "wq.linkConfirm": "関連付けを確定",
  "wq.decision.statement": "決定内容",
  "wq.decision.rationale": "決定の理由",
  "wq.decision.impact": "影響",
  "wq.decision.evidence":
    "この Decision を裏付ける Evidence（複数選択可。Evidence を選ばない場合は Judgment が必要です）",
  "wq.followUps": "後続の Action Request（それぞれアプリが識別子を割り当てます）",
  "wq.followUp.subject": "件名",
  "wq.followUp.details": "内容",
  "wq.followUp.owner": "担当者（Stakeholder id）",
  "wq.followUp.due": "期日",
  "wq.followUp.classification": "分類",
  "wq.followUp.remove": "これを削除",
  "wq.followUp.add": "後続の Action Request を追加",

  "wq.outOfSync":
    "Work Queue の 2 つの情報源が異なる Ledger リビジョンを読み込んだため、項目を表示していません。2 つの時点をつなぎ合わせると、正しく見えて誤った一覧になります。",
  "wq.headline": "今日対応すること",
  "wq.lede":
    "並び順：約束が守られなかったもの、作業が止まっているもの、Evidence に問題があるもの、期限が近いもの、責任者がいないもの。各項目にここに並ぶ理由を示しています。",
  "wq.filters": "種類で絞り込む（何も選ばない場合はすべて表示）",
  "wq.filterChip": "{kind}（{count}）",
  "wq.onlyFlagged": "注意が必要な項目のみ",
  "wq.empty": "現在、対応が必要な項目はありません。",
  "wq.column.kind": "種類",
  "wq.column.item": "項目",
  "wq.column.attention": "要注意",
  "wq.column.deadline": "期限",
  "wq.column.placement": "ここに並ぶ理由",
  "wq.column.next": "次の手順",
  "wq.allowed": "状態上可能な操作：{intents}",
  "wq.promised": "完了予定 {date}",
  "wq.inDetail": "詳細ウィンドウで操作",
  "wq.previous": "前のページ",
  "wq.next": "次のページ",
} as const satisfies Record<keyof typeof WORK_QUEUE_EN, string>;
