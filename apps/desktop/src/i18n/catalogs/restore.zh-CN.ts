import type { RESTORE_EN } from "./restore.en";

/** Operational Restore 与 S11 System Health 的简体中文文案。 */
export const RESTORE_ZH_CN = {
  "restore.open": "从备份还原…",
  "restore.title": "从备份还原",
  "restore.cancel": "取消",
  "restore.choose.lede":
    "选择一份在这台电脑或其他电脑上做的 Operational Backup。在你确认最后一步之前，什么都不会改变。",
  "restore.choose.button": "选择备份文件…",
  "restore.choose.dialogTitle": "选择要还原的备份",
  "restore.choose.filterName": "PMC 备份",
  "restore.file": "备份文件：{name}",
  "restore.passphrase.label": "这份备份的密码短语",
  "restore.passphrase.check": "检查备份",
  "restore.checking": "正在检查备份…可能需要一分钟。",
  "restore.backup": "创建于 {time}，Ledger schema {schema}，{count} 条记录。",
  "restore.recovery.running": "替换任何内容之前，PMC 会先备份当前的工作区。可能需要一分钟。",
  "restore.preview.lede": "还原要做的就是下面这些。确认前若有任何变动，这次还原就会作废。",
  "restore.preview.backup": "备份",
  "restore.preview.now": "当前",
  "restore.now": "{count} 条记录，最后更改于 {time}。",
  "restore.now.noChange": "{count} 条记录，尚无更改。",
  "restore.preview.replaced": "会被替换",
  "restore.replaced":
    "Product Ledger，以及备份中的设置：语言、时区、主题、保留期限、AI 与日志设置。",
  "restore.preview.kept": "不会被替换",
  "restore.kept": "Product Vault、备份文件夹与密码短语设置，以及其他所有备份。",
  "restore.preview.recovery": "恢复用备份",
  "restore.recovery.done": "当前的工作区已于 {time} 备份并验证完成。",
  "restore.needsUpgrade": "这份备份来自较早版本的 PMC。还原后，PMC 会先请你升级，才能使用。",
  "restore.confirm.label": "请输入备份的创建日期以确认：{date}",
  "restore.confirm.button": "用这份备份替换",
  "restore.reject": "不还原",
  "restore.replacing": "正在替换…请勿关闭 PMC。",
  "restore.result.restored": "已还原 {time} 的备份。原来的内容已存为 {recovery} 的备份。",
  "restore.result.unchanged": "还原失败，没有任何更改。",
  "restore.result.putBack": "开始替换后还原失败。PMC 已把原来的工作区放回去。",
  "restore.result.recoveryFailed":
    "还原失败，而且 PMC 无法把原来的工作区放回去。System Health 会列出要还原的恢复用备份。",
  "restore.result.interrupted":
    "还原在完成前停止了。现在不会再有任何更改；PMC 下次启动时会把原来的工作区放回去。",
  "restore.continue": "继续",
  "restore.openSystemHealth": "打开 System Health",

  "policyStrip.restoring": "还原中",
  "backup.strip.restoring": "正在还原。新记录与更改要等还原结束后才会受理。",

  "health.headline": "PMC 能否读取并保存你的记录",
  "health.loading": "正在读取系统状态。",
  "health.unavailable": "目前无法读取系统状态。{message}",
  "health.ledger.label": "Product Ledger",
  "health.ledger.ready": "已打开，可以读取。",
  "health.ledger.replacing": "正在由还原替换中。",
  "health.ledger.restore_recovery_required":
    "未打开。有一次还原中途停止，现在的文件可能既不是原来的 Ledger，也不是那份备份。",
  "health.ledger.upgrade_required": "尚未打开。它来自较早版本的 PMC，必须先升级。",
  "health.ledger.open_failed": "无法打开。",
  "health.recoveryBackup": "从备份文件夹还原恢复用备份 {name}，就能回到原来的状态。",
  "health.quit": "退出 PMC",

  "safeError.desktop.restore_running": "正在还原。请等还原结束后再试一次。",
  "safeError.desktop.ledger_unavailable": "Product Ledger 没有打开。原因请看 System Health。",
  "safeError.desktop.restore_file_unreadable": "无法读取这个备份文件。",
  "safeError.desktop.restore_wrong_passphrase": "这个密码短语打不开这份备份。",
  "safeError.desktop.restore_damaged": "这份备份已损坏：内容检查未通过。",
  "safeError.desktop.restore_newer_version":
    "这份备份是由较新版本的 PMC 做的。请先更新 PMC 再还原。",
  "safeError.desktop.restore_unsupported_old": "这份备份来自太旧的 PMC 版本，无法还原。",
  "safeError.desktop.restore_recovery_backup_failed":
    "当前工作区的备份失败，所以没有替换任何内容。",
  "safeError.desktop.restore_preview_stale": "这份预览已经过时。请重新选择备份。",
  "safeError.desktop.restore_confirmation_mismatch": "输入的日期不是这份备份的创建日期。",
  "safeError.desktop.restore_failed_unchanged": "还原失败，没有任何更改。",
  "safeError.desktop.restore_live_only": "还原只适用于 Live 工作区。",

  "upgrade.title": "升级这个工作区",
  "upgrade.body":
    "PMC {version} 用较新的格式保存你的记录。升级只需要一会儿。PMC 会先备份你的工作区；任何一步失败，都不会有任何更改。",
  "upgrade.fact.current": "当前格式",
  "upgrade.fact.new": "新格式",
  "upgrade.schema": "schema {schema}",
  "upgrade.fact.records": "记录数",
  "upgrade.fact.lastBackup": "最近一次验证过的备份",
  "upgrade.fact.noBackup": "没有",
  "upgrade.backupFirst": "请先备份：设置备份文件夹与密码短语，再升级。",
  "upgrade.run": "升级",
  "upgrade.backingUp": "正在备份你的工作区…",
  "upgrade.upgrading": "正在升级…",
  "upgrade.result.upgraded": "工作区已升级。{time} 的备份保存了升级前的状态。",
  "upgrade.result.rolledBack": "升级没有完成，没有任何更改。",
  "upgrade.result.unreadable": "工作区已升级，但 PMC 无法打开它。{time} 的备份保存了升级前的状态。",
  "upgrade.result.unknown": "PMC 无法判断升级是否完成。{time} 的备份保存了升级前的工作区。",
  "upgrade.retry": "再试一次",
  "upgrade.blocked.title": "无法打开这个工作区",
  "upgrade.newer": "这个工作区是由较新版本的 PMC 创建的。请先更新 PMC 再打开。",
  "upgrade.unsupportedOld": "这个工作区是由开发版的 PMC 创建的，无法升级。",
  "health.ledger.unsupported_old": "未打开。它是由开发版的 PMC 创建的，无法升级。",
  "health.ledger.newer_version": "未打开。它是由较新版本的 PMC 创建的。",
  "safeError.desktop.upgrade_live_only": "升级只适用于 Live 工作区。",
  "safeError.desktop.upgrade_not_required": "这个工作区不需要升级。",
  "safeError.desktop.upgrade_backup_failed":
    "升级前的备份失败，所以没有开始升级。请检查备份文件夹后再试一次。",
  "safeError.desktop.upgrade_failed_unchanged": "升级没有完成，没有任何更改。",
  "upgrade.trainingReset":
    "这个 Training 工作区来自较早版本的 PMC。它不会升级，请用示例工作区重置。",
  "safeError.desktop.upgrade_outcome_unknown":
    "PMC 无法判断升级是否完成。升级前刚做的备份保存了原来的工作区。",
  "restore.now.unavailable":
    "PMC 无法打开当前的 Product Ledger，所以无法得知它的记录数与最后变更时间。",
  "restore.recovery.preserved":
    "PMC 已把当前的 Ledger 文件完整保存为「{name}」并重新核对，逐字节相符。PMC 无法确认它是可用的 Product Ledger，所以它不是 Operational Backup，也不算你最近一次验证过的备份。",
  "restore.preserve.running":
    "替换任何内容之前，PMC 会先把当前的 Ledger 文件完整保存一份并核对。可能需要一分钟。",
  "restore.result.restoredPreserved": "已还原 {time} 的备份。原本在这里的文件保存为「{name}」。",
  "restore.result.putBackPreserved":
    "开始替换后还原失败。PMC 已把原本的 Ledger 文件原样放回。Product Ledger 仍然无法打开；你可以试另一份备份，或退出 PMC。",
  "health.restore.backupFirst":
    "还原前，PMC 会先把当前的 Ledger 文件保存一份到备份文件夹。请先设置备份文件夹与恢复密语。",
  "restore.choose.recovery": "使用恢复用备份「{name}」",
  "restore.choose.other": "选择其他备份文件…",
  "restore.result.sourceChanged":
    "当前的 Ledger 文件在预览之后有变动，所以没有替换任何内容。请重新开始还原，查看现在会被替换的内容。",
} as const satisfies Record<keyof typeof RESTORE_EN, string>;
