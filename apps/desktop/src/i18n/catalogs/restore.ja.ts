import type { RESTORE_EN } from "./restore.en";

/** Operational Restore と S11 System Health の日本語の文言。 */
export const RESTORE_JA = {
  "restore.open": "バックアップから復元…",
  "restore.title": "バックアップから復元",
  "restore.cancel": "キャンセル",
  "restore.choose.lede":
    "このコンピューターまたは別のコンピューターで作成した Operational Backup を選んでください。最後の手順を確定するまで、何も変わりません。",
  "restore.choose.button": "バックアップファイルを選択…",
  "restore.choose.dialogTitle": "復元するバックアップを選択",
  "restore.choose.filterName": "PMC バックアップ",
  "restore.file": "バックアップファイル: {name}",
  "restore.passphrase.label": "このバックアップのパスフレーズ",
  "restore.passphrase.check": "バックアップを確認",
  "restore.checking": "バックアップを確認しています…1 分ほどかかることがあります。",
  "restore.backup": "{time} に作成、Ledger schema {schema}、{count} 件のレコード。",
  "restore.recovery.running":
    "何かを置き換える前に、PMC は現在のワークスペースをバックアップします。1 分ほどかかることがあります。",
  "restore.preview.lede":
    "復元で行われるのは、以下のとおりです。確定する前に何か変わると、この復元は取り消されます。",
  "restore.preview.backup": "バックアップ",
  "restore.preview.now": "現在",
  "restore.now": "{count} 件のレコード、最終変更 {time}。",
  "restore.now.noChange": "{count} 件のレコード、変更はまだありません。",
  "restore.preview.replaced": "置き換わるもの",
  "restore.replaced":
    "Product Ledger と、バックアップ内の設定(言語、タイムゾーン、テーマ、保存期間、AI とログの設定)。",
  "restore.preview.kept": "置き換わらないもの",
  "restore.kept":
    "Product Vault、バックアップフォルダーとパスフレーズの設定、そのほかのすべてのバックアップ。",
  "restore.preview.recovery": "復旧用バックアップ",
  "restore.recovery.done": "現在のワークスペースを {time} にバックアップし、検証しました。",
  "restore.needsUpgrade":
    "このバックアップは以前のバージョンの PMC のものです。復元後、使う前に PMC がアップグレードを求めます。",
  "restore.confirm.label": "確定するには、バックアップの作成日を入力してください: {date}",
  "restore.confirm.button": "このバックアップで置き換える",
  "restore.reject": "復元しない",
  "restore.replacing": "置き換えています…PMC を閉じないでください。",
  "restore.result.restored":
    "{time} のバックアップを復元しました。以前の内容は {recovery} のバックアップとして保存されています。",
  "restore.result.unchanged": "復元に失敗しました。何も変更されていません。",
  "restore.result.putBack":
    "置き換えを始めた後で復元に失敗しました。PMC は以前のワークスペースを元に戻しました。",
  "restore.result.recoveryFailed":
    "復元に失敗し、PMC は以前のワークスペースを元に戻せませんでした。System Health に、復元すべき復旧用バックアップが表示されます。",
  "restore.result.interrupted":
    "復元は完了する前に止まりました。いまはこれ以上何も変わりません。次に PMC を起動すると、以前のワークスペースに戻します。",
  "restore.continue": "続ける",
  "restore.openSystemHealth": "System Health を開く",

  "policyStrip.restoring": "復元中",
  "backup.strip.restoring": "復元中です。新しいレコードや変更は、復元が終わるまで受け付けません。",

  "health.headline": "PMC があなたのレコードを読み取り、保存できるか",
  "health.loading": "システムの状態を読み取っています。",
  "health.unavailable": "いまはシステムの状態を読み取れません。{message}",
  "health.ledger.label": "Product Ledger",
  "health.ledger.ready": "開いていて、読み取れます。",
  "health.ledger.replacing": "復元により置き換え中です。",
  "health.ledger.restore_recovery_required":
    "開いていません。復元が途中で止まったため、いまのファイルは元の Ledger でもバックアップでもない可能性があります。",
  "health.ledger.upgrade_required":
    "まだ開いていません。以前のバージョンの PMC のもので、先にアップグレードが必要です。",
  "health.ledger.open_failed": "開けませんでした。",
  "health.recoveryBackup":
    "バックアップフォルダーから復旧用バックアップ {name} を復元すると、元の状態に戻れます。",
  "health.quit": "PMC を終了",

  "safeError.desktop.restore_running": "復元中です。終わってからもう一度お試しください。",
  "safeError.desktop.ledger_unavailable":
    "Product Ledger が開いていません。理由は System Health で確認できます。",
  "safeError.desktop.restore_file_unreadable": "このバックアップファイルは読み取れません。",
  "safeError.desktop.restore_wrong_passphrase":
    "このパスフレーズでは、このバックアップを開けません。",
  "safeError.desktop.restore_damaged":
    "このバックアップは破損しています。内容の確認に失敗しました。",
  "safeError.desktop.restore_newer_version":
    "このバックアップは新しいバージョンの PMC で作成されました。復元するには PMC を更新してください。",
  "safeError.desktop.restore_unsupported_old":
    "このバックアップは古すぎるバージョンの PMC のもので、復元できません。",
  "safeError.desktop.restore_recovery_backup_failed":
    "現在のワークスペースのバックアップに失敗したため、何も置き換えていません。",
  "safeError.desktop.restore_preview_stale":
    "このプレビューはもう最新ではありません。バックアップを選び直してください。",
  "safeError.desktop.restore_confirmation_mismatch":
    "入力した日付がバックアップの作成日と一致しません。",
  "safeError.desktop.restore_failed_unchanged": "復元に失敗しました。何も変更されていません。",
  "safeError.desktop.restore_live_only": "復元できるのは Live ワークスペースだけです。",

  "upgrade.title": "このワークスペースをアップグレード",
  "upgrade.body":
    "PMC {version} は新しい形式でレコードを保存します。アップグレードはすぐに終わります。PMC は先にワークスペースをバックアップし、どこかで失敗しても何も変わりません。",
  "upgrade.fact.current": "現在の形式",
  "upgrade.fact.new": "新しい形式",
  "upgrade.schema": "schema {schema}",
  "upgrade.fact.records": "レコード数",
  "upgrade.fact.lastBackup": "最後に検証したバックアップ",
  "upgrade.fact.noBackup": "なし",
  "upgrade.backupFirst":
    "先にバックアップ: バックアップフォルダーとパスフレーズを設定してからアップグレードしてください。",
  "upgrade.run": "アップグレード",
  "upgrade.backingUp": "ワークスペースをバックアップしています…",
  "upgrade.upgrading": "アップグレードしています…",
  "upgrade.result.upgraded":
    "ワークスペースをアップグレードしました。{time} のバックアップに以前の状態が保存されています。",
  "upgrade.result.rolledBack": "アップグレードは完了しませんでした。何も変更されていません。",
  "upgrade.result.unreadable":
    "ワークスペースはアップグレードされましたが、PMC で開けませんでした。{time} のバックアップに以前の状態が保存されています。",
  "upgrade.result.unknown":
    "アップグレードが完了したかどうか PMC では判断できません。{time} のバックアップに以前のワークスペースが保存されています。",
  "upgrade.retry": "もう一度試す",
  "upgrade.blocked.title": "このワークスペースは開けません",
  "upgrade.newer":
    "このワークスペースは新しいバージョンの PMC で作成されました。開くには PMC を更新してください。",
  "upgrade.unsupportedOld":
    "このワークスペースは開発版の PMC で作成されたため、アップグレードできません。",
  "health.ledger.unsupported_old":
    "開いていません。開発版の PMC で作成されたため、アップグレードできません。",
  "health.ledger.newer_version": "開いていません。新しいバージョンの PMC で作成されました。",
  "safeError.desktop.upgrade_live_only": "アップグレードできるのは Live ワークスペースだけです。",
  "safeError.desktop.upgrade_not_required":
    "このワークスペースはアップグレードの必要がありません。",
  "safeError.desktop.upgrade_backup_failed":
    "アップグレード前のバックアップに失敗したため、アップグレードは始まっていません。バックアップフォルダーを確認して、もう一度お試しください。",
  "safeError.desktop.upgrade_failed_unchanged":
    "アップグレードは完了しませんでした。何も変更されていません。",
  "upgrade.trainingReset":
    "この Training ワークスペースは以前のバージョンの PMC のものです。アップグレードはせず、サンプルワークスペースでリセットしてください。",
  "safeError.desktop.upgrade_outcome_unknown":
    "アップグレードが完了したかどうか PMC では判断できません。直前に作成したバックアップに以前のワークスペースが保存されています。",
  "restore.now.unavailable":
    "PMC は現在の Product Ledger を開けないため、記録数と最終変更日時は分かりません。",
  "restore.recovery.preserved":
    "PMC は現在の Ledger ファイルの完全なコピーを「{name}」として保存し、再確認しました。コピーはバイト単位で一致しました。使用できる Product Ledger であるとは確認できないため、Operational Backup ではなく、最後に検証されたバックアップにも数えません。",
  "restore.preserve.running":
    "何かを置き換える前に、PMC は現在の Ledger ファイルの完全なコピーを保存して確認します。1 分ほどかかることがあります。",
  "restore.result.restoredPreserved":
    "{time} のバックアップを復元しました。ここにあったファイルは「{name}」として保存されています。",
  "restore.result.putBackPreserved":
    "置き換えを始めた後に復元が失敗しました。PMC は以前の Ledger ファイルを元のとおりに戻しました。Product Ledger はまだ開けません。別のバックアップを試すか、PMC を終了してください。",
  "health.restore.backupFirst":
    "復元の前に、PMC は現在の Ledger ファイルのコピーをバックアップ先フォルダーに保存します。先にバックアップ先フォルダーと復元パスフレーズを設定してください。",
  "restore.choose.recovery": "復旧用バックアップ「{name}」を使う",
  "restore.choose.other": "別のバックアップファイルを選択…",
  "restore.result.sourceChanged":
    "プレビューの後に現在の Ledger ファイルが変わったため、何も置き換えていません。復元をやり直して、今置き換わる内容を確認してください。",
} as const satisfies Record<keyof typeof RESTORE_EN, string>;
