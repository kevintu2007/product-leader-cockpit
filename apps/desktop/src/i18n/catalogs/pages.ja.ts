import type { PAGES_EN } from "./pages.en";

/** People、Reviews & Reports、Product Vault、Settings の日本語の文言。キーは英語版と同じです。 */
export const PAGES_JA = {
  "people.kind.person": "個人",
  "people.kind.organization": "組織",
  "people.none": "なし",
  "people.headline": "誰が何を担当し、誰がまだ回答を待っているか",
  "people.lede":
    "ここにある Stakeholder と関係は、あなたが作成・管理するもので、ユーザーアカウントではありません。回答待ちのリクエストはまだ約束ではなく、相手が受け入れると Action Request は Action になります。",
  "people.empty":
    "Stakeholder はまだいません。Stakeholder を作成すると、担当していることと依存していることがここに表示されます。",
  "people.caption": "識別子の順に並んでいます。全 {total} 人中 {from}〜{to} 人目を表示しています。",
  "people.column.stakeholder": "Stakeholder",
  "people.column.kind": "種類",
  "people.column.responsible": "担当",
  "people.column.dependsOn": "依存",
  "people.column.waiting": "回答待ちのリクエスト",
  "people.column.classification": "分類",
  "people.previous": "前のページ",
  "people.next": "次のページ",

  "reviews.route": "Review に必要なデータ",
  "reviews.headline": "Review の前に、まだ開いている事項を確認する",
  "reviews.lede":
    "Work Queue で現在フラグが付いている事項を、Executive Cockpit の注目順に並べています。対応するには Work Queue に移動してください。",
  "reviews.period": "レビュー期間",
  "reviews.noPeriod": "利用できるレビュー期間がありません：{reason}。",
  "reviews.notYet": "このバージョンでは、まだ Fact Pack の作成やレポートの承認はできません。",
  "reviews.open": "Work Queue でフラグが付いた事項",
  "reviews.openNone": "現在、Work Queue でフラグが付いた事項はありません。",
  "reviews.count.one": "全 {count} 件。",
  "reviews.count.other": "全 {count} 件。",
  "reviews.goToQueue": "Work Queue で対応する",

  "vault.reason.notConfigured": "このワークスペースには Product Vault が設定されていません。",
  "vault.reason.invalidRoot":
    "Product Vault の場所が存在しないか、フォルダーではないか、リンクです。",
  "vault.reason.unknown": "原因は不明です。",
  "vault.pausedBecause":
    "{reason}フィンガープリントの固定と再観測は一時停止します。Evidence の関連付けには影響しません。",
  "vault.headline": "Evidence ファイルと現在の状態",
  "vault.lede":
    "ファイルはあなた自身の Product Vault に置かれ、Ledger には各 Evidence の用途、フィンガープリント、検証結果、分類が記録されます。",
  "vault.available": "Vault は使用できます",
  "vault.unavailable": "Vault は使用できません",
  "vault.availableHint":
    "Product の Evidence タブから、フィンガープリントの固定や再観測ができます。",
  "vault.count": "全 {count} 件、識別子の順に並んでいます。",
  "vault.empty":
    "Ledger にはまだ Evidence がありません。Product の Evidence タブで関連付けると、ここに表示されます。",
  "vault.column.evidence": "Evidence",
  "vault.column.role": "用途",
  "vault.column.verification": "検証状態",
  "vault.column.fingerprint": "フィンガープリント",
  "vault.column.classification": "分類",
  "vault.column.version": "バージョン",
  "vault.roleUnset": "未指定",
  "vault.verificationOn": "{verification}（{date}）",
  "vault.pinned": "固定済み",
  "vault.notPinned": "未固定",

  "settings.headline": "表示方法とこのワークスペースの状態",
  "settings.lede":
    "文字サイズはこのコンピューターの画面だけに影響し、Windows の表示スケールとは別です。テーマは右上のボタンで切り替えます。",
  "settings.textSize": "文字サイズ",
  "settings.language": "言語",
  "settings.languageSystem": "システムの言語（{language}）",
  "settings.languageFailed": "言語を保存できませんでした。{message}",
  "settings.sampleBody":
    "おはようございます。今日は成果に本当に影響することから始めましょう。Demo Product Atlas には日付を過ぎたマイルストーンがあり、2／2 件の KPI に観測値があります。",
  "settings.sampleLabel": "マイルストーン：日付を過ぎた、最も早い日付 2026‑08‑15",
  "settings.dataSources": "データソース",
  "settings.loading": "Ledger と Vault の状態を読み込んでいます。",
  "settings.unavailable": "現在 Ledger または Vault の状態を読み込めません。{message}",
  "settings.ledgerReadable":
    "読み取れます。スキーマバージョン {schema}、Ledger のバージョン {revision}。",
  "settings.vaultNotSet": "未設定",
  "vaultRoot.open.choose": "Vault フォルダーを選択…",
  "vaultRoot.open.change": "Vault フォルダーを変更…",
  "vault.reason.changeUnresolved":
    "Vault フォルダーの変更が中断され、完了できませんでした。PMC を再起動して完了してください。",
  "safeError.desktop.vault_change_unresolved":
    "Vault フォルダーの変更が中断され、完了できなかったため、Vault は使われていません。PMC を再起動して完了してください。",
  "vaultRoot.title": "Product Vault フォルダー",
  "vaultRoot.cancel": "キャンセル",
  "vaultRoot.choose.lede":
    "このワークスペースが Evidence を読み込むフォルダーを選択します。変更の前に、PMC はこのワークスペースをバックアップし、フォルダー内のすべての Evidence ファイルを確認します。",
  "vaultRoot.backupFirst":
    "Vault を変更する前に、PMC は復元できるようこのワークスペースをバックアップします。先にバックアップ先フォルダーと復元パスフレーズを設定してください。",
  "vaultRoot.choose.button": "Vault フォルダーを選択…",
  "vaultRoot.choose.dialogTitle": "Product Vault フォルダーを選択",
  "vaultRoot.folder": "フォルダー: {name}",
  "vaultRoot.preparing":
    "このワークスペースをバックアップし、新しいフォルダー内のすべての Evidence ファイルを確認しています…1 分ほどかかることがあります。",
  "vaultRoot.preview.lede":
    "変更される内容は次のとおりです。Vault と Ledger の中身は移動しません。",
  "vaultRoot.preview.now": "現在",
  "vaultRoot.preview.new": "新規",
  "vaultRoot.preview.evidence": "Evidence",
  "vaultRoot.preview.noEvidence":
    "現在の Vault を参照する Evidence はないため、対応付けし直すものはありません。",
  "vaultRoot.preview.resolved":
    "新しいフォルダーでは、{count} 件の参照のうち {resolved} 件が同じ内容です。",
  "vaultRoot.preview.recovery": "復旧用の証拠",
  "vaultRoot.preview.recoveryVerified": "{time} にバックアップして検証済み",
  "vaultRoot.confirm.phrase": "VAULT を変更",
  "vaultRoot.confirm.label": "確認のため「{phrase} {code}」と入力してください",
  "vaultRoot.confirm.button": "このフォルダーを使う",
  "vaultRoot.reject": "変更しない",
  "vaultRoot.changing": "変更中です…PMC を閉じないでください。",
  "vaultRoot.result.changed": "このワークスペースは「{name}」から Evidence を読み込みます。",
  "vaultRoot.result.notChanged": "何も変更されていません。",
  "vaultRoot.done": "完了",
  "safeError.desktop.vault_change_active":
    "別の Vault フォルダーの変更が進行中です。先に完了するかキャンセルしてください。",
  "safeError.desktop.vault_not_live":
    "変更できるのは Live ワークスペースの Vault フォルダーだけです。",
  "safeError.desktop.vault_folder_unusable":
    "このフォルダーは Vault にできません。読み取れる既存のフォルダーを選び、ショートカットやリンクは選ばないでください。",
  "safeError.desktop.vault_folder_unchanged":
    "このフォルダーはすでにこのワークスペースの Vault です。",
  "safeError.desktop.vault_folder_in_use":
    "このフォルダーは PMC がすでに使っています。PMC 自身のものでなく、バックアップフォルダーを含まず、その中にもないフォルダーを選んでください。",
  "safeError.desktop.vault_recovery_backup_stale":
    "この変更のために作成したバックアップがワークスペースと一致しなくなりました。もう一度お試しください。",
  "safeError.desktop.vault_evidence_unpinned":
    "{count} 件の Evidence 参照に固定された指紋がないため、新しいフォルダーのファイルが同じだと証明できません。先に固定してください（Product → Evidence）。",
  "safeError.desktop.vault_evidence_unresolved":
    "{count} 件の Evidence 参照が新しいフォルダーで同じ内容になりません。先にそのファイルを移動または復元してください。",
  "safeError.desktop.vault_preview_stale":
    "このプレビューは古くなっています。フォルダーを選び直してください。",
  "safeError.desktop.vault_confirmation_mismatch": "コードがこのプレビューと一致しません。",
  "safeError.desktop.vault_changed_but_not_recorded":
    "フォルダーは変更されましたが、PMC はこの変更を監査ログに記録できませんでした。",
  "safeError.desktop.vault_change_not_recorded":
    "PMC はこの手順を記録できなかったため、何も変更していません。もう一度お試しください。",
  "safeError.desktop.vault_change_failed":
    "Vault フォルダーを変更できませんでした。何も変更されていません。もう一度お試しください。",
  "evidenceFile.open": "ファイルから Evidence を追加…",
  "evidenceFile.title": "ファイルから Evidence を追加",
  "evidenceFile.lede":
    "Vault フォルダー内のファイルを選んでください。PMC は場所とフィンガープリントを記録し、ファイル自体はそのまま残ります。",
  "evidenceFile.choose": "ファイルを選択…",
  "evidenceFile.chooseAnother": "別のファイルを選択…",
  "evidenceFile.dialogTitle": "Product Vault 内のファイルを選択",
  "evidenceFile.file": "ファイル：{name}",
  "evidenceFile.observed": "{time} に観測",
  "evidenceFile.changed": "選択した後にファイルが変更されました。",
  "evidenceFile.existing":
    "Evidence {id} がすでにこのファイルを参照しているため、新しく作成しません。",
  "evidenceFile.existingLinked":
    "Evidence {id} はすでにこのファイルを参照しており、この Product にもリンク済みです。",
  "evidenceFile.linkExisting": "この Product にリンク",
  "evidenceFile.sameContent": "Evidence {id} は同じ内容です。",
  "evidenceFile.create": "作成",
  "evidenceFile.createAndLink": "作成してこの Product にリンク",
  "evidenceFile.cancel": "キャンセル",
  "evidenceFile.creating": "作成しています…",
  "evidenceFile.linking": "リンクしています…",
  "evidenceFile.created": "Evidence {id} を作成しました。",
  "evidenceFile.createdAndLinked": "Evidence {id} を作成し、{product} にリンクしました。",
  "evidenceFile.linkedExisting": "Evidence {id} を {product} にリンクしました。",
  "evidenceFile.createdNotLinked": "Evidence {id} を作成しました。まだリンクされていません。",
  "evidenceFile.retryLink": "今すぐリンク",
  "evidenceFile.done": "完了",
  "safeError.desktop.evidence_file_outside_vault": "Vault フォルダー内のファイルを選んでください。",
  "safeError.desktop.evidence_file_unreadable": "ファイルを読み取れませんでした。",
  "safeError.desktop.evidence_file_choice_stale":
    "この選択は古くなっています。ファイルをもう一度選んでください。",
  "safeError.desktop.sample_backup_refused": "サンプルワークスペースはバックアップされません。",
  "safeError.desktop.workspace_not_chosen": "先に始め方を選んでください。",
  "safeError.desktop.sample_foreign":
    "サンプルフォルダーに PMC が書いていないものがあるため、そのままにしました。",
  "safeError.desktop.sample_is_open":
    "先に自分のワークスペースに切り替えてください。サンプルデータは開いている間は変更できません。",
  "safeError.desktop.sample_operation_in_progress":
    "サンプルデータの別の変更が完了していません。少し待ってから再試行してください。",
  "safeError.desktop.sample_nothing_to_delete": "削除するサンプルデータはありません。",
  "safeError.desktop.sample_delete_not_prepared":
    "この削除は待機していません。最初からやり直してください。",
  "safeError.desktop.sample_delete_expired":
    "このプレビューは期限切れです。何も削除されていません。最初からやり直してください。",
  "safeError.desktop.sample_delete_changed":
    "プレビュー後にサンプルデータが変わりました。何も削除されていません。最初からやり直してください。",
  "safeError.desktop.sample_confirmation_mismatch":
    "入力した語句が一致しません。何も削除されていません。",
  "safeError.desktop.sample_not_recorded":
    "PMC はこの手順を監査ログに記録できませんでした。もう一度お試しください。",
  "safeError.desktop.sample_failed":
    "サンプルデータを準備できませんでした。あなたのワークスペースには影響ありません。もう一度お試しください。",
  "safeError.desktop.workspace_not_first_run": "この選択は初回起動時に一度だけ行います。",
  "safeError.desktop.workspace_choice_not_saved":
    "選択を保存できなかったため、PMC は再起動しませんでした。もう一度お試しください。",
  "safeError.desktop.sample_reset_failed":
    "サンプルデータをリセットできませんでした。PMC は今すぐ、または次回の起動時に元の状態に戻します。",
  "safeError.desktop.sample_delete_failed":
    "サンプルデータは削除されなかったか、完全には削除されませんでした。PMC は次回の起動時にもう一度確認します。",
  "safeError.desktop.sample_unavailable":
    "現在サンプルデータを読み取れません。何も変更されていません。",
  "firstRun.title": "始め方を選んでください",
  "firstRun.lede":
    "どちらかを選んで始めます。あとで「設定 → ワークスペース」から切り替えられます。",
  "firstRun.live.title": "自分のワークスペースで始める",
  "firstRun.live.body":
    "空のワークスペースが開きます。記録はこのコンピューターだけに保存されます。",
  "firstRun.live.setup":
    "続いて「設定」で、バックアップフォルダー、復旧用パスフレーズ、Vault フォルダーの順に設定します。パスフレーズがないとバックアップは復元できません。",
  "firstRun.sample.title": "サンプルデータで学ぶ",
  "firstRun.sample.body":
    "学習用の合成データです。あなたの作業とは分かれていて、いつでもリセットや削除ができます。",
  "firstRun.preparing": "サンプルデータを準備しています…1 分ほどかかることがあります。",
  "firstRun.saving": "選択を保存しています…",
  "firstRun.tryAgain": "まだ何も選ばれていません。準備ができたらもう一度選んでください。",
  "workspace.title": "ワークスペース",
  "workspace.openLive": "自分のワークスペースが開いています。",
  "workspace.openSample":
    "サンプルデータが開いています。合成データで、あなたの作業とは分かれています。",
  "workspace.sampleFellBack":
    "サンプルデータが選ばれていましたが開けなかったため、自分のワークスペースを開きました。理由はシステムの状態で確認できます。",
  "workspace.switchToSample": "サンプルデータに切り替える",
  "workspace.switchToLive": "自分のワークスペースに切り替える",
  "workspace.switchToSample.confirm":
    "PMC が再起動してサンプルデータを開きます。自分のワークスペースは変わりません。",
  "workspace.switchToLive.confirm": "PMC が再起動して自分のワークスペースを開きます。",
  "workspace.switchAndRestart": "切り替えて再起動",
  "workspace.restarting": "PMC を再起動しています…",
  "sampleWorkspace.badge": "サンプルワークスペース",
  "sampleWorkspace.badge.open": "「設定 → ワークスペース」を開く",
  "sampleWorkspace.manage": "サンプルデータ",
  "sampleWorkspace.fromLiveOnly":
    "リセットと削除は自分のワークスペースから行います。先に切り替えてください。",
  "sampleWorkspace.cancel": "キャンセル",
  "sampleWorkspace.close": "閉じる",
  "sampleWorkspace.done": "完了",
  "sampleWorkspace.reset.open": "サンプルデータをリセット…",
  "sampleWorkspace.reset": "サンプルデータをリセット",
  "sampleWorkspace.reset.confirm":
    "サンプルデータが最初の状態に戻ります。自分のワークスペースには影響しません。",
  "sampleWorkspace.resetting": "サンプルデータをリセットしています…1 分ほどかかることがあります。",
  "sampleWorkspace.reset.done": "サンプルデータを最初の状態に戻しました。",
  "sampleWorkspace.delete.open": "サンプルデータを削除…",
  "sampleWorkspace.delete.title": "サンプルデータを削除",
  "sampleWorkspace.delete.preparing": "サンプルデータの内容を読み取っています…",
  "sampleWorkspace.delete.lede":
    "削除で取り除かれるのは次のとおりです。確定する前に何か変わると、この削除は取り消されます。",
  "sampleWorkspace.delete.what": "サンプルデータ",
  "sampleWorkspace.delete.seed": "{seedId}、バージョン {version}",
  "sampleWorkspace.delete.unknown":
    "このプレビューはこの画面で表示できない変更を示しているため、ここでは承認できません。何も削除されていません。",
  "sampleWorkspace.delete.parts": "取り除かれるもの",
  "sampleWorkspace.delete.part.ledger": "その Product Ledger",
  "sampleWorkspace.delete.part.vault": "その合成 Vault",
  "sampleWorkspace.delete.part.generated": "生成されたファイル",
  "sampleWorkspace.delete.effect": "影響",
  "sampleWorkspace.delete.effectValue":
    "元に戻せません。自分のワークスペース、設定、バックアップには影響しません。",
  "sampleWorkspace.delete.expires": "プレビューの有効期限",
  "sampleWorkspace.delete.settingsRevision": "設定のリビジョン",
  "sampleWorkspace.delete.inventory": "内容のダイジェスト",
  "sampleWorkspace.delete.payload": "プレビューのダイジェスト",
  "sampleWorkspace.delete.confirmLabel": "確認のため「{phrase}」と入力してください",
  "sampleWorkspace.delete.phrase": "サンプルデータを削除",
  "sampleWorkspace.delete.confirm": "サンプルデータを削除",
  "sampleWorkspace.delete.reject": "残す",
  "sampleWorkspace.delete.deleting": "サンプルデータを削除しています…",
  "sampleWorkspace.delete.deleted":
    "サンプルデータを削除しました。サンプルデータに切り替えると、もう一度準備されます。",
  "sampleWorkspace.delete.notDeleted": "サンプルデータは削除されていません。何も変わっていません。",
  "health.ledger.first_run": "まだ開いていません。先に始め方を選んでください。",
  "health.settings.setAside":
    "設定を読み取れなかったため、PMC は元の設定を別に保管し、既定の設定で起動しました。",
  "health.settings.unavailable": "いまは設定を読み取ることも保存することもできません。",
  "health.sample.unresolved":
    "サンプルデータへの変更が途中のまま完了できなかったため、サンプルデータは使えません。",
  "health.sample.missing":
    "サンプルデータが選ばれていましたが見つからなかったため、自分のワークスペースを開きました。",
  "health.sample.foreign":
    "サンプルフォルダーに PMC が書き込んでいないものがあるため、そのままにしてあります。サンプルデータは使えません。",
  "health.sample.cleanupPending":
    "削除待ちのサンプルフォルダーが {count} 個残っています。PMC が次回以降の起動時に取り除きます。",
  "gettingStarted.title": "はじめに",
  "gettingStarted.lede":
    "この順番でワークスペースを設定してください。各ステップは PMC が確認できる状態から判断され、すべて完了するとこの一覧は消えます。",
  "gettingStarted.backupFolder": "バックアップフォルダーを選ぶ",
  "gettingStarted.backupFolder.why":
    "バックアップは選んだフォルダーに保存されます。別のドライブがおすすめです。",
  "gettingStarted.passphrase": "復旧用パスフレーズを設定する",
  "gettingStarted.passphrase.why":
    "バックアップはこれで暗号化されます。これがないとバックアップは復元できず、なくしたパスフレーズを PMC が取り戻すこともできません。",
  "gettingStarted.firstBackup": "最初のバックアップを作る",
  "gettingStarted.firstBackup.why":
    "バックアップの検証が済むまで、PMC は新しい記録や変更を受け付けません。",
  "gettingStarted.vault": "Product Vault フォルダーを選ぶ",
  "gettingStarted.vault.why":
    "Evidence ファイルを置くフォルダーです。PMC は各ファイルの場所と指紋だけを記録し、ファイルをコピーしません。",
  "gettingStarted.firstProduct": "最初の Product を追加する",
  "gettingStarted.firstProduct.why":
    "Portfolio の「新しい Product…」から追加します。Cockpit が Portfolio Lens に配置します。",
  "gettingStarted.status.done": "完了",
  "gettingStarted.status.next": "次",
  "gettingStarted.status.waits": "上のステップの完了待ち",
  "gettingStarted.status.unknown": "PMC はまだ判断できません",
  "gettingStarted.openSettings": "設定を開く",
  "gettingStarted.openPortfolio": "Portfolio を開く",
  "settings.vaultAvailable": "使用可能",
  "settings.vaultUnavailable": "使用不可",
  "backup.headline": "バックアップ",
  "backup.loading": "バックアップの状態を読み込んでいます。",
  "backup.dueLede":
    "バックアップの期限です。検証済みのバックアップができるまで、このワークスペースでは記録の追加や変更ができません。",
  "backup.folder.label": "バックアップ先フォルダー",
  "backup.folder.notSet": "未設定",
  "backup.folder.set": "設定済み",
  "backup.folder.available": "使用できます",
  "backup.folder.unavailable":
    "使用できません。ドライブを接続するか、別のフォルダーを選んでください",
  "backup.folder.choose": "フォルダーを選ぶ…",
  "backup.folder.dialogTitle": "PMC がバックアップを保存するフォルダーを選択",
  "backup.passphrase.label": "復元パスフレーズ",
  "backup.passphrase.notSet": "未設定",
  "backup.passphrase.session": "このセッションで設定済み",
  "backup.passphrase.remembered": "この Windows アカウントに保存済み",
  "backup.passphrase.setUp": "パスフレーズを設定…",
  "backup.last.label": "前回のバックアップ",
  "backup.last.none": "まだバックアップはありません",
  "backup.last.at": "{time} に検証済み。次の期限は {next} です。",
  "backup.orphans":
    "このフォルダーには PMC の記録にないバックアップファイルが {count} 件あり、数に含めていません。",
  "backup.run": "今すぐバックアップ",
  "backup.running": "バックアップ中です…1 分ほどかかることがあります。",
  "backup.done": "{time} にバックアップして検証しました。",
  "backup.openBackups": "バックアップを開く",
  "backup.strip.due": "先にバックアップすると、記録の追加や変更ができるようになります。",
  "backup.strip.running": "バックアップ中です…終わるまで記録の追加や変更はお待ちください。",
  "backup.passphrase.title": "復元パスフレーズの設定",
  "backup.passphrase.generatedLede":
    "このパスフレーズは一度だけ表示されます。書き留めるかパスワード管理ツールに保存し、下にもう一度入力してください。",
  "backup.passphrase.generating": "パスフレーズを作成しています。",
  "backup.passphrase.retype": "そのまま入力",
  "backup.passphrase.useOwn": "自分のパスフレーズを使う",
  "backup.passphrase.useGenerated": "生成したパスフレーズを使う",
  "backup.passphrase.ownRule":
    "20 文字以上、または 6 語以上で、よくあるパスワードだけで作らないでください。",
  "backup.passphrase.own": "パスフレーズ",
  "backup.passphrase.ownAgain": "もう一度入力",
  "backup.passphrase.acknowledge":
    "PMC はこのパスフレーズを復元できません。これがないと、このバックアップはこのコンピューターでもほかのコンピューターでも復元できません。",
  "backup.passphrase.remember": "自動バックアップのため、この Windows アカウントに保存する。",
  "backup.passphrase.confirm": "このパスフレーズを使う",
  "backup.passphrase.cancel": "キャンセル",
} as const satisfies Record<keyof typeof PAGES_EN, string>;
