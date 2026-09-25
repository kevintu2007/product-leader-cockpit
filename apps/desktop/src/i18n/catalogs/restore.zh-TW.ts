import type { RESTORE_EN } from "./restore.en";

/** Operational Restore 與 S11 System Health 的繁體中文文案。 */
export const RESTORE_ZH_TW = {
  "restore.open": "從備份還原…",
  "restore.title": "從備份還原",
  "restore.cancel": "取消",
  "restore.choose.lede":
    "選擇一份在這台電腦或其他電腦上做的 Operational Backup。在你確認最後一步之前，什麼都不會改變。",
  "restore.choose.button": "選擇備份檔…",
  "restore.choose.dialogTitle": "選擇要還原的備份",
  "restore.choose.filterName": "PMC 備份",
  "restore.file": "備份檔：{name}",
  "restore.passphrase.label": "這份備份的密語",
  "restore.passphrase.check": "檢查備份",
  "restore.checking": "正在檢查備份…可能需要一分鐘。",
  "restore.backup": "建立於 {time}，Ledger schema {schema}，{count} 筆紀錄。",
  "restore.recovery.running": "取代任何東西之前，PMC 會先備份目前的工作區。可能需要一分鐘。",
  "restore.preview.lede": "還原會做的事就是下面這些。確認前若有任何變動，這次還原就會作廢。",
  "restore.preview.backup": "備份",
  "restore.preview.now": "目前",
  "restore.now": "{count} 筆紀錄，最後變更於 {time}。",
  "restore.now.noChange": "{count} 筆紀錄，尚無變更。",
  "restore.preview.replaced": "會被取代",
  "restore.replaced":
    "Product Ledger，以及備份中的設定：語言、時區、主題、保留期限、AI 與記錄設定。",
  "restore.preview.kept": "不會被取代",
  "restore.kept": "Product Vault、備份資料夾與密語設定，以及其他所有備份。",
  "restore.preview.recovery": "復原用備份",
  "restore.recovery.done": "目前的工作區已於 {time} 備份並驗證完成。",
  "restore.needsUpgrade": "這份備份來自較早版本的 PMC。還原後，PMC 會先請你升級，才能使用。",
  "restore.confirm.label": "請輸入備份的建立日期以確認：{date}",
  "restore.confirm.button": "以這份備份取代",
  "restore.reject": "不還原",
  "restore.replacing": "正在取代…請勿關閉 PMC。",
  "restore.result.restored": "已還原 {time} 的備份。原本的內容已存成 {recovery} 的備份。",
  "restore.result.unchanged": "還原失敗，沒有任何變更。",
  "restore.result.putBack": "開始取代後還原失敗。PMC 已把原本的工作區放回去。",
  "restore.result.recoveryFailed":
    "還原失敗，而且 PMC 無法把原本的工作區放回去。System Health 會列出要還原的復原用備份。",
  "restore.result.interrupted":
    "還原在完成前停止了。現在不會再有任何變更；PMC 下次啟動時會把原本的工作區放回去。",
  "restore.continue": "繼續",
  "restore.openSystemHealth": "開啟 System Health",

  "policyStrip.restoring": "還原中",
  "backup.strip.restoring": "正在還原。新紀錄與變更要等還原結束後才會受理。",

  "health.headline": "PMC 能否讀取並保存你的紀錄",
  "health.loading": "正在讀取系統狀態。",
  "health.unavailable": "目前無法讀取系統狀態。{message}",
  "health.ledger.label": "Product Ledger",
  "health.ledger.ready": "已開啟，可以讀取。",
  "health.ledger.replacing": "正在由還原取代中。",
  "health.ledger.restore_recovery_required":
    "未開啟。有一次還原中途停止，現在的檔案可能既不是原本的 Ledger，也不是那份備份。",
  "health.ledger.upgrade_required": "尚未開啟。它來自較早版本的 PMC，必須先升級。",
  "health.ledger.open_failed": "無法開啟。",
  "health.recoveryBackup": "從備份資料夾還原復原用備份 {name}，就能回到原本的狀態。",
  "health.quit": "結束 PMC",

  "safeError.desktop.restore_running": "正在還原。請等還原結束後再試一次。",
  "safeError.desktop.ledger_unavailable": "Product Ledger 沒有開啟。原因請看 System Health。",
  "safeError.desktop.restore_file_unreadable": "無法讀取這個備份檔。",
  "safeError.desktop.restore_wrong_passphrase": "這個密語打不開這份備份。",
  "safeError.desktop.restore_damaged": "這份備份已損壞：內容檢查未通過。",
  "safeError.desktop.restore_newer_version":
    "這份備份是由較新版本的 PMC 做的。請先更新 PMC 再還原。",
  "safeError.desktop.restore_unsupported_old": "這份備份來自太舊的 PMC 版本，無法還原。",
  "safeError.desktop.restore_recovery_backup_failed":
    "目前工作區的備份失敗，所以沒有取代任何東西。",
  "safeError.desktop.restore_preview_stale": "這份預覽已經過時。請重新選擇備份。",
  "safeError.desktop.restore_confirmation_mismatch": "輸入的日期不是這份備份的建立日期。",
  "safeError.desktop.restore_failed_unchanged": "還原失敗，沒有任何變更。",
  "safeError.desktop.restore_live_only": "還原只適用於 Live 工作區。",

  "upgrade.title": "升級這個工作區",
  "upgrade.body":
    "PMC {version} 用較新的格式保存你的紀錄。升級只需要一下子。PMC 會先備份你的工作區；任何一步失敗，都不會有任何變更。",
  "upgrade.fact.current": "目前格式",
  "upgrade.fact.new": "新格式",
  "upgrade.schema": "schema {schema}",
  "upgrade.fact.records": "紀錄數",
  "upgrade.fact.lastBackup": "最近一次驗證過的備份",
  "upgrade.fact.noBackup": "沒有",
  "upgrade.backupFirst": "請先備份：設定備份資料夾與密語，再升級。",
  "upgrade.run": "升級",
  "upgrade.backingUp": "正在備份你的工作區…",
  "upgrade.upgrading": "正在升級…",
  "upgrade.result.upgraded": "工作區已升級。{time} 的備份保存了升級前的狀態。",
  "upgrade.result.rolledBack": "升級沒有完成，沒有任何變更。",
  "upgrade.result.unreadable": "工作區已升級，但 PMC 無法開啟它。{time} 的備份保存了升級前的狀態。",
  "upgrade.result.unknown": "PMC 無法判斷升級是否完成。{time} 的備份保存了升級前的工作區。",
  "upgrade.retry": "再試一次",
  "upgrade.blocked.title": "無法開啟這個工作區",
  "upgrade.newer": "這個工作區是由較新版本的 PMC 建立的。請先更新 PMC 再開啟。",
  "upgrade.unsupportedOld": "這個工作區是由開發版的 PMC 建立的，無法升級。",
  "health.ledger.unsupported_old": "未開啟。它是由開發版的 PMC 建立的，無法升級。",
  "health.ledger.newer_version": "未開啟。它是由較新版本的 PMC 建立的。",
  "safeError.desktop.upgrade_live_only": "升級只適用於 Live 工作區。",
  "safeError.desktop.upgrade_not_required": "這個工作區不需要升級。",
  "safeError.desktop.upgrade_backup_failed":
    "升級前的備份失敗，所以沒有開始升級。請檢查備份資料夾後再試一次。",
  "safeError.desktop.upgrade_failed_unchanged": "升級沒有完成，沒有任何變更。",
  "upgrade.trainingReset":
    "這個 Training 工作區來自較早版本的 PMC。它不會升級，請用範例工作區重設。",
  "safeError.desktop.upgrade_outcome_unknown":
    "PMC 無法判斷升級是否完成。升級前剛做的備份保存了原本的工作區。",
  "restore.now.unavailable":
    "PMC 無法開啟目前的 Product Ledger，所以無法得知它的紀錄數與最後變更時間。",
  "restore.recovery.preserved":
    "PMC 已把目前的 Ledger 檔案完整保存為「{name}」並重新核對，逐位元組相符。PMC 無法確認它是可用的 Product Ledger，所以它不是 Operational Backup，也不算你最近一次驗證過的備份。",
  "restore.preserve.running":
    "取代任何東西之前，PMC 會先把目前的 Ledger 檔案完整保存一份並核對。可能需要一分鐘。",
  "restore.result.restoredPreserved": "已還原 {time} 的備份。原本在這裡的檔案保存為「{name}」。",
  "restore.result.putBackPreserved":
    "開始取代後還原失敗。PMC 已把原本的 Ledger 檔案原樣放回。Product Ledger 仍然無法開啟；你可以試另一份備份，或結束 PMC。",
  "health.restore.backupFirst":
    "還原前，PMC 會先把目前的 Ledger 檔案保存一份到備份資料夾。請先設定備份資料夾與復原密語。",
  "restore.choose.recovery": "使用復原用備份「{name}」",
  "restore.choose.other": "選擇其他備份檔…",
  "restore.result.sourceChanged":
    "目前的 Ledger 檔案在預覽之後有變動，所以沒有取代任何東西。請重新開始還原，查看現在會被取代的內容。",
} as const satisfies Record<keyof typeof RESTORE_EN, string>;
