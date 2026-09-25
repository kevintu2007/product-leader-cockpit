//! One window (item ⑩): the second PMC launched for a profile says so in the
//! person's language and stops, before any window, Ledger or settings
//! document is touched. The hold itself is `pmc_platform::instance_lock`.

use pmc_platform::instance_lock::{InstanceLock, InstanceLockError};
use pmc_platform::settings::{peek_locale, ProtectedSettingsRoot};

/// What the second launch says, in the configured language: title and body.
/// `und` (follow the system) and anything unknown fall back to English; the
/// host cannot ask the webview, which does not exist yet.
pub fn already_running_copy(locale: &str) -> (&'static str, &'static str) {
    match locale {
        // "Running or starting": the other launch may not have its window yet.
        "zh-TW" => (
            "Product Mission Control 已經在執行中",
            "另一個 PMC 正在執行或正在啟動。這一次不會再開一個視窗。",
        ),
        "zh-CN" => (
            "Product Mission Control 已经在运行",
            "另一个 PMC 正在运行或正在启动。这一次不会再打开一个窗口。",
        ),
        "ja" => (
            "Product Mission Control はすでに実行中です",
            "別の PMC が実行中か起動中です。このたびは新しいウィンドウを開きません。",
        ),
        "ko" => (
            "Product Mission Control이 이미 실행 중입니다",
            "다른 PMC가 실행 중이거나 시작하는 중입니다. 이번에는 창을 열지 않습니다.",
        ),
        "es" => (
            "Product Mission Control ya se está ejecutando",
            "Otro PMC se está ejecutando o iniciando. Este inicio no abrirá otra ventana.",
        ),
        _ => (
            "Product Mission Control is already running",
            "Another PMC is running or starting. This launch will not open another window.",
        ),
    }
}

/// The locale the settings document already holds; `und` when there is no
/// document yet or it cannot be read. Read as the file is — never through
/// the store, which would initialise a missing document or set an invalid
/// one aside: the second launch writes nothing.
fn configured_locale(root: &ProtectedSettingsRoot) -> String {
    peek_locale(root).unwrap_or_else(|| "und".to_owned())
}

/// Take the profile's hold, or tell the person another PMC has it and return
/// `None` so the caller stops. A lock file that cannot be opened for any
/// other reason is fatal, like the protected root itself.
pub fn claim(root: &ProtectedSettingsRoot) -> Option<InstanceLock> {
    match InstanceLock::acquire(root) {
        Ok(lock) => Some(lock),
        Err(InstanceLockError::AlreadyRunning) => {
            let (title, body) = already_running_copy(&configured_locale(root));
            crate::native_dialogs::show_already_running(title, body);
            None
        }
        Err(InstanceLockError::Io(error)) => {
            panic!("PLATFORM_INSTANCE_LOCK_FAILED: {error}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::already_running_copy;

    #[test]
    fn every_ui_language_has_its_own_copy_and_the_rest_fall_back_to_english() {
        let english = already_running_copy("en");
        for locale in ["zh-TW", "zh-CN", "ja", "ko", "es"] {
            let copy = already_running_copy(locale);
            assert_ne!(copy, english, "{locale}");
            assert!(!copy.0.is_empty() && !copy.1.is_empty(), "{locale}");
        }
        assert_eq!(already_running_copy("und"), english);
        assert_eq!(already_running_copy("fr"), english);
    }
}
